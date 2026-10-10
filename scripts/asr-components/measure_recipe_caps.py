#!/usr/bin/env python3
"""Measure locked upstream ASR archives without installing or executing them.

This tool is deliberately cache-only. Populate the hash-addressed cache through
the separately authorized downloader first. Every cache input is size/SHA-256
verified before inspection. No model repository or model weights are fetched.

Measured per-input installed_bytes/max_files are exact archive measurements:
runtime links become regular files, as in build.unpack_runtime; wheels include
ALL regular ZIP members, including scripts/headers that assembly later removes.
With --allow-unmeasured, absent wheels receive explicit conservative extraction
caps. Such caps are not measurements or guarantees the unseen wheel will fit.
Pack-level installed_bytes/max_files are enforced conservative CAPS, not actual
installed usage or evidence that the target runtime/inference works.
"""
from __future__ import annotations

import argparse
import email
import json
from pathlib import Path
import posixpath
import tarfile
import urllib.parse
import zipfile

from build import ROOT, safe_name, sha256
from own_cpu_recipe import PACK_ID, own_wheel, cached_component, reviewed_wheel_members

PIP_VERSION = '26.2.1'
ASSEMBLY_BASE_BYTES = 2 * 1024 * 1024
ASSEMBLY_BASE_FILES = 256
PER_WHEEL_METADATA_BYTES = 64 * 1024
PER_WHEEL_METADATA_FILES = 8
RECORD_ROW_OVERHEAD_BYTES = 512
UNMEASURED_MIN_BYTES = 16 * 1024 * 1024
UNMEASURED_EXPANSION_FACTOR = 16
UNMEASURED_MAX_FILES = 8192
UNMEASURED_FINAL_BYTES = 4 * 1024 * 1024 * 1024
UNMEASURED_FINAL_FILES = 100_000
U64_MAX = 2**64 - 1


def validate_source(item, kind, component=None):
    parsed = urllib.parse.urlparse(item['url'])
    if parsed.scheme != 'https' or parsed.username or parsed.password or parsed.port not in (None, 443):
        raise ValueError('Unsafe upstream URL')
    if kind == 'runtime':
        if parsed.hostname != 'github.com' or not parsed.path.startswith('/astral-sh/python-build-standalone/releases/download/'):
            raise ValueError('Runtime must come from the pinned Astral release')
    elif own_wheel(item):
        if component is None or any(item.get(key) != value for key, value in component['wheel'].items()):
            raise ValueError('Own CPU wheel requires matching pinned publication proof')
    elif parsed.hostname != 'files.pythonhosted.org' or not parsed.path.startswith('/packages/'):
        raise ValueError('Wheel must come from the pinned official PyPI file host')
    digest = item['sha256']
    if len(digest) != 64 or any(c not in '0123456789abcdef' for c in digest):
        raise ValueError('Invalid locked SHA-256')
    if type(item['bytes']) is not int or not 0 < item['bytes'] <= U64_MAX:
        raise ValueError('Invalid locked byte count')


def verified_input(item, cache, kind, component=None):
    validate_source(item, kind, component)
    digest = item['sha256']
    path = cache / digest
    if path.is_symlink() or not path.is_file():
        raise ValueError(f'Missing regular cached input: {digest}')
    if path.stat().st_size != item['bytes'] or sha256(path) != digest:
        raise ValueError(f'Cached input differs from the size/SHA-256 lock: {digest}')
    return path


def runtime_measurements(archive, entrypoint, site_packages):
    """Use the same internal tar-link normalization as build.unpack_runtime."""
    with tarfile.open(archive, 'r:gz') as source:
        members = {}
        for member in source.getmembers():
            name = member.name.rstrip('/')
            safe_name(name)
            if name in members:
                raise ValueError(f'Duplicate runtime archive member: {name}')
            if name != 'python' and not name.startswith('python/'):
                raise ValueError('Runtime member escaped python/')
            members[name] = member

        def resolve(name, seen=()):
            if name in seen or len(seen) > 16:
                raise ValueError('Cyclic runtime link')
            safe_name(name)
            if not name.startswith('python/'):
                raise ValueError('Runtime link escaped python/')
            member = members[name]
            if member.isreg():
                return member
            if member.issym():
                return resolve(posixpath.normpath(posixpath.join(posixpath.dirname(name), member.linkname)), (*seen, name))
            if member.islnk():
                return resolve(posixpath.normpath(member.linkname), (*seen, name))
            raise ValueError(f'Unsupported runtime member: {name}')

        regular = [(name, resolve(name)) for name, member in members.items() if not member.isdir()]
        for name, _ in regular:
            safe_name(name.removeprefix('python/'))
        if 'python/' + entrypoint not in dict(regular):
            raise ValueError('Runtime entrypoint is missing')
        metadata_name = f'python/{site_packages}/pip-{PIP_VERSION}.dist-info/METADATA'
        metadata = email.message_from_bytes(source.extractfile(resolve(metadata_name)).read())
        if metadata.get('Name') != 'pip' or metadata.get('Version') != PIP_VERSION:
            raise ValueError('Runtime does not contain the reviewed pip version')
        return {
            'installed_bytes': sum(member.size for _, member in regular),
            'max_files': len(regular),
            'normalized_links': sum(member.issym() or member.islnk() for member in members.values()),
        }


def wheel_measurements(archive):
    with zipfile.ZipFile(archive) as source:
        total = count = record_bytes = 0
        members = reviewed_wheel_members(source, max_files=UNMEASURED_FINAL_FILES,
                                         max_member_bytes=UNMEASURED_FINAL_BYTES,
                                         max_total_bytes=UNMEASURED_FINAL_BYTES)
        for member in members:
            name = member.filename
            if member.is_dir():
                continue
            total += member.file_size
            count += 1
            # Add an entire replacement RECORD rather than estimating its delta.
            # Double UTF-8 path bytes allows CSV quoting; 512 bytes per row covers
            # relocated scheme prefixes, hash, decimal size, and separators.
            record_bytes += 2 * len(name.encode('utf-8')) + RECORD_ROW_OVERHEAD_BYTES
        return {'installed_bytes': total, 'max_files': count, 'record_bytes_cap': record_bytes}


def measure(cache, allow_unmeasured=False):
    config = json.loads((ROOT / 'packs.json').read_text(encoding='utf-8'))
    runtime_cache = {}
    wheel_cache = {}
    output = {}
    for pack in config['packs']:
        platform = pack['platform']
        lock = json.loads((ROOT / 'locks' / (pack['id'] + '.json')).read_text(encoding='utf-8'))
        if lock['platform'] != platform or lock['python'] != '3.12':
            raise ValueError('Wheel lock target differs from the runtime')
        component = None
        if any(own_wheel(wheel) for wheel in lock['wheels']):
            if pack['id'] != PACK_ID:
                raise ValueError('Own CPU wheel is not allowed in another engine recipe')
            component = cached_component(lock, cache, root=ROOT)
        pinned_runtime = config['runtime'][platform]
        site = 'Lib/site-packages' if platform == 'windows-x64' else 'lib/python3.12/site-packages'
        runtime_key = (pinned_runtime['sha256'], pinned_runtime['entrypoint'], site)
        if runtime_key not in runtime_cache:
            archive = verified_input(pinned_runtime, cache, 'runtime')
            runtime_cache[runtime_key] = runtime_measurements(archive, pinned_runtime['entrypoint'], site)
        measured_runtime = runtime_cache[runtime_key]
        runtime = {key: pinned_runtime[key] for key in ('url', 'bytes', 'sha256', 'entrypoint')}
        runtime.update({key: measured_runtime[key] for key in ('installed_bytes', 'max_files')})
        runtime.update(site_packages=site, pip_version=PIP_VERSION, count_kind='measured')
        wheels = []
        missing = []
        record_bytes = 0
        for pinned_wheel in lock['wheels']:
            validate_source(pinned_wheel, 'wheel', component)
            digest = pinned_wheel['sha256']
            if digest not in wheel_cache:
                if allow_unmeasured and not own_wheel(pinned_wheel) and not (cache / digest).exists() and not (cache / digest).is_symlink():
                    byte_cap = max(UNMEASURED_MIN_BYTES, pinned_wheel['bytes'] * UNMEASURED_EXPANSION_FACTOR)
                    if byte_cap > U64_MAX:
                        raise ValueError('Conservative extraction cap overflows u64')
                    wheel_cache[digest] = {
                        'installed_bytes': byte_cap,
                        'max_files': UNMEASURED_MAX_FILES,
                        'record_bytes_cap': 0,
                        'count_kind': 'conservative_cap',
                    }
                else:
                    archive = verified_input(pinned_wheel, cache, 'wheel', component)
                    wheel_cache[digest] = {**wheel_measurements(archive), 'count_kind': 'measured'}
            measured_wheel = wheel_cache[digest]
            wheel = {key: pinned_wheel[key] for key in ('name', 'version', 'filename', 'url', 'bytes', 'sha256')}
            wheel.update({key: measured_wheel[key] for key in ('installed_bytes', 'max_files', 'count_kind')})
            wheels.append(wheel)
            record_bytes += measured_wheel['record_bytes_cap']
            if wheel['count_kind'] == 'conservative_cap':
                missing.append({key: wheel[key] for key in ('name', 'version', 'filename', 'url', 'bytes', 'sha256')})
        allowance_bytes = ASSEMBLY_BASE_BYTES + PER_WHEEL_METADATA_BYTES * len(wheels) + record_bytes
        allowance_files = ASSEMBLY_BASE_FILES + PER_WHEEL_METADATA_FILES * len(wheels)
        raw_bytes = runtime['installed_bytes'] + sum(w['installed_bytes'] for w in wheels)
        raw_files = runtime['max_files'] + sum(w['max_files'] for w in wheels)
        installed_bytes_cap = UNMEASURED_FINAL_BYTES if missing else raw_bytes + allowance_bytes
        final_files_cap = UNMEASURED_FINAL_FILES if missing else raw_files + allowance_files
        if max(raw_bytes, raw_files, installed_bytes_cap, final_files_cap) > U64_MAX:
            raise ValueError('Aggregate extraction cap overflows u64')
        output[pack['id']] = {
            'runtime': runtime,
            'wheels': wheels,
            'download_bytes': runtime['bytes'] + sum(w['bytes'] for w in wheels),
            'installed_bytes': installed_bytes_cap,
            'max_files': final_files_cap,
            'count_kind': 'conservative_cap',
            'assembly_allowance': {
                'installed_bytes': allowance_bytes,
                'max_files': allowance_files,
                'base_bytes_cap': ASSEMBLY_BASE_BYTES,
                'base_files_cap': ASSEMBLY_BASE_FILES,
                'per_wheel_metadata_bytes_cap': PER_WHEEL_METADATA_BYTES,
                'per_wheel_metadata_files_cap': PER_WHEEL_METADATA_FILES,
                'replacement_record_bytes_cap': record_bytes,
                'applies_to': 'Measured-input cap derivation; mixed-input packs instead use the independent 4 GiB/100,000-file final-tree cap.' if missing else 'Final-tree cap is the input totals plus this allowance.',
            },
            'measurement': {
                'method': 'size-and-sha256-verified archives; no installation or execution',
                'runtime_links_normalized': measured_runtime['normalized_links'],
                'input_expanded_bytes_cap': raw_bytes,
                'input_regular_files_cap': raw_files,
                'measured_input_expanded_bytes': runtime['installed_bytes'] + sum(w['installed_bytes'] for w in wheels if w['count_kind'] == 'measured'),
                'measured_input_regular_files': runtime['max_files'] + sum(w['max_files'] for w in wheels if w['count_kind'] == 'measured'),
                'input_bounds': 'Measured entries are exact unpruned regular-member totals, including every regular wheel ZIP member and dereferenced runtime links. Conservative entries are unmeasured enforced limits: max(16 MiB, compressed bytes * 16), and 8,192 regular files. An unseen wheel may exceed the cap and must then fail closed.',
                'output_bounds': 'Caps, not measured installed usage. All-measured packs add complete replacement RECORD allowances, 64 KiB/eight metadata files per wheel, and 2 MiB/256 fixed assembly files. Packs with any unmeasured input instead have an independent 4 GiB/100,000-file final-tree ceiling. Final output must be checked against its cap; per-input file caps are not a prediction that all inputs reach those maxima.',
                'all_inputs_measured': not missing,
                'unmeasured_wheels': missing,
                'inference_verified': False,
            },
        }
    return output


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--cache', type=Path, required=True)
    parser.add_argument('--output', type=Path, default=ROOT / 'recipe-caps.json')
    parser.add_argument('--check', action='store_true', help='Fail if the committed output differs; do not write.')
    parser.add_argument('--allow-unmeasured', action='store_true', help='Emit explicitly labeled conservative caps for missing wheels; never download them.')
    args = parser.parse_args()
    measured = measure(args.cache.resolve(), args.allow_unmeasured)
    contents = json.dumps(measured, ensure_ascii=False, indent=2, sort_keys=True) + '\n'
    if args.check:
        if args.output.read_text(encoding='utf-8') != contents:
            raise ValueError('Recipe caps differ from the verified pinned inputs')
    else:
        args.output.write_text(contents, encoding='utf-8', newline='\n')
    for pack_id, recipe in measured.items():
        print(f'{pack_id}: {len(recipe["wheels"])} wheels; {recipe["download_bytes"]} download bytes; '
              f'output CAP {recipe["installed_bytes"]} bytes / {recipe["max_files"]} files')


if __name__ == '__main__':
    main()
