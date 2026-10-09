#!/usr/bin/env python3
"""One bounded install-time entrypoint for verified, private offline wheel assembly.

Invoke only with the exact PBS private interpreter in isolated mode. The caller
owns/reaps this process and can cancel it. This process cannot spawn children or
open network connections. The downloader, not pip, acquires every input.
"""
from __future__ import annotations
import argparse
import csv
import hashlib
import importlib.metadata
import json
import os
from pathlib import Path, PureWindowsPath
import re
import runpy
import shutil
import site
import sys
import tempfile

PIP_VERSION = '26.2.1'
PYTHON_VERSION = '3.12.15'


def local_file_uri(path):
    """Accept local extended Windows drives, never UNC/device/network paths."""
    value = str(path)
    if value.startswith('\\\\?\\') and re.match(r'^[A-Za-z]:\\', value[4:]):
        value = value[4:]
    if value.startswith('\\\\'):
        raise ValueError('Remote UNC or device wheel paths are not allowed')
    if re.match(r'^[A-Za-z]:\\', value):
        return PureWindowsPath(value).as_uri()
    return Path(value).resolve().as_uri()


def digest(path):
    with path.open('rb') as f:
        return hashlib.file_digest(f, 'sha256').hexdigest()


def wheel_requirements(lock, wheelhouse):
    """Only verified local wheel URLs, each with one exact approved hash."""
    requirements = []; expected = set()
    for wheel in lock['wheels']:
        name, version, filename = wheel['name'], wheel['version'], wheel['filename']
        if not re.fullmatch(r'[A-Za-z0-9][A-Za-z0-9_.-]*', name) or not re.fullmatch(r'[A-Za-z0-9_.+!-]+', version):
            raise ValueError('Unsafe package identity')
        if Path(filename).name != filename or not filename.endswith('.whl') or '\\' in filename or ':' in filename:
            raise ValueError('Only named wheels are allowed')
        path = wheelhouse / filename
        if path.is_symlink() or not path.is_file() or path.stat().st_size != wheel['bytes'] or digest(path) != wheel['sha256']:
            raise ValueError('Verified wheel bytes are missing or changed: ' + filename)
        if filename in expected:
            raise ValueError('Duplicate wheel')
        expected.add(filename)
        requirements.append(f'{name} @ {local_file_uri(path.resolve())} --hash=sha256:{wheel["sha256"]}')
    if {p.name for p in wheelhouse.iterdir()} != expected:
        raise ValueError('Wheelhouse must contain exactly the approved files')
    return '\n'.join(requirements) + '\n'


def offline_guard(event, args):
    if event in {'socket.connect', 'socket.getaddrinfo', 'socket.bind', 'subprocess.Popen',
                 'os.system', 'os.posix_spawn', 'os.fork', 'os.exec'}:
        raise RuntimeError('Private component assembly cannot use network or start child processes: ' + event)


def remove_bootstrap_installer(destination):
    # The byte-pinned PBS bootstrap has its own vendor-build direct_url.json.
    # Its identity was checked before launch; it is not an installed engine wheel.
    # Remove exactly that installer after it exits, before validating all engine
    # provenance. Never exempt an unknown retained package from validation.
    for path in (destination / 'pip', destination / f'pip-{PIP_VERSION}.dist-info'):
        if not path.is_dir() or path.is_symlink():
            raise RuntimeError('Private bootstrap installer layout changed')
        shutil.rmtree(path)


def normalize_removed_records(destination, root, removed):
    """Drop only rows for deliberately removed artifacts, retaining wheel hashes.

    pip hashes generated local direct_url metadata and path-bearing launchers in
    RECORD. Keeping those rows after removing the files leaks staging-dependent
    hashes into otherwise identical installations.
    """
    removed = {path.resolve() for path in removed}
    count = 0
    for record in sorted(destination.glob('*.dist-info/RECORD')):
        with record.open(encoding='utf-8', newline='') as stream:
            rows = list(csv.reader(stream))
        retained = []
        for row in rows:
            if len(row) != 3 or '\\' in row[0]:
                raise RuntimeError('Unexpected installed RECORD row')
            path = (destination / row[0]).resolve()
            if not path.is_relative_to(root):
                raise RuntimeError('Installed RECORD escaped the private runtime')
            if path in removed:
                count += 1
            else:
                if Path(row[0]).is_absolute() or PureWindowsPath(row[0]).is_absolute():
                    raise RuntimeError('Absolute installed RECORD path')
                retained.append(row)
        with record.open('w', encoding='utf-8', newline='') as stream:
            csv.writer(stream, lineterminator='\n').writerows(retained)
    return count


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--runtime-root', type=Path, required=True)
    parser.add_argument('--wheel-lock', type=Path, required=True)
    parser.add_argument('--wheelhouse', type=Path, required=True)
    args = parser.parse_args()
    root, wheelhouse = args.runtime_root.resolve(), args.wheelhouse.resolve()
    if not sys.flags.isolated or not sys.flags.no_site or site.ENABLE_USER_SITE or Path(sys.prefix).resolve() != root or not Path(sys.executable).resolve().is_relative_to(root):
        raise RuntimeError('Assembly requires the isolated private interpreter')
    if sys.version.split()[0] != PYTHON_VERSION:
        raise RuntimeError('Unreviewed private Python version')
    if (root / 'component.json').exists() or (root / 'ASSEMBLY.json').exists():
        raise RuntimeError('Assembly only accepts a fresh staging runtime')
    destination = root / ('Lib/site-packages' if sys.platform == 'win32' else 'lib/python3.12/site-packages')
    # -S prevents every startup .pth/sitecustomize hook. Add only the private
    # library directory directly; do not call site.addsitedir()/site.main().
    sys.path.append(str(destination))
    distribution = importlib.metadata.distribution('pip')
    if distribution.version != PIP_VERSION or not Path(distribution.locate_file('pip')).resolve().is_relative_to(root):
        raise RuntimeError('Unreviewed or external pip')
    lock = json.loads(args.wheel_lock.read_text(encoding='utf-8'))
    contents = wheel_requirements(lock, wheelhouse)
    # The only reviewed bootstrap-package overlays. Remove the old package
    # completely so --ignore-installed cannot leave stale setuptools modules.
    # pip itself is retained until its in-process invocation finishes.
    if any(wheel['name'].lower().replace('_', '-') == 'setuptools' for wheel in lock['wheels']):
        bootstrap = [destination / name for name in ('setuptools', '_distutils_hack', 'pkg_resources', 'distutils-precedence.pth')]
        bootstrap.extend(destination.glob('setuptools-*.dist-info'))
        for path in bootstrap:
            if path.exists(): shutil.rmtree(path) if path.is_dir() else path.unlink()
    home = root / '.assembly-home'; home.mkdir()
    requirements = root / '.assembly-requirements.txt'
    requirements.write_text(contents, encoding='utf-8', newline='\n')
    # Do not inherit PIP_*, proxies, user config, credentials, Python config or PATH.
    keep_names = {'SYSTEMROOT', 'WINDIR', 'COMSPEC', 'SYSTEMDRIVE', 'PROCESSOR_ARCHITECTURE',
                  'PROCESSOR_ARCHITEW6432', 'PROCESSOR_IDENTIFIER', 'PROCESSOR_LEVEL',
                  'PROCESSOR_REVISION', 'NUMBER_OF_PROCESSORS'}
    keep = {key: value for key, value in os.environ.items() if key.upper() in keep_names}
    os.environ.clear(); os.environ.update(keep)
    os.environ.update({'HOME': str(home), 'USERPROFILE': str(home), 'APPDATA': str(home / 'AppData'),
                       'LOCALAPPDATA': str(home / 'Local'), 'TMP': str(home), 'TEMP': str(home),
                       'PATH': str(home / 'no-executables'), 'PIP_CONFIG_FILE': os.devnull,
                       'HF_HUB_OFFLINE': '1', 'DO_NOT_TRACK': '1', 'LANG': 'C.UTF-8'})
    tempfile.tempdir = str(home)
    # The pinned pip26.2.1 _load_config_files explicitly skips ALL config files
    # when PIP_CONFIG_FILE == os.devnull, even with --isolated. Assert it too.
    sys.addaudithook(offline_guard)
    from pip._internal.configuration import Configuration
    configuration = Configuration(isolated=True); configuration.load()
    if list(configuration.items()):
        raise RuntimeError('External pip configuration was not disabled')
    from pip._internal.locations import get_scheme
    scheme = get_scheme('luma-component', prefix=str(root))
    if any(not Path(getattr(scheme, key)).resolve().is_relative_to(root)
           for key in ('purelib', 'platlib', 'headers', 'scripts', 'data')):
        raise RuntimeError('pip installation scheme escaped the private prefix')
    if any(Path(getattr(scheme, key)).resolve() != destination for key in ('purelib', 'platlib')):
        raise RuntimeError('pip installation scheme differs from the approved private site')
    sys.argv = ['pip', '--isolated', '--disable-pip-version-check', '--no-cache-dir', 'install',
                '--no-index', '--no-deps', '--only-binary=:all:',
                '--require-hashes', '--no-compile', '--no-warn-script-location', '--ignore-installed',
                '--prefix', str(root), '--requirement', str(requirements)]
    try:
        runpy.run_module('pip', run_name='__main__')
    except SystemExit as exc:
        if exc.code not in (None, 0):
            raise RuntimeError(f'Private offline pip failed: {exc.code}') from exc
    remove_bootstrap_installer(destination)
    # pip records local wheel URIs. Validate and remove this app-unneeded generated
    # metadata; immutable upstream provenance is retained below without CI paths.
    approved = {local_file_uri((wheelhouse / w['filename']).resolve()): w['sha256'] for w in lock['wheels']}
    removed_direct_urls = 0
    removed = set()
    for path in destination.glob('*.dist-info/direct_url.json'):
        data = json.loads(path.read_text(encoding='utf-8'))
        expected = approved.get(data.get('url'))
        archive = data.get('archive_info', {})
        actual = archive.get('hashes', {}).get('sha256') or archive.get('hash', '').removeprefix('sha256=')
        if expected is None or actual != expected:
            raise RuntimeError('Unexpected direct wheel provenance for ' + path.parent.name)
        removed.add(path.resolve()); path.unlink(); removed_direct_urls += 1
    # The exact setuptools .pth is accepted during preflight only as install-time
    # data. A finished runtime must not execute this startup hook.
    startup_hook = destination / 'distutils-precedence.pth'
    if startup_hook.exists():
        removed.add(startup_hook.resolve()); startup_hook.unlink()
    requirements.unlink(); shutil.rmtree(home)
    # Install-time tooling does not survive into the usable runtime. No second
    # package-install path is available to a transcription request.
    if (destination.parent / 'ensurepip').exists():
        shutil.rmtree(destination.parent / 'ensurepip')
    if (root / 'Scripts').exists():
        removed.update(path.resolve() for path in (root / 'Scripts').rglob('*') if path.is_file())
        shutil.rmtree(root / 'Scripts')
    if (root / 'bin').exists():
        for path in (root / 'bin').iterdir():
            if not path.name.startswith('python'):
                removed.add(path.resolve()); path.unlink()
    removed_record_rows = normalize_removed_records(destination, root, removed)
    for path in list(root.rglob('__pycache__')):
        shutil.rmtree(path)
    for path in root.rglob('*.pyc'): path.unlink()
    report = {'schema': 1, 'method': 'private-offline-pip', 'python': PYTHON_VERSION, 'pip': PIP_VERSION,
              'wheels': len(lock['wheels']), 'wheel_lock_sha256': digest(args.wheel_lock),
              'wheel_requirement_identity_sha256': hashlib.sha256(json.dumps([{k: w[k] for k in ('name','version','filename','sha256')} for w in lock['wheels']], sort_keys=True).encode()).hexdigest(),
              'no_index': True, 'no_dependencies': True, 'binary_only': True, 'require_hashes': True,
              'pip_config_disabled': True, 'network_guard': True, 'subprocess_guard': True,
              'source_builds': False, 'startup_site_hooks': False, 'generated_launchers_removed': True,
              'installer_removed': True,
              'installation_scheme': 'private-prefix',
              'removed_artifact_record_rows': removed_record_rows,
              'direct_url_metadata': False,
              'direct_url_metadata_removed': removed_direct_urls,
              'upstream_wheels': [{k: w[k] for k in ('name','version','filename','url','bytes','sha256')} for w in lock['wheels']]}
    (root / 'ASSEMBLY.json').write_text(json.dumps(report, indent=2, sort_keys=True) + '\n', encoding='utf-8', newline='\n')
    print(json.dumps(report, sort_keys=True))


if __name__ == '__main__':
    main()
