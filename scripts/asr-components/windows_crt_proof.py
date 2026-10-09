"""Native CI assembly using the same exact direct-Microsoft CRT contract."""
import hashlib
import json
import os
from pathlib import Path
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


def helper_process(command, env, cwd):
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
            return child.returncode, out.decode('utf-8'), err.decode('utf-8')
        except BaseException:
            if child.poll() is None: child.kill()
            child.wait()
            raise


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
