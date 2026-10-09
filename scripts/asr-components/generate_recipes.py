#!/usr/bin/env python3
"""Generate review-only runtime candidates from committed upstream pins.

No download or catalog activation occurs. Every generated candidate remains
unavailable until separate native, license, and dependency review is complete.
"""
import argparse
import hashlib
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parent
REPO = ROOT.parent.parent
PYTHON_FIELDS = ('url', 'bytes', 'sha256', 'installed_bytes', 'max_files', 'entrypoint', 'site_packages', 'pip_version')
WHEEL_FIELDS = ('name', 'version', 'filename', 'url', 'bytes', 'sha256', 'installed_bytes', 'max_files')
TERM_FIELDS = ('id', 'version', 'sha256', 'url', 'raw_sha256', 'source_encoding')


def digest(data):
    return hashlib.sha256(data).hexdigest()


def inventory_text(pack, lock):
    lines = [f"{pack['label']}: fixed upstream dependency inventory", '',
             'This disclosure records the selected upstream artifacts and their declared license metadata.',
             'It does not replace upstream license texts, expand a license grant, or establish redistribution clearance.',
             'Model weights are a separate optional download and are not included in this engine recipe.', '',
             'Native imports and inference support must be verified separately before this candidate is enabled.', '']
    for wheel in lock['wheels']:
        lines.extend([f"{wheel['name']} {wheel['version']}",
                      'Declared license metadata: ' + (wheel.get('license') or 'not declared'),
                      'Artifact: ' + wheel['filename'], 'Official source: ' + wheel['url'],
                      'SHA-256: ' + wheel['sha256'], ''])
    return '\n'.join(lines)


def verified_term(record, directory):
    path = directory / record['path']
    if path.is_symlink() or not path.resolve().is_relative_to(directory.resolve()):
        raise ValueError('Term text escaped its source directory')
    data = path.read_bytes()
    if digest(data) != record['sha256']:
        raise ValueError('Term display hash mismatch: ' + record['id'])
    return {**{key: record[key] for key in TERM_FIELDS if key in record}, 'text': data.decode('utf-8')}


def generate(catalog, caps, config, proprietary, terms_directory):
    license_lock = json.loads((ROOT / 'licenses.lock.json').read_text(encoding='utf-8'))
    candidates = []
    for current in catalog['runtimes']:
        runtime = dict(current)
        runtime.pop('archive', None); runtime.pop('recipe', None); runtime.pop('plan_sha256', None)
        pack_id = runtime['id']
        # The original official CT2 wheel contains NVIDIA-restricted artifacts.
        # A CPU-user acknowledgement cannot authorize an excluded hardware use.
        if pack_id == 'faster-whisper-cpu-windows-x64':
            runtime.update(installed_bytes=0, max_files=0,
                           unavailable_reason='A reviewed CPU-only CTranslate2 wheel and complete app-local native dependencies are required before this engine can be installed.')
            candidates.append(runtime); continue
        pack = next(pack for pack in config['packs'] if pack['id'] == pack_id)
        bound = caps[pack_id]
        lock = json.loads((ROOT / 'locks' / (pack_id + '.json')).read_text(encoding='utf-8'))
        if len(bound['wheels']) != len(lock['wheels']):
            raise ValueError('Recipe caps are stale')
        for cap, wheel in zip(bound['wheels'], lock['wheels']):
            if any(cap[key] != wheel[key] for key in ('name', 'version', 'filename', 'url', 'bytes', 'sha256')):
                raise ValueError('Recipe caps differ from locked wheel inputs')
        notice = next(item for item in license_lock['files'] if item['platform'] == runtime['platform'] and item['path'].endswith('/LICENSE.cpython.txt'))
        raw = (ROOT / notice['path']).read_bytes()
        if len(raw) != notice['bytes'] or digest(raw) != notice['sha256']:
            raise ValueError('Python license notice differs from its lock')
        text = raw.decode('utf-8').replace('\r\n', '\n')
        terms = [{'id': 'python-3-12-15-license', 'version': '3.12.15', 'text': text,
                  'sha256': digest(text.encode('utf-8')), 'raw_sha256': notice['sha256'],
                  'source_encoding': 'utf-8', 'url': 'https://docs.python.org/3.12/license.html'}]
        disclosure = inventory_text(pack, lock)
        engine_name = 'mlx-whisper' if pack['backend'] == 'mlx-whisper' else 'qwen-asr'
        engine = next(wheel for wheel in lock['wheels'] if wheel['name'] == engine_name)
        terms.append({'id': pack_id + '-dependency-inventory', 'version': config['version'],
                      'text': disclosure, 'sha256': digest(disclosure.encode('utf-8')),
                      'url': f'https://pypi.org/project/{engine_name}/{engine["version"]}/'})
        terms.extend(verified_term(term, terms_directory) for term in proprietary['terms'] if pack_id in term['applies_to'])
        engine_lock = ROOT / 'engine-terms.lock.json'
        if engine_lock.exists():
            for term in json.loads(engine_lock.read_text(encoding='utf-8'))['terms']:
                if pack_id in term['applies_to']:
                    terms.append(verified_term(term, ROOT))
        runtime.update(installed_bytes=bound['installed_bytes'], max_files=bound['max_files'],
                       license='Multiple upstream licenses and notices; review the exact setup terms',
                       unavailable_reason='Direct-upstream setup is awaiting final native assembly, engine-license and dependency review. This candidate cannot be installed yet.',
                       recipe={'schema': 1, 'python': {key: bound['runtime'][key] for key in PYTHON_FIELDS},
                               'wheels': [{key: wheel[key] for key in WHEEL_FIELDS} for wheel in bound['wheels']],
                               'terms': terms})
        candidates.append(runtime)
    return {'schema': 1, 'status': 'review-only-not-active', 'runtimes': candidates,
            'evidence': 'Per-input extraction limits and their measured/conservative classification are in recipe-caps.json. This file never activates catalog entries; all unavailable reasons must be preserved until final review.'}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--check', action='store_true')
    args = parser.parse_args()
    terms = REPO / 'src-tauri/resources/asr/terms'
    result = generate(json.loads((REPO / 'src-tauri/resources/asr/catalog.json').read_text(encoding='utf-8')),
                      json.loads((ROOT / 'recipe-caps.json').read_text(encoding='utf-8')),
                      json.loads((ROOT / 'packs.json').read_text(encoding='utf-8')),
                      json.loads((terms / 'sources.json').read_text(encoding='utf-8')), terms)
    data = json.dumps(result, ensure_ascii=False, indent=2, sort_keys=True) + '\n'
    if args.check:
        if args.output.read_text(encoding='utf-8') != data: raise ValueError('Review-only recipe candidates are stale')
    else:
        args.output.write_text(data, encoding='utf-8', newline='\n')
    print('Review-only recipe candidates:', len(result['runtimes']))
    print('SHA-256:', digest(data.encode('utf-8')))


if __name__ == '__main__': main()
