"""Small cached Tiny/JFK fixtures for the existing real Rust worker test."""
import json
import os
from pathlib import Path
import re
import shutil
import stat
import sys
import wave
from build import ROOT, dump, safe_name, sha256

CPU_FILENAME = 'ctranslate2-4.8.2-1lumacpu-cp312-cp312-win_amd64.whl'
CPU_URL = 'https://github.com/csic21/luma-subtitle/releases/download/asr-ct2-cpu-4.8.2-1/' + CPU_FILENAME
REPEATS = 100
MAX_REPEAT_BYTES = 36 * 1024 * 1024
REPLAY_PRODUCER = '3006b955bb248e83e95f2dcc48c3001194513800'
REPLAY_WHEEL_SHA = '4644c5eb94492ab61d9749ee122e12c612c03634495d9cb59b6c18e3404cd1c0'
REPLAY_WHEEL_BYTES = 25_058_630


def final_cpu_recipe(candidate):
    recipe = candidate.get('recipe')
    if candidate.get('id') != 'faster-whisper-cpu-windows-x64' or not recipe:
        return False
    matches = [w for w in recipe['wheels'] if w['name'] == 'ctranslate2']
    if len(matches) != 1: return False
    wheel = matches[0]
    if (wheel['filename'], wheel['url'], wheel['version']) != (CPU_FILENAME, CPU_URL, '4.8.2'):
        return False
    pinned = next(w for w in json.loads((ROOT/'locks/faster-whisper-cpu-windows-x64.json').read_text(encoding='utf-8'))['wheels'] if w['name'] == 'ctranslate2')
    if any(wheel[field] != pinned[field] for field in ('filename','url','version','sha256','bytes')):
        raise ValueError('CPU worker proof recipe differs from the reviewed source lock')
    if recipe.get('windows_crt') != 'msvc-14.44.35211-x64':
        raise ValueError('Final CPU worker proof requires its fixed private CRT')
    return True


def copy_cached(item, cache, destination):
    source = cache/item['sha256']
    if source.is_symlink() or not source.is_file() or source.stat().st_size != item['bytes'] or sha256(source) != item['sha256']:
        raise ValueError('Real worker fixture must already exist in the verified Tiny/JFK cache')
    destination.parent.mkdir(parents=True, exist_ok=True)
    shutil.copyfile(source, destination)
    if destination.stat().st_size != item['bytes'] or sha256(destination) != item['sha256']:
        raise ValueError('Copied fixture differs from the verified cached bytes')


def repeat_audio(source, destination):
    with wave.open(str(source),'rb') as original:
        if (original.getnchannels(),original.getsampwidth(),original.getframerate(),original.getcomptype()) != (1,2,16000,'NONE'):
            raise ValueError('Cancellation fixture requires mono16kHz PCM16')
        frames = original.getnframes()
        if frames != 176000: raise ValueError('Cancellation fixture requires the reviewed eleven-second JFK clip')
        pcm = original.readframes(frames)
        if len(pcm) != frames*2: raise ValueError('Truncated source audio')
    if len(pcm)*REPEATS+44 > MAX_REPEAT_BYTES: raise ValueError('Cancellation fixture exceeds its fixed bound')
    with wave.open(str(destination),'wb') as output:
        output.setnchannels(1);output.setsampwidth(2);output.setframerate(16000)
        for _ in range(REPEATS): output.writeframesraw(pcm)
    return {'repeats':REPEATS,'bytes':destination.stat().st_size,'sha256':sha256(destination),
            'duration_ms':frames*REPEATS*1000//16000}


def prepare(cache, directory):
    directory.mkdir(parents=True,exist_ok=False)
    directory=directory.resolve(strict=True);cache=cache.resolve(strict=True)
    pins=json.loads((ROOT/'fixtures.json').read_text(encoding='utf-8'))
    model=directory/'tiny model';replacement=directory/'tiny replacement model 子 日本語 é';audio=directory/'jfk.wav';long_audio=directory/'jfk-repeat.wav'
    for item in pins['faster_whisper_tiny']['files']:
        copy_cached(item,cache,model.joinpath(*safe_name(item['path']).parts))
        copy_cached(item,cache,replacement.joinpath(*safe_name(item['path']).parts))
    copy_cached(pins['audio'],cache,audio)
    repeated=repeat_audio(audio,long_audio)
    evidence={'model_revision':pins['faster_whisper_tiny']['version'],'audio_sha256':pins['audio']['sha256'],
              'cancellation_audio':repeated,'replacement_model_same_pinned_bytes':True,'new_downloads':False}
    dump(directory/'fixture.json',evidence)
    return {'LUMA_ASR_TEST_MODEL':str(model),'LUMA_ASR_TEST_REPLACEMENT_MODEL':str(replacement),'LUMA_ASR_TEST_AUDIO':str(audio),
            'LUMA_ASR_TEST_LONG_AUDIO':str(long_audio),'LUMA_ASR_TEST_OUTPUT':str(directory/'results')},evidence


def owned_path(path, owner, *, file=False):
    """Validate, rather than silently normalize, a ready receipt's owned path."""
    path=Path(path);owner=Path(owner)
    if not path.is_absolute() or not path.is_relative_to(owner) or path.resolve(strict=True)!=path:
        raise ValueError('Replay path is not canonical and owned')
    for part in (path,*path.parents):
        info=part.lstat()
        if stat.S_ISLNK(info.st_mode) or getattr(info,'st_file_attributes',0)&0x400:
            raise ValueError('Replay path has a link or reparse ancestor')
        if part==owner:break
    if not (path.is_file() if file else path.is_dir()):raise ValueError('Replay path has wrong file type')
    return path


def bounded_json(path, owner, limit=2*1024*1024):
    path=owned_path(path,owner,file=True)
    if path.stat().st_size>limit:raise ValueError('Replay receipt exceeds metadata bound')
    return json.loads(path.read_text(encoding='utf-8'))


def validate_replay_ready(ready_path,cache,fixture_dir,reports,source_sha,runner_temp):
    if not re.fullmatch('[a-f0-9]{40}',source_sha):raise ValueError('Exact verifier source SHA required')
    owner=Path(runner_temp).resolve(strict=True)
    work=owned_path(owner/'ct2-replay-private',owner)
    cache=owned_path(cache,owner);reports=owned_path(reports,owner)
    if cache!=owner/'ct2-replay-downloads' or reports!=owner/'ct2-replay-reports':
        raise ValueError('Unexpected diagnostic replay cache or report root')
    fixture_dir=Path(fixture_dir)
    if fixture_dir!=owner/'ct2-replay-rust-fixture' or fixture_dir.exists() or fixture_dir.is_symlink():
        raise ValueError('Rust fixture must use its fresh fixed owned root')
    if Path(ready_path)!=reports/'replay-runtime.json':raise ValueError('Unexpected ready receipt path')
    ready=bounded_json(ready_path,owner)
    result=bounded_json(reports/'replay-result.json',owner)
    if (ready.get('schema')!=1 or ready.get('publication_authorized') is not False
            or ready.get('producer_source_sha')!=REPLAY_PRODUCER or ready.get('wheel_sha256')!=REPLAY_WHEEL_SHA
            or result.get('schema')!=1 or result.get('kind')!='ct2-lifecycle-replay'
            or result.get('producer_source_sha')!=REPLAY_PRODUCER or result.get('verifier_source_sha')!=source_sha
            or result.get('publication_authorized') is not False):
        raise ValueError('Diagnostic replay source or wheel provenance differs')
    drain=bounded_json(reports/'replay-child-drain.json',owner,4096)
    if (set(drain)!={'schema','owner','case','drained','pid'} or drain['schema']!=1
            or drain['owner']!='replay-driver' or drain['drained'] is not True
            or not isinstance(drain['case'],str) or not re.fullmatch(r'(default|one)-(eof|unload|switch)',drain['case'])
            or type(drain['pid']) is not int or drain['pid']<=0):
        raise ValueError('Prior diagnostic child has not been confirmed drained')
    runtime=owned_path(ready['root'],work)
    if runtime!=work/'Relocated private Python é 测试':raise ValueError('Unexpected replay runtime root')
    python=owned_path(ready['python'],runtime,file=True)
    if python!=runtime/'python.exe' or Path(ready['cache'])!=cache:raise ValueError('Replay interpreter/cache mismatch')
    original=owned_path(ready['original_worker'],work,file=True)
    if original!=work/'luma/src-tauri/src/asr/worker.py':raise ValueError('Unexpected original worker source')
    inputs=bounded_json(reports/'original-inputs.json',owner)
    original_pin=inputs['recipes']['luma/src-tauri/src/asr/worker.py']
    if original.stat().st_size!=original_pin['bytes'] or sha256(original)!=original_pin['sha256']:
        raise ValueError('Original worker differs from the verified source export')
    wheel=owned_path(work/'artifact'/CPU_FILENAME,work,file=True)
    if wheel.stat().st_size!=REPLAY_WHEEL_BYTES or sha256(wheel)!=REPLAY_WHEEL_SHA:
        raise ValueError('Exact diagnostic wheel bytes differ')
    lock_path=work/'runtime-lock.json';lock=bounded_json(lock_path,owner)
    assembly=bounded_json(runtime/'ASSEMBLY.json',owner)
    tuple_fields=('name','version','filename','url','bytes','sha256')
    selected=[{k:w[k] for k in tuple_fields} for w in lock['wheels']]
    ct2=[w for w in selected if w['name']=='ctranslate2']
    if (ct2!=[{'name':'ctranslate2','version':'4.8.2','filename':CPU_FILENAME,
               'url':'local-diagnostic-only:'+CPU_FILENAME,'bytes':REPLAY_WHEEL_BYTES,'sha256':REPLAY_WHEEL_SHA}]
            or assembly.get('schema')!=1 or assembly.get('method')!='private-offline-pip'
            or assembly.get('python')!='3.12.15' or assembly.get('pip')!='26.2.1'
            or assembly.get('wheel_lock_sha256')!=sha256(lock_path) or assembly.get('upstream_wheels')!=selected
            or any(assembly.get(k) is not True for k in ('no_index','no_dependencies','binary_only','require_hashes',
                    'pip_config_disabled','network_guard','subprocess_guard','generated_launchers_removed','installer_removed'))
            or assembly.get('source_builds') is not False
            or bounded_json(reports/'whole-runtime-native.json',owner).get('passed') is not True):
        raise ValueError('Replay runtime setup/closure receipt missing or inconsistent')
    pins=json.loads((ROOT/'fixtures.json').read_text(encoding='utf-8'))
    expected=[{k:item[k] for k in ('path','bytes','sha256')} for item in pins['faster_whisper_tiny']['files']]
    if ready.get('fixture_files')!=expected:raise ValueError('Replay Tiny fixture revision differs')
    for key,name in (('model','model'),('switch_model','model-switch')):
        model=owned_path(ready[key],work)
        if model!=work/name:raise ValueError('Unexpected replay model directory')
        for item in expected:
            member=owned_path(model.joinpath(*safe_name(item['path']).parts),model,file=True)
            if member.stat().st_size!=item['bytes'] or sha256(member)!=item['sha256']:
                raise ValueError('Replay model copy differs from pinned bytes')
    audio=owned_path(ready['audio'],work,file=True)
    if (audio!=work/'jfk.wav' or audio.stat().st_size!=pins['audio']['bytes'] or sha256(audio)!=pins['audio']['sha256']):
        raise ValueError('Replay audio differs from pinned JFK bytes')
    return {'python':python,'original_worker_sha256':sha256(original),'owner':owner,'work':work,
            'cache':cache,'fixture_dir':fixture_dir,'reports':reports}


def run_replay_lifecycle(ready_path,cache,fixture_dir,reports,source_sha,runner_temp):
    """Diagnostic only: verified replay bytes, actual new-source Rust manager."""
    from windows_test_job import JobRunError, native_tree_smoke, run_owned_tree
    checked=validate_replay_ready(ready_path,cache,fixture_dir,reports,source_sha,runner_temp)
    if sys.platform!='win32':raise RuntimeError('Exact CPU replay Rust proof requires native Windows')
    report={'schema':1,'kind':'ct2-rust-manager-lifecycle','publication_authorized':False,'passed':False,
            'producer_source_sha':REPLAY_PRODUCER,'verifier_source_sha':source_sha,'wheel_sha256':REPLAY_WHEEL_SHA,
            'original_worker_sha256':checked['original_worker_sha256'],
            'embedded_worker_sha256':sha256(ROOT.parents[1]/'src-tauri/src/asr/worker.py'),
            'tree_drained':True,'cargo_started':False,'new_fixture_downloads':False,
            'global_managed_path_selection_tested':False}
    report_path=checked['reports']/'rust-lifecycle.json'
    if report_path.exists():raise ValueError('Rust replay report must be fresh')
    fixture=checked['fixture_dir'];cargo_result=None
    try:
        env,fixture_report=prepare(checked['cache'],fixture);report['fixture']=fixture_report
        report['tree_drained']=False;dump(report_path,report)
        report['tree_smoke']=native_tree_smoke(fixture)
        report['tree_drained']=True;dump(report_path,report)
        repo=ROOT.parents[1]
        git=shutil.which('git');cargo=shutil.which('cargo');rustc=shutil.which('rustc')
        if not git or not cargo or not rustc:raise ValueError('Native Rust/Git build tools are missing')
        report['tree_drained']=False;dump(report_path,report)
        identity=run_owned_tree([str(Path(git).resolve()),'-C',str(repo),'rev-parse','HEAD'],cwd=fixture,timeout=15,output_limit=4096)
        report['tree_drained']=identity['tree_drained'];dump(report_path,report)
        if identity['exit_code']!=0 or identity['stdout'].strip()!=source_sha:
            raise ValueError('Rust verifier checkout differs from the exact requested source')
        report['toolchain']={}
        for name,executable in (('cargo',cargo),('rustc',rustc)):
            report['tree_drained']=False;dump(report_path,report)
            version=run_owned_tree([str(Path(executable).resolve()),'--version'],cwd=fixture,timeout=15,output_limit=4096)
            report['tree_drained']=version['tree_drained'];dump(report_path,report)
            text=version['stdout'].strip();match=re.fullmatch(name+r' (\d+)\.(\d+)\.(\d+)(?:[^\r\n]*)',text)
            if version['exit_code']!=0 or not match or tuple(map(int,match.groups()))<(1,80,0):
                raise ValueError('Native Rust toolchain is absent or below the declared minimum')
            report['toolchain'][name]=text
        report['toolchain']['purpose']='diagnostic verifier build; no wheel reproducibility claim'
        environment={key:value for key,value in os.environ.items() if not key.upper().startswith('LUMA_ASR_TEST_')}
        environment.update(env)
        environment.update(LUMA_ASR_TEST_PYTHON=str(checked['python']),LUMA_ASR_TEST_VERIFIER_SOURCE_SHA=source_sha,
                           LUMA_ASR_TEST_ENGINE='whisper-accelerated',LUMA_ASR_TEST_DEVICE='cpu',
                           HF_HUB_OFFLINE='1',TRANSFORMERS_OFFLINE='1',HF_HUB_DISABLE_TELEMETRY='1',
                           HF_HUB_DISABLE_IMPLICIT_TOKEN='1',DO_NOT_TRACK='1')
        command=[str(Path(cargo).resolve()),'test','--manifest-path',str(repo/'src-tauri/Cargo.toml'),'--locked',
                 'real_optional_worker_transcribes_exports_reuses_and_cancels','--','--ignored','--nocapture']
        report.update(tree_drained=False,cargo_started=True);dump(report_path,report)
        try: cargo_result=run_owned_tree(command,cwd=repo,env=environment,timeout=1800,output_limit=8*1024*1024)
        except JobRunError as error:
            cargo_result=error.result;raise
        finally:
            if cargo_result is not None:
                report['tree_drained']=cargo_result['tree_drained']
                report['cargo']={k:v for k,v in cargo_result.items() if k not in ('stdout','stderr')}
                log=(cargo_result['stdout']+'\n'+cargo_result['stderr']).encode('utf-8')[:8*1024*1024]
                (checked['reports']/'rust-cargo.log').write_bytes(log)
        if cargo_result['exit_code']!=0:raise RuntimeError('Actual Rust manager lifecycle test failed')
        lifecycle=bounded_json(fixture/'results/lifecycle.json',fixture,64*1024)
        expected=['model-replacement','release-idle','legacy-release','active-cancellation','shutdown']
        if (lifecycle.get('passed') is not True or lifecycle.get('verifier_source_sha')!=source_sha
                or lifecycle.get('embedded_worker_sha256')!=report['embedded_worker_sha256']
                or lifecycle.get('injected_lease_retention_tested') is not True
                or lifecycle.get('cold_warm_srt_export_tested') is not True
                or lifecycle.get('active_cancellation_recovery_tested') is not True
                or lifecycle.get('global_managed_path_selection_tested') is not False
                or [t.get('operation') for t in lifecycle.get('transitions',[])]!=expected
                or any(t.get('process_exited_when_lease_available') is not True or t.get('script_removed') is not True
                       for t in lifecycle['transitions'])):
            raise ValueError('Rust lifecycle evidence is incomplete or has different source identity')
        report['lifecycle']=lifecycle;report['passed']=True
    except BaseException as error:
        report['error']=type(error).__name__+': '+str(error)
        if isinstance(error,JobRunError):report['tree_drained']=error.result['tree_drained']
        raise
    finally:
        partial=fixture/'results/lifecycle.json'
        if partial.is_file() and not partial.is_symlink() and partial.stat().st_size<=64*1024:
            try:report['lifecycle']=bounded_json(partial,fixture,64*1024)
            except (ValueError,OSError) as error:report['partial_report_error']=str(error)
        dump(report_path,report)
        print(json.dumps(report,ensure_ascii=True,sort_keys=True),flush=True)
        if fixture.exists() and report['tree_drained']:shutil.rmtree(fixture)
    return report


def main():
    import argparse
    parser=argparse.ArgumentParser(description='Diagnostic-only exact-wheel Rust lifecycle replay')
    for flag in ('replay-ready','cache','fixture-dir','reports'):
        parser.add_argument('--'+flag,type=Path,required=True)
    parser.add_argument('--verifier-source-sha',required=True)
    args=parser.parse_args()
    run_replay_lifecycle(args.replay_ready,args.cache,args.fixture_dir,args.reports,args.verifier_source_sha,os.environ['RUNNER_TEMP'])


if __name__=='__main__':main()
