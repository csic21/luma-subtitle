#!/usr/bin/env python3
"""One fixed diagnostic artifact, fresh private runtime, no native compilation."""
import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path, PurePosixPath
import re
import shutil
import stat
import subprocess
import sys
import time
import zipfile

HERE = Path(__file__).resolve().parent
PRODUCER = '3006b955bb248e83e95f2dcc48c3001194513800'
RUN_ID = 37953677195
ARTIFACT_ID = 11629892995
ZIP_PIN = (15478967, '864aa80bbda7cb11f59c6bdc4f418e37ee1ca66c3a7e5583703fcc16b1b9e222')
WHEEL = 'ctranslate2-4.8.2-1lumacpu-cp312-cp312-win_amd64.whl'
SOURCES = 'luma-ct2-cpu-4.8.2-1-sources.zip'
NOTICES = 'luma-ct2-cpu-4.8.2-1-notices.zip'
MANIFEST = 'diagnostic-manifest.json'
PINS = {
    WHEEL: (25058630, '4644c5eb94492ab61d9749ee122e12c612c03634495d9cb59b6c18e3404cd1c0'),
    SOURCES: (13935077, 'c193ede80310da389aab0637b78493f0c7ba7ea0c6debac68701ec961306fd4a'),
    NOTICES: (84941, '73667a775f8a46408b265415f1a8b0a085570292be2b64210bad3526b131f69f'),
    MANIFEST: (19012, 'fad46ac92720c75897393521942fc163b2ce183adef62ee7cda9938cb91a30b9'),
}
CASES = tuple((threads, case) for threads in ('default', 'one') for case in ('eof', 'unload', 'switch'))


def dump(path, value):
    path = Path(path); path.parent.mkdir(parents=True, exist_ok=True)
    temp = path.with_suffix('.tmp')
    temp.write_text(json.dumps(value, sort_keys=True, indent=2, ensure_ascii=True) + '\n', encoding='utf-8', newline='\n')
    temp.replace(path)


def sha(data):
    return hashlib.sha256(data).hexdigest()


def check_bytes(data, pin):
    if (len(data), sha(data)) != pin:
        raise ValueError('Pinned diagnostic bytes changed')


def safe_member(info):
    name = info.filename
    mode = info.external_attr >> 16
    if (not name or '\\' in name or ':' in name or PurePosixPath(name).is_absolute()
            or any(part in ('', '.', '..') for part in name.split('/'))
            or info.is_dir() or info.flag_bits & 1
            or (stat.S_IFMT(mode) not in (0, stat.S_IFREG)) or info.file_size > 50_000_000):
        raise ValueError('Unsafe diagnostic archive member')
    return name


def validate_manifest(manifest):
    expected = {'repository': 'csic21/luma-subtitle', 'run_id': RUN_ID, 'run_attempt': 1,
                'job': 'windows-cpu-proof', 'head_sha': PRODUCER, 'source_sha': PRODUCER,
                'ref': 'refs/heads/feat/optional-asr-engines', 'event_name': 'push',
                'workflow_ref': 'csic21/luma-subtitle/.github/workflows/asr-ct2-cpu.yml@refs/heads/feat/optional-asr-engines'}
    assets = [{'name': name, 'bytes': PINS[name][0], 'sha256': PINS[name][1]} for name in sorted((WHEEL, SOURCES, NOTICES))]
    if (manifest.get('schema_version') != 1 or manifest.get('kind') != 'ct2-cpu-diagnostic'
            or manifest.get('origin') != expected or manifest.get('assets') != assets
            or manifest.get('publication_authorized') is not False or manifest.get('installable') is not False
            or manifest.get('inference_passed') is not False
            or manifest.get('verification') != {'outcome': 'timed_out', 'stage': 'native-verifier', 'started': True, 'timeout_seconds': 900}
            or manifest.get('build_checks') != {'compiler_probe': True, 'executable_bytes_modified': False,
                'fresh_fixed_root_repeatability': True, 'static_runtime_closure': True}):
        raise ValueError('Diagnostic producer provenance changed')


def verify_extract(artifact, work):
    artifact, work = Path(artifact), Path(work)
    if artifact.is_symlink() or not artifact.is_file() or artifact.stat().st_size != ZIP_PIN[0]:
        raise ValueError('Expected exact diagnostic ZIP file')
    check_bytes(artifact.read_bytes(), ZIP_PIN)
    if work.exists() or work.is_symlink() or not work.is_absolute() or work != work.resolve():
        raise ValueError('Replay work root must be fresh and canonical')
    with zipfile.ZipFile(artifact) as archive:
        infos = archive.infolist()
        names = [safe_member(info) for info in infos]
        if len(names) != 4 or set(names) != set(PINS):
            raise ValueError('Expected exactly four diagnostic files')
        payloads = {}
        for info in infos:
            if info.file_size != PINS[info.filename][0]:
                raise ValueError('Diagnostic member size changed')
            payloads[info.filename] = archive.read(info)
            check_bytes(payloads[info.filename], PINS[info.filename])
    manifest = json.loads(payloads[MANIFEST]); validate_manifest(manifest)
    # Validate source export/provenance before any artifact Python/PowerShell is
    # imported or executed. Only source-only luma/ recipes are extracted here.
    import io
    recipes = {}
    with zipfile.ZipFile(io.BytesIO(payloads[SOURCES])) as archive:
        infos = archive.infolist(); names = [safe_member(info) for info in infos]
        if len(set(name.casefold() for name in names)) != len(names) or sum(i.file_size for i in infos) > 50_000_000:
            raise ValueError('Duplicate or excessive source archive')
        if json.loads(archive.read('BUILD-PROVENANCE.json')) != manifest['provenance']:
            raise ValueError('Source bundle build provenance differs')
        if json.loads(archive.read('SOURCE-EXPORTS.json')) != manifest['source_exports']:
            raise ValueError('Source omission inventory differs')
        for info in infos:
            if info.filename.startswith('luma/'):
                recipes[info.filename] = archive.read(info)
    for name in ('build_cpu.py', 'prepare_toolchain.ps1', 'verify_runtime.py', 'sources.lock.json'):
        if 'luma/scripts/asr-components/ct2-cpu/' + name not in recipes:
            raise ValueError('Original source recipe is missing')
    work.mkdir(parents=True)
    artifacts = work / 'artifact'; artifacts.mkdir()
    for name, data in payloads.items(): (artifacts / name).write_bytes(data)
    for name, data in recipes.items():
        destination = work.joinpath(*PurePosixPath(name).parts)
        destination.parent.mkdir(parents=True, exist_ok=True); destination.write_bytes(data)
    return work / 'luma', manifest, {name: {'bytes': len(data), 'sha256': sha(data)} for name, data in sorted(recipes.items())}


def original_helpers(source):
    cpu = source / 'scripts/asr-components/ct2-cpu'
    components = cpu.parent
    names = ('build', 'native_inventory', 'license_inventory', 'crt_proof', 'repro_diagnostics')
    if any(name in sys.modules for name in names):
        raise ValueError('Replay process already imported a potentially different recipe')
    sys.path[:0] = [str(cpu), str(components)]
    spec = importlib.util.spec_from_file_location('original_build_cpu', cpu / 'build_cpu.py')
    module = importlib.util.module_from_spec(spec); spec.loader.exec_module(module)
    for name in names:
        if not Path(sys.modules[name].__file__).resolve().is_relative_to(source):
            raise ValueError('Imported recipe escaped the verified original source')
    return module


def clean_environment(home):
    clean = {k: v for k, v in os.environ.items() if k.upper() in {'SYSTEMROOT', 'WINDIR', 'COMSPEC', 'SYSTEMDRIVE'}}
    clean.update(HOME=str(home), USERPROFILE=str(home), TEMP=str(home), TMP=str(home), APPDATA=str(home/'AppData'),
                 LOCALAPPDATA=str(home/'Local'), PATH=str(home/'no-executables'), PYTHONPATH=str(home/'poison'),
                 PYTHONHOME=str(home/'invalid-python'), HF_HOME=str(home/'empty-hf-cache'), HF_HUB_OFFLINE='1',
                 TRANSFORMERS_OFFLINE='1', HF_HUB_DISABLE_TELEMETRY='1', HF_HUB_DISABLE_IMPLICIT_TOKEN='1', DO_NOT_TRACK='1')
    return clean


def run_case(argv, home, reports, environment, timeout):
    logfile = reports.parent / (reports.name + '.log')
    result = {'timeout_seconds': timeout, 'normal_exit': False, 'passed': False}
    started = time.monotonic()
    drain_path = reports.parent / 'replay-child-drain.json'
    checkpoint = {'schema': 1, 'owner': 'replay-driver', 'case': reports.name, 'drained': False, 'pid': None}
    dump(drain_path, checkpoint)
    with logfile.open('xb') as stream:
        child = subprocess.Popen(argv, cwd=home, env=environment, stdout=stream, stderr=subprocess.STDOUT)
        deadline = started + timeout
        try:
            checkpoint['pid'] = child.pid
            dump(drain_path, checkpoint)
            while True:
                code = child.poll()
                if code is not None:
                    result.update(returncode=code, normal_exit=code == 0, outcome='exited')
                    break
                if logfile.stat().st_size > 4_000_000:
                    result['outcome'] = 'output_bound_exceeded'
                    break
                remaining = deadline - time.monotonic()
                if remaining <= 0:
                    result['outcome'] = 'timed_out'
                    break
                try:
                    child.wait(timeout=min(0.2, remaining))
                except subprocess.TimeoutExpired:
                    pass
        finally:
            if child.poll() is None:
                child.kill()
            # Always reap this owned direct child, including cancellation and
            # output-bound failures. No orphan is treated as a completed case.
            child.wait(timeout=10)
            checkpoint['drained'] = True
            dump(drain_path, checkpoint)
    result['elapsed_seconds_observed'] = time.monotonic() - started
    result['owned_child_reaped'] = True
    result['observed_log_bytes'] = logfile.stat().st_size
    if result['observed_log_bytes'] > 4_000_000:
        result['outcome'] = 'output_bound_exceeded'
        with logfile.open('r+b') as stream:
            stream.truncate(4_000_000)
        result['log_truncated'] = True
    result['log_bytes'] = logfile.stat().st_size
    report = reports/'result.json'
    if report.is_file():
        data = json.loads(report.read_text())
        result['passed'] = (result['normal_exit'] and result['outcome'] == 'exited'
                            and data.get('passed') is True)
    return result


def main():
    parser = argparse.ArgumentParser()
    for name in ('artifact', 'work', 'cache', 'reports'):
        parser.add_argument('--' + name, type=Path, required=True)
    parser.add_argument('--verifier-source-sha', required=True)
    args = parser.parse_args()
    for name in ('artifact', 'work', 'cache', 'reports'): setattr(args, name, getattr(args, name).resolve())
    if sys.platform != 'win32' or sys.version_info[:3] != (3, 12, 10) or sys.maxsize <= 2**32:
        raise RuntimeError('Replay requires exact native x64 Windows orchestrator 3.12.10')
    if not re.fullmatch('[a-f0-9]{40}', args.verifier_source_sha): raise ValueError('Exact verifier source SHA required')
    current = subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=HERE, text=True).strip()
    if current != args.verifier_source_sha: raise ValueError('Diagnostic verifier checkout differs')
    if args.reports.exists() or args.reports.is_symlink(): raise ValueError('Replay reports must be fresh')
    args.reports.mkdir(parents=True)
    status = {'schema': 1, 'kind': 'ct2-lifecycle-replay', 'publication_authorized': False,
              'producer_source_sha': PRODUCER, 'producer_run_id': RUN_ID, 'producer_artifact_id': ARTIFACT_ID,
              'verifier_source_sha': current, 'passed': False, 'cases': [], 'performance_claim': False, 'children_drained': False}
    try:
        source, manifest, recipes = verify_extract(args.artifact, args.work)
        dump(args.reports/'original-inputs.json', {'artifact_sha256': ZIP_PIN[1], 'manifest': manifest, 'recipes': recipes,
             'diagnostic_sources': {p.name: {'bytes': p.stat().st_size, 'sha256': sha(p.read_bytes())}
                                    for p in (HERE/'replay_driver.py', HERE/'lifecycle_probe.py')}})
        old = original_helpers(source); cpu = source/'scripts/asr-components/ct2-cpu'
        pwsh = Path(os.environ['ProgramFiles'])/'PowerShell/7/pwsh.exe'
        if not pwsh.is_file(): raise ValueError('Official hosted PowerShell 7 unavailable')
        old.command([pwsh, '-NoProfile', '-NonInteractive', '-File', cpu/'prepare_toolchain.ps1', '-Reports', args.reports, '-Lock', cpu/'sources.lock.json'],
                    cwd=source, env=os.environ.copy(), logfile=args.reports/'toolchain.log', timeout=120)
        toolchain = old.load(args.reports/'toolchain.json')
        old.dump(args.reports/'installed-license-evidence.json', old.discover_installed_licenses(
            toolchain['installation_path'], toolchain['redist_path'], toolchain['product_id'], toolchain['visual_studio_version']))
        runtime = args.work/'fresh-runtime'
        runtime_item = old.load(source/'scripts/asr-components/packs.json')['runtime']['windows-x64']
        old.unpack_runtime(old.fetch(runtime_item, args.cache), runtime)
        lock = old.load(source/'scripts/asr-components/locks/faster-whisper-cpu-windows-x64.json')
        wheelhouse = args.work/'wheelhouse'; wheelhouse.mkdir(); selected = []
        for item in lock['wheels']:
            if item['name'] == 'ctranslate2':
                item = dict(item, filename=WHEEL, bytes=PINS[WHEEL][0], sha256=PINS[WHEEL][1], url='local-diagnostic-only:' + WHEEL)
                wheel = args.work/'artifact'/WHEEL
            else: wheel = old.fetch(item, args.cache)
            selected.append(item); shutil.copyfile(wheel, wheelhouse/item['filename'])
        if sum(item['name'] == 'ctranslate2' for item in selected) != 1: raise ValueError('Original runtime CT2 count changed')
        runtime_lock = args.work/'runtime-lock.json'; old.dump(runtime_lock, dict(lock, wheels=selected))
        old.command([runtime/'python.exe', '-I', '-S', '-B', '-X', 'utf8', source/'scripts/asr-components/assemble.py',
                    '--runtime-root', runtime, '--wheel-lock', runtime_lock, '--wheelhouse', wheelhouse], cwd=args.work,
                    env=old.build_environment(runtime), logfile=args.reports/'assembly.log', timeout=600)
        old.copy_proof_crt(runtime, args.reports)
        relocated = args.work/'Relocated private Python é 测试'; runtime.rename(relocated); runtime = relocated
        native = old.closure(old.inventory(runtime)); dump(args.reports/'whole-runtime-native.json', native)
        if native['passed'] is not True: raise ValueError('Replay private static closure failed')
        fixture = old.load(source/'scripts/asr-components/fixtures.json'); model = args.work/'model'; model.mkdir()
        for item in fixture['faster_whisper_tiny']['files']:
            destination = model.joinpath(*old.safe_path(item['path']).parts); destination.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(old.fetch(item, args.cache), destination)
        switch = args.work/'model-switch'; shutil.copytree(model, switch)
        fixture_pins = []
        for item in fixture['faster_whisper_tiny']['files']:
            for directory in (model, switch):
                copied = directory.joinpath(*old.safe_path(item['path']).parts)
                if copied.stat().st_size != item['bytes'] or old.digest(copied) != item['sha256']:
                    raise ValueError('Copied lifecycle model fixture changed')
            fixture_pins.append({key: item[key] for key in ('path', 'bytes', 'sha256')})
        audio = args.work/'jfk.wav'; shutil.copyfile(old.fetch(fixture['audio'], args.cache), audio)
        dump(args.reports/'replay-runtime.json', {'schema': 1, 'publication_authorized': False, 'python': str(runtime/'python.exe'),
             'root': str(runtime), 'model': str(model), 'audio': str(audio), 'cache': str(args.cache), 'wheel_sha256': PINS[WHEEL][1],
             'original_worker': str(source/'src-tauri/src/asr/worker.py'), 'producer_source_sha': PRODUCER,
             'switch_model': str(switch), 'fixture_files': fixture_pins})
        deadline = time.monotonic() + 900
        for threads, case in CASES:
            timeout = min(120, deadline-time.monotonic())
            if timeout <= 0: raise RuntimeError('Diagnostic aggregate900s deadline reached')
            name = threads + '-' + case; home = args.work/('home-' + name); home.mkdir()
            (home/'poison').mkdir(); (home/'poison/sitecustomize.py').write_text("raise RuntimeError('Inherited Python path used')\n")
            output = args.reports/name
            argv = [str(runtime/'python.exe'), '-I', '-B', '-X', 'utf8', str(HERE/'lifecycle_probe.py'),
                    '--root', str(runtime), '--worker', str(source/'src-tauri/src/asr/worker.py'),
                    '--original-verifier', str(cpu/'verify_runtime.py'), '--model', str(model), '--switch-model', str(switch),
                    '--audio', str(audio), '--reports', str(output), '--case', case, '--threads', threads]
            status['children_drained'] = False
            dump(args.reports/'replay-result.json', status)
            measured = run_case(argv, home, output, clean_environment(home), timeout)
            status['children_drained'] = measured['owned_child_reaped'] is True
            status['cases'].append(dict(measured, name=name, cpu_threads=threads, case=case))
            dump(args.reports/'replay-result.json', status)
        status['comparison_completed'] = True
        status['baseline_passed'] = all(c['passed'] for c in status['cases'] if c['cpu_threads'] == 'default')
        status['single_thread_passed'] = all(c['passed'] for c in status['cases'] if c['cpu_threads'] == 'one')
        status['passed'] = all(c['passed'] for c in status['cases'])
        if not status['passed']: raise RuntimeError('At least one lifecycle case failed; baseline outcome retained')
    except BaseException as error:
        status.update(passed=False, error=type(error).__name__ + ': ' + str(error)); raise
    finally:
        dump(args.reports/'replay-result.json', status)
        print(json.dumps(status, ensure_ascii=True, indent=2), flush=True)


if __name__ == '__main__':
    main()
