#!/usr/bin/env python3
"""Maintainer-only lock generation. Never called by the app or component CI.

Requires an explicitly installed uv and packaging. Review every resulting lock
change. Build consumers use exact JSON URLs/hashes and never invoke a resolver.
"""
import argparse
from concurrent.futures import ThreadPoolExecutor
import hashlib
import json
from pathlib import Path
import re
import urllib.request
from packaging.tags import cpython_tags, compatible_tags, mac_platforms
from packaging.utils import parse_wheel_filename

ROOT = Path(__file__).resolve().parent


def request(url):
    for attempt in range(3):
        try:
            with urllib.request.urlopen(url, timeout=120) as response:
                return response.read()
        except Exception:
            if attempt == 2:
                raise


def wheel_for(name, version, platform, hashes):
    platforms = ['win_amd64'] if platform == 'windows-x64' else list(mac_platforms((14, 0), 'arm64'))
    tags = list(cpython_tags((3, 12), platforms=platforms)) + list(compatible_tags((3, 12), 'cp312', platforms))
    ranking = {tag: i for i, tag in enumerate(tags)}
    # Official publisher uploads on PyPI; no index/host substitution.
    data = json.loads(request(f'https://pypi.org/pypi/{name}/{version}/json'))
    info = data['info']; candidates = []
    source = [{k: entry[k] for k in ('filename', 'url', 'size', 'digests')} for entry in data['urls'] if entry['packagetype'] == 'sdist']
    for entry in data['urls']:
        if entry['packagetype'] != 'bdist_wheel' or entry.get('yanked'):
            continue
        _, _, _, wt = parse_wheel_filename(entry['filename'])
        if wt.intersection(ranking):
            candidates.append((min(ranking[t] for t in wt if t in ranking), entry))
    if not candidates:
        raise RuntimeError(f'No {platform} cp312 wheel: {name}=={version}')
    entry = min(candidates, key=lambda c: c[0])[1]
    digest = entry['digests']['sha256']
    if digest not in hashes:
        raise RuntimeError(f'Wheel hash is not in the reviewed resolver result: {entry["filename"]}')
    size = entry.get('size')
    if size is None:
        with urllib.request.urlopen(urllib.request.Request(entry['url'], method='HEAD'), timeout=120) as response:
            size = int(response.headers['Content-Length'])
    return {'name': name, 'version': version, 'filename': entry['filename'], 'url': entry['url'], 'bytes': size, 'sha256': digest,
            'license': info.get('license_expression') or info.get('license') or 'See wheel notices',
            'project_urls': info.get('project_urls') or {}, 'sources': source}


def main():
    ap = argparse.ArgumentParser(); ap.add_argument('--requirements', type=Path, required=True); ap.add_argument('--platform', choices=['windows-x64','macos-arm64'], required=True); ap.add_argument('--output', type=Path, required=True)
    args = ap.parse_args()
    text = args.requirements.read_text(encoding='utf-8')
    lines = re.split(r'\n(?=[A-Za-z0-9][A-Za-z0-9_.-]*==)', text)
    packages = []
    for line in lines:
        match = re.match(r'([A-Za-z0-9_.-]+)==([^\s\\]+)', line)
        if match:
            packages.append((*match.groups(), args.platform, set(re.findall(r'--hash=sha256:([0-9a-f]{64})', line))))
    with ThreadPoolExecutor(max_workers=8) as pool:
        wheels = sorted(pool.map(lambda p: wheel_for(*p), packages), key=lambda w: w['name'])
    doc = {'schema': 1, 'platform': args.platform, 'python': '3.12', 'resolution_date': '2026-10-09', 'resolver': 'uv 0.12.23',
           'requirements_sha256': hashlib.sha256(args.requirements.read_bytes()).hexdigest(), 'wheels': wheels}
    args.output.write_text(json.dumps(doc, ensure_ascii=False, indent=2) + '\n', encoding='utf-8')
    print(args.output, len(wheels), 'wheels;', sum(w['bytes'] for w in wheels), 'download bytes')


if __name__ == '__main__':
    main()
