"""Cache-only bridge from immutable shipping pins to the existing CPU proof.

No release discovery, downloads, placeholder pins or catalog activation. The
shipping wheel lock must be updated only from independently verified published
assets. Source-build proofs continue to use ct2-cpu/build_cpu.py unchanged.
"""
from __future__ import annotations
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import re
import stat
import struct
import subprocess
import zipfile

ROOT = Path(__file__).resolve().parent
VALIDATOR = ROOT / 'validate_cpu_recipe_proof.cjs'
PACK_ID = 'faster-whisper-cpu-windows-x64'
PREFIX = 'https://github.com/csic21/luma-subtitle/releases/download/asr-ct2-cpu-4.8.2-1/'
WHEEL = 'ctranslate2-4.8.2-1lumacpu-cp312-cp312-win_amd64.whl'
SOURCES = 'luma-ct2-cpu-4.8.2-1-sources.zip'
NOTICES = 'luma-ct2-cpu-4.8.2-1-notices.zip'
PROOF = 'publication-proof.json'
MAX_ASSET = 100_000_000
MAX_PROOF = 4_000_000
IDENTITY_FIELDS = ('name', 'version', 'filename', 'url', 'bytes', 'sha256')


def sha(data):
    return hashlib.sha256(data).hexdigest()


def pin_valid(item, limit=MAX_ASSET):
    return (isinstance(item, dict) and type(item.get('bytes')) is int
            and 0 < item['bytes'] < limit and isinstance(item.get('sha256'), str)
            and re.fullmatch('[a-f0-9]{64}', item['sha256']) is not None)


def own_wheel(item):
    return isinstance(item, dict) and all(item.get(key) == value for key, value in {
        'name': 'ctranslate2', 'version': '4.8.2', 'filename': WHEEL, 'url': PREFIX + WHEEL}.items())


def cpu_wheel(lock):
    wheels = [wheel for wheel in lock['wheels'] if wheel.get('name') == 'ctranslate2']
    if len(wheels) != 1 or not own_wheel(wheels[0]) or not pin_valid(wheels[0]):
        raise ValueError('Shipping CPU recipe requires the exact pinned own CPU wheel')
    if lock.get('platform') != 'windows-x64' or lock.get('python') != '3.12':
        raise ValueError('Own CPU wheel requires its Windows x64 CPython 3.12 lock')
    return wheels[0]


def publication_pin(lock):
    cpu_wheel(lock)
    component = lock.get('cpu_component')
    if (not isinstance(component, dict) or set(component) != {'source_sha', 'proof'}
            or not isinstance(component['source_sha'], str)
            or not re.fullmatch('[a-f0-9]{40}', component['source_sha'])
            or not pin_valid(component['proof'], MAX_PROOF)
            or set(component['proof']) != {'url', 'bytes', 'sha256'}
            or component['proof']['url'] != PREFIX + PROOF):
        raise ValueError('Own CPU recipe needs an exact published component source and proof pin')
    return component['proof']


def plain_path(path, *, directory=False):
    """Apply the existing owned-path lstat policy, including Windows junctions."""
    path = Path(path).absolute()
    try:
        for part in (path, *path.parents):
            info = part.lstat()
            if stat.S_ISLNK(info.st_mode) or getattr(info, 'st_file_attributes', 0) & 0x400:
                raise ValueError('CPU component path has a link or reparse ancestor')
        mode = path.lstat().st_mode
        if not (stat.S_ISDIR(mode) if directory else stat.S_ISREG(mode)):
            raise ValueError('CPU component path has the wrong file type')
        if path.resolve(strict=True) != path:
            raise ValueError('CPU component path is not canonical')
    except OSError as error:
        raise ValueError('Missing regular size-pinned CPU component input') from error
    return path


def checked_bytes(path, pin, limit=MAX_ASSET):
    path = Path(path)
    if not pin_valid(pin, limit):
        raise ValueError('Invalid bounded CPU component pin')
    path = plain_path(path)
    if path.stat().st_size != pin['bytes']:
        raise ValueError('Missing regular size-pinned CPU component input')
    with path.open('rb') as source:
        data = source.read(pin['bytes'] + 1)
    if len(data) != pin['bytes'] or sha(data) != pin['sha256']:
        raise ValueError('CPU component input differs from its exact bytes/hash pin')
    return data


def unique_keys(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise ValueError('Duplicate CPU proof JSON key')
        result[key] = value
    return result


def pinned_component(lock, proof_path):
    """Read and cross-check proof bytes anchored in the immutable shipping lock.

    This verifies the CI input, not live release state. A human-reviewed lock
    update after public-asset verification remains the activation prerequisite.
    """
    wheel = cpu_wheel(lock)
    proof_pin = publication_pin(lock)
    if proof_path is None:
        raise ValueError('Pinned published CPU component proof is required')
    raw = checked_bytes(proof_path, proof_pin, MAX_PROOF)
    proof = json.loads(raw, object_pairs_hook=unique_keys)
    if not isinstance(proof, dict):
        raise ValueError('CPU publication proof must be an object')
    assets = proof.get('assets')
    expected = {WHEEL, SOURCES, NOTICES}
    if (not isinstance(assets, list) or len(assets) != 3
            or any(not pin_valid(item) or set(item) != {'name', 'bytes', 'sha256'} for item in assets)
            or {item['name'] for item in assets} != expected):
        raise ValueError('CPU publication proof has an unexpected asset set')
    selected = next(item for item in assets if item['name'] == WHEEL)
    if any(selected[field] != wheel[field] for field in ('bytes', 'sha256')):
        raise ValueError('CPU publication proof identifies another wheel')
    if (proof.get('source_sha') != lock['cpu_component']['source_sha']
            or proof.get('provenance', {}).get('luma_source_sha') != lock['cpu_component']['source_sha']):
        raise ValueError('Pinned CPU proof component source differs from the shipping lock')
    return {'component_source_sha': lock['cpu_component']['source_sha'], 'publication_proof': dict(proof_pin),
            'wheel': {key: wheel[key] for key in IDENTITY_FIELDS}, 'proof': proof}


def published_component(lock, proof_path, *, root=ROOT):
    """Run the existing full release-proof validator before generation/CI."""
    component = pinned_component(lock, proof_path)
    proof = component['proof']; assets = proof['assets']
    locks = {}; lock_pins = {}
    for key in ('sources', 'notices'):
        name = key + '.lock.json'
        # The publisher pins immutable Git blobs; normalize only checkout CRLF.
        data = (root / 'ct2-cpu' / name).read_bytes().replace(b'\r\n', b'\n')
        locks[key] = json.loads(data)
        lock_pins[key] = {'name': name, 'bytes': len(data), 'sha256': sha(data)}
    request = {'source_sha': lock['cpu_component']['source_sha'], 'assets': assets, 'locks': lock_pins}
    result = subprocess.run(['node', str(VALIDATOR)],
                            input=json.dumps({'proof': proof, 'request': request, 'locks': locks}),
                            capture_output=True, text=True, encoding='utf-8', timeout=30)
    if result.returncode != 0 or result.stdout.strip() != 'CPU_RECIPE_PROOF_OK':
        raise ValueError('Existing CPU publication validator rejected the recipe proof: ' + result.stderr[-2000:])
    return component


def cached_component(lock, cache, *, root=ROOT):
    return published_component(lock, Path(cache) / publication_pin(lock)['sha256'], root=root)


def validate_wheel_extra(data):
    """Match archive.rs: ZIP64/NTFS/extended timestamps only, never aliases."""
    while data:
        if len(data) < 4:
            raise ValueError('Malformed wheel ZIP extra metadata')
        kind, size = struct.unpack_from('<HH', data)
        if size > len(data) - 4 or kind not in {0x0001, 0x000a, 0x5455}:
            raise ValueError('Unreviewed wheel ZIP extra metadata or link/alias')
        data = data[4 + size:]


def reviewed_wheel_members(archive, *, max_files, max_member_bytes, max_total_bytes):
    """Preflight every ZIP record before any engine-specific payload selection.

    Use the reference builder's safe POSIX path policy, with raw-name identity
    and Windows case/path alias checks. Opening every entry additionally asks
    zipfile to check its local-header name and compressed-member boundaries;
    this does not read, decompress or extract its payload.
    """
    from build import safe_name
    members = archive.infolist()
    if not 0 < len(members) <= max_files:
        raise ValueError('Wheel archive member count exceeds its bound')
    seen = set(); paths = {}; total = 0
    for member in members:
        raw = member.orig_filename
        if (not isinstance(raw, str) or not raw or not raw.isascii() or len(raw) > 2048
                or any(ord(char) < 32 or ord(char) == 127 for char in raw) or '\\' in raw
                or raw != member.filename):
            raise ValueError('Wheel archive raw name is unsafe or normalized')
        directory = raw.endswith('/')
        name = raw[:-1] if directory else raw
        safe_name(name)
        parts = name.split('/')
        # Windows strips trailing dots/spaces, allowing distinct ZIP paths to
        # alias one installed file. Refuse these aliases on every build host.
        if len(parts) > 32:
            raise ValueError('Wheel archive path exceeds the component limit')
        for part in parts:
            stem = part.split('.')[0].upper()
            reserved = stem in {'CON', 'PRN', 'AUX', 'NUL', 'CLOCK$'} or (
                len(stem) == 4 and stem[:3] in {'COM', 'LPT'} and stem[3].isdigit())
            if part.endswith(('.', ' ')) or reserved:
                raise ValueError('Wheel archive member has an unsafe path alias')
        key = name.casefold()
        if key in seen:
            raise ValueError('Wheel archive has duplicate or case-colliding members')
        seen.add(key)
        for index in range(1, len(parts) + 1):
            prefix = '/'.join(parts[:index]); folded = prefix.casefold()
            kind = 'directory' if index < len(parts) or directory else 'file'
            if folded in paths and paths[folded] != (prefix, kind):
                raise ValueError('Wheel archive has a case or file/directory path collision')
            paths[folded] = (prefix, kind)
        kind = stat.S_IFMT(member.external_attr >> 16)
        if (member.flag_bits & 0x61 or member.compress_type not in (zipfile.ZIP_STORED, zipfile.ZIP_DEFLATED)
                or type(member.file_size) is not int or not 0 <= member.file_size <= max_member_bytes
                or type(member.compress_size) is not int or member.compress_size < 0
                or (directory and (kind not in (0, stat.S_IFDIR) or member.file_size != 0))
                or (not directory and (kind not in (0, stat.S_IFREG) or member.external_attr & 0x10))):
            raise ValueError('Wheel archive member type, encryption, compression or size is unsafe')
        validate_wheel_extra(member.extra)
        total += member.file_size
        if total > max_total_bytes:
            raise ValueError('Wheel archive expanded bytes exceed its bound')
    # Check ignored metadata entries too: a malicious local-header spelling
    # must not be hidden behind a safe central-directory name.
    for member in members:
        archive.fp.seek(member.header_offset)
        header = archive.fp.read(30)
        if len(header) != 30 or header[:4] != b'PK\x03\x04':
            raise ValueError('Invalid wheel local header')
        flags, method = struct.unpack_from('<HH', header, 6)
        name_size, extra_size = struct.unpack_from('<HH', header, 26)
        if (not 0 < name_size <= 2048 or flags & 0x61 or flags != member.flag_bits
                or method != member.compress_type):
            raise ValueError('Wheel local header differs from its reviewed metadata')
        if archive.fp.read(name_size) != member.orig_filename.encode('ascii'):
            raise ValueError('Wheel local and central names differ')
        extra = archive.fp.read(extra_size)
        if len(extra) != extra_size:
            raise ValueError('Truncated wheel local metadata')
        validate_wheel_extra(extra)
        with archive.open(member):
            pass
    return members


def verify_installed_wheel(root, wheel, cache, provenance):
    """Match installed CT2 source/native files to the whole-hash-anchored wheel."""
    import io
    raw = checked_bytes(Path(cache) / wheel['sha256'], wheel)
    site = Path(root) / 'Lib/site-packages'
    provenance_path = site / 'ctranslate2-4.8.2.dist-info/LUMA_CPU_BUILD.json'
    checked = set(); package_files = set(); package_directories = {'ctranslate2'}
    with zipfile.ZipFile(io.BytesIO(raw)) as archive:
        members = reviewed_wheel_members(archive, max_files=2000,
                                         max_member_bytes=MAX_ASSET - 1, max_total_bytes=MAX_ASSET - 1)
        for member in members:
            name = member.filename
            if name.startswith('ctranslate2/'):
                parts = name.rstrip('/').split('/')
                for index in range(1, len(parts) + (1 if member.is_dir() else 0)):
                    package_directories.add('/'.join(parts[:index]))
                if not member.is_dir():
                    package_files.add(name)
            if member.is_dir():
                continue
            if name.startswith('ctranslate2/') or name == 'ctranslate2-4.8.2.dist-info/LUMA_CPU_BUILD.json':
                with archive.open(member) as source:
                    data = source.read(member.file_size + 1)
                if len(data) != member.file_size:
                    raise ValueError('Truncated CPU wheel member')
                target = plain_path(site / name)
                if target.stat().st_size != len(data):
                    raise ValueError('Installed CPU wheel member differs from the pinned wheel')
                with target.open('rb') as installed_source:
                    if installed_source.read(len(data) + 1) != data:
                        raise ValueError('Installed CPU wheel member differs from the pinned wheel')
                checked.add(name)
                if name.endswith('/LUMA_CPU_BUILD.json') and json.loads(data) != provenance:
                    raise ValueError('Installed CPU wheel differs from published build provenance')
    required = {'ctranslate2/ctranslate2.dll', 'ctranslate2/_ext.cp312-win_amd64.pyd',
                'ctranslate2-4.8.2.dist-info/LUMA_CPU_BUILD.json'}
    if not required.issubset(checked):
        raise ValueError('Published CPU wheel is missing its native files or provenance')
    # No generated/importable extras are permitted: assembly removes pyc and
    # every verifier/worker uses -B. In particular, _ext/__init__.py must never
    # shadow the exact anchored extension module. Do not follow reparse dirs.
    stack = [plain_path(site / 'ctranslate2', directory=True)]
    actual_files = set(); actual_directories = {'ctranslate2'}
    while stack:
        with os.scandir(stack.pop()) as entries:
            for entry in entries:
                path = Path(entry.path)
                relative = path.relative_to(site).as_posix()
                info = path.lstat()
                if stat.S_ISDIR(info.st_mode):
                    path = plain_path(path, directory=True)
                    if relative not in package_directories:
                        raise ValueError('Installed CT2 tree has an unexpected directory')
                    actual_directories.add(relative); stack.append(path)
                else:
                    plain_path(path)
                    if relative not in package_files:
                        raise ValueError('Installed CT2 tree has an unexpected file')
                    actual_files.add(relative)
    if actual_files != package_files or actual_directories != package_directories:
        raise ValueError('Installed CT2 tree differs from the exact pinned wheel')
    return provenance_path


def source_verifier():
    path = ROOT / 'ct2-cpu/verify_runtime.py'
    spec = importlib.util.spec_from_file_location('luma_cpu_source_verifier', path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def prepare_published_worker(worker, runtime_root, cache, component, app_source_sha, destination):
    if not isinstance(app_source_sha, str) or not re.fullmatch('[a-f0-9]{40}', app_source_sha):
        raise ValueError('Exact current application source SHA required')
    shipping_lock = json.loads((ROOT / 'locks' / (PACK_ID + '.json')).read_text(encoding='utf-8'))
    pinned = pinned_component(shipping_lock, Path(cache) / publication_pin(shipping_lock)['sha256'])
    if component != pinned:
        raise ValueError('Reference component identity differs from the immutable shipping lock')
    worker = plain_path(worker)
    expected = plain_path(ROOT.parent.parent / 'src-tauri/src/asr/worker.py')
    source = worker.read_bytes()
    if worker.is_symlink() or source != expected.read_bytes():
        raise ValueError('Reference worker differs from the current application source')
    provenance = component['proof']['provenance']
    installed = verify_installed_wheel(runtime_root, component['wheel'], cache, provenance)
    verifier = source_verifier()
    _, identity = verifier.load_source_proof_worker(worker, Path(runtime_root).resolve(), installed,
                                                    sha(installed.read_bytes()), app_source_sha,
                                                    component_source_sha=component['component_source_sha'],
                                                    publication_proof=Path(cache) / component['publication_proof']['sha256'])
    prepared = source.decode('utf-8').replace('\r\n', '\n').replace(verifier.CT2_CPU_POLICY_MARKER, verifier.CT2_CPU_POLICY_SOURCE, 1)
    with Path(destination).open('x', encoding='utf-8', newline='\n') as output:
        output.write(prepared)
    identity.update(source_sha=app_source_sha, component_source_sha=component['component_source_sha'],
                    selection='published-component-provenance', publication_proof=component['publication_proof'],
                    managed_receipt_selection_tested=False, global_managed_path_selection_tested=False)
    return identity


def strict_cpu_closure(files):
    from windows_native_inventory import _pe, GPU
    report = _pe.closure(files)
    extra = re.compile(r'(?:cuda|openmp|libgomp|cupti|cufile|nvfatbin|nvptxcompiler|nvblas|nvtoolsext|nccl|^npp)', re.I)
    forbidden = lambda name: bool(_pe.FORBIDDEN.search(name) or GPU.search(name) or extra.search(name))
    report['forbidden_files'] = [item['path'] for item in files if forbidden(Path(item['path']).name)]
    for item in report['dependencies']:
        if forbidden(item['name']):
            item['resolution'] = 'forbidden'
    report['blocked_dependencies'] = [item for item in report['dependencies']
                                      if item['resolution'] in {'forbidden', 'missing_private_crt', 'unresolved'}]
    report['passed'] = not report['forbidden_files'] and not report['blocked_dependencies']
    report['scope'] = 'Own CPU component: no CUDA/cuDNN/MKL/OpenMP/TBB files or normal/delay imports.'
    return report
