#!/usr/bin/env python3
"""Test extracted pack using only its private interpreter in a poisoned env."""
from __future__ import annotations
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import zipfile
from build import ROOT, ALLOWED, dump, embed_nagisa, fetch, safe_name, sha256
from fixture_paths import temporary_root


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
    (poison / 'sitecustomize.py').write_text("raise RuntimeError('inherited Python path leaked into component')\n", encoding='utf-8')
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
    host_security_modules = []
    libraries = checks['loaded_native_libraries'](host_security_modules)
    assert libraries > 0
    print(json.dumps({'schema':1, 'first_jit_initialization':True, 'successful_proof_reused':True,
                      'numba_threading':proof, 'checked_after_qwen_imports':True,
                      'private_native_libraries_checked':libraries,
                      'verified_host_security_modules':host_security_modules,
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
    security = result.get('verified_host_security_modules')
    if (not isinstance(security, list) or any(not isinstance(item, dict) or item.get('verified') is not True
            or item.get('kind') != 'windows-defender-amsi' for item in security)):
        raise ValueError('Managed runtime probe returned invalid host-security evidence')
    result['embedded_worker_sha256'] = sha256(worker)
    return result



def prepare_cpu_smoke(manifest, root, worker, cache, destination):
    from own_cpu_recipe import PACK_ID, cached_component, prepare_published_worker
    if manifest['id'] != PACK_ID or manifest['platform'] != 'windows-x64' or manifest['backend'] != 'faster-whisper':
        raise ValueError('Own CPU smoke requires the exact managed Windows pack')
    lock = json.loads((ROOT / 'locks' / (PACK_ID + '.json')).read_text(encoding='utf-8'))
    component = cached_component(lock, cache, root=ROOT)
    identity = {key: value for key, value in component.items() if key != 'proof'}
    if manifest.get('component_provenance') != identity:
        raise ValueError('Reference runtime manifest differs from the pinned CPU component')
    policy = prepare_published_worker(worker, root, cache, component, manifest['source_sha'], destination)
    component['proof_path'] = Path(cache) / component['publication_proof']['sha256']
    return component, policy


def cpu_reference_inference(executable, worker, root, model, switch_model, audio, env, work, manifest, component):
    """Reuse native Tiny/cleanup checks, retaining separate proof-layer identity."""
    if model.resolve() == switch_model.resolve():
        raise ValueError('Reference CPU model switch requires a distinct directory')
    destination = work / 'reference-cpu-inference.json'
    provenance = root / 'Lib/site-packages/ctranslate2-4.8.2.dist-info/LUMA_CPU_BUILD.json'
    auditor = ROOT / 'self_test.py'
    command = [str(executable), '-I', '-B', '-u', '-X', 'utf8', str(ROOT / 'ct2-cpu/verify_runtime.py'),
               '--root', str(root), '--model', str(model), '--switch-model', str(switch_model),
               '--audio', str(audio), '--worker', str(worker), '--report', str(destination),
               '--host-auditor', str(auditor), '--host-auditor-sha256', sha256(auditor),
               '--build-provenance', str(provenance), '--build-provenance-sha256', sha256(provenance),
               '--source-sha', manifest['source_sha'], '--component-source-sha', component['component_source_sha'],
               '--publication-proof', str(component['proof_path'])]
    from windows_crt_proof import helper_process
    from real_worker_fixture import bounded_json
    code, out, err = helper_process(command, env, work, timeout=900, output_limit=1_000_000)
    if code:
        raise RuntimeError(f'Reference CPU inference/cleanup failed ({code}):\n{out[-12000:]}\n{err[-12000:]}')
    report = bounded_json(destination, work, 4_000_000)
    policy = report.get('cpu_thread_policy', {})
    lifecycle = report.get('source_worker_lifecycle', {})
    if (report.get('passed') is not True or report.get('inference', {}).get('cold_and_warm') is not True
            or report.get('host_crt_fallback_allowed') is not False
            or report.get('offline_audit_enabled') is not True
            or report.get('isolated') is not True or report.get('system_python_used') is not False
            or report.get('inherited_python_path_ignored') is not True or not report.get('loaded_modules')
            or report.get('host_auditor', {}).get('source_sha') != manifest['source_sha']
            or report.get('host_auditor', {}).get('sha256') != sha256(auditor)
            or report.get('verifier', {}).get('source_sha') != manifest['source_sha']
            or report.get('verifier', {}).get('sha256') != sha256(ROOT / 'ct2-cpu/verify_runtime.py')
            or policy.get('source_sha') != manifest['source_sha']
            or policy.get('component_source_sha') != component['component_source_sha']
            or type(policy.get('cpu_threads')) is not int or policy.get('cpu_threads') != 1 or policy.get('variant') != 'luma-cpu-seq-1'
            or policy.get('managed_receipt_selection_tested') is not False
            or policy.get('constructor_overridden') is not False
            or policy.get('build_provenance_sha256') != sha256(provenance)
            or policy.get('worker_sha256') != hashlib.sha256(worker.read_text(encoding='utf-8').encode('utf-8')).hexdigest()
            or any(lifecycle.get(key) is not True for key in ('eof_after_inference', 'model_switch_after_inference', 'explicit_unload_after_inference'))):
        raise ValueError('Reference CPU inference evidence is incomplete or source-mismatched')
    from own_cpu_recipe import strict_cpu_closure
    loaded = [{'path': item['path'], 'normal': [], 'delay': []} for item in report['loaded_modules']]
    if not strict_cpu_closure(loaded)['passed']:
        raise ValueError('Reference CPU inference loaded a forbidden native library')
    report.update(proof_layer='reference-runtime-source-policy', normal_process_exit=True,
                  published_component=component['component_source_sha'],
                  managed_receipt_selection_tested=False, global_managed_path_selection_tested=False)
    return report


def main():
    p = argparse.ArgumentParser(); p.add_argument('--manifest', type=Path, required=True); p.add_argument('--worker', type=Path, required=True); p.add_argument('--cache', type=Path, required=True)
    a = p.parse_args(); manifest = json.loads(a.manifest.read_text(encoding='utf-8')); output = a.manifest.parent
    archive = output / manifest['archive']['url'].rsplit('/', 1)[-1]
    assert sha256(archive) == manifest['archive']['sha256'] and archive.stat().st_size == manifest['archive']['bytes']
    # Canonicalize only this newly owned root before deriving any child paths.
    # Windows TEMP may use an 8.3 spelling; installed-member guards stay strict.
    with temporary_root(prefix='luma-asr-clean-') as work:
        unpacked = work / 'first-location'; extract(archive, unpacked)
        assert sum(p.stat().st_size for p in unpacked.rglob('*') if p.is_file()) == manifest['installed_bytes']
        assert len([p for p in unpacked.rglob('*') if p.is_file()]) == manifest['max_files']
        env = clean_environment(work / 'clean-home')
        nagisa = None; worker = a.worker.resolve()
        managed_cpu = manifest['platform'] == 'windows-x64' and manifest['backend'] == 'faster-whisper'
        cpu_component = cpu_policy = cpu_reference = None
        managed_qwen = manifest['platform'] == 'windows-x64' and manifest['backend'] == 'qwen3-asr'
        if managed_qwen:
            probe = work / 'nagisa-probe.py'
            probe.write_text(embed_nagisa((ROOT / 'probe_nagisa.py').read_text(encoding='utf-8')), encoding='utf-8')
            baseline = nagisa_probe(unpacked / manifest['entrypoint'], probe, env, work)
            worker = work / 'managed-worker.py'
            worker.write_text(embed_nagisa(a.worker.read_text(encoding='utf-8'), managed_worker=True), encoding='utf-8')
        relocated = work / 'Relocated private runtime é 测试'; unpacked.rename(relocated)
        executable = relocated / manifest['entrypoint']
        if managed_cpu:
            worker = work / 'managed-cpu-worker.py'
            cpu_component, cpu_policy = prepare_cpu_smoke(manifest, relocated, a.worker.resolve(), a.cache, worker)
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
            if managed_cpu:
                from own_cpu_recipe import strict_cpu_closure
                native_inventory = strict_cpu_closure(native_inventory['files'])
                print('WINDOWS_OWN_CPU_CLOSURE=' + diagnostic_json(native_inventory), flush=True)
                if not native_inventory['passed']:
                    raise ValueError('Own CPU runtime contains forbidden or unresolved native dependencies')
            else:
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
        if native_inventory is not None and not managed_cpu:
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
            fixture = json.loads((ROOT / 'fixtures.json').read_text(encoding='utf-8')); a.cache.mkdir(parents=True, exist_ok=True)
            # Build-time, reviewed public fixture downloads. Workers stay offline.
            ALLOWED.update({'huggingface.co', 'raw.githubusercontent.com'})
            model = work / 'fixture-model'; model.mkdir()
            for item in fixture['faster_whisper_tiny']['files']:
                target = model.joinpath(*safe_name(item['path']).parts); target.parent.mkdir(parents=True, exist_ok=True)
                shutil.copyfile(fetch(item, a.cache), target)
            audio = work / 'jfk.wav'; shutil.copyfile(fetch(fixture['audio'], a.cache), audio)
            common = {'engine': 'whisper-accelerated', 'device': 'cpu', 'model_path': str(model), 'audio_path': str(audio), 'language': 'en'}
            if managed_cpu:
                switched = work / 'fixture model switch 子 日本語 é'
                shutil.copytree(model, switched)
                cpu_reference = cpu_reference_inference(executable, a.worker.resolve(), relocated, model, switched, audio, env, work, manifest, cpu_component)
                frames = []
            else:
                frames = worker_requests(executable, worker, [dict(common, id='probe', op='probe'), dict(common, id='cold', op='transcribe'), dict(common, id='warm', op='transcribe')], env, work)
            if managed_cpu:
                inference = {'tested': True, 'backend': 'faster-whisper', 'device': 'cpu',
                             **cpu_reference['inference'], 'model_revision': fixture['faster_whisper_tiny']['version'],
                             'audio_sha256': fixture['audio']['sha256']}
            else:
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
                  'own_cpu_policy': cpu_policy, 'own_cpu_reference': cpu_reference,
                  'windows_native_inventory': native_inventory,
                  'limitations': ['No CUDA validation or CUDA redistribution.', 'No Developer ID signing, notarization, Gatekeeper bypass, or clean-GUI-machine validation.']}
        dump(output / f'{manifest["id"]}.smoke.json', report)
        print(diagnostic_json(report, indent=2))


if __name__ == '__main__':
    main()
