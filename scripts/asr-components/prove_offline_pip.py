#!/usr/bin/env python3
"""Native proof of the shared, private offline assembly entrypoint. No uploads."""
import argparse
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
from build import ROOT, build, dump, sha256
from smoke import diagnostic_json
from prepare_cpu_proof import PACK_ID, shipping_cpu_lock


def owned(command, timeout=900, env=None):
    child = subprocess.Popen(command, env=env)
    try:
        result = child.wait(timeout=timeout)
    except BaseException:
        child.kill(); child.wait()
        raise
    if result:
        raise RuntimeError(f'Proof child failed: {result}')


def validate_native_worker_lifecycle(lifecycle,fixture,source_sha):
    expected=['model-replacement','release-idle','legacy-release','active-cancellation','shutdown']
    if (not re.fullmatch('[a-f0-9]{40}',source_sha or '') or lifecycle.get('schema')!=1
            or lifecycle.get('passed') is not True or lifecycle.get('verifier_source_sha')!=source_sha
            or lifecycle.get('embedded_worker_sha256')!=sha256(ROOT.parent.parent/'src-tauri/src/asr/worker.py')
            or lifecycle.get('model_revision')!=fixture['model_revision']
            or lifecycle.get('audio_sha256')!=fixture['audio_sha256']
            or lifecycle.get('managed_receipt_policy_selection_tested') is not True
            or lifecycle.get('cpu_thread_policy')!='luma-cpu-seq-1:cpu_threads=1'
            or lifecycle.get('global_managed_path_selection_tested') is not False
            or any(lifecycle.get(key) is not True for key in ('injected_lease_retention_tested',
                    'cold_warm_srt_export_tested','active_cancellation_recovery_tested'))
            or [t.get('operation') for t in lifecycle.get('transitions',[])]!=expected
            or any(t.get('process_exited_when_lease_available') is not True or t.get('script_removed') is not True
                   for t in lifecycle['transitions'])):
        raise ValueError('Receipt-selected native worker lifecycle evidence is incomplete or mismatched')


def prove_native_installer(pack_id, output, cache, recipe_candidates, *, progress=None,source_sha=None):
    candidate = next(runtime for runtime in json.loads(recipe_candidates.read_text(encoding='utf-8'))['runtimes'] if runtime['id'] == pack_id)
    from real_worker_fixture import final_cpu_recipe, prepare
    worker_required = pack_id == PACK_ID and shipping_cpu_lock() is not None
    if worker_required and not final_cpu_recipe(candidate):
        raise ValueError('Published own CPU recipe requires the receipt-selected native worker lifecycle')
    recipe = candidate.get('recipe')
    if recipe is None:
        return {'tested': False, 'reason': candidate['unavailable_reason']}
    inputs = output / 'recipe inputs é 测试'; inputs.mkdir()
    fixtures = output/'real worker fixtures é 测试'; owns_fixtures = False
    runtime_path = output/'recipe-runtime.json'
    tree_drained=True
    try:
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
        env = {key:value for key,value in os.environ.items() if not key.upper().startswith('LUMA_ASR_TEST_')}
        env.update(LUMA_ASR_RECIPE_RUNTIME=str(runtime_path),LUMA_ASR_RECIPE_INPUTS=str(inputs))
        if source_sha is not None:env['LUMA_ASR_TEST_VERIFIER_SOURCE_SHA']=source_sha
        worker_fixture = None
        if final_cpu_recipe(candidate):
            if fixtures.exists(): raise ValueError('Worker fixture output must be fresh')
            owns_fixtures = True
            fixture_env, worker_fixture = prepare(cache,fixtures); env.update(fixture_env)
        test = 'native_direct_recipe_installs_repairs_and_removes'
        result={'tested': True, 'passed': False, 'test': test,
                'real_worker_lifecycle': {'tested': bool(worker_fixture), 'fixtures': worker_fixture,
                'managed_use_lease_tested': False,'global_managed_path_selection_tested':False,
                'scope': 'Receipt-selected production CPU policy under a test-owned managed root/catalog; global embedded-catalog selection is not tested.'}}
        command=['cargo', 'test', '--manifest-path', str(ROOT.parent.parent / 'src-tauri/Cargo.toml'), '--locked',
                 test, '--', '--ignored', '--nocapture']
        if sys.platform=='win32':
            from windows_test_job import JobRunError, run_owned_cargo_test
            cargo=shutil.which('cargo')
            if not cargo:raise ValueError('Native Cargo build tool is missing')
            command[0]=str(Path(cargo).resolve(strict=True));job_result=None
            result['windows_owned_job']={'tree_drained':False,'job_start_pending':True}
            if progress:progress(result)
            tree_drained=False
            try:
                job_result=run_owned_cargo_test(command[0],manifest=ROOT.parent.parent/'src-tauri/Cargo.toml',
                                              test_name='asr_components::tests::'+test,env=env,timeout=1800,output_limit=8*1024*1024)
            except JobRunError as error:
                job_result=error.result;raise
            finally:
                if job_result is not None:
                    tree_drained=job_result.get('tree_drained') is True
                    result['windows_owned_job']={key:value for key,value in job_result.items() if key not in ('stdout','stderr')}
                    # These are the same Cargo/test streams formerly inherited
                    # by this process. The supervisor has already bounded them.
                    print(job_result.get('stdout',''),end='',flush=True)
                    print(job_result.get('stderr',''),end='',file=sys.stderr,flush=True)
                if progress:progress(result)
            if not tree_drained:raise RuntimeError('Native installer Windows Job drain was not confirmed')
            if job_result['exit_code']!=0:raise RuntimeError(f"Native installer Cargo failed: {job_result['exit_code']}")
        else:owned(command,timeout=1800,env=env)
        if worker_fixture:
            from real_worker_fixture import bounded_json
            lifecycle=bounded_json(Path(env['LUMA_ASR_TEST_OUTPUT'])/'lifecycle.json',fixtures,64*1024)
            result['real_worker_lifecycle']['lifecycle']=lifecycle
            if progress:progress(result)
            validate_native_worker_lifecycle(lifecycle,worker_fixture,source_sha)
            result['real_worker_lifecycle']['managed_use_lease_tested']=True
        result['passed']=True
        return result
    finally:
        if tree_drained:
            shutil.rmtree(inputs)
            runtime_path.unlink(missing_ok=True)
            if owns_fixtures and fixtures.exists(): shutil.rmtree(fixtures)


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
    crt_signatures = None;crt_cab_paths=None
    if windows_crt:
        crt_signatures = json.loads((output / 'direct-crt-signatures.json').read_text(encoding='utf-8-sig'))
        crt_cab_paths = json.loads((output / 'direct-crt-cab-paths.json').read_text(encoding='utf-8'))
        shutil.rmtree(windows_crt)
    assert one['archive'] == two['archive'], 'Offline pip output must reproduce exactly'
    assert (first / (args.pack + '.manifest.json')).read_bytes() == (second / (args.pack + '.manifest.json')).read_bytes()
    assembly = json.loads((first / 'staging' / args.pack / 'ASSEMBLY.json').read_text(encoding='utf-8'))
    shutil.rmtree(second)  # Reclaim only this proof's disposable build output.
    owned([sys.executable, '-B', str(ROOT / 'smoke.py'), '--manifest', str(first / (args.pack + '.manifest.json')),
           '--worker', str(args.worker.resolve()), '--cache', str(cache)])
    smoke = json.loads((first / (args.pack + '.smoke.json')).read_text(encoding='utf-8'))
    # Exact archive/rebuild/import evidence is in memory. Reclaim only this
    # disposable reference output before Rust exercises its independent staging,
    # installed copy and atomic repair. User-space requirements stay enforced.
    shutil.rmtree(first)
    report = {'schema': 1, 'pack_id': args.pack, 'source_sha': args.source_sha,
              'method': 'private-offline-pip', 'reproducible': True,
              'recipe_candidates_sha256': sha256(args.recipe_candidates),
              'assembly': assembly, 'runtime_smoke': smoke,
              'windows_crt_signatures': crt_signatures,
              'windows_crt_cab_paths': crt_cab_paths,
              'app_helper_test': None,
              'native_installer_test': {'tested':False,'passed':False}, 'publication_authorized': False}
    destination = output / (args.pack + '.offline-pip-proof.json')
    def native_progress(value):
        report['native_installer_test']=value
        # Proof-housekeeping checkpoint, not production runtime state. A killed
        # caller must not mistake missing final drain evidence for safe cleanup.
        dump(destination,{**report,'passed':False,'stage':'native-installer-running'})
    stage='native-installer'
    try:
        report['native_installer_test']=prove_native_installer(args.pack,output,cache,args.recipe_candidates.resolve(),progress=native_progress,source_sha=args.source_sha)
        stage='app-helper'
        if windows_crt:
            from windows_crt_proof import prove_app_helper
            report['app_helper_test']=prove_app_helper(cache,output)
    except BaseException as error:
        report.update(passed=False,stage=stage,error=type(error).__name__+': '+str(error)[:2000])
        dump(destination,report);print(diagnostic_json(report,indent=2),flush=True)
        raise
    dump(destination, report); print(diagnostic_json(report, indent=2))
    print('OFFLINE_PIP_PROOF_SHA256=' + sha256(destination))
    # The proof's binaries were reclaimed above and are never uploaded.


if __name__ == '__main__': main()
