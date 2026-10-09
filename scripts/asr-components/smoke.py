#!/usr/bin/env python3
"""Test extracted pack using only its private interpreter in a poisoned env."""
from __future__ import annotations
import argparse
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import zipfile
from build import ROOT, ALLOWED, dump, embed_nagisa, fetch, safe_name, sha256


def diagnostic_json(value, **kwargs):
    # The build host's redirected Windows stdout can still be cp1252. Keep log
    # transport ASCII-safe; the proof files themselves retain exact UTF-8 text.
    return json.dumps(value, ensure_ascii=True, **kwargs)


def extract(archive, destination):
    destination.mkdir(parents=True)
    with zipfile.ZipFile(archive) as z:
        names = set()
        for entry in z.infolist():
            path = safe_name(entry.filename)
            if entry.filename in names or entry.is_dir() or (entry.external_attr >> 16) & 0o170000 != 0o100000:
                raise ValueError('Archive must contain distinct regular files only')
            names.add(entry.filename)
            target = destination.joinpath(*path.parts)
            target.parent.mkdir(parents=True, exist_ok=True)
            with z.open(entry) as f, target.open('wb') as out:
                shutil.copyfileobj(f, out)
            target.chmod((entry.external_attr >> 16) & 0o777)


def clean_environment(home):
    home.mkdir(parents=True)
    keep = {'SYSTEMROOT', 'WINDIR', 'COMSPEC', 'TEMP', 'TMP', 'SYSTEMDRIVE',
            'PROCESSOR_ARCHITECTURE', 'PROCESSOR_ARCHITEW6432', 'PROCESSOR_IDENTIFIER',
            'PROCESSOR_LEVEL', 'PROCESSOR_REVISION', 'NUMBER_OF_PROCESSORS'}
    env = {k: v for k, v in os.environ.items() if k.upper() in keep}
    env.update({'HOME': str(home), 'USERPROFILE': str(home), 'APPDATA': str(home / 'AppData'),
                'LOCALAPPDATA': str(home / 'Local'), 'PATH': str(home / 'no-executables'),
                'PYTHONPATH': str(home / 'poison'), 'PYTHONHOME': str(home / 'not-python'),
                'PYTHONNOUSERSITE': '1', 'HF_HOME': str(home / 'empty-hf-cache'),
                'HF_HUB_OFFLINE': '1', 'TRANSFORMERS_OFFLINE': '1', 'HF_HUB_DISABLE_TELEMETRY': '1',
                'HF_HUB_DISABLE_IMPLICIT_TOKEN': '1', 'DO_NOT_TRACK': '1', 'LANG': 'C.UTF-8',
                'NUMBA_CACHE_DIR': str(home / 'numba-cache')})
    poison = home / 'poison'; poison.mkdir()
    (poison / 'sitecustomize.py').write_text("raise RuntimeError('inherited Python path leaked into component')\n")
    return env


def worker_requests(executable, worker, requests, env, cwd, timeout=600):
    proc = subprocess.run([str(executable), '-I', '-B', '-u', '-X', 'utf8', str(worker)],
                          input=''.join(json.dumps(r) + '\n' for r in requests), text=True,
                          encoding='utf-8', capture_output=True, env=env, cwd=cwd, timeout=timeout)
    if proc.returncode:
        raise RuntimeError(f'Private worker failed ({proc.returncode}): {proc.stderr[-12000:]}')
    try:
        frames = [json.loads(line) for line in proc.stdout.splitlines() if line]
    except ValueError as exc:
        raise AssertionError('Worker stdout was not clean JSON lines') from exc
    if not frames:
        raise AssertionError('Worker emitted no protocol response')
    return frames


def terminate_idle_worker(command, env, cwd):
    idle = subprocess.Popen(command, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                            stderr=subprocess.PIPE, env=env, cwd=cwd)
    try:
        # communicate() would close stdin and turn this into an EOF test.
        idle.wait(timeout=0.2)
        idle.communicate(timeout=15)
        raise AssertionError('Idle worker exited without EOF or termination')
    except subprocess.TimeoutExpired:
        idle.kill(); idle.communicate(timeout=15)
        assert idle.returncode is not None


def nagisa_probe(executable, probe, env, cwd, *flags):
    result = subprocess.run([str(executable), '-I', '-B', '-u', '-X', 'utf8', str(probe), *flags],
                            capture_output=True, text=True, encoding='utf-8', env=env, cwd=cwd, timeout=180)
    if result.returncode:
        raise RuntimeError(f'Native Nagisa probe failed ({result.returncode}):\n{result.stdout}\n{result.stderr}')
    return json.loads(result.stdout.splitlines()[-1])


MANAGED_RUNTIME_PROBE = '''import json, os, runpy, sys
from pathlib import Path
namespace = runpy.run_path(sys.argv[1], run_name='luma_native_runtime_probe')
namespace = namespace['runtime'].__globals__
assert namespace.get('LUMA_MANAGED_QWEN_RUNTIME') is True
namespace['configure_offline']()
protocol = os.fdopen(os.dup(sys.stdout.fileno()), 'w', encoding='utf-8', buffering=1)
os.dup2(sys.stderr.fileno(), sys.stdout.fileno())
sys.addaudithook(namespace['offline_audit'])
try:
    first = namespace['runtime']('qwen3-asr-transformers', 'cpu')
    proof = namespace['_luma_numba_proof']
    assert proof['selected'] == 'workqueue' and proof['numeric_passed'] and proof['parallel_jit_tested']
    second = namespace['runtime']('qwen3-asr-transformers', 'cpu')
    assert namespace['_luma_numba_proof'] is proof
    assert first['device'] == second['device'] == 'cpu'
    for name in ('core', 'module'):
        assert first[name] is second[name]
        assert Path(first[name].__file__).resolve().is_relative_to(Path(sys.prefix).resolve())
    assert namespace['luma_check_numba_workqueue'](require_initialized=True) == 'workqueue'
    checks = runpy.run_path(str(Path(sys.prefix) / 'self_test.py'), run_name='luma_native_library_check')
    libraries = checks['loaded_native_libraries']()
    assert libraries > 0
    print(json.dumps({'schema':1, 'first_jit_initialization':True, 'successful_proof_reused':True,
                      'numba_threading':proof, 'checked_after_qwen_imports':True,
                      'private_native_libraries_checked':libraries,
                      'device':'cpu', 'inference':False, 'model_weights_loaded':False}),file=protocol)
finally:
    protocol.close()
'''


def managed_worker_runtime_probe(executable, worker, env, cwd):
    # Reuse the existing owned/file-backed 120-second, 16-KiB child primitive.
    # This calls the actual embedded worker entrypoint; no substitute backend,
    # model files, model loading or private-package modification is involved.
    from windows_crt_proof import helper_process
    try:
        code, out, err = helper_process([str(executable), '-I', '-B', '-u', '-X', 'utf8', '-c',
                                        MANAGED_RUNTIME_PROBE, str(worker)], env, cwd)
    except BaseException as exc:
        raise RuntimeError('Managed worker runtime probe failed: ' + str(exc)) from exc
    if code:
        raise RuntimeError(f'Managed worker runtime probe failed ({code}):\n{out}\n{err}')
    lines = [line for line in out.splitlines() if line.strip()]
    if len(lines) != 1:
        raise ValueError('Managed runtime probe must emit exactly one metadata record')
    result = json.loads(lines[0])
    expected = {'schema':1, 'first_jit_initialization':True, 'successful_proof_reused':True,
                'checked_after_qwen_imports':True, 'device':'cpu', 'inference':False, 'model_weights_loaded':False}
    if any(result.get(key) != value for key, value in expected.items()):
        raise ValueError('Managed runtime probe returned unexpected evidence')
    if type(result.get('private_native_libraries_checked')) is not int or result['private_native_libraries_checked'] <= 0:
        raise ValueError('Managed runtime probe did not verify loaded native origins')
    result['embedded_worker_sha256'] = sha256(worker)
    return result


def main():
    p = argparse.ArgumentParser(); p.add_argument('--manifest', type=Path, required=True); p.add_argument('--worker', type=Path, required=True); p.add_argument('--cache', type=Path, required=True)
    a = p.parse_args(); manifest = json.loads(a.manifest.read_text()); output = a.manifest.parent
    archive = output / manifest['archive']['url'].rsplit('/', 1)[-1]
    assert sha256(archive) == manifest['archive']['sha256'] and archive.stat().st_size == manifest['archive']['bytes']
    with tempfile.TemporaryDirectory(prefix='luma-asr-clean-') as temp:
        work = Path(temp); unpacked = work / 'first-location'; extract(archive, unpacked)
        assert sum(p.stat().st_size for p in unpacked.rglob('*') if p.is_file()) == manifest['installed_bytes']
        assert len([p for p in unpacked.rglob('*') if p.is_file()]) == manifest['max_files']
        env = clean_environment(work / 'clean-home')
        nagisa = None; worker = a.worker.resolve()
        managed_qwen = manifest['platform'] == 'windows-x64' and manifest['backend'] == 'qwen3-asr'
        if managed_qwen:
            probe = work / 'nagisa-probe.py'
            probe.write_text(embed_nagisa((ROOT / 'probe_nagisa.py').read_text(encoding='utf-8')), encoding='utf-8')
            baseline = nagisa_probe(unpacked / manifest['entrypoint'], probe, env, work)
            worker = work / 'managed-worker.py'
            worker.write_text(embed_nagisa(a.worker.read_text(encoding='utf-8'), managed_worker=True), encoding='utf-8')
        relocated = work / 'Relocated private runtime é 测试'; unpacked.rename(relocated)
        executable = relocated / manifest['entrypoint']
        if managed_qwen:
            adapted = nagisa_probe(executable, probe, env, work, '--adapt')
            assert baseline['words'] == adapted['words'] and baseline['postags'] == adapted['postags']
            assert baseline['upstream_finders_added'] == adapted['upstream_finders_added']
            failure = nagisa_probe(executable, probe, env, work, '--adapt', '--forced-failure')
            assert baseline['upstream_finders_added'] == failure['upstream_finders_added']
            nagisa = {'baseline': baseline, 'unicode': adapted, 'failure': failure, 'japanese_tokens_match': True}
            print('NAGISA_UNICODE_PROOF=' + diagnostic_json(nagisa), flush=True)
        native_inventory = None
        if manifest['platform'] == 'windows-x64':
            from windows_native_inventory import inventory, inactive_numba_plugin, validate_configured_closure
            optional = None
            if managed_qwen:
                lock = json.loads((ROOT / 'locks/qwen3-asr-cpu-windows-x64.json').read_text(encoding='utf-8'))
                wheel = next(item for item in lock['wheels'] if item['name'] == 'numba')
                optional = inactive_numba_plugin(relocated, a.cache, wheel)
            native_inventory = inventory(relocated, optional)
            print('WINDOWS_NATIVE_CLOSURE=' + json.dumps({'passed': native_inventory['passed'],
                  'native_files': len(native_inventory['files']), 'blocked_dependencies': native_inventory['blocked_dependencies'],
                  'gpu_files': native_inventory['gpu_files'], 'inactive_optional_plugins': native_inventory['inactive_optional_plugins'],
                  'required_closure_passed': native_inventory['required_closure_passed'],
                  'full_tree_closure_passed': native_inventory['full_tree_closure_passed']}, sort_keys=True), flush=True)
            assert native_inventory['required_closure_passed'], 'Configured Windows runtime has unresolved required native dependencies'
        result = subprocess.run([str(executable), '-I', '-B', '-u', '-X', 'utf8', str(relocated / 'self_test.py')],
                                capture_output=True, text=True, encoding='utf-8', env=env, cwd=work, timeout=240)
        if result.returncode:
            raise RuntimeError(f'Private self-test failed ({result.returncode}):\n{result.stdout}\n{result.stderr}')
        imports = json.loads(result.stdout.splitlines()[-1])
        actual_worker_runtime = None
        if managed_qwen:
            actual_worker_runtime = managed_worker_runtime_probe(executable, worker, env, relocated)
            print('MANAGED_QWEN_WORKER_RUNTIME=' + diagnostic_json(actual_worker_runtime), flush=True)
        if native_inventory is not None:
            validate_configured_closure(native_inventory, imports)
            print('WINDOWS_CONFIGURED_RUNTIME_POLICY=' + diagnostic_json({key: native_inventory[key] for key in (
                'passed', 'required_closure_passed', 'full_tree_closure_passed', 'inactive_optional_plugins', 'configured_threading_proof')
                if key in native_inventory}), flush=True)
        requests = [{'id': 'offline-url', 'op': 'probe', 'engine': manifest['engine'], 'device': manifest['device'], 'model_path': 'https://invalid.example/never-download'}]
        frames = worker_requests(executable, worker, requests, env, work)
        response = frames[-1]
        assert response['id'] == 'offline-url' and response['event'] == 'probe' and response['ready'] is False
        assert response['code'] == 'local_path_required', response
        terminate_idle_worker([str(executable), '-I', '-B', '-u', '-X', 'utf8', str(worker)], env, work)
        recovered = worker_requests(executable, worker, requests, env, work)
        assert recovered[-1]['code'] == 'local_path_required'
        inference = {'tested': False, 'reason': 'Qwen imports/API only; model weights and memory fit are untested.' if manifest['backend'] == 'qwen3-asr' else 'MLX tiny fixture access is paused; no model inference performed.'}
        if manifest['backend'] == 'faster-whisper':
            fixture = json.loads((ROOT / 'fixtures.json').read_text()); a.cache.mkdir(parents=True, exist_ok=True)
            # Build-time, reviewed public fixture downloads. Workers stay offline.
            ALLOWED.update({'huggingface.co', 'raw.githubusercontent.com'})
            model = work / 'fixture-model'; model.mkdir()
            for item in fixture['faster_whisper_tiny']['files']:
                target = model.joinpath(*safe_name(item['path']).parts); target.parent.mkdir(parents=True, exist_ok=True)
                shutil.copyfile(fetch(item, a.cache), target)
            audio = work / 'jfk.wav'; shutil.copyfile(fetch(fixture['audio'], a.cache), audio)
            common = {'engine': 'whisper-accelerated', 'device': 'cpu', 'model_path': str(model), 'audio_path': str(audio), 'language': 'en'}
            frames = worker_requests(executable, worker, [dict(common, id='probe', op='probe'), dict(common, id='cold', op='transcribe'), dict(common, id='warm', op='transcribe')], env, work)
            by_id = {f['id']: f for f in frames if f.get('event') in {'probe', 'result', 'error'}}
            assert by_id['probe']['ready'], by_id
            for key in ('cold', 'warm'):
                result = by_id[key]
                assert result['event'] == 'result' and result['device'] == 'cpu', result
                assert 'country' in ' '.join(s['text'] for s in result['segments']).lower(), result
                previous = 0
                for segment in result['segments']:
                    assert previous <= segment['start_ms'] < segment['end_ms'] <= 11001
                    previous = segment['end_ms']
            assert not by_id['cold']['reused'] and by_id['warm']['reused']
            inference = {'tested': True, 'backend': 'faster-whisper', 'device': 'cpu', 'cold_and_warm': True,
                         'segments': by_id['cold']['segments'], 'model_revision': fixture['faster_whisper_tiny']['version'],
                         'audio_sha256': fixture['audio']['sha256']}
        report = {'schema': 1, 'pack_id': manifest['id'], 'source_sha': manifest['source_sha'],
                  'archive_sha256': manifest['archive']['sha256'], 'relocated': True, 'isolated': True,
                  'system_python_used': False, 'system_packages_used': False, 'offline_protocol_tested': True,
                  'worker_test': {'passed': True, 'checks': ['json-lines', 'offline-path-rejection', 'clean-eof-shutdown', 'idle-termination', 'recovery']},
                  'imports': imports, 'inference': inference,
                  'nagisa_unicode': nagisa,
                  'actual_worker_runtime': actual_worker_runtime,
                  'windows_native_inventory': native_inventory,
                  'limitations': ['No CUDA validation or CUDA redistribution.', 'No Developer ID signing, notarization, Gatekeeper bypass, or clean-GUI-machine validation.']}
        dump(output / f'{manifest["id"]}.smoke.json', report)
        print(diagnostic_json(report, indent=2))


if __name__ == '__main__':
    main()
