"""Native CI assembly using the same exact direct-Microsoft CRT contract."""
import hashlib
import base64
from contextlib import contextmanager
import ctypes
import importlib.util
import json
import os
from pathlib import Path, PureWindowsPath
import shutil
import subprocess
import struct
import sys
import tempfile
import time

ROOT = Path(__file__).resolve().parent
CONTRACT = ROOT / 'ct2-cpu/direct-crt/package.lock.json'
ID = 'msvc-14.44.35211-x64'
COMPANIONS = {
    'vcruntime140.dll': (124544, 'd5e4d9a3e835fa679450145d6a7d94e36573a509317111904d9b3712c30d9066'),
    'vcruntime140_1.dll': (49792, '1f2d41c4aa5db0bc33ebf7b66d72943a817d7ce6cbe880502a9403823633093f'),
}
SIGNERS = {'installer': 'e4ab39116a7dc57d073164eb1c840b1fb8334a8c920b92efafea19112dce643b',
           'msvcp140.dll': '2ebcd329b745ae0efb6a72bb8a471b29c3994b074dbf94bdc2bb65936c54f4a7',
           'msvcp140_1.dll': '2ebcd329b745ae0efb6a72bb8a471b29c3994b074dbf94bdc2bb65936c54f4a7'}


def verified(path, size, digest):
    if path.is_symlink() or not path.is_file() or path.stat().st_size != size:
        raise ValueError('Direct CRT input is not the expected regular file')
    with path.open('rb') as stream:
        if hashlib.file_digest(stream, 'sha256').hexdigest() != digest:
            raise ValueError('Direct CRT input differs from its exact hash')


def prepare(output, cache, source_sha):
    if sys.platform != 'win32': raise ValueError('Direct CRT proof is native Windows only')
    output.mkdir(parents=True, exist_ok=True); cache.mkdir(parents=True, exist_ok=True)
    work = output / 'direct-crt-input'; manifest = output / 'direct-crt-extraction.json'
    signatures = output / 'direct-crt-signatures.json'
    # Reuse the sole reviewed Python extractor. It never executes the installer.
    subprocess.run([sys.executable, '-B', str(ROOT / 'ct2-cpu/direct-crt/audit_direct_crt.py'),
                    '--work-dir', str(work), '--report', str(manifest)], check=True, timeout=600)
    pwsh = shutil.which('pwsh')
    if not pwsh: raise RuntimeError('Native Microsoft signature verification requires runner PowerShell7')
    subprocess.run([pwsh, '-NoLogo', '-NoProfile', '-NonInteractive', '-File',
                    str(ROOT / 'ct2-cpu/verify_direct_crt_signatures.ps1'), '-Manifest', str(manifest),
                    '-WorkRoot', str(work), '-Report', str(signatures)],
                   env=dict(os.environ, SOURCE_SHA=source_sha), check=True, timeout=300)
    lock = json.loads(CONTRACT.read_text(encoding='utf-8'))
    sig = json.loads(signatures.read_text(encoding='utf-8-sig'))
    if sig.get('passed') is not True or sig.get('source_sha') != source_sha or sig.get('source_url') != lock['source_url']:
        raise ValueError('Direct CRT signature evidence is incomplete')
    if sorted(item['role'] for item in sig['files']) != sorted(SIGNERS):
        raise ValueError('Unexpected signed CRT role set')
    for item in sig['files']:
        if item['signature_status'] != 'Valid' or item['signer_certificate_der_sha256'] != SIGNERS[item['role']] or item['file_version'] != lock['version']:
            raise ValueError('Direct CRT signer/version differs from the exact reviewed certificate')
    installer = work / 'VC_redist.x64.exe'; pin = lock['installer']
    verified(installer, pin['bytes'], pin['sha256'])
    target = cache / pin['sha256']
    if target.exists(): verified(target, pin['bytes'], pin['sha256'])
    else: shutil.copyfile(installer, target)
    prove_cab_paths(work, output, source_sha)
    return work


def install(runtime, source):
    lock = json.loads(CONTRACT.read_text(encoding='utf-8'))
    signatures = json.loads((source.parent / 'direct-crt-signatures.json').read_text(encoding='utf-8-sig'))
    if signatures.get('passed') is not True: raise ValueError('Direct CRT signatures were not verified')
    # Check every source and existing same-version PBS companion before writing.
    for name, (size, digest) in COMPANIONS.items(): verified(runtime / name, size, digest)
    for name, item in {**lock['dlls'], **lock['notices']}.items(): verified(source / name, item['bytes'], item['sha256'])
    for name in lock['dlls']:
        if (runtime / name).exists(): raise ValueError('Refusing to overwrite a private CRT library')
    for name in lock['dlls']:
        with (runtime / name).open('xb') as output: output.write((source / name).read_bytes())
    notices = runtime / 'licenses' / ID; notices.mkdir(parents=True)
    for name in lock['notices']:
        with (notices / name).open('xb') as output: output.write((source / name).read_bytes())
    # No absolute staging path, timestamp or host environment enters the runtime.
    evidence = {'schema': 1, 'contract_id': ID, 'source_url': lock['source_url'],
                'contract_sha256': hashlib.sha256(CONTRACT.read_bytes()).hexdigest(),
                'installer_sha256': lock['installer']['sha256'], 'installer_executed': False,
                'global_installation': False, 'original_payloads_unmodified': True,
                'dlls': lock['dlls'], 'notices': lock['notices'],
                'signature_verified': True, 'signer_certificate_der_sha256': SIGNERS,
                'scope': 'CI private assembly; production installer independently verifies and extracts the same package'}
    (notices / 'provenance.json').write_text(json.dumps(evidence, sort_keys=True, indent=2) + '\n', encoding='utf-8', newline='\n')


def signature_result(stdout, role, mode, lock):
    prefix = 'LUMA_CRT_SIGNATURE '
    lines = [line for line in stdout.splitlines() if line.strip()]
    if len(lines) != 1 or not lines[0].startswith(prefix):
        raise ValueError('Real app helper did not emit exactly one bounded signature record')
    result = json.loads(lines[0][len(prefix):])
    expected = {'schema': 1, 'role': role, 'mode': mode, 'sha256': lock['installer']['sha256'],
                'signer_sha256': SIGNERS['installer']}
    if result != expected: raise ValueError('Real app helper returned an unexpected signature identity')
    return result


def gui_subsystem(path):
    with path.open('rb') as stream: data = stream.read(4096)
    if data[:2] != b'MZ' or len(data) < 64: raise ValueError('Built app is not a PE image')
    pe = struct.unpack_from('<I', data, 0x3c)[0]
    if pe + 94 > len(data) or data[pe:pe+4] != b'PE\0\0': raise ValueError('Invalid built app PE header')
    if struct.unpack_from('<H', data, pe + 4)[0] != 0x8664: raise ValueError('Built app is not Windows x64')
    subsystem = struct.unpack_from('<H', data, pe + 24 + 68)[0]
    if subsystem != 2: raise ValueError('Real app helper proof requires the Windows GUI subsystem')
    return subsystem


def helper_process_bytes(command, env, cwd):
    # Redirect inherited handles as the shipping app does. File-backed bounded
    # output avoids detached reader threads and unbounded capture buffers.
    with tempfile.TemporaryFile(dir=cwd) as stdout, tempfile.TemporaryFile(dir=cwd) as stderr:
        child = subprocess.Popen(command, stdin=subprocess.DEVNULL, stdout=stdout, stderr=stderr, env=env, cwd=cwd)
        started = time.monotonic()
        try:
            while child.poll() is None:
                if time.monotonic() - started > 120: raise TimeoutError('Real CRT app helper timed out')
                if os.fstat(stdout.fileno()).st_size + os.fstat(stderr.fileno()).st_size > 16_384:
                    raise ValueError('Real CRT app helper exceeded its output bound')
                time.sleep(0.05)
            child.wait(); stdout.seek(0); stderr.seek(0)
            out, err = stdout.read(16_385), stderr.read(16_385)
            if len(out) + len(err) > 16_384: raise ValueError('Real CRT app helper exceeded its output bound')
            return child.returncode, out, err
        except BaseException:
            if child.poll() is None: child.kill()
            child.wait()
            raise


def helper_process(command, env, cwd):
    code, out, err = helper_process_bytes(command, env, cwd)
    return code, out.decode('utf-8'), err.decode('utf-8')


def verbatim_disk_path(path):
    path = PureWindowsPath(path)
    drive = path.drive.removeprefix('\\\\?\\')
    if not path.is_absolute() or len(drive) != 2 or drive[1] != ':' or not drive[0].isascii() or not drive[0].isalpha():
        raise ValueError('CAB path proof requires a local absolute disk path')
    if '..' in path.parts: raise ValueError('CAB path proof rejects parent traversal')
    return Path(str(path) if str(path).startswith('\\\\?\\') else '\\\\?\\' + str(path))


@contextmanager
def held_cabinet(path):
    # Match production FILE_SHARE_READ: keep the exact verified CAB immutable
    # while the existing system expand.exe reads it. No policy/ACL changes.
    from ctypes import wintypes
    kernel = ctypes.WinDLL('kernel32', use_last_error=True)
    create = kernel.CreateFileW
    create.argtypes = (wintypes.LPCWSTR, wintypes.DWORD, wintypes.DWORD, ctypes.c_void_p, wintypes.DWORD, wintypes.DWORD, wintypes.HANDLE)
    create.restype = wintypes.HANDLE
    close = kernel.CloseHandle; close.argtypes = (wintypes.HANDLE,); close.restype = wintypes.BOOL
    handle = create(str(path), 0x80000000, 1, None, 3, 0x80, None)
    if handle == ctypes.c_void_p(-1).value: raise ctypes.WinError(ctypes.get_last_error())
    try: yield
    finally: close(handle)


def cab_environment(work, temporary, system_directory):
    kept = {'SYSTEMROOT', 'WINDIR', 'COMSPEC', 'SYSTEMDRIVE', 'PROCESSOR_ARCHITECTURE', 'PROCESSOR_ARCHITEW6432', 'NUMBER_OF_PROCESSORS'}
    env = {key: value for key, value in os.environ.items() if key.upper() in kept}
    env['PATH'] = ';'.join(map(str, (work, work / 'bin', system_directory)))
    for key in ('HOME', 'USERPROFILE', 'APPDATA', 'LOCALAPPDATA', 'TMP', 'TEMP', 'TMPDIR'): env[key] = str(temporary)
    env.update(HF_HUB_OFFLINE='1', TRANSFORMERS_OFFLINE='1', HF_DATASETS_OFFLINE='1',
               HF_HUB_DISABLE_IMPLICIT_TOKEN='1', HF_HUB_DISABLE_TELEMETRY='1', DO_NOT_TRACK='1', LANG='C.UTF-8')
    return env


def prove_cab_paths(source, output, source_sha):
    """Diagnostic only: compare the old argv with fixed names under a Unicode cwd.

    The Rust managed installer remains the required production extraction gate.
    A legacy invocation may fail; every fixed-name output must match its pin.
    """
    if sys.platform != 'win32': raise ValueError('CAB path proof requires native Windows')
    spec = importlib.util.spec_from_file_location('luma_crt_audit', ROOT / 'ct2-cpu/direct-crt/audit_direct_crt.py')
    audit = importlib.util.module_from_spec(spec); spec.loader.exec_module(audit)
    expand = audit.system_expand(); lock = json.loads(CONTRACT.read_text(encoding='utf-8'))
    results = []; parent_cwd = Path.cwd()
    with tempfile.TemporaryDirectory(prefix='CRT CAB paths é 测试 ', dir=output) as tmp:
        work = verbatim_disk_path(Path(tmp).resolve(strict=True))
        temporary = work / 'private-temp'; temporary.mkdir()
        env = cab_environment(work, temporary, expand.parent)
        for name, pin in lock['containers'].items():
            verified(source / name, pin['bytes'], pin['sha256']); shutil.copyfile(source / name, work / name)
        cases = [('legacy-absolute-verbatim', 'attached.cab', lock['containers']['attached.cab'], lock['minimum_cab']),
                 ('fixed-relative', 'attached.cab', lock['containers']['attached.cab'], lock['minimum_cab'])]
        cases += [('fixed-relative', 'minimum-x64.cab', lock['minimum_cab'], pin) for pin in lock['dlls'].values()]
        cases += [('fixed-relative', 'ux.cab', lock['containers']['ux.cab'], pin) for pin in lock['notices'].values()]
        for form, cabinet_name, cabinet_pin, pin in cases:
            cabinet = work / cabinet_name; member = pin['member']
            directory_name = ('legacy-' if form.startswith('legacy-') else 'extract-') + member
            destination = work / directory_name; destination.mkdir()
            args = [str(cabinet), '-F:' + member, str(destination)] if form.startswith('legacy-') else [cabinet_name, '-F:' + member, directory_name]
            with held_cabinet(cabinet):
                verified(cabinet, cabinet_pin['bytes'], cabinet_pin['sha256'])
                code, out, err = helper_process_bytes([str(expand), *args], env, work)
            result = {'argument_form': form, 'cabinet': cabinet_name, 'member': member, 'exit_code': code,
                      'stdout': out.decode('utf-8', errors='replace'), 'stderr': err.decode('utf-8', errors='replace'),
                      'stdout_base64': base64.b64encode(out).decode('ascii'), 'stderr_base64': base64.b64encode(err).decode('ascii')}
            # Emit exact bounded status/bytes even if the required path later
            # fails; successful legacy behavior is evidence too, never assumed.
            print('LUMA_CRT_CAB_PATH ' + json.dumps(result, ensure_ascii=True), flush=True)
            results.append(result)
            if code:
                if form.startswith('legacy-'): continue
                raise RuntimeError(f'Fixed relative CAB extraction failed for {member}: exit {code}; see LUMA_CRT_CAB_PATH')
            if sorted(item.name for item in destination.iterdir()) != [member]: raise ValueError('CAB path proof produced unexpected outputs')
            verified(destination / member, pin['bytes'], pin['sha256']); result['verified_sha256'] = pin['sha256']
            if form == 'fixed-relative' and member == lock['minimum_cab']['member']:
                (destination / member).rename(work / 'minimum-x64.cab')
        if Path.cwd() != parent_cwd: raise AssertionError('CAB path proof changed the parent working directory')
    report = {'schema': 1, 'source_sha': source_sha, 'passed': True, 'results': results,
              'unicode_verbatim_cwd': True, 'input_held_read_only': True, 'installer_executed': False,
              'scope': 'Diagnostic only; the independent Rust managed install/repair/remove gate is still required'}
    (output / 'direct-crt-cab-paths.json').write_text(json.dumps(report, indent=2) + '\n', encoding='utf-8', newline='\n')
    return report


def prove_app_helper(cache, output):
    """Exercise main.rs early dispatch, not the cfg(test) child executable."""
    if sys.platform != 'win32': raise ValueError('Real CRT app helper proof requires Windows')
    from smoke import clean_environment
    repo = ROOT.parent.parent; manifest = repo / 'src-tauri/Cargo.toml'
    subprocess.run(['cargo', 'rustc', '--locked', '--manifest-path', str(manifest), '--bin', 'luma-subtitle',
                    '--', '-C', 'debug-assertions=no'], check=True, timeout=1800)
    target = Path(os.environ.get('CARGO_TARGET_DIR', repo / 'src-tauri/target')).resolve()
    app = target / 'debug/luma-subtitle.exe'
    if app.is_symlink() or not app.is_file(): raise ValueError('Built app executable is missing')
    subsystem = gui_subsystem(app)
    lock = json.loads(CONTRACT.read_text(encoding='utf-8')); package = cache / lock['installer']['sha256']
    verified(package, lock['installer']['bytes'], lock['installer']['sha256'])
    results = {}
    with tempfile.TemporaryDirectory(prefix='Real app helper é 测试 ', dir=output) as tmp:
        work = Path(tmp); env = clean_environment(work / 'home'); env.update(TEMP=str(work), TMP=str(work))
        relocated_app = work / 'luma-subtitle.exe'; shutil.copyfile(app, relocated_app)
        relocated_package = work / package.name; shutil.copyfile(package, relocated_package)
        verified(relocated_package, lock['installer']['bytes'], lock['installer']['sha256'])
        for mode in ('online', 'cache-only'):
            code, stdout, stderr = helper_process([str(relocated_app), '--luma-verify-crt', mode, 'installer', str(relocated_package)], env, work)
            if code:
                raise RuntimeError(f'Real app helper {mode} failed ({code}): {stdout[-2000:]} {stderr[-1000:]}')
            results[mode] = signature_result(stdout, 'installer', mode, lock)
        for label, role, path in (('invalid-role', 'unreviewed-role', relocated_package), ('invalid-path', 'installer', work / 'not-approved.exe')):
            code, stdout, _stderr = helper_process([str(relocated_app), '--luma-verify-crt', 'cache-only', role, str(path)], env, work)
            if code == 0 or 'LUMA_CRT_SIGNATURE ' not in stdout:
                raise AssertionError('Real app helper failed to reject ' + label)
            results[label] = {'rejected': True, 'exit_code': code}
    return {'tested': True, 'passed': True, 'entrypoint': 'main.rs early --luma-verify-crt dispatch',
            'app_sha256': hashlib.sha256(app.read_bytes()).hexdigest(), 'results': results,
            'pe_subsystem': subsystem, 'build_scope': 'Actual main.rs GUI-subsystem helper branch; not a full release or bundle build',
            'unicode_private_cwd': True, 'poisoned_path': True, 'installer_executed': False,
            'ui_launched': False, 'trust_cache_manually_modified': False,
            'normal_platform_revocation_cache_behavior': True}
