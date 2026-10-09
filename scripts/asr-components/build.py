#!/usr/bin/env python3
"""Build private ASR components from reviewed byte-locked vendor artifacts.

The build host's Python orchestrates downloads and archive construction only.
Runtime testing always invokes the shipped interpreter, in isolated mode.
"""
from __future__ import annotations
import argparse
import ast
import email
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import platform
import posixpath
import shutil
import stat
import struct
import subprocess
import sys
import tarfile
import urllib.parse
import urllib.request
import zipfile

ROOT = Path(__file__).resolve().parent
MAX_ASSET = 2_000_000_000  # Below GitHub's individual release-asset limit.
ALLOWED = {'github.com', 'files.pythonhosted.org', 'pypi.org'}


def embed_nagisa(source, managed_worker=False):
    marker = '# LUMA_NAGISA_COMPAT_SOURCE'
    if source.count(marker) != 1:
        raise ValueError('Expected exactly one reviewed Nagisa source marker')
    helper = (ROOT.parent.parent / 'src-tauri/src/asr/nagisa_compat.py').read_text(encoding='utf-8')
    if managed_worker:
        helper += '\nLUMA_MANAGED_QWEN_RUNTIME = True\n'
    return source.replace(marker, helper)


def sha256(path):
    with path.open('rb') as f:
        return hashlib.file_digest(f, 'sha256').hexdigest()


def dump(path, data):
    path.write_text(json.dumps(data, ensure_ascii=False, indent=2, sort_keys=True) + '\n', encoding='utf-8', newline='\n')


def safe_name(name):
    if not name or '\\' in name or ':' in name or '\x00' in name:
        raise ValueError(f'Unsafe archive member {name!r}')
    path = PurePosixPath(name)
    if path.is_absolute() or any(p in {'', '.', '..'} for p in name.split('/')):
        raise ValueError(f'Unsafe archive member {name!r}')
    return path


def fetch(item, cache):
    url = item['url']; parsed = urllib.parse.urlparse(url)
    if parsed.scheme != 'https' or parsed.hostname not in ALLOWED or parsed.username or parsed.password:
        raise ValueError(f'Unapproved locked source {url}')
    digest = item['sha256']
    if len(digest) != 64 or any(c not in '0123456789abcdef' for c in digest):
        raise ValueError('Invalid pinned digest')
    target = cache / digest
    if target.exists() and target.stat().st_size == item['bytes'] and sha256(target) == digest:
        return target
    partial = target.with_suffix('.partial')
    try:
        with urllib.request.urlopen(url, timeout=180) as response, partial.open('wb') as output:
            total = 0
            while chunk := response.read(1024 * 1024):
                total += len(chunk)
                if total > item['bytes']:
                    raise ValueError(f'Download exceeds pinned size: {url}')
                output.write(chunk)
        if partial.stat().st_size != item['bytes'] or sha256(partial) != digest:
            raise ValueError(f'Pinned download verification failed: {url}')
        partial.replace(target)
    finally:
        partial.unlink(missing_ok=True)
    return target


def unpack_runtime(archive, target):
    """Dereference upstream internal links; output contains regular files only."""
    with tarfile.open(archive, 'r:gz') as source:
        members = {m.name.rstrip('/'): m for m in source.getmembers()}
        total = 0
        def resolve(name, seen=()):
            if name in seen or len(seen) > 16:
                raise ValueError('Cyclic runtime link')
            safe_name(name)
            if not name.startswith('python/'):
                raise ValueError('Runtime link escaped python/')
            m = members[name]
            if m.isreg():
                return m
            if m.issym():
                return resolve(posixpath.normpath(posixpath.join(posixpath.dirname(name), m.linkname)), (*seen, name))
            if m.islnk():
                return resolve(posixpath.normpath(m.linkname), (*seen, name))
            raise ValueError(f'Unsupported runtime member: {name}')
        for name, member in sorted(members.items()):
            if member.isdir():
                continue
            regular = resolve(name)
            relative = safe_name(name.removeprefix('python/'))
            dest = target.joinpath(*relative.parts)
            total += regular.size
            if total > 1_000_000_000:
                raise ValueError('Runtime exceeds extraction ceiling')
            dest.parent.mkdir(parents=True, exist_ok=True)
            with source.extractfile(regular) as f, dest.open('wb') as output:
                shutil.copyfileobj(f, output)
            dest.chmod(0o755 if regular.mode & 0o111 else 0o644)


def install_wheel(archive, root, site):
    """Install wheel library/data files without pip, generated launchers or pyc."""
    notices = []
    with zipfile.ZipFile(archive) as z:
        for member in sorted(z.infolist(), key=lambda m: m.filename):
            if member.is_dir():
                continue
            path = safe_name(member.filename)
            if stat.S_ISLNK(member.external_attr >> 16):
                raise ValueError('Wheel contains a symlink')
            parts = path.parts
            if parts[0].endswith('.data'):
                if len(parts) < 3:
                    raise ValueError('Invalid wheel data scheme')
                if parts[1] in {'scripts', 'headers'}:
                    continue
                if parts[1] in {'purelib', 'platlib'}:
                    dest = site.joinpath(*parts[2:])
                elif parts[1] == 'data':
                    dest = root.joinpath(*parts[2:])
                else:
                    raise ValueError('Unsupported wheel data scheme')
            else:
                dest = site.joinpath(*parts)
            content = z.read(member)
            if dest.exists() and dest.read_bytes() != content:
                raise ValueError(f'Conflicting wheel file: {dest}')
            dest.parent.mkdir(parents=True, exist_ok=True)
            dest.write_bytes(content)
            dest.chmod(0o755 if (member.external_attr >> 16) & 0o111 else 0o644)
            if any(t in dest.name.lower() for t in ('license', 'copying', 'notice', 'author')):
                notices.append(dest.relative_to(root).as_posix())
    return sorted(notices)


def clean(root, site):
    # Nothing in the app executes pip, ensurepip, generated console scripts, or
    # code built against the CI interpreter. Distribution notices remain intact.
    for name in ('pip',):
        for path in site.glob(name + '*'):
            shutil.rmtree(path) if path.is_dir() else path.unlink()
    stdlib = site.parent
    for path in [stdlib / 'ensurepip', root / 'Scripts']:
        if path.exists():
            shutil.rmtree(path)
    for path in list(root.rglob('__pycache__')):
        shutil.rmtree(path)
    for path in root.rglob('*.pyc'):
        path.unlink()
    if (root / 'bin').exists():
        for path in (root / 'bin').iterdir():
            if not path.name.startswith('python'):
                path.unlink()


def write_licenses(root, site, runtime, wheels):
    inventory = [{'name': 'CPython and bundled standard-library dependencies', 'version': runtime['version'],
                  'vendor': runtime['vendor'], 'source_url': runtime['source_url'],
                  'notice_files': sorted(str(p.relative_to(root)).replace('\\', '/') for p in root.rglob('*')
                                         if p.is_file() and not p.is_relative_to(site) and any(x in p.name.lower() for x in ('license', 'copying', 'notice')))}]
    for wheel in wheels:
        dist = next((p for p in site.glob('*.dist-info') if p.name.lower().replace('_', '-').startswith(wheel['name'].lower().replace('_', '-') + '-')), None)
        if dist is None:
            raise ValueError(f'Missing distribution metadata for {wheel["name"]}')
        metadata = email.message_from_bytes((dist / 'METADATA').read_bytes())
        notices = [p for p in dist.rglob('*') if p.is_file() and any(t in p.name.lower() for t in ('license', 'copying', 'notice', 'author'))]
        binary_source = None
        if wheel['name'] in {'torch', 'torchaudio', 'torchvision'}:
            version_file = site / wheel['name'] / 'version.py'
            # Read the wheel's actual build revision without executing it; the
            # release tag can differ from the binary build commit.
            for node in ast.walk(ast.parse(version_file.read_text(encoding='utf-8'))):
                if isinstance(node, ast.Assign) and isinstance(node.value, ast.Constant) and isinstance(node.value.value, str):
                    if any(isinstance(target, ast.Name) and target.id in {'git_version', '__git_version__'} for target in node.targets):
                        revision = node.value.value
                        if len(revision) != 40 or any(c not in '0123456789abcdef' for c in revision):
                            raise ValueError('Invalid upstream binary source revision')
                        repository = wheel['upstream_source']['repository']
                        binary_source = {'commit': revision, 'source_url': repository + '/tree/' + revision}
        inventory.append({'name': wheel['name'], 'version': wheel['version'], 'wheel': wheel['filename'], 'wheel_sha256': wheel['sha256'],
                          'license_expression': metadata.get('License-Expression'), 'license': wheel['license'],
                          'project_urls': wheel['project_urls'], 'sources': wheel['sources'],
                          'upstream_source': wheel.get('upstream_source'),
                          'binary_source': binary_source,
                          'notice_files': sorted(set(wheel.get('installed_notice_files', [])) | {p.relative_to(root).as_posix() for p in notices}),
                          'metadata_file': (dist / 'METADATA').relative_to(root).as_posix()})
    dump(root / 'LICENSES.json', inventory)
    (root / 'THIRD_PARTY_NOTICES.txt').write_text(
        'Luma Subtitle private ASR component\n\n'
        'CPython, the runtime libraries, and each Python distribution retain their\n'
        'respective licenses. See LICENSES.json and all included license/notice files.\n'
        'License metadata is an inventory, not a replacement for these license texts.\n'
        'No model weights are included. Package source locations and wheel hashes\n'
        'are recorded in LICENSES.json. See ASR_COMPONENTS_BUILD.md in the release.\n', encoding='utf-8')


def copy_runtime_licenses(root, platform):
    lock = json.loads((ROOT / 'licenses.lock.json').read_text())
    selected = [f for f in lock['files'] if f['platform'] == platform]
    if not selected:
        raise ValueError('Missing runtime license inventory')
    for entry in selected:
        source = ROOT.joinpath(*safe_name(entry['path']).parts)
        if source.stat().st_size != entry['bytes'] or sha256(source) != entry['sha256']:
            raise ValueError('Runtime license file hash mismatch')
        relative = Path(entry['path']).relative_to('licenses', 'runtime-' + platform)
        dest = root / 'licenses' / 'python-build-standalone' / relative
        dest.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(source, dest)
    dump(root / 'licenses' / 'python-build-standalone' / 'SOURCE.json', lock['sources'][platform])


def pe_imports(path):
    """Read bounded PE import names; fail closed for unreviewed delay imports."""
    data = path.read_bytes()
    if data[:2] != b'MZ':
        raise ValueError('Expected PE image')
    pe = struct.unpack_from('<I', data, 0x3c)[0]
    if data[pe:pe + 4] != b'PE\0\0':
        raise ValueError('Invalid PE signature')
    sections = struct.unpack_from('<H', data, pe + 6)[0]
    optional_size = struct.unpack_from('<H', data, pe + 20)[0]
    optional = pe + 24
    if struct.unpack_from('<H', data, optional)[0] != 0x20b:
        raise ValueError('Expected 64-bit PE image')
    directories = optional + 112
    if struct.unpack_from('<II', data, directories + 13 * 8) != (0, 0):
        raise ValueError('Unreviewed delayed native dependency')
    imports_rva, imports_size = struct.unpack_from('<II', data, directories + 8)
    table = optional + optional_size
    def offset(rva):
        for i in range(sections):
            at = table + i * 40
            _, address, size, raw = struct.unpack_from('<IIII', data, at + 8)
            if address <= rva < address + size:
                return raw + rva - address
        raise ValueError('PE RVA is outside file-backed sections')
    imports = []
    for i in range(min(imports_size // 20, 1024)):
        desc = struct.unpack_from('<IIIII', data, offset(imports_rva) + i * 20)
        if desc == (0, 0, 0, 0, 0):
            return imports
        start = offset(desc[3]); end = data.find(b'\0', start, start + 512)
        if end < 0:
            raise ValueError('Unbounded PE library name')
        imports.append(data[start:end].decode('ascii').lower())
    raise ValueError('Unterminated PE import table')


def prune_reviewed_files(root, pack_id):
    policy = json.loads((ROOT / 'pruning.json').read_text())['packs'].get(pack_id)
    if not policy:
        return
    library = root.joinpath(*safe_name(policy['verify_pe_imports']).parts)
    if sha256(library) != policy['native_library_sha256']:
        raise ValueError('Native library changed; CPU pruning needs review')
    imports = pe_imports(library)
    if any('cudnn' in name or 'cublas' in name or 'cudart' in name for name in imports):
        raise ValueError('CPU component unexpectedly imports CUDA/cuDNN')
    for entry in policy['files']:
        path = root.joinpath(*safe_name(entry['path']).parts)
        if path.stat().st_size != entry['bytes'] or sha256(path) != entry['sha256']:
            raise ValueError('Pruned file differs from reviewed wheel')
        path.unlink()
    dump(root / 'INSTALLATION_CHANGES.json', {'schema': 1, 'policy': policy,
         'normal_pe_imports': imports, 'delay_imports': [],
         'validation_required': 'Native CPU import and actual Tiny cold/warm inference before publication.'})


def copy_supplemental_notices(root, pack):
    lock = json.loads((ROOT / 'supplemental-notices.lock.json').read_text())
    selected = []
    for entry in lock['files']:
        if pack['id'] not in entry['packs']:
            continue
        source = ROOT.joinpath(*safe_name(entry['path']).parts)
        if source.stat().st_size != entry['bytes'] or sha256(source) != entry['sha256']:
            raise ValueError('Supplemental notice verification failed')
        dest = root.joinpath(*safe_name(entry['destination']).parts)
        dest.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(source, dest)
        selected.append(entry)
    dump(root / 'licenses' / 'SUPPLEMENTAL-SOURCES.json', {'schema': 1, 'files': selected})


def archive_tree(root, output):
    # ZIP_STORED is bigger; DEFLATE gives deterministic output for the same pinned
    # build interpreter/zlib. The workflow independently builds twice and compares.
    with zipfile.ZipFile(output, 'w', compression=zipfile.ZIP_DEFLATED, compresslevel=9, allowZip64=True) as z:
        for path in sorted(root.rglob('*')):
            if path.is_symlink():
                raise ValueError(f'Unexpected output symlink: {path}')
            if not path.is_file():
                continue
            info = zipfile.ZipInfo(path.relative_to(root).as_posix(), (2026, 1, 1, 0, 0, 0))
            info.create_system = 3
            info.external_attr = (stat.S_IFREG | (0o755 if path.stat().st_mode & 0o111 else 0o644)) << 16
            info.compress_type = zipfile.ZIP_DEFLATED
            info._compresslevel = 9
            with path.open('rb') as f, z.open(info, 'w', force_zip64=True) as out:
                shutil.copyfileobj(f, out)
    if output.stat().st_size >= MAX_ASSET:
        raise ValueError('Component exceeds release-asset limit')


def run_offline_assembly(root, runtime, lock_path, wheels, cache, output):
    wheelhouse = output / 'wheelhouse'; wheelhouse.mkdir()
    for wheel in wheels:
        shutil.copyfile(fetch(wheel, cache), wheelhouse / wheel['filename'])
    poison = output / 'poison'; poison.mkdir()
    (poison / 'sitecustomize.py').write_text("raise RuntimeError('Inherited Python path used')\n")
    (poison / 'pip.conf').write_text('[global]\nindex-url = https://invalid.example/no-network\ntarget = /never-use-inherited-pip-target\n')
    env = dict(os.environ, PATH=str(poison / 'no-executables'), PYTHONHOME=str(poison / 'not-python'),
               PYTHONPATH=str(poison), PIP_CONFIG_FILE=str(poison / 'pip.conf'),
               PIP_INDEX_URL='https://invalid.example/no-network', PIP_TARGET=str(poison / 'must-not-be-written'))
    command = [str(root / runtime['entrypoint']), '-I', '-S', '-B', '-u', '-X', 'utf8', str(ROOT / 'assemble.py'),
               '--runtime-root', str(root), '--wheel-lock', str(lock_path), '--wheelhouse', str(wheelhouse)]
    child = subprocess.Popen(command, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, encoding='utf-8', env=env)
    try:
        transcript = child.communicate(timeout=600)[0]
    except BaseException:
        child.kill(); child.communicate()
        raise
    print(transcript, flush=True)
    if child.returncode:
        raise RuntimeError('Owned private offline-pip assembly failed')
    if (poison / 'must-not-be-written').exists():
        raise RuntimeError('Inherited pip target was used')
    shutil.rmtree(wheelhouse); shutil.rmtree(poison)


def build(pack_id, output, cache, source_sha, native=True, installer='wheel'):
    config = json.loads((ROOT / 'packs.json').read_text())
    pack = next(p for p in config['packs'] if p['id'] == pack_id)
    native_platform = 'windows-x64' if sys.platform == 'win32' and platform.machine().lower() in {'amd64', 'x86_64'} else 'macos-arm64' if sys.platform == 'darwin' and platform.machine() == 'arm64' else None
    if native and native_platform != pack['platform']:
        raise ValueError(f'Build requires native {pack["platform"]}; got {sys.platform}/{platform.machine()}')
    if len(source_sha) != 40 or any(c not in '0123456789abcdef' for c in source_sha):
        raise ValueError('Expected immutable source commit SHA')
    runtime = config['runtime'][pack['platform']]
    lock_path = ROOT / 'locks' / (pack_id + '.json')
    lock = json.loads(lock_path.read_text())
    if lock['platform'] != pack['platform']:
        raise ValueError('Lock platform mismatch')
    output.mkdir(parents=True, exist_ok=True); cache.mkdir(parents=True, exist_ok=True)
    root = output / 'staging' / pack_id
    if root.exists():
        raise ValueError(f'Build output must be fresh: {root}')
    root.mkdir(parents=True)
    unpack_runtime(fetch(runtime, cache), root)
    site = root / ('Lib/site-packages' if pack['platform'] == 'windows-x64' else 'lib/python3.12/site-packages')
    site.mkdir(parents=True, exist_ok=True)
    if installer == 'pip':
        if not native:
            raise ValueError('Offline pip proof requires the native private interpreter')
        run_offline_assembly(root, runtime, lock_path, lock['wheels'], cache, output)
    elif installer == 'wheel':
        # Existing reference assembly, retained to compare native proof results.
        for name in ('pip', 'setuptools', '_distutils_hack', 'pkg_resources'):
            for path in site.glob(name + '*'):
                shutil.rmtree(path) if path.is_dir() else path.unlink()
        (site / 'distutils-precedence.pth').unlink(missing_ok=True)
        for wheel in lock['wheels']:
            print(f'Verifying {wheel["filename"]}', flush=True)
            wheel['installed_notice_files'] = install_wheel(fetch(wheel, cache), root, site)
    else:
        raise ValueError('Unknown reviewed assembly method')
    clean(root, site)
    prune_reviewed_files(root, pack_id)
    copy_runtime_licenses(root, pack['platform'])
    copy_supplemental_notices(root, pack)
    self_test = (ROOT / 'self_test.py').read_text(encoding='utf-8')
    if pack['platform'] == 'windows-x64' and pack['backend'] == 'qwen3-asr':
        self_test = embed_nagisa(self_test)
    (root / 'self_test.py').write_text(self_test, encoding='utf-8', newline='\n')
    write_licenses(root, site, config['runtime'], lock['wheels'])
    dump(root / 'component.json', {'schema': 1, **pack, 'version': config['version'], 'source_sha': source_sha,
                                  'entrypoint': runtime['entrypoint'], 'python_version': config['runtime']['version'],
                                  'runtime_sha256': runtime['sha256'], 'lock_sha256': sha256(lock_path)})
    filename = f'luma-asr-{pack_id}-{config["version"]}.zip'
    archive = output / filename
    # Use the byte-pinned PRIVATE runtime's zlib, never the mutable runner zlib.
    if native:
        subprocess.run([str(root / runtime['entrypoint']), '-I', '-B', str(Path(__file__).resolve()),
                        '--archive-only', str(root), str(archive)], check=True)
    else:
        archive_tree(root, archive)
    files = [p for p in root.rglob('*') if p.is_file()]
    result = {'schema': 1, 'source_sha': source_sha, 'release_tag': config['release_tag'], **pack,
              'version': config['version'], 'entrypoint': runtime['entrypoint'],
              'license': 'Multiple third-party licenses; see bundled LICENSES.json and notices',
              'license_url': f'https://github.com/{config["repository"]}/blob/{config["release_tag"]}/docs/ASR_COMPONENTS_BUILD.md',
              'installed_bytes': sum(p.stat().st_size for p in files), 'max_files': len(files),
              'archive': {'url': f'https://github.com/{config["repository"]}/releases/download/{config["release_tag"]}/{filename}',
                          'bytes': archive.stat().st_size, 'sha256': sha256(archive)},
              'evidence': 'See separate native smoke report. This manifest alone is not test or publication proof.'}
    dump(output / f'{pack_id}.manifest.json', result)
    shutil.copyfile(root / 'LICENSES.json', output / f'LICENSES-{pack_id}.json')
    print(json.dumps(result, indent=2))
    print('MANIFEST_SHA256=' + sha256(output / f'{pack_id}.manifest.json'))
    return result


def main():
    if len(sys.argv) == 4 and sys.argv[1] == '--archive-only':
        archive_tree(Path(sys.argv[2]), Path(sys.argv[3]))
        return
    p = argparse.ArgumentParser(); p.add_argument('--pack', required=True); p.add_argument('--output', type=Path, required=True); p.add_argument('--cache', type=Path, required=True); p.add_argument('--source-sha', required=True)
    a = p.parse_args(); build(a.pack, a.output.resolve(), a.cache.resolve(), a.source_sha)


if __name__ == '__main__':
    main()
