"""Explicit-request-only bounded Qwen inference proof. No ordinary PR invocation.

Full inference requires the relocated private runtime and fresh RAM/disk gates.
The separate README diagnostic reads only its fixed metadata file, never weights.
"""
import hashlib
import io
import json
import os
from pathlib import Path
import re
import socket
import shutil
import ssl
import subprocess
import sys
import tempfile
import time
import urllib.request
import urllib.error
from urllib.parse import urlsplit

GIB = 1024 ** 3
MIN_AVAILABLE_RAM = 13 * GIB
MIN_FREE_DISK = 12 * GIB
MODEL_BYTES = 3_720_689_099
DOWNLOAD_ERROR_PREFIX = 'QWEN_FIXTURE_ERROR='
MAX_DOWNLOAD_ERROR_BYTES = 4096
MAX_DOWNLOAD_OUTPUT_BYTES = 8192
DOWNLOAD_TIMEOUT_SECONDS = 600
README_TIMEOUT_SECONDS = 30
README_BYTES = 57456
README_SHA256 = '5058416891bc47a2051557765997e8c42f8eb78a0e33c3e775bd17d4b0ba4d50'
DOWNLOAD_REAP_TIMEOUT_SECONDS = 5
DOWNLOAD_ERROR_CATEGORIES = frozenset({
    'http-error', 'http-status', 'size-mismatch', 'hash-mismatch', 'download-timeout',
    'dns-error', 'tls-error', 'network-error', 'filesystem-error', 'unexpected-error',
    'unreviewed-source', 'redirect-rejected',
})
PINS = {
    'qwen3-asr-0-6b': '5eb144179a02acc5e5ba31e748d22b0cf3e303b0',
    'qwen3-forced-aligner-0-6b': 'c7cbfc2048c462b0d63a45797104fc9db3ad62b7',
}
README_IDENTITY = {
    'component': 'qwen3-asr-0-6b', 'revision': PINS['qwen3-asr-0-6b'],
    'path': 'README.md', 'expected_bytes': README_BYTES, 'expected_sha256': README_SHA256,
}
README_REQUEST = {**README_IDENTITY, 'max_bytes': README_BYTES, 'deadline_seconds': README_TIMEOUT_SECONDS}
README_ITEM = {
    'path': 'README.md', 'bytes': README_BYTES, 'sha256': README_SHA256,
    'url': 'https://huggingface.co/Qwen/Qwen3-ASR-0.6B/resolve/' + PINS['qwen3-asr-0-6b'] + '/README.md',
}


def request_selector(request):
    selector = request.get('selector')
    if selector not in ('full-inference', 'readme-diagnostic'):
        raise ValueError('An exact supported proof selector is required')
    if selector == 'readme-diagnostic':
        value = request.get('diagnostic')
        if (type(value) is not dict or set(value) != set(README_REQUEST)
                or any(type(value[k]) is not type(v) or value[k] != v for k, v in README_REQUEST.items())):
            raise ValueError('Only the bounded exact README diagnostic is authorized')
    elif 'diagnostic' in request:
        raise ValueError('Full inference cannot include a diagnostic override')
    return selector


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
            raise DownloadCheckError('redirect-rejected')
        return super().redirect_request(req, fp, code, msg, headers, newurl)


class ReadmeRedirects(FixtureRedirects):
    """Retain urllib's redirect policy without consuming redirect bodies."""
    def __init__(self, deadline):
        super().__init__(True)
        self.deadline = deadline

    def redirect_request(self, req, fp, code, msg, headers, newurl):
        if req.get_method() != 'GET':
            raise DownloadCheckError('redirect-rejected')
        checked = super().redirect_request(req, fp, code, msg, headers, newurl)
        # Never forward a supplied Host, Authorization, Cookie, proxy credential,
        # body or other request header to the next approved origin.
        return urllib.request.Request(checked.full_url, method='GET', unverifiable=True)

    def http_error_302(self, req, fp, code, msg, headers):
        # HTTPRedirectHandler normally calls fp.read() with no size limit before
        # following a redirect. Close the real response immediately and retain
        # its URL/loop handling using an empty owned body instead. No redirect
        # payload bytes are consumed, including on rejected targets or loops.
        fp.close()
        remaining = self.deadline - time.monotonic()
        if remaining <= 0:
            raise TimeoutError()
        req.timeout = min(req.timeout, remaining)
        with io.BytesIO() as empty_body:
            return super().http_error_302(req, empty_body, code, msg, headers)

    http_error_301 = http_error_303 = http_error_307 = http_error_308 = http_error_302


class DownloadCheckError(ValueError):
    """A fixed category, never a URL or arbitrary server/exception text."""
    def __init__(self, category):
        self.category = category if type(category) is str and category in DOWNLOAD_ERROR_CATEGORIES else 'unexpected-error'
        super().__init__(self.category)


class FixtureDownloadError(ValueError):
    def __init__(self, evidence):
        self.evidence = evidence
        super().__init__(json.dumps(evidence, ensure_ascii=True, sort_keys=True))


def error_category(exc):
    if isinstance(exc, DownloadCheckError):
        category = exc.category
        return category if category in DOWNLOAD_ERROR_CATEGORIES else 'unexpected-error'
    if isinstance(exc, urllib.error.HTTPError): return 'http-error'
    network_error = isinstance(exc, urllib.error.URLError)
    if network_error: exc = exc.reason
    if isinstance(exc, socket.gaierror): return 'dns-error'
    if isinstance(exc, (TimeoutError, socket.timeout)): return 'download-timeout'
    if isinstance(exc, ssl.SSLError): return 'tls-error'
    if network_error or isinstance(exc, ConnectionError): return 'network-error'
    if isinstance(exc, OSError): return 'filesystem-error'
    return 'unexpected-error'


def download(item, destination, deadline, identity=None):
    """Exact pinned bytes, a total deadline and a bounded blocking read."""
    digest = hashlib.sha256(); total = 0
    status = None
    try:
        destination.parent.mkdir(parents=True, exist_ok=True)
        if time.monotonic() >= deadline: raise TimeoutError()
        huggingface = urlsplit(item['url']).hostname == 'huggingface.co'
        if not allowed_download_url(item['url'], huggingface):
            raise DownloadCheckError('unreviewed-source')
        # No inherited proxy credentials or request authentication. All targets are
        # public immutable files; only reviewed official CDN redirects are allowed.
        opener = urllib.request.build_opener(urllib.request.ProxyHandler({}), FixtureRedirects(huggingface))
        with opener.open(item['url'], timeout=min(30, max(0.01, deadline - time.monotonic()))) as response, destination.open('xb') as output:
            status = response.status
            if status != 200: raise DownloadCheckError('http-status')
            while True:
                if time.monotonic() >= deadline: raise TimeoutError()
                chunk = response.read(1024 * 1024)
                if not chunk: break
                total += len(chunk)
                if total > item['bytes']: raise DownloadCheckError('size-mismatch')
                digest.update(chunk); output.write(chunk)
        if total != item['bytes']: raise DownloadCheckError('size-mismatch')
        if digest.hexdigest() != item['sha256']: raise DownloadCheckError('hash-mismatch')
    except Exception as exc:
        if isinstance(exc, urllib.error.HTTPError): status = exc.code
        if type(status) is not int or not 100 <= status <= 599: status = None
        raise FixtureDownloadError({'schema': 1, 'category': error_category(exc),
                                    'file': identity, 'bytes_received': total,
                                    'http_status': status}) from None



def download_readme(deadline):
    """Read only one hard-coded immutable README, without writing its content.

    Require a known exact HTTP length and identity encoding so no payload read
    exceeds the 57,456-byte cap. Missing/ambiguous length fails closed. A success
    says nothing about the original failure, weight CDN, or model inference.
    """
    digest = hashlib.sha256(); total = 0; status = None
    try:
        if time.monotonic() >= deadline: raise TimeoutError()
        opener = urllib.request.build_opener(urllib.request.ProxyHandler({}), ReadmeRedirects(deadline))
        with opener.open(README_ITEM['url'], timeout=min(30, max(0.01, deadline - time.monotonic()))) as response:
            status = response.status
            if status != 200: raise DownloadCheckError('http-status')
            lengths = response.headers.get_all('Content-Length', [])
            encodings = response.headers.get_all('Content-Encoding', [])
            if (lengths != [str(README_BYTES)] or response.headers.get_all('Transfer-Encoding', [])
                    or encodings not in ([], ['identity'])):
                raise DownloadCheckError('size-mismatch')
            while total < README_BYTES:
                if time.monotonic() >= deadline: raise TimeoutError()
                chunk = response.read(min(8192, README_BYTES - total))
                if not chunk: break
                total += len(chunk); digest.update(chunk)
        if total != README_BYTES: raise DownloadCheckError('size-mismatch')
        if digest.hexdigest() != README_SHA256: raise DownloadCheckError('hash-mismatch')
    except Exception as exc:
        if isinstance(exc, urllib.error.HTTPError): status = exc.code
        if type(status) is not int or not 100 <= status <= 599: status = None
        raise FixtureDownloadError({'schema': 1, 'category': error_category(exc),
                                    'file': dict(README_IDENTITY), 'bytes_received': total,
                                    'http_status': status, 'weights_complete': False,
                                    'transfer_state': 'partial' if total else 'not-observed'}) from None


def readme_child_error(stderr):
    if isinstance(stderr, bytes): stderr = stderr.decode('utf-8', 'strict')
    if (not isinstance(stderr, str) or len(stderr.encode('utf-8')) > MAX_DOWNLOAD_ERROR_BYTES
            or not stderr.startswith(DOWNLOAD_ERROR_PREFIX)):
        raise ValueError('Invalid README diagnostic')
    record = json.loads(stderr[len(DOWNLOAD_ERROR_PREFIX):])
    for category in ('invalid-request', 'oversized-diagnostic'):
        if (type(record) is dict and type(record.get('schema')) is int
                and record == unconfirmed_child_error(category, None)): return record
    expected = {'schema', 'category', 'file', 'bytes_received', 'http_status', 'weights_complete', 'transfer_state'}
    if (type(record) is not dict or set(record) != expected or type(record['schema']) is not int
            or record['schema'] != 1 or record['category'] not in DOWNLOAD_ERROR_CATEGORIES
            or record['file'] != README_IDENTITY or record['weights_complete'] is not False
            or type(record['bytes_received']) is not int or not 0 <= record['bytes_received'] <= README_BYTES
            or record['transfer_state'] != ('partial' if record['bytes_received'] else 'not-observed')
            or (record['http_status'] is not None and (type(record['http_status']) is not int or not 100 <= record['http_status'] <= 599))):
        raise ValueError('Diagnostic differs from the exact README')
    return record


def readme_succeeded(stdout):
    # Fixed byte-for-byte protocol: no extra fields, duplicate keys or text.
    return stdout == (json.dumps({'readme_verified': True, 'metadata_bytes': README_BYTES}) + '\n').encode('utf-8')



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


def fixture_files(models, audio):
    """Identities come only from the pinned request, never a redirect or response."""
    reviewed_models({'models': models}); reviewed_audio(audio)
    entries = []
    for model in models:
        for item in model['files']:
            identity = {'component': model['id'], 'revision': model['version'], 'path': item['path'],
                        'expected_bytes': item['bytes'], 'expected_sha256': item['sha256']}
            entries.append((item, Path(model['id']) / item['path'], identity))
    entries.append((audio, Path('jfk.wav'), {
        'component': 'ggml-org/whisper.cpp', 'revision': 'd1be6fde11ac6e0407606b4e42fe72d34add8037',
        'path': 'samples/jfk.wav', 'expected_bytes': audio['bytes'], 'expected_sha256': audio['sha256']}))
    return entries


def fetch_pair(models, audio, root):
    entries = fixture_files(models, audio)
    deadline = time.monotonic() + 600
    completed_bytes = 0
    for index, (item, path, identity) in enumerate(entries):
        try:
            download(item, root / path, deadline, identity)
        except FixtureDownloadError as exc:
            exc.evidence.update(files_completed=index, completed_bytes=completed_bytes,
                                weights_complete=index == 18,
                                # Count only returned payload chunks, not unknown
                                # socket buffering or HTTP error response bodies.
                                transfer_state='partial' if completed_bytes or exc.evidence['bytes_received'] else 'not-observed')
            raise FixtureDownloadError(exc.evidence) from None
        completed_bytes += item['bytes']


def utf8_tail(value, limit):
    """Bound bytes without emitting a split UTF-8 code point."""
    if limit <= 0: return ''
    data = value if isinstance(value, bytes) else value.encode('utf-8', 'replace')
    return data[-limit:].decode('utf-8', 'ignore')


def startup_error_tail(stderr):
    # Do not attempt to guess which arbitrary text is a credential. Only retain
    # known exception names; all messages, paths, headers and environment are omitted.
    tail = utf8_tail(stderr or '', MAX_DOWNLOAD_ERROR_BYTES)
    names = re.findall(r'\b(SyntaxError|IndentationError|ModuleNotFoundError|ImportError|'
                       r'PermissionError|FileNotFoundError|OSError|RuntimeError):', tail)
    return utf8_tail('\n'.join(name + ': [details redacted]' for name in names[-4:])
                     or '[unrecognized child stderr omitted]', 512)


def child_error(stderr, request_path):
    """Validate every emitted field against the reviewed request before reporting."""
    if isinstance(stderr, bytes): stderr = stderr.decode('utf-8', 'strict')
    if not isinstance(stderr, str) or len(stderr.encode('utf-8')) > MAX_DOWNLOAD_ERROR_BYTES:
        raise ValueError('Invalid diagnostic size')
    if not stderr.startswith(DOWNLOAD_ERROR_PREFIX): raise ValueError('Missing diagnostic marker')
    record = json.loads(stderr[len(DOWNLOAD_ERROR_PREFIX):])
    for category in ('invalid-request', 'oversized-diagnostic'):
        if (type(record) is dict and type(record.get('schema')) is int
                and record == unconfirmed_child_error(category, None)): return record
    common = {'schema', 'category', 'file', 'bytes_received', 'http_status'}
    complete = common | {'files_completed', 'completed_bytes', 'weights_complete', 'transfer_state'}
    if (not isinstance(record, dict) or set(record) != complete or type(record['schema']) is not int
            or record['schema'] != 1 or record['category'] not in DOWNLOAD_ERROR_CATEGORIES):
        raise ValueError('Invalid diagnostic schema')
    if request_path.stat().st_size > 65536: raise ValueError('Oversized request')
    request = json.loads(request_path.read_text(encoding='utf-8'))
    entries = fixture_files(request['models'], request['audio'])
    index = record['files_completed']
    if type(index) is not int or not 0 <= index < len(entries): raise ValueError('Invalid file count')
    expected = entries[index][2]
    received = record['bytes_received']
    if (record['file'] != expected or type(received) is not int or not 0 <= received <= expected['expected_bytes'] + 1024*1024
            or type(record['completed_bytes']) is not int
            or record['completed_bytes'] != sum(e[0]['bytes'] for e in entries[:index])
            or record['weights_complete'] is not (index == 18)
            or record['transfer_state'] != ('partial' if record['completed_bytes'] or received else 'not-observed')
            or (record['http_status'] is not None and (type(record['http_status']) is not int or not 100 <= record['http_status'] <= 599))):
        raise ValueError('Diagnostic differs from pinned request')
    return record


def unconfirmed_child_error(category, stderr, returncode=None):
    # No current-file assertion can be recovered after an abrupt child timeout or
    # startup failure. Unknown transfer state must never be described as zero bytes.
    return {'schema': 1, 'category': category, 'file': None, 'transfer_state': 'unknown',
            'weights_complete': None, 'child_exit_code': returncode,
            'stderr_tail': startup_error_tail(stderr)}


def download_succeeded(stdout):
    """One exact completion object, with integer counts and no duplicate keys."""
    def unique_object(pairs):
        value = {}
        for key, item in pairs:
            if key in value: raise ValueError('Duplicate completion key')
            value[key] = item
        return value
    if len(stdout) > 256: return False
    try:
        value = json.loads(stdout.decode('utf-8'), object_pairs_hook=unique_object)
        return (type(value) is dict and set(value) == {'downloaded_files', 'model_bytes'}
                and type(value['downloaded_files']) is int and value['downloaded_files'] == 19
                and type(value['model_bytes']) is int and value['model_bytes'] == MODEL_BYTES)
    except (ValueError, TypeError):
        return False


def download_child(request_path, env, work, selector='full-inference'):
    if selector not in ('full-inference', 'readme-diagnostic'):
        raise ValueError('Unsupported download selector')
    readme_only = selector == 'readme-diagnostic'
    timeout = README_TIMEOUT_SECONDS if readme_only else DOWNLOAD_TIMEOUT_SECONDS
    # A single owned child provides a hard aggregate deadline even if a socket
    # read is blocked. As in bounded_worker_requests, file-backed streams are
    # polled while live, then read within the cap; every failure kills and reaps.
    child = None
    reap_confirmed = False
    try:
        with tempfile.TemporaryFile(dir=work) as stdout, tempfile.TemporaryFile(dir=work) as stderr:
            started = time.monotonic()
            child = subprocess.Popen([sys.executable, '-I', '-B', '-u', '-X', 'utf8', str(Path(__file__).resolve()),
                                      '--diagnose-readme' if readme_only else '--download', str(request_path)], stdin=subprocess.DEVNULL,
                                     stdout=stdout, stderr=stderr, env=env, cwd=work)
            try:
                while child.poll() is None:
                    if time.monotonic() - started > timeout:
                        raise FixtureDownloadError(unconfirmed_child_error('aggregate-timeout', None))
                    if os.fstat(stdout.fileno()).st_size + os.fstat(stderr.fileno()).st_size > MAX_DOWNLOAD_OUTPUT_BYTES:
                        raise FixtureDownloadError(unconfirmed_child_error('child-output-limit', None))
                    time.sleep(0.05)
                child.wait(timeout=DOWNLOAD_REAP_TIMEOUT_SECONDS)
                reap_confirmed = True
                if time.monotonic() - started > timeout:
                    raise FixtureDownloadError(unconfirmed_child_error('aggregate-timeout', None))
                if os.fstat(stdout.fileno()).st_size + os.fstat(stderr.fileno()).st_size > MAX_DOWNLOAD_OUTPUT_BYTES:
                    raise FixtureDownloadError(unconfirmed_child_error('child-output-limit', None))
                stdout.seek(0); stderr.seek(0)
                out, err = stdout.read(MAX_DOWNLOAD_OUTPUT_BYTES+1), stderr.read(MAX_DOWNLOAD_OUTPUT_BYTES+1)
                if len(out) + len(err) > MAX_DOWNLOAD_OUTPUT_BYTES:
                    raise FixtureDownloadError(unconfirmed_child_error('child-output-limit', None))
                if child.returncode:
                    try:
                        evidence = readme_child_error(err) if readme_only else child_error(err, request_path)
                    except (ValueError, TypeError, KeyError, OSError):
                        evidence = unconfirmed_child_error('child-startup-or-protocol-error', err, child.returncode)
                    raise FixtureDownloadError(evidence)
                if not (readme_succeeded(out) if readme_only else download_succeeded(out)):
                    raise FixtureDownloadError(unconfirmed_child_error('child-protocol-error', err))
            except BaseException as exc:
                kill_failed = False
                try:
                    # Do not depend on a status query that may itself have failed.
                    # Popen.kill() also handles an already-terminated child.
                    child.kill()
                except OSError:
                    kill_failed = True
                finally:
                    # A poll/kill error must not skip waiting for our owned child.
                    try:
                        child.wait(timeout=DOWNLOAD_REAP_TIMEOUT_SECONDS)
                        reap_confirmed = True
                    except (OSError, subprocess.TimeoutExpired):
                        evidence = unconfirmed_child_error('child-reap-unconfirmed', None)
                        evidence['child_reap_confirmed'] = False
                        raise FixtureDownloadError(evidence) from None
                if kill_failed:
                    evidence = unconfirmed_child_error('child-process-error', None)
                    evidence['child_reap_confirmed'] = reap_confirmed
                    raise FixtureDownloadError(evidence) from None
                if isinstance(exc, FixtureDownloadError):
                    exc.evidence['child_reap_confirmed'] = reap_confirmed
                    raise FixtureDownloadError(exc.evidence) from None
                raise
    except OSError:
        category = 'child-launch-error' if child is None else 'child-process-error'
        evidence = unconfirmed_child_error(category, None)
        evidence['child_reap_confirmed'] = None if child is None else reap_confirmed
        raise FixtureDownloadError(evidence) from None


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
    root = None; preserve = False
    try:
        root = Path(tempfile.mkdtemp(prefix='qwen-bounded-fixture-', dir=work))
        request_path = root / 'download-request.json'
        request_path.write_text(json.dumps({'models': models, 'audio': audio, 'root': str(root)}), encoding='utf-8')
        report['download_attempted'] = True
        try:
            download_child(request_path, env, work)
        except FixtureDownloadError as exc:
            report['download_failure'] = exc.evidence
            preserve = exc.evidence.get('child_reap_confirmed') is False
            if preserve: report['fixture_cleanup_blocked'] = 'Owned downloader child reap is unconfirmed'
            # True only for the verified complete pair, even if audio then fails.
            report['weights_downloaded'] = exc.evidence.get('weights_complete') is True
            raise
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
        try:
            if root is not None and not preserve: shutil.rmtree(root)
        finally:
            report['fixture_cache_removed'] = root is None or not root.exists()
    return report



def run_readme_diagnostic(env, work, catalog, report=None):
    if report is None: report = {}
    models = reviewed_models(catalog)
    if models[0]['files'][0] != README_ITEM:
        raise ValueError('The first pinned downloader source is no longer the exact README')
    report.update(verified=False, file=dict(README_IDENTITY), max_bytes=README_BYTES,
                  deadline_seconds=README_TIMEOUT_SECONDS, metadata_only=True,
                  content_uploaded=False, weights_downloaded=False, audio_downloaded=False,
                  inference_tested=False, weight_cdn_tested=False, original_failure_cause_established=False)
    root = None; preserve = False
    try:
        root = Path(tempfile.mkdtemp(prefix='qwen-readme-diagnostic-', dir=work))
        request_path = root / 'readme-request.json'
        request_path.write_text(json.dumps({'root': str(root)}), encoding='utf-8')
        report['download_attempted'] = True
        try:
            download_child(request_path, env, work, selector='readme-diagnostic')
        except FixtureDownloadError as exc:
            report['download_failure'] = exc.evidence
            preserve = exc.evidence.get('child_reap_confirmed') is False
            if preserve: report['fixture_cleanup_blocked'] = 'Owned downloader child reap is unconfirmed'
            raise
        report.update(verified=True, bytes_received=README_BYTES, http_status=200,
                      scope='exact-pinned-readme-only')
    finally:
        try:
            if root is not None and not preserve: shutil.rmtree(root)
        finally:
            report['fixture_cache_removed'] = root is None or not root.exists()
    return report


def readme_download_main(request_path):
    try:
        request_path = request_path.resolve()
        if request_path.stat().st_size > 65536: raise ValueError('Oversized request')
        request = json.loads(request_path.read_text(encoding='utf-8'))
        if type(request) is not dict or set(request) != {'root'}: raise ValueError('Invalid README request')
        root = Path(request['root']).resolve()
        if root != request_path.parent or not root.is_dir(): raise ValueError('Invalid owned directory')
        download_readme(time.monotonic() + README_TIMEOUT_SECONDS)
    except Exception as exc:
        evidence = exc.evidence if isinstance(exc, FixtureDownloadError) else unconfirmed_child_error('invalid-request', None)
        message = DOWNLOAD_ERROR_PREFIX + json.dumps(evidence, ensure_ascii=True, sort_keys=True) + '\n'
        if len(message.encode('utf-8')) > MAX_DOWNLOAD_ERROR_BYTES:
            message = DOWNLOAD_ERROR_PREFIX + json.dumps(unconfirmed_child_error('oversized-diagnostic', None)) + '\n'
        print(message, end='', file=sys.stderr, flush=True)
        return 1
    print(json.dumps({'readme_verified': True, 'metadata_bytes': README_BYTES}), flush=True)
    return 0



def download_main(request_path):
    try:
        request_path = request_path.resolve()
        if request_path.stat().st_size > 65536: raise ValueError('Oversized request')
        request = json.loads(request_path.read_text(encoding='utf-8'))
        root = Path(request['root']).resolve()
        if root != request_path.parent or not root.is_dir(): raise ValueError('Invalid owned directory')
        fetch_pair(request['models'], request['audio'], root)
    except Exception as exc:
        evidence = exc.evidence if isinstance(exc, FixtureDownloadError) else unconfirmed_child_error('invalid-request', None)
        message = DOWNLOAD_ERROR_PREFIX + json.dumps(evidence, ensure_ascii=True, sort_keys=True) + '\n'
        if len(message.encode('utf-8')) > MAX_DOWNLOAD_ERROR_BYTES:
            message = DOWNLOAD_ERROR_PREFIX + json.dumps(unconfirmed_child_error('oversized-diagnostic', None)) + '\n'
        print(message, end='', file=sys.stderr, flush=True)
        return 1
    print(json.dumps({'downloaded_files': 19, 'model_bytes': MODEL_BYTES}), flush=True)
    return 0


if __name__ == '__main__':
    if len(sys.argv) != 3 or sys.argv[1] not in ('--download', '--diagnose-readme'):
        raise SystemExit('Only the bounded internal fixture downloader is supported')
    raise SystemExit((readme_download_main if sys.argv[1] == '--diagnose-readme' else download_main)(Path(sys.argv[2])))
