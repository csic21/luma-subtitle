#!/usr/bin/env python3
"""Build the supported CPU-only variant; all binaries remain local to this job.

This is a proof pipeline, not a publisher. It never fetches the vendor CT2 wheel.
The bootstrap Python only downloads/checks files; the pinned private PBS Python
builds the extension and performs all runtime checks.
"""
from __future__ import annotations
import argparse
import ast
import base64
import csv
import email
import hashlib
import io
import json
import os
from pathlib import Path, PurePosixPath
import re
import shutil
import subprocess
import sys
import tarfile
import urllib.parse
import urllib.request
import zipfile

HERE = Path(__file__).resolve().parent
COMPONENTS = HERE.parent
sys.path.insert(0, str(COMPONENTS))
from build import unpack_runtime
from native_inventory import closure, digest, inventory
from license_inventory import discover as discover_installed_licenses
from crt_proof import copy_proof_crt
from repro_diagnostics import compare_wheels

ALLOWED = {'codeload.github.com', 'github.com', 'files.pythonhosted.org', 'huggingface.co', 'raw.githubusercontent.com'}


def dump(path, value):
    Path(path).parent.mkdir(parents=True, exist_ok=True)
    Path(path).write_text(json.dumps(value, indent=2, sort_keys=True, ensure_ascii=False) + '\n', encoding='utf-8', newline='\n')


def load(path):
    return json.loads(Path(path).read_text(encoding='utf-8-sig'))


def safe_path(name):
    path = PurePosixPath(name)
    if path.is_absolute() or '\\' in name or ':' in name or any(p in ('', '.', '..') for p in name.split('/')):
        raise ValueError('Unsafe archive path')
    return path


def fetch(item, cache):
    url = urllib.parse.urlsplit(item['url'])
    if url.scheme != 'https' or url.hostname not in ALLOWED or url.username or url.password:
        raise ValueError('Unapproved artifact host')
    if not re.fullmatch('[0-9a-f]{64}', item['sha256']) or not 0 < item['bytes'] < 1_000_000_000:
        raise ValueError('Invalid artifact identity')
    cache.mkdir(parents=True, exist_ok=True)
    path = cache / item['sha256']
    if not path.exists():
        partial = path.with_suffix('.part')
        try:
            with urllib.request.urlopen(item['url'], timeout=90) as response, partial.open('wb') as output:
                total = 0
                while chunk := response.read(1024 * 1024):
                    total += len(chunk)
                    if total > item['bytes']:
                        raise ValueError('Download exceeds locked size')
                    output.write(chunk)
            if partial.stat().st_size != item['bytes'] or digest(partial) != item['sha256']:
                raise ValueError('Download differs from locked bytes')
            partial.replace(path)
        finally:
            if partial.exists():
                partial.unlink()
    if path.is_symlink() or path.stat().st_size != item['bytes'] or digest(path) != item['sha256']:
        raise ValueError('Cached artifact differs from locked bytes')
    return path


def extract_source(archive, destination):
    """Only one-root, regular-file source archives; do not permit links/devices."""
    destination.mkdir(parents=True, exist_ok=True)
    seen = set()
    with tarfile.open(archive, 'r:gz') as source:
        members = source.getmembers()
        roots = {safe_path(m.name.rstrip('/')).parts[0] for m in members}
        if len(roots) != 1:
            raise ValueError('Source archive must have one root')
        total = 0
        for member in members:
            parts = safe_path(member.name.rstrip('/')).parts[1:]
            if not parts:
                if not member.isdir():
                    raise ValueError('Source archive root must be a directory')
                continue
            if member.isdir():
                continue
            if not member.isreg() or parts in seen:
                raise ValueError('Source contains links, devices or duplicates')
            seen.add(parts)
            total += member.size
            if total > 200_000_000:
                raise ValueError('Source archive extraction bound exceeded')
            target = destination.joinpath(*parts)
            target.parent.mkdir(parents=True, exist_ok=True)
            with source.extractfile(member) as stream, target.open('wb') as out:
                shutil.copyfileobj(stream, out)


def validate_lock(lock):
    if lock['schema'] != 1 or lock['publication_authorized'] is not False:
        raise ValueError('This proof cannot authorize publication')
    if lock['python'] != '3.12.15' or lock['wheel_build_tag'] != '1lumacpu':
        raise ValueError('Unreviewed wheel identity')
    sources = {s['name']: s for s in lock['sources']}
    if set(sources) != {'ctranslate2', 'onednn', 'cpu_features', 'spdlog'}:
        raise ValueError('Unreviewed native source closure')
    if sources['ctranslate2']['commit'] != 'd44d2d069eb88c7b7804da864c10c201501cb4a9':
        raise ValueError('Unreviewed CT2 source')
    for source in sources.values():
        if not re.fullmatch('[0-9a-f]{40}', source['commit']) or source['url'] != f'https://codeload.github.com/{source["repository"]}/tar.gz/{source["commit"]}':
            raise ValueError('Source must be an immutable official commit archive')
    used = {i['path']: i['commit'] for i in lock['ctranslate2_submodules'] if i['used']}
    if used != {'third_party/cpu_features': sources['cpu_features']['commit'], 'third_party/spdlog': sources['spdlog']['commit']}:
        raise ValueError('Submodule pin differs from the upstream source tree')
    ct = lock['ct2_cmake']
    for option in ['WITH_MKL', 'WITH_CUDA', 'WITH_CUDNN', 'WITH_HIP', 'WITH_RUY', 'WITH_OPENBLAS', 'WITH_ACCELERATE', 'WITH_TENSOR_PARALLEL', 'WITH_FLASH_ATTN', 'CUDA_DYNAMIC_LOADING']:
        if ct[option] != 'OFF':
            raise ValueError('Forbidden backend is enabled')
    if (ct['OPENMP_RUNTIME'], ct['WITH_DNNL'], ct['BUILD_SHARED_LIBS']) != ('NONE', 'ON', 'ON'):
        raise ValueError('Unsupported CPU backend selection')
    dn = lock['onednn_cmake']
    if (dn['DNNL_CPU_RUNTIME'], dn['DNNL_GPU_RUNTIME'], dn['DNNL_LIBRARY_TYPE'], dn['DNNL_BLAS_VENDOR']) != ('SEQ', 'NONE', 'STATIC', 'NONE'):
        raise ValueError('Unreviewed oneDNN runtime')
    for options in (ct, dn):
        if options['CMAKE_MSVC_RUNTIME_LIBRARY'] != 'MultiThreadedDLL':
            raise ValueError('CRT flags must match the supported upstream Python extension')
    if any(w['name'] == 'ctranslate2' for w in lock['build_wheels']):
        raise ValueError('Vendor CT2 wheel must never enter the build')


def command(argv, *, cwd, env, logfile, timeout=3600):
    print('RUN:', subprocess.list2cmdline([str(x) for x in argv]), flush=True)
    with Path(logfile).open('ab') as log:
        proc = subprocess.run([str(x) for x in argv], cwd=cwd, env=env, stdout=log, stderr=subprocess.STDOUT, timeout=timeout)
    if proc.returncode:
        tail = Path(logfile).read_text(encoding='utf-8', errors='replace')[-16000:]
        raise RuntimeError(f'Command failed ({proc.returncode}): {argv[0]}\n{tail}')


def build_environment(runtime):
    env = {k: v for k, v in os.environ.items() if not k.upper().startswith(('PYTHON', 'PIP_', 'CONDA', 'VIRTUAL_ENV', 'CT2_', 'DNNL_', 'ONEDNN_', 'MKL', 'INTEL', 'ONEAPI', 'CUDA', 'CUDNN'))}
    env.update(PIP_CONFIG_FILE=os.devnull, PYTHONDONTWRITEBYTECODE='1', PYTHONUTF8='1',
               CMAKE_BUILD_PARALLEL_LEVEL='3', DISTUTILS_USE_SDK='1', MSSdk='1')
    return env


def verified_tool_payload(runtime, archive, member, installed_relative):
    installed = runtime / installed_relative
    if installed.is_symlink() or not installed.is_file() or not installed.resolve().is_relative_to(runtime.resolve()):
        raise ValueError('Pinned build executable is missing from its private installation path')
    with zipfile.ZipFile(archive) as wheel:
        if wheel.namelist().count(member) != 1:
            raise ValueError('Pinned build executable is not unique in its source wheel')
        expected = hashlib.sha256(wheel.read(member)).hexdigest()
    if digest(installed) != expected:
        raise ValueError('Installed build executable differs from the pinned wheel payload')
    return installed


def install_build_tools(runtime, lock, cache, work, env, reports):
    wheelhouse = work / 'build-wheelhouse'; wheelhouse.mkdir()
    requirements = []
    for item in lock['build_wheels']:
        target = wheelhouse / item['filename']
        shutil.copyfile(fetch(item, cache), target)
        requirements.append(f'{target.as_uri()} --hash=sha256:{item["sha256"]}')
    req = work / 'build-requirements.txt'; req.write_text('\n'.join(requirements) + '\n', encoding='utf-8')
    executable = runtime / 'python.exe'
    command([executable, '-I', '-B', '-m', 'pip', '--isolated', '--disable-pip-version-check', '--no-cache-dir',
             'install', '--no-index', '--no-deps', '--require-hashes', '--only-binary=:all:', '--no-compile',
             '--force-reinstall', '-r', req], cwd=work, env=env, logfile=reports / 'tools.log', timeout=300)
    # Ninja 1.11.1.4 uses the wheel .data/scripts scheme, unlike CMake's
    # package-owned data/bin path. Never fall back to a runner-global executable.
    layouts = [('cmake', 'cmake/data/bin/cmake.exe', 'Lib/site-packages/cmake/data/bin/cmake.exe', 'cmake version 3.31.6'),
               ('ninja', 'ninja-1.11.1.4.data/scripts/ninja.exe', 'Scripts/ninja.exe', '1.11.1.git.kitware.jobserver-1')]
    paths, details = [], []
    for name, member, installed_relative, expected_version in layouts:
        item = next(w for w in lock['build_wheels'] if w['name'] == name)
        path = verified_tool_payload(runtime, wheelhouse / item['filename'], member, installed_relative)
        result = subprocess.run([str(path), '--version'], cwd=work, env=env, check=True,
                                capture_output=True, text=True, timeout=30)
        if not result.stdout.splitlines() or result.stdout.splitlines()[0] != expected_version:
            raise ValueError('Pinned build executable reports an unexpected version: ' + name)
        paths.append(path)
        details.append({'name': name, 'installed_path': installed_relative, 'wheel_member': member,
                        'sha256': digest(path), 'version': expected_version, 'source_wheel_sha256': item['sha256']})
    dump(reports / 'build-tool-payloads.json', details)
    return tuple(paths)


def notice_files():
    lock = load(HERE / 'notices.lock.json'); result = {}
    for item in lock['files']:
        path = HERE.joinpath(*safe_path(item['path']).parts)
        if path.stat().st_size != item['bytes'] or digest(path) != item['sha256']:
            raise ValueError('Native notice does not match reviewed source')
        result[item['path']] = path.read_bytes()
    return lock, result


def canonical_wheel(original, destination, provenance, runtime):
    """Add notices/build provenance, remove GPU classifier, and normalize ZIP/RECORD.

    No executable payload is rewritten, removed or swapped.
    """
    _, notices = notice_files()
    with zipfile.ZipFile(original) as archive:
        entries = {}
        for item in archive.infolist():
            if item.is_dir():
                continue
            safe_path(item.filename)
            if item.filename in entries:
                raise ValueError('Duplicate wheel entry')
            entries[item.filename] = archive.read(item)
    info = 'ctranslate2-4.8.2.dist-info'
    if info + '/METADATA' not in entries or info + '/WHEEL' not in entries:
        raise ValueError('Missing extension wheel metadata')
    metadata = email.message_from_bytes(entries[info + '/METADATA'])
    wheel_metadata = email.message_from_bytes(entries[info + '/WHEEL'])
    if (metadata.get_all('Name'), metadata.get_all('Version'), wheel_metadata.get_all('Build'), wheel_metadata.get_all('Tag')) != (['ctranslate2'], ['4.8.2'], ['1lumacpu'], ['cp312-cp312-win_amd64']):
        raise ValueError('Unexpected extension wheel identity')
    entries[info + '/METADATA'] = b'\n'.join(line for line in entries[info + '/METADATA'].split(b'\n') if not line.startswith(b'Classifier: Environment :: GPU'))
    for name, value in notices.items():
        entries[info + '/' + name] = value
    # pybind11 is a header dependency, not merely a disposable build tool.
    candidates = list((runtime / 'Lib/site-packages/pybind11-2.11.1.dist-info').rglob('LICENSE*'))
    if len(candidates) != 1:
        raise ValueError('Pinned pybind11 BSD notice is missing')
    pybind_notice = candidates[0].read_bytes()
    expected = next(i for i in load(HERE / 'notices.lock.json')['files'] if i['component'] == 'pybind11')
    if hashlib.sha256(pybind_notice).hexdigest() != expected['sha256']:
        raise ValueError('Built pybind11 header license changed')
    entries[info + '/LUMA_CPU_BUILD.json'] = (json.dumps(provenance, indent=2, sort_keys=True) + '\n').encode()
    record = info + '/RECORD'; entries.pop(record, None)
    rows = []
    for name, value in sorted(entries.items()):
        hashed = base64.urlsafe_b64encode(hashlib.sha256(value).digest()).rstrip(b'=').decode()
        rows.append((name, 'sha256=' + hashed, str(len(value))))
    rows.append((record, '', ''))
    out = io.StringIO(newline=''); csv.writer(out, lineterminator='\n').writerows(rows)
    entries[record] = out.getvalue().encode()
    with zipfile.ZipFile(destination, 'w', compression=zipfile.ZIP_STORED) as archive:
        for name, value in sorted(entries.items()):
            item = zipfile.ZipInfo(name, (2026, 1, 1, 0, 0, 0)); item.create_system = 3
            item.external_attr = 0o100644 << 16
            archive.writestr(item, value)


def validate_source_version(source):
    """Read the pinned module's literal version without executing upstream code."""
    try:
        statements = ast.parse(source).body
    except SyntaxError as error:
        raise ValueError('Source version module is not valid Python') from error
    if statements and isinstance(statements[0], ast.Expr) and isinstance(statements[0].value, ast.Constant) and isinstance(statements[0].value.value, str):
        statements = statements[1:]  # An optional module docstring is metadata.
    if len(statements) != 1 or not isinstance(statements[0], ast.Assign):
        raise ValueError('Source version module must contain exactly one literal assignment')
    assignment = statements[0]
    if (len(assignment.targets) != 1 or not isinstance(assignment.targets[0], ast.Name)
            or assignment.targets[0].id != '__version__' or not isinstance(assignment.value, ast.Constant)
            or assignment.value.value != '4.8.2'):
        raise ValueError('Source version is not literal 4.8.2')


def deterministic_environment(work, env, lock):
    mapped_root = r'C:\luma-ct2-build'
    return dict(env, SOURCE_DATE_EPOCH=str(lock['source_date_epoch']),
                _CL_=f'/Brepro /Z7 /experimental:deterministic /pathmap:{work}={mapped_root}',
                _LINK_='/Brepro /INCREMENTAL:NO', CTRANSLATE2_ROOT=str(work / 'ct2-install'))


def probe_path_mapping(args, env, lock):
    work = args.work / 'compiler-probe'; work.mkdir()
    source = work / 'probe.cpp'
    source.write_text('const char* source_file = __FILE__;\n', encoding='ascii')
    output = args.reports / 'compiler-probe.log'
    command(['cl', '/nologo', '/EP', source], cwd=work,
            env=deterministic_environment(work, env, lock), logfile=output, timeout=30)
    text = output.read_text(encoding='utf-8', errors='replace')
    expected = json.dumps(r'C:\luma-ct2-build\probe.cpp')
    if expected not in text or re.search(r'warning D(?:9002|9007)', text):
        raise RuntimeError('Pinned compiler did not apply the required deterministic path mapping')
    dump(args.reports / 'compiler-probe.json', {'schema': 1, 'passed': True,
         'mapped_source': r'C:\luma-ct2-build\probe.cpp', 'output_sha256': digest(output),
         'compiler_flags': '/Brepro /Z7 /experimental:deterministic /pathmap:<build-root>=C:\\luma-ct2-build'})


def enforce_independent_proofs(status, functional_proof):
    error = None
    try:
        functional_proof()
        status['native_inference_passed'] = True
    except Exception as failure:
        error = failure
        status['functional_error'] = f'{type(failure).__name__}: {failure}'
    if not status['wheel_reproduced']:
        status['reproducibility_error'] = 'Independent builds differ; reproducibility has not been established'
        raise RuntimeError(status['reproducibility_error'])
    if error is not None:
        raise error


def build_once(number, args, lock, runtime, cmake, ninja, env, provenance):
    work = args.work / f'build-{number}'; work.mkdir()
    sources = work / 'sources'; sources.mkdir()
    for item in lock['sources']:
        destination = (sources / 'ctranslate2/third_party' / item['name']) if item['name'] in {'cpu_features', 'spdlog'} else sources / item['name']
        extract_source(fetch(item, args.cache), destination)
    ct = sources / 'ctranslate2'; dn = sources / 'onednn'
    validate_source_version((ct / 'python/ctranslate2/version.py').read_text(encoding='utf-8'))
    local_env = deterministic_environment(work, env, lock)
    common = ['-G', 'Ninja', '-DCMAKE_BUILD_TYPE=Release', f'-DCMAKE_MAKE_PROGRAM={ninja}',
              '-DCMAKE_C_COMPILER=cl', '-DCMAKE_CXX_COMPILER=cl', '-DCMAKE_POLICY_DEFAULT_CMP0091=NEW',
              '-DCMAKE_SHARED_LINKER_FLAGS=/Brepro /INCREMENTAL:NO', '-DCMAKE_EXE_LINKER_FLAGS=/Brepro /INCREMENTAL:NO']
    log = args.reports / f'build-{number}.log'
    for name, source, options in [('onednn', dn, lock['onednn_cmake']), ('ct2', ct, lock['ct2_cmake'])]:
        build_dir, install = work / (name + '-build'), work / (name + '-install')
        flags = [f'-D{k}={v}' for k, v in sorted(options.items())]
        if name == 'ct2':
            flags += [f'-DDNNL_INCLUDE_DIR={work / "onednn-install/include"}', f'-DDNNL_LIBRARY={work / "onednn-install/lib/dnnl.lib"}']
        command([cmake, '-S', source, '-B', build_dir, *common, *flags, f'-DCMAKE_INSTALL_PREFIX={install}'], cwd=work, env=local_env, logfile=log)
        command([cmake, '--build', build_dir, '--parallel', '3'], cwd=work, env=local_env, logfile=log, timeout=3600)
        command([cmake, '--install', build_dir], cwd=work, env=local_env, logfile=log)
        # These cache reports expose actual values, including discovered libraries.
        shutil.copyfile(build_dir / 'CMakeCache.txt', args.reports / f'build-{number}-{name}-CMakeCache.txt')
    dll = work / 'ct2-install/bin/ctranslate2.dll'
    if not dll.is_file():
        raise ValueError('Upstream CT2 install did not produce its supported shared DLL')
    shutil.copyfile(dll, ct / 'python/ctranslate2/ctranslate2.dll')
    command([runtime / 'python.exe', '-I', '-B', 'setup.py', 'bdist_wheel', '--build-number', lock['wheel_build_tag']],
            cwd=ct / 'python', env=local_env, logfile=log, timeout=1200)
    wheels = list((ct / 'python/dist').glob('*.whl'))
    expected = 'ctranslate2-4.8.2-1lumacpu-cp312-cp312-win_amd64.whl'
    if len(wheels) != 1 or wheels[0].name != expected:
        raise ValueError('Unexpected wheel build tag')
    destination = work / expected
    canonical_wheel(wheels[0], destination, provenance, runtime)
    # Inventory CT2's payload independently of the surrounding Python runtime.
    payload = work / 'wheel-payload'; payload.mkdir()
    with zipfile.ZipFile(destination) as archive:
        for item in archive.infolist():
            if Path(item.filename).suffix.lower() not in {'.dll', '.pyd', '.exe'}:
                continue
            target = payload.joinpath(*safe_path(item.filename).parts)
            target.parent.mkdir(parents=True, exist_ok=True); target.write_bytes(archive.read(item))
    native = closure(inventory(payload))
    if {i['path'] for i in native['files']} != {'ctranslate2/ctranslate2.dll', 'ctranslate2/_ext.cp312-win_amd64.pyd'} or any(i['machine'] != '0x8664' for i in native['files']):
        raise ValueError('Unexpected CPU wheel native payload')
    dump(args.reports / f'wheel-{number}-native.json', native)
    if native['forbidden_files'] or any(d['resolution'] == 'forbidden' for d in native['dependencies']):
        raise ValueError('CPU wheel contains/imports a forbidden runtime')
    return destination


def private_proof(args, lock, wheel, runtime_archive, env):
    root = args.work / 'fresh-runtime'
    unpack_runtime(runtime_archive, root)
    upstream = load(COMPONENTS / 'locks/faster-whisper-cpu-windows-x64.json')
    selected = []
    wheelhouse = args.work / 'runtime-wheelhouse'; wheelhouse.mkdir()
    for item in upstream['wheels']:
        if item['name'] == 'ctranslate2':
            item = {'name': 'ctranslate2', 'version': '4.8.2', 'filename': wheel.name,
                    'bytes': wheel.stat().st_size, 'sha256': digest(wheel),
                    'url': 'local-proof-only:' + wheel.name}
            source = wheel
        else:
            source = fetch(item, args.cache)
        selected.append(item); shutil.copyfile(source, wheelhouse / item['filename'])
    if sum(w['name'] == 'ctranslate2' for w in selected) != 1:
        raise ValueError('Runtime lock must contain exactly one CPU CT2 variant')
    runtime_lock = dict(upstream, wheels=selected)
    runtime_lock_path = args.work / 'runtime-wheel-lock.json'; dump(runtime_lock_path, runtime_lock)
    command([root / 'python.exe', '-I', '-S', '-B', '-X', 'utf8', COMPONENTS / 'assemble.py', '--runtime-root', root,
             '--wheel-lock', runtime_lock_path, '--wheelhouse', wheelhouse], cwd=args.work, env=env,
            logfile=args.reports / 'assembly.log', timeout=600)
    copy_proof_crt(root, args.reports)
    relocated = args.work / 'Relocated private Python é 测试'; root.rename(relocated); root = relocated
    native = closure(inventory(root)); dump(args.reports / 'whole-runtime-native.json', native)
    if not native['passed']:
        missing = sorted({x['name'] for x in native['blocked_dependencies']})
        raise RuntimeError('Clean-runtime dependency gate blocked; no host-global DLL fallback: ' + ', '.join(missing))
    fixture = load(COMPONENTS / 'fixtures.json')
    model = args.work / 'model'; model.mkdir()
    for item in fixture['faster_whisper_tiny']['files']:
        target = model.joinpath(*safe_path(item['path']).parts); target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(fetch(item, args.cache), target)
    audio = args.work / 'jfk.wav'; shutil.copyfile(fetch(fixture['audio'], args.cache), audio)
    home = args.work / 'empty-home'; home.mkdir()
    clean = {k: v for k, v in os.environ.items() if k.upper() in {'SYSTEMROOT', 'WINDIR', 'COMSPEC', 'SYSTEMDRIVE'}}
    clean.update(HOME=str(home), USERPROFILE=str(home), TEMP=str(home), TMP=str(home), APPDATA=str(home / 'AppData'),
                 LOCALAPPDATA=str(home / 'Local'), PATH=str(home / 'no-executables'),
                 PYTHONHOME=str(home / 'invalid-python'), PYTHONPATH=str(home / 'poison'),
                 HF_HOME=str(home / 'empty-hf-cache'), HF_HUB_OFFLINE='1', TRANSFORMERS_OFFLINE='1',
                 HF_HUB_DISABLE_TELEMETRY='1', HF_HUB_DISABLE_IMPLICIT_TOKEN='1', DO_NOT_TRACK='1')
    poison = home / 'poison'; poison.mkdir()
    (poison / 'sitecustomize.py').write_text("raise RuntimeError('Inherited Python path was used')\n", encoding='utf-8')
    command([root / 'python.exe', '-I', '-B', '-X', 'utf8', HERE / 'verify_runtime.py',
             '--root', root, '--model', model, '--audio', audio, '--worker', args.worker,
             '--report', args.reports / 'inference.json'], cwd=home, env=clean,
            logfile=args.reports / 'inference.log', timeout=900)


def main():
    parser = argparse.ArgumentParser()
    for field in ('work', 'reports', 'cache', 'worker'):
        parser.add_argument('--' + field, type=Path, required=True)
    parser.add_argument('--source-sha', required=True)
    args = parser.parse_args()
    for field in ('work', 'reports', 'cache', 'worker'):
        setattr(args, field, getattr(args, field).resolve())
    args.reports.mkdir(parents=True, exist_ok=True)
    status = {'schema': 1, 'source_sha': args.source_sha, 'publication_authorized': False,
              'wheel_reproduced': False, 'native_inference_passed': False,
              'limitations': ['Sequential oneDNN GEMMs may be slower than upstream MKL/OpenMP builds.',
                             'No CPU/GPU performance claim; no clean GUI-machine or non-AVX hardware claim.',
                             'Hosted runner OS is not hermetically pinned; tool versions and hashes are reported.']}
    try:
        if sys.platform != 'win32' or sys.maxsize <= 2**32:
            raise RuntimeError('Native Windows x64 build required')
        if sys.version_info[:3] != (3, 12, 10):
            raise RuntimeError('The build orchestrator must be exact CPython 3.12.10')
        status['orchestrator_python'] = sys.version
        if not re.fullmatch('[0-9a-f]{40}', args.source_sha):
            raise ValueError('Exact reviewed source SHA required')
        actual = subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=COMPONENTS, text=True).strip()
        if actual != args.source_sha:
            raise ValueError('Checked-out source does not match the requested commit')
        lock = load(HERE / 'sources.lock.json'); validate_lock(lock)
        notices, _ = notice_files()
        if notices.get('notice_inputs_complete') is not True:
            raise RuntimeError('Native notice inputs are incomplete: ' + '; '.join(notices.get('blockers', [])))
        toolchain = load(args.reports / 'toolchain.json')
        if (toolchain['arch'], toolchain['host_arch']) != ('x64', 'x64'):
            raise RuntimeError('Native x64 host and target compiler required')
        for field in ('visual_studio_version', 'vc_tools_version', 'windows_sdk_version'):
            if toolchain[field] != lock['toolchain'][field]:
                raise RuntimeError('Build tool version changed; review the inventory before repinning: ' + field)
        installed_licenses = discover_installed_licenses(toolchain['installation_path'], toolchain['redist_path'], toolchain['product_id'], toolchain['visual_studio_version'])
        dump(args.reports / 'installed-license-evidence.json', installed_licenses)
        if args.work.exists():
            raise ValueError('Build needs a new, empty work directory')
        args.work.mkdir(parents=True)
        cfg = load(COMPONENTS / 'packs.json')
        if (cfg['runtime']['version'], cfg['runtime']['vendor_release']) != ('3.12.15', '20261003'):
            raise ValueError('Unreviewed PBS runtime')
        runtime_item = cfg['runtime']['windows-x64']
        runtime_archive = fetch(runtime_item, args.cache)
        runtime = args.work / 'build-python'; unpack_runtime(runtime_archive, runtime)
        if not (runtime / 'libs/python312.lib').is_file() or not (runtime / 'include/Python.h').is_file():
            raise RuntimeError('Pinned PBS archive lacks the Python development inputs')
        env = build_environment(runtime)
        cmake, ninja = install_build_tools(runtime, lock, args.cache, args.work, env, args.reports)
        probe_path_mapping(args, env, lock)
        provenance = {'schema': 1, 'variant': lock['variant'], 'luma_source_sha': args.source_sha,
                      'upstream': lock['sources'], 'ct2_cmake': lock['ct2_cmake'], 'onednn_cmake': lock['onednn_cmake'],
                      'build_wheels': lock['build_wheels'], 'python_runtime': runtime_item,
                      'source_date_epoch': lock['source_date_epoch'], 'toolchain': toolchain,
                      'native_compile_flags': '/Brepro /Z7 /experimental:deterministic /pathmap:<build-root>=C:\\luma-ct2-build',
                      'native_link_flags': '/Brepro /INCREMENTAL:NO',
                      'notices': load(HERE / 'notices.lock.json'), 'publication_authorized': False,
                      'packaging_changes': ['PE executable payloads are not modified.', 'Wheel build tag: 1lumacpu.',
                                            'Remove GPU classifier; add native licenses and provenance; normalize ZIP and RECORD.']}
        # Build-local paths and image diagnostics are evidence, not wheel inputs.
        provenance['toolchain'] = {k: v for k, v in toolchain.items() if k in {'visual_studio_version', 'vc_tools_version', 'windows_sdk_version', 'compiler_version', 'binary_hashes', 'product_id'}}
        dump(args.reports / 'provenance.json', provenance)
        first = build_once(1, args, lock, runtime, cmake, ninja, env, provenance)
        status['wheel'] = {'filename': first.name, 'bytes': first.stat().st_size, 'sha256': digest(first)}
        second = build_once(2, args, lock, runtime, cmake, ninja, env, provenance)
        status['second_sha256'] = digest(second)
        comparison = compare_wheels(first, second)
        dump(args.reports / 'wheel-comparison.json', comparison)
        status['wheel_reproduced'] = comparison['identical']
        status['different_wheel_members'] = comparison['different_members']
        # Functional diagnostics are independent: a repeatability mismatch must
        # not suppress first-wheel closure/import/Tiny evidence. Final failure is
        # retained even if that exact first wheel works correctly.
        enforce_independent_proofs(status, lambda: private_proof(args, lock, first, runtime_archive, env))
        status['passed'] = True
    except BaseException as error:
        status.update(passed=False, error=f'{type(error).__name__}: {error}')
        raise
    finally:
        dump(args.reports / 'result.json', status)
        print(json.dumps(status, indent=2, ensure_ascii=False), flush=True)


if __name__ == '__main__':
    main()
