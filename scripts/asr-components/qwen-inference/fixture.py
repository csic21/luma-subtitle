"""Explicit-request-only bounded Qwen inference proof. No ordinary PR invocation.

Call only after the relocated private runtime has passed its ordinary checks.
No downloads occur until fresh available-RAM and free-disk measurements pass.
"""
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import time
import urllib.request
from urllib.parse import urlsplit

GIB = 1024 ** 3
MIN_AVAILABLE_RAM = 13 * GIB
MIN_FREE_DISK = 12 * GIB
MODEL_BYTES = 3_720_689_099
PINS = {
    'qwen3-asr-0-6b': '5eb144179a02acc5e5ba31e748d22b0cf3e303b0',
    'qwen3-forced-aligner-0-6b': 'c7cbfc2048c462b0d63a45797104fc9db3ad62b7',
}
CAPACITY_CODE = '''import json, pathlib, shutil, sys
assert sys.flags.isolated
root = pathlib.Path(sys.prefix).resolve()
def guard(event, args):
    if event in ('socket.connect','socket.getaddrinfo','socket.bind','subprocess.Popen','os.system','os.posix_spawn','os.fork'):
        raise RuntimeError('Capacity measurement must stay offline')
sys.addaudithook(guard)
import psutil
assert pathlib.Path(psutil.__file__).resolve().is_relative_to(root)
assert psutil.__version__ == '7.2.2'
ram = psutil.virtual_memory()
print(json.dumps({'available_ram_bytes': ram.available, 'total_ram_bytes': ram.total,
                  'free_disk_bytes': shutil.disk_usage('.').free, 'psutil_version': psutil.__version__}))
'''


def resource_gate(measurement):
    names = ('available_ram_bytes', 'total_ram_bytes', 'free_disk_bytes')
    if any(type(measurement.get(k)) is not int or measurement[k] < 0 for k in names):
        return False, 'Native available memory/disk measurement is invalid.'
    if measurement['available_ram_bytes'] > measurement['total_ram_bytes']:
        return False, 'Native available memory exceeds physical memory.'
    if measurement['available_ram_bytes'] < MIN_AVAILABLE_RAM:
        return False, 'Less than 13 GiB available physical RAM; no Qwen model files downloaded.'
    if measurement['free_disk_bytes'] < MIN_FREE_DISK:
        return False, 'Less than 12 GiB free disk; no Qwen model files downloaded.'
    return True, None


def allowed_download_url(url, huggingface):
    parsed = urlsplit(url)
    if (parsed.scheme != 'https' or parsed.username or parsed.password or parsed.fragment
            or parsed.port not in (None, 443)):
        return False
    if not huggingface:
        return parsed.hostname == 'raw.githubusercontent.com'
    return (parsed.hostname in {'huggingface.co', 'cdn-lfs.huggingface.co', 'cdn-lfs.hf.co',
                                'cdn-lfs-us-1.hf.co', 'cdn-lfs-eu-1.hf.co'}
            or bool(parsed.hostname and parsed.hostname.endswith('.xethub.hf.co')))


class FixtureRedirects(urllib.request.HTTPRedirectHandler):
    max_repeats = 2
    max_redirections = 8

    def __init__(self, huggingface):
        self.huggingface = huggingface

    def redirect_request(self, req, fp, code, msg, headers, newurl):
        if not allowed_download_url(newurl, self.huggingface):
            raise ValueError('Fixture redirected outside the explicit official download hosts')
        return super().redirect_request(req, fp, code, msg, headers, newurl)


def download(item, destination, deadline):
    """Exact pinned bytes, a total deadline and a bounded blocking read."""
    destination.parent.mkdir(parents=True, exist_ok=True)
    digest = hashlib.sha256(); total = 0
    if time.monotonic() >= deadline:
        raise TimeoutError('Qwen fixture download exceeded its ten-minute budget')
    huggingface = urlsplit(item['url']).hostname == 'huggingface.co'
    if not allowed_download_url(item['url'], huggingface):
        raise ValueError('Unreviewed fixture download host')
    # No inherited proxy credentials or request authentication. All targets are
    # public immutable files; only reviewed official CDN redirects are allowed.
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}), FixtureRedirects(huggingface))
    with opener.open(item['url'], timeout=min(30, max(0.01, deadline - time.monotonic()))) as response, destination.open('xb') as output:
        if response.status != 200:
            raise ValueError('Fixture download did not return a complete public file')
        while True:
            if time.monotonic() >= deadline:
                raise TimeoutError('Qwen fixture download exceeded its ten-minute budget')
            chunk = response.read(1024 * 1024)
            if not chunk: break
            total += len(chunk)
            if total > item['bytes']: raise ValueError('Fixture exceeds exact approved byte count')
            digest.update(chunk); output.write(chunk)
    if total != item['bytes'] or digest.hexdigest() != item['sha256']:
        raise ValueError('Fixture does not match its approved byte count and digest')


def reviewed_models(catalog):
    models = []
    for name, revision in PINS.items():
        matches = [m for m in catalog['models'] if m['id'] == name]
        if len(matches) != 1 or matches[0]['version'] != revision:
            raise ValueError('Only the reviewed 0.6B ASR and aligner pair is authorized')
        model = matches[0]
        if len(model['files']) != 9 or len({f['path'] for f in model['files']}) != 9:
            raise ValueError('Reviewed model file set changed')
        prefix = 'https://huggingface.co/Qwen/' + ('Qwen3-ASR-0.6B' if model['role'] == 'model' else 'Qwen3-ForcedAligner-0.6B') + '/resolve/' + revision + '/'
        for item in model['files']:
            path = item['path']
            if not path or Path(path).name != path or '/' in path or '\\' in path or ':' in path or path in ('.', '..'):
                raise ValueError('Unexpected model fixture layout')
            if item['url'] != prefix + path or type(item['bytes']) is not int or item['bytes'] <= 0:
                raise ValueError('Unexpected model fixture source')
            if len(item['sha256']) != 64 or any(c not in '0123456789abcdef' for c in item['sha256']):
                raise ValueError('Invalid model fixture digest')
        models.append(model)
    if sum(f['bytes'] for m in models for f in m['files']) != MODEL_BYTES:
        raise ValueError('Approved model pair size changed')
    return models


def reviewed_audio(audio):
    if (audio['url'] != 'https://raw.githubusercontent.com/ggml-org/whisper.cpp/d1be6fde11ac6e0407606b4e42fe72d34add8037/samples/jfk.wav'
            or audio['sha256'] != '59dfb9a4acb36fe2a2affc14bacbee2920ff435cb13cc314a08c13f66ba7860e' or audio['bytes'] != 352078):
        raise ValueError('Unreviewed audio fixture')


def fetch_pair(models, audio, root):
    reviewed_models({'models': models}); reviewed_audio(audio)
    deadline = time.monotonic() + 600
    for model in models:
        directory = root / model['id']; directory.mkdir()
        for item in model['files']:
            download(item, directory / item['path'], deadline)
    download(audio, root / 'jfk.wav', deadline)


def download_child(request_path, env, work):
    # A single owned child provides a hard aggregate deadline even if a socket
    # read is blocked. subprocess.run kills and reaps on timeout; no detached
    # downloads can survive the proof or begin inference after a timed-out fetch.
    result = subprocess.run([sys.executable, '-I', '-B', '-u', '-X', 'utf8', str(Path(__file__).resolve()),
                             '--download', str(request_path)], stdin=subprocess.DEVNULL,
                            capture_output=True, text=True, encoding='utf-8', env=env, cwd=work,
                            timeout=600, check=True)
    if json.loads(result.stdout) != {'downloaded_files': 19, 'model_bytes': MODEL_BYTES}:
        raise ValueError('Bounded fixture downloader returned unexpected evidence')


def bounded_worker_requests(executable, worker, requests, env, cwd, timeout=900, max_output_bytes=2*1024*1024):
    data = ''.join(json.dumps(item) + '\n' for item in requests).encode('utf-8')
    if len(data) > 1024*1024:
        raise ValueError('Bounded worker input exceeds its fixed limit')
    with tempfile.TemporaryFile(dir=cwd) as stdin, tempfile.TemporaryFile(dir=cwd) as stdout, tempfile.TemporaryFile(dir=cwd) as stderr:
        stdin.write(data); stdin.seek(0)
        child = subprocess.Popen([str(executable), '-I', '-B', '-u', '-X', 'utf8', str(worker)],
                                 stdin=stdin, stdout=stdout, stderr=stderr, env=env, cwd=cwd)
        started = time.monotonic()
        try:
            while child.poll() is None:
                if time.monotonic() - started > timeout:
                    raise TimeoutError('Bounded Qwen worker exceeded its inference deadline')
                if os.fstat(stdout.fileno()).st_size + os.fstat(stderr.fileno()).st_size > max_output_bytes:
                    raise ValueError('Bounded Qwen worker exceeded its output limit')
                time.sleep(0.05)
            child.wait(); stdout.seek(0); stderr.seek(0)
            out, err = stdout.read(max_output_bytes+1), stderr.read(max_output_bytes+1)
            if len(out)+len(err) > max_output_bytes:
                raise ValueError('Bounded Qwen worker exceeded its output limit')
            if child.returncode:
                raise RuntimeError(f'Bounded Qwen worker failed ({child.returncode}): ' + err[-4000:].decode('utf-8', 'replace'))
            frames = [json.loads(line) for line in out.decode('utf-8').splitlines() if line]
            if not frames: raise ValueError('Bounded Qwen worker emitted no protocol frames')
            return frames
        except BaseException:
            if child.poll() is None: child.kill()
            child.wait()
            raise


def run(executable, worker, env, work, catalog, audio, worker_requests, report=None):
    if report is None: report = {}
    report.update(tested=False, backend='qwen3-asr', device='cpu',
                  minimum_available_ram_bytes=MIN_AVAILABLE_RAM, minimum_free_disk_bytes=MIN_FREE_DISK,
                  weights_downloaded=False, cold_only=True)
    try:
        child = subprocess.run([str(executable), '-I', '-B', '-u', '-X', 'utf8', '-c', CAPACITY_CODE],
                               capture_output=True, text=True, encoding='utf-8', env=env, cwd=work, timeout=30, check=True)
        measurement = json.loads(child.stdout)
    except (subprocess.SubprocessError, ValueError) as exc:
        report['reason'] = 'Native resource measurement unavailable; no model files downloaded: ' + str(exc)[:500]
        return report
    report['resources_before_download'] = measurement
    allowed, reason = resource_gate(measurement)
    if not allowed:
        report['reason'] = reason
        return report
    models = reviewed_models(catalog)
    reviewed_audio(audio)
    # This private throwaway directory is the sole fixture cache. Never use the
    # shared runtime download cache, upload it, or retain weights between jobs.
    root = None
    try:
        with tempfile.TemporaryDirectory(prefix='qwen-bounded-fixture-', dir=work) as tmp:
            root = Path(tmp)
            request_path = root / 'download-request.json'
            request_path.write_text(json.dumps({'models': models, 'audio': audio, 'root': str(root)}), encoding='utf-8')
            download_child(request_path, env, work)
            paths = {model['role']: root / model['id'] for model in models}
            wav = root / 'jfk.wav'
            report['weights_downloaded'] = True
            request = {'id': 'bounded-qwen-cold', 'op': 'transcribe', 'engine': 'qwen3-asr', 'device': 'cpu',
                       'model_path': str(paths['model']), 'aligner_path': str(paths['aligner']),
                       'audio_path': str(wav), 'language': 'en'}
            started = time.monotonic()
            frames = worker_requests(executable, worker, [request], env, work, timeout=900)
            terminal = [f for f in frames if f.get('id') == request['id'] and f.get('event') in ('result', 'error')]
            if len(terminal) != 1 or terminal[0].get('event') != 'result' or terminal[0].get('device') != 'cpu':
                raise AssertionError('Bounded Qwen inference failed: ' + repr(terminal)[:2000])
            result = terminal[0]
            if result.get('reused') is not False or 'country' not in ' '.join(s['text'] for s in result['segments']).lower():
                raise AssertionError('Qwen fixture did not produce the expected cold transcription')
            previous = 0
            for segment in result['segments']:
                if not previous <= segment['start_ms'] < segment['end_ms'] <= 11001:
                    raise AssertionError('Qwen fixture has invalid aligned timestamps')
                previous = segment['end_ms']
            report.update(tested=True, elapsed_seconds=time.monotonic() - started,
                          models=PINS, model_download_bytes=MODEL_BYTES, audio_sha256=audio['sha256'],
                          segments=result['segments'], forced_alignment_tested=True)
    finally:
        report['fixture_cache_removed'] = root is None or not root.exists()
    return report


if __name__ == '__main__':
    if len(sys.argv) != 3 or sys.argv[1] != '--download':
        raise SystemExit('Only the bounded internal fixture downloader is supported')
    request_path = Path(sys.argv[2]).resolve()
    if request_path.stat().st_size > 65536:
        raise ValueError('Oversized bounded fixture request')
    request = json.loads(request_path.read_text(encoding='utf-8'))
    root = Path(request['root']).resolve()
    if root != request_path.parent or not root.is_dir():
        raise ValueError('Fixture directory must be the owned request directory')
    fetch_pair(request['models'], request['audio'], root)
    print(json.dumps({'downloaded_files': 19, 'model_bytes': MODEL_BYTES}), flush=True)
