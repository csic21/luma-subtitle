"""Request-only source-pinned Qwen ASR+alignment proof. No release operations."""
import argparse
import json
from pathlib import Path
import shutil
import subprocess
import sys
import time

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))
from build import build, dump, embed_nagisa, sha256
from smoke import clean_environment, diagnostic_json
from prove_offline_pip import owned
from fixture import run as model_smoke, bounded_worker_requests


def validate_ready(report, pack, source):
    if (report['schema'] != 1 or report['source_sha'] != source or report['pack_id'] != pack
            or report['method'] != 'private-offline-pip' or report['reproducible'] is not True
            or report['native_installer_test'].get('passed') is not True
            or report['runtime_smoke']['relocated'] is not True
            or report['runtime_smoke']['imports']['tested_device'] != 'cpu'
            or report['runtime_smoke']['imports']['isolated'] is not True
            or report['runtime_smoke']['imports']['user_site'] is not False
            or report['runtime_smoke']['system_python_used'] is not False
            or report['runtime_smoke']['system_packages_used'] is not False
            or report['runtime_smoke']['worker_test']['passed'] is not True):
        raise ValueError('A genuine exact-source runtime/installer proof must pass before weights')
    if pack.endswith('windows-x64'):
        if (report.get('app_helper_test', {}).get('passed') is not True
                or report['runtime_smoke']['windows_native_inventory']['passed'] is not True
                or report['runtime_smoke']['nagisa_unicode']['japanese_tokens_match'] is not True):
            raise ValueError('Windows requires genuine GUI helper, native closure and Unicode Japanese proof before weights')


def cleanup_private_outputs(output, pack):
    """Only the newly created proof directory is owned; preserve JSON evidence."""
    errors=[]
    def remove(path):
        try:
            if path.is_symlink() or path.is_file(): path.unlink()
            else: shutil.rmtree(path)
        except OSError as exc: errors.append(f'{path.name}: {exc}')
    for path in output.iterdir():
        if path.name == pack+'.qwen-inference-proof.json' and path.is_file() and not path.is_symlink():
            continue
        if path.name == 'native-readiness' and path.is_dir() and not path.is_symlink():
            for child in path.iterdir():
                if child.name == pack+'.offline-pip-proof.json' and child.is_file() and not child.is_symlink():
                    continue
                remove(child)
        else: remove(path)
    return errors


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--source-sha', required=True)
    parser.add_argument('--pack', choices=['qwen3-asr-cpu-windows-x64','qwen3-asr-cpu-macos-arm64'], required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--request-copy', type=Path, required=True)
    args=parser.parse_args();repo=ROOT.parent.parent;output=args.output.resolve()
    if len(args.source_sha)!=40 or any(c not in '0123456789abcdef' for c in args.source_sha): raise ValueError('Exact source required')
    head=subprocess.check_output(['git','rev-parse','HEAD'],cwd=repo,text=True).strip()
    if head!=args.source_sha: raise ValueError('Checkout differs from requested tested source')
    request=json.loads(args.request_copy.read_text(encoding='utf-8'))
    if request['source_sha']!=args.source_sha or request['pack_id']!=args.pack or request['purpose']!='one-time-feature-branch-qwen-inference-proof':
        raise ValueError('Validated request differs from proof invocation')
    output.mkdir(parents=True,exist_ok=False)
    cache=output/'private-runtime-cache';native=output/'native-readiness'
    candidates=output/'recipe-candidates.json';fresh=output/'single-inference-runtime'
    native_report=native/(args.pack+'.offline-pip-proof.json')
    report={'schema':1,'source_sha':args.source_sha,'pack_id':args.pack,'purpose':request['purpose'],
            'request_sha256':sha256(args.request_copy),'fresh_native_proof_sha256':None,
            'prior_native_evidence':request['prior_native_evidence'],'metadata_only':True,'model_cache_uploaded':False,
            'inference':{'tested':False,'weights_downloaded':False},'stage':'native-readiness'}
    destination=output/(args.pack+'.qwen-inference-proof.json');started=time.monotonic()
    try:
        owned([sys.executable,'-B',str(ROOT/'generate_recipes.py'),'--output',str(candidates)])
        worker=repo/'src-tauri/src/asr/worker.py'
        owned([sys.executable,'-B',str(ROOT/'prove_offline_pip.py'),'--pack',args.pack,'--source-sha',args.source_sha,
               '--output',str(native),'--cache',str(cache),'--worker',str(worker),'--recipe-candidates',str(candidates)],timeout=3000)
        readiness=json.loads(native_report.read_text(encoding='utf-8'));validate_ready(readiness,args.pack,args.source_sha)
        report['fresh_native_proof_sha256']=sha256(native_report)
        # Actual install/repair/cancel/remove and Windows GUI helper have passed;
        # the native proof removed duplicate assemblies. Build only one runtime.
        report['stage']='single-inference-runtime';crt=None
        if args.pack.endswith('windows-x64'):
            from windows_crt_proof import prepare
            crt=prepare(fresh,cache,args.source_sha)
        manifest=build(args.pack,fresh,cache,args.source_sha,installer='pip',windows_crt=crt)
        if crt:shutil.rmtree(crt)
        archive=fresh/manifest['archive']['url'].rsplit('/',1)[-1];archive.unlink()
        runtime=fresh/'staging'/args.pack
        env=clean_environment(output/'private-inference-home')
        actual_worker=worker
        if args.pack.endswith('windows-x64'):
            actual_worker=output/'managed-inference-worker.py'
            actual_worker.write_text(embed_nagisa(worker.read_text(encoding='utf-8'),managed_worker=True),encoding='utf-8')
        catalog=json.loads((repo/'src-tauri/resources/asr/catalog.json').read_text(encoding='utf-8'))
        audio=json.loads((ROOT/'fixtures.json').read_text(encoding='utf-8'))['audio']
        report['stage']='resource-gated-model-inference'
        model_smoke(runtime/manifest['entrypoint'],actual_worker,env,output,catalog,audio,bounded_worker_requests,report=report['inference'])
        report['outcome']='passed' if report['inference']['tested'] else 'skipped-resource-gate'
    except BaseException as exc:
        report.update(outcome='failed',error=f'{type(exc).__name__}: {str(exc)[:2000]}')
        raise
    finally:
        # Failure at any stage retains evidence and removes the private outputs.
        errors=cleanup_private_outputs(output,args.pack)
        report['private_outputs_removed']=not errors
        if errors: report.update(outcome='failed',cleanup_errors=errors)
        report['elapsed_seconds']=time.monotonic()-started
        dump(destination,report);print(diagnostic_json(report,indent=2),flush=True)
        print('QWEN_INFERENCE_PROOF_SHA256='+sha256(destination),flush=True)
    if report['outcome']=='failed': raise RuntimeError('Private proof cleanup failed')


if __name__=='__main__':main()
