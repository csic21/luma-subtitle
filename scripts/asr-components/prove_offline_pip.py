#!/usr/bin/env python3
"""Native proof of the shared, private offline assembly entrypoint. No uploads."""
import argparse
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
from build import ROOT, build, dump, sha256


def owned(command, timeout=900, env=None):
    child = subprocess.Popen(command, env=env)
    try:
        result = child.wait(timeout=timeout)
    except BaseException:
        child.kill(); child.wait()
        raise
    if result:
        raise RuntimeError(f'Proof child failed: {result}')


def prove_native_installer(pack_id, output, cache, recipe_candidates):
    candidate = next(runtime for runtime in json.loads(recipe_candidates.read_text(encoding='utf-8'))['runtimes'] if runtime['id'] == pack_id)
    recipe = candidate.get('recipe')
    if recipe is None:
        return {'tested': False, 'reason': candidate['unavailable_reason']}
    inputs = output / 'recipe inputs é 测试'; inputs.mkdir()
    runtime_path = output / 'recipe-runtime.json'
    dump(runtime_path, candidate)
    artifacts = [(recipe['python'], recipe['python']['sha256'])] + [(wheel, wheel['filename']) for wheel in recipe['wheels']]
    if recipe.get('windows_crt'):
        from windows_crt_proof import CONTRACT, ID
        if recipe['windows_crt'] != ID: raise ValueError('Unreviewed direct CRT recipe')
        crt = json.loads(CONTRACT.read_text(encoding='utf-8'))['installer']
        artifacts.append((crt, crt['sha256']))
    for artifact, name in artifacts:
        source = cache / artifact['sha256']
        if source.is_symlink() or not source.is_file() or source.stat().st_size != artifact['bytes'] or sha256(source) != artifact['sha256']:
            raise RuntimeError('Native installer proof requires the already verified input cache')
        shutil.copyfile(source, inputs / name)
    env = dict(os.environ, LUMA_ASR_RECIPE_RUNTIME=str(runtime_path), LUMA_ASR_RECIPE_INPUTS=str(inputs))
    test = 'native_direct_recipe_installs_repairs_and_removes'
    owned(['cargo', 'test', '--manifest-path', str(ROOT.parent.parent / 'src-tauri/Cargo.toml'), '--locked',
           test, '--', '--ignored', '--nocapture'], timeout=1800, env=env)
    shutil.rmtree(inputs); runtime_path.unlink()
    return {'tested': True, 'passed': True, 'test': test}


def main():
    parser = argparse.ArgumentParser(); parser.add_argument('--pack', required=True); parser.add_argument('--source-sha', required=True)
    parser.add_argument('--output', type=Path, required=True); parser.add_argument('--cache', type=Path, required=True)
    parser.add_argument('--worker', type=Path, required=True)
    parser.add_argument('--recipe-candidates', type=Path, required=True)
    args = parser.parse_args(); output = args.output.resolve(); cache = args.cache.resolve()
    candidate = next(item for item in json.loads(args.recipe_candidates.read_text(encoding='utf-8'))['runtimes'] if item['id'] == args.pack)
    windows_crt = None
    if candidate.get('recipe', {}).get('windows_crt'):
        from windows_crt_proof import ID, prepare
        if candidate['recipe']['windows_crt'] != ID: raise ValueError('Unreviewed direct CRT recipe')
        windows_crt = prepare(output, cache, args.source_sha)
    first, second = output / 'first', output / 'second'
    one = build(args.pack, first, cache, args.source_sha, installer='pip', windows_crt=windows_crt)
    two = build(args.pack, second, cache, args.source_sha, installer='pip', windows_crt=windows_crt)
    crt_signatures = None
    if windows_crt:
        crt_signatures = json.loads((output / 'direct-crt-signatures.json').read_text(encoding='utf-8-sig'))
        shutil.rmtree(windows_crt)
    assert one['archive'] == two['archive'], 'Offline pip output must reproduce exactly'
    assert (first / (args.pack + '.manifest.json')).read_bytes() == (second / (args.pack + '.manifest.json')).read_bytes()
    assembly = json.loads((first / 'staging' / args.pack / 'ASSEMBLY.json').read_text())
    shutil.rmtree(second)  # Reclaim only this proof's disposable build output.
    owned([sys.executable, '-B', str(ROOT / 'smoke.py'), '--manifest', str(first / (args.pack + '.manifest.json')),
           '--worker', str(args.worker.resolve()), '--cache', str(cache)])
    smoke = json.loads((first / (args.pack + '.smoke.json')).read_text())
    # Exact archive/rebuild/import evidence is in memory. Reclaim only this
    # disposable reference output before Rust exercises its independent staging,
    # installed copy and atomic repair. User-space requirements stay enforced.
    shutil.rmtree(first)
    native_installer = prove_native_installer(args.pack, output, cache, args.recipe_candidates.resolve())
    app_helper = None
    if windows_crt:
        from windows_crt_proof import prove_app_helper
        app_helper = prove_app_helper(cache, output)
    report = {'schema': 1, 'pack_id': args.pack, 'source_sha': args.source_sha,
              'method': 'private-offline-pip', 'reproducible': True,
              'recipe_candidates_sha256': sha256(args.recipe_candidates),
              'assembly': assembly, 'runtime_smoke': smoke,
              'windows_crt_signatures': crt_signatures,
              'app_helper_test': app_helper,
              'native_installer_test': native_installer, 'publication_authorized': False}
    destination = output / (args.pack + '.offline-pip-proof.json')
    dump(destination, report); print(json.dumps(report, indent=2, ensure_ascii=False))
    print('OFFLINE_PIP_PROOF_SHA256=' + sha256(destination))
    # The proof's binaries were reclaimed above and are never uploaded.


if __name__ == '__main__': main()
