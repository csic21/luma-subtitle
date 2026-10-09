import copy
import hashlib
import json
from pathlib import Path
import unittest
from unittest.mock import patch
import wave
import windows_test_job as job
from fixture_paths import temporary_root
import real_worker_fixture as f
import prove_offline_pip as proof


def pin(data): return {'bytes':len(data),'sha256':hashlib.sha256(data).hexdigest()}


class RealWorkerFixtureTests(unittest.TestCase):
    def test_original_reference_and_other_engines_cannot_enable_hook(self):
        self.assertFalse(f.final_cpu_recipe({'id':'faster-whisper-cpu-windows-x64'}))
        self.assertFalse(f.final_cpu_recipe({'id':'qwen3-asr-cpu-windows-x64','recipe':{}}))
        wheel={'name':'ctranslate2','version':'4.8.2',
               'filename':'ctranslate2-4.8.2-cp312-cp312-win_amd64.whl',
               'url':'https://files.pythonhosted.org/upstream-reference.whl'}
        self.assertFalse(f.final_cpu_recipe({'id':'faster-whisper-cpu-windows-x64','recipe':{'wheels':[wheel]}}))

    def test_final_gate_requires_exact_origin_reviewed_hash_and_crt(self):
        wheel={'name':'ctranslate2','version':'4.8.2','filename':f.CPU_FILENAME,'url':f.CPU_URL,'bytes':123,'sha256':'a'*64}
        candidate={'id':'faster-whisper-cpu-windows-x64','unavailable_reason':'review pending',
                   'recipe':{'windows_crt':'msvc-14.44.35211-x64','wheels':[wheel]}}
        with temporary_root() as root, patch.object(f,'ROOT',root):
            (root/'locks').mkdir();(root/'locks/faster-whisper-cpu-windows-x64.json').write_text(json.dumps({'wheels':[wheel]}))
            self.assertTrue(f.final_cpu_recipe(candidate))
            for field,value in [('sha256','b'*64),('bytes',124)]:
                bad=copy.deepcopy(candidate);bad['recipe']['wheels'][0][field]=value
                with self.assertRaises(ValueError):f.final_cpu_recipe(bad)
            bad=copy.deepcopy(candidate);bad['recipe']['windows_crt']='other'
            with self.assertRaises(ValueError):f.final_cpu_recipe(bad)
            for field,value in [('url','https://unreviewed.invalid/wheel.whl'),('filename','ctranslate2-upstream.whl')]:
                bad=copy.deepcopy(candidate);bad['recipe']['wheels'][0][field]=value
                self.assertFalse(f.final_cpu_recipe(bad))

    def test_cached_copy_never_downloads_or_accepts_modified_bytes(self):
        with temporary_root() as root:
            item=pin(b'tiny');source=root/item['sha256'];source.write_bytes(b'tiny')
            f.copy_cached(item,root,root/'copy');self.assertEqual((root/'copy').read_bytes(),b'tiny')
            source.write_bytes(b'evil')
            with self.assertRaises(ValueError):f.copy_cached(item,root,root/'other')

    def test_prepared_cancellation_audio_is_bounded_streamed_source_repetition(self):
        with temporary_root() as root:
            cache=root/'cache';cache.mkdir();metadata=root/'metadata';metadata.mkdir()
            audio=root/'audio.wav';pcm=b'\x01\x00'*176000
            with wave.open(str(audio),'wb') as output:
                output.setparams((1,2,16000,0,'NONE','not compressed'));output.writeframes(pcm)
            a=pin(audio.read_bytes());a['path']='jfk.wav'
            model=pin(b'fixture');model['path']='model.bin'
            (cache/a['sha256']).write_bytes(audio.read_bytes());(cache/model['sha256']).write_bytes(b'fixture')
            (metadata/'fixtures.json').write_text(json.dumps({'audio':a,'faster_whisper_tiny':{'version':'reviewed','files':[model]}}))
            with patch.object(f,'ROOT',metadata):env,report=f.prepare(cache,root/'fixtures é 测试')
            repeated=Path(env['LUMA_ASR_TEST_LONG_AUDIO'])
            self.assertEqual(report['cancellation_audio']['bytes'],35_200_044)
            self.assertEqual(report['cancellation_audio']['duration_ms'],1_100_000)
            self.assertFalse(report['new_downloads'])
            self.assertTrue(report['replacement_model_same_pinned_bytes'])
            original=Path(env['LUMA_ASR_TEST_MODEL']);replacement=Path(env['LUMA_ASR_TEST_REPLACEMENT_MODEL'])
            self.assertNotEqual(original,replacement)
            self.assertEqual((original/'model.bin').read_bytes(),(replacement/'model.bin').read_bytes())
            self.assertEqual(f.sha256(replacement/'model.bin'),model['sha256'])
            with wave.open(str(repeated),'rb') as source:
                for _ in range(100):self.assertEqual(source.readframes(176000),pcm)
                self.assertEqual(source.readframes(1),b'')
            self.assertEqual(f.sha256(repeated),report['cancellation_audio']['sha256'])

    def test_non_windows_native_failure_cleans_copied_inputs_and_partial_fixtures(self):
        # This covers the legacy non-Windows owned() path. Windows Job failure
        # and confirmed/unknown drain cleanup are mocked in test_prove_offline_pip.
        for where in ('prepare','cargo'):
            with self.subTest(where=where), temporary_root() as root:
                cache=root/'cache';cache.mkdir();output=root/'output';output.mkdir()
                python=pin(b'python');(cache/python['sha256']).write_bytes(b'python')
                candidate={'id':'fixture','recipe':{'python':python,'wheels':[]}}
                candidates=root/'candidates.json';candidates.write_text(json.dumps({'runtimes':[candidate]}))
                def prepare(cache,directory):
                    directory.mkdir();(directory/'partial-model').write_bytes(b'fixture')
                    if where=='prepare':raise RuntimeError('expected preparation failure')
                    return {},{'fixture':True}
                with patch.object(proof.sys,'platform','darwin'),patch.object(f,'final_cpu_recipe',return_value=True), patch.object(f,'prepare',side_effect=prepare), patch.object(proof,'owned',side_effect=RuntimeError('expected cargo failure')):
                    with self.assertRaisesRegex(RuntimeError,'expected'):
                        proof.prove_native_installer('fixture',output,cache,candidates)
                self.assertEqual(list(output.iterdir()),[])
                self.assertTrue((cache/python['sha256']).exists())

    def test_repeat_rejects_unreviewed_format_or_duration(self):
        with temporary_root() as root:
            audio=root/'bad.wav'
            with wave.open(str(audio),'wb') as output:
                output.setparams((1,2,16000,0,'NONE','not compressed'));output.writeframes(b'\0'*32)
            with self.assertRaises(ValueError):f.repeat_audio(audio,root/'out.wav')


class ReplayLifecycleTests(unittest.TestCase):
    def fixture(self,root):
        work=root/'ct2-replay-private';work.mkdir()
        runtime=work/'Relocated private Python é 测试';runtime.mkdir();(runtime/'python.exe').write_bytes(b'python')
        reports=root/'ct2-replay-reports';reports.mkdir();cache=root/'ct2-replay-downloads';cache.mkdir()
        source=work/'luma/src-tauri/src/asr/worker.py';source.parent.mkdir(parents=True);source.write_bytes(b'original')
        pins={'audio':dict(pin(b'audio'),path='jfk.wav'),'faster_whisper_tiny':{'version':'fixture','files':[dict(pin(b'model'),path='model.bin')]}}
        for name in ('model','model-switch'):
            (work/name).mkdir();(work/name/'model.bin').write_bytes(b'model')
        (work/'jfk.wav').write_bytes(b'audio')
        wheel=work/'artifact'/f.CPU_FILENAME;wheel.parent.mkdir();wheel.write_bytes(b'wheel')
        item={'name':'ctranslate2','version':'4.8.2','filename':f.CPU_FILENAME,'url':'local-diagnostic-only:'+f.CPU_FILENAME,**pin(b'wheel')}
        f.dump(work/'runtime-lock.json',{'wheels':[item]})
        assembly={'schema':1,'method':'private-offline-pip','python':'3.12.15','pip':'26.2.1',
                  'wheel_lock_sha256':f.sha256(work/'runtime-lock.json'),'upstream_wheels':[item],'source_builds':False}
        assembly.update({k:True for k in ('no_index','no_dependencies','binary_only','require_hashes','pip_config_disabled',
                         'network_guard','subprocess_guard','generated_launchers_removed','installer_removed')})
        f.dump(runtime/'ASSEMBLY.json',assembly);f.dump(reports/'whole-runtime-native.json',{'passed':True})
        f.dump(reports/'original-inputs.json',{'recipes':{'luma/src-tauri/src/asr/worker.py':pin(b'original')}})
        result={'schema':1,'kind':'ct2-lifecycle-replay','producer_source_sha':f.REPLAY_PRODUCER,
                'verifier_source_sha':'a'*40,'publication_authorized':False,'passed':False}
        f.dump(reports/'replay-result.json',result)
        f.dump(reports/'replay-child-drain.json',{'schema':1,'owner':'replay-driver','case':'one-switch','drained':True,'pid':123})
        ready={'schema':1,'publication_authorized':False,'producer_source_sha':f.REPLAY_PRODUCER,'wheel_sha256':item['sha256'],
               'root':str(runtime),'python':str(runtime/'python.exe'),'cache':str(cache),'original_worker':str(source),
               'model':str(work/'model'),'switch_model':str(work/'model-switch'),'audio':str(work/'jfk.wav'),
               'fixture_files':pins['faster_whisper_tiny']['files']}
        f.dump(reports/'replay-runtime.json',ready)
        metadata=root/'metadata';metadata.mkdir();f.dump(metadata/'fixtures.json',pins)
        return work,reports,cache,metadata,ready

    def test_ready_validates_exact_sources_wheel_receipts_and_copy_hashes(self):
        with temporary_root() as root:
            work,reports,cache,metadata,ready=self.fixture(root)
            args=(reports/'replay-runtime.json',cache,root/'ct2-replay-rust-fixture',reports,'a'*40,root)
            with patch.object(f,'ROOT',metadata),patch.object(f,'REPLAY_WHEEL_SHA',pin(b'wheel')['sha256']),patch.object(f,'REPLAY_WHEEL_BYTES',5):
                self.assertEqual(f.validate_replay_ready(*args)['python'],Path(ready['python']))
                for key,value in [('producer_source_sha','b'*40),('wheel_sha256','b'*64),('python',str(root/'external.exe'))]:
                    bad=dict(ready,**{key:value});f.dump(args[0],bad)
                    with self.assertRaises((ValueError,FileNotFoundError)):f.validate_replay_ready(*args)
                f.dump(args[0],ready)
                result=json.loads((reports/'replay-result.json').read_text());result['verifier_source_sha']='c'*40
                f.dump(reports/'replay-result.json',result)
                with self.assertRaisesRegex(ValueError,'provenance'):f.validate_replay_ready(*args)
                result['verifier_source_sha']='a'*40;f.dump(reports/'replay-result.json',result)
                drain=json.loads((reports/'replay-child-drain.json').read_text());drain['drained']=False
                f.dump(reports/'replay-child-drain.json',drain)
                with self.assertRaisesRegex(ValueError,'confirmed drained'):f.validate_replay_ready(*args)
                drain['drained']=True;f.dump(reports/'replay-child-drain.json',drain)
                (work/'model-switch/model.bin').write_bytes(b'wrong')
                with self.assertRaisesRegex(ValueError,'model copy'):f.validate_replay_ready(*args)

    def test_path_validation_rejects_escape_and_links(self):
        with temporary_root() as root:
            owner=root/'owner';owner.mkdir();file=owner/'file';file.write_bytes(b'yes')
            with self.assertRaises(ValueError):f.owned_path(root,owner)
            try:(owner/'alias').symlink_to(file)
            except OSError:self.skipTest('symlinks unavailable for this fixture')
            with self.assertRaises(ValueError):f.owned_path(owner/'alias',owner,file=True)

    def test_replay_failure_report_precedes_launch_and_cleanup_waits_for_tree(self):
        for drained in (True,False):
            with self.subTest(drained=drained),temporary_root() as root:
                fixture=root/'ct2-replay-rust-fixture';reports=root/'reports';reports.mkdir()
                checked={'python':root/'python.exe','original_worker_sha256':'b'*64,'cache':root,
                         'fixture_dir':fixture,'reports':reports}
                def prepare(cache,directory):directory.mkdir();return {},{'fake':True}
                def run(command,**kwargs):
                    if 'rev-parse' in command:return {'exit_code':0,'stdout':'a'*40,'tree_drained':True}
                    if '--version' in command:return {'exit_code':0,'stdout':Path(command[0]).stem+' 1.90.0 (fixture)','tree_drained':True}
                    before=json.loads((reports/'rust-lifecycle.json').read_text())
                    self.assertFalse(before['tree_drained']);self.assertTrue(before['cargo_started'])
                    self.assertEqual(kwargs['env']['LUMA_ASR_TEST_ENGINE'],'whisper-accelerated')
                    self.assertEqual(kwargs['env']['LUMA_ASR_TEST_DEVICE'],'cpu')
                    raise job.JobRunError('expected bounded timeout',{'tree_drained':drained,'reason':'timeout','stdout':'partial','stderr':''})
                with patch.object(f,'validate_replay_ready',return_value=checked),patch.object(f.sys,'platform','win32'), \
                     patch.object(f,'prepare',side_effect=prepare),patch.object(f.shutil,'which',side_effect=lambda name:str(root/(name+'.exe'))), \
                     patch.object(job,'native_tree_smoke',return_value={'tree_drained':True}), \
                     patch.object(job,'run_owned_tree',side_effect=run),patch('builtins.print'):
                    with self.assertRaises(job.JobRunError):f.run_replay_lifecycle(None,None,None,None,'a'*40,root)
                report=json.loads((reports/'rust-lifecycle.json').read_text())
                self.assertFalse(report['passed']);self.assertEqual(report['tree_drained'],drained)
                self.assertEqual(fixture.exists(),not drained)
                self.assertEqual((reports/'rust-cargo.log').read_text(),'partial\n')

    def test_early_prepare_failure_records_and_cleans_partial_fixture(self):
        with temporary_root() as root:
            fixture=root/'fixture';reports=root/'reports';reports.mkdir()
            checked={'python':root/'python.exe','original_worker_sha256':'b'*64,'cache':root,'fixture_dir':fixture,'reports':reports}
            def prepare(cache,directory):directory.mkdir();raise ValueError('expected bad cached fixture')
            with patch.object(f,'validate_replay_ready',return_value=checked),patch.object(f.sys,'platform','win32'), \
                 patch.object(f,'prepare',side_effect=prepare),patch('builtins.print'):
                with self.assertRaises(ValueError):f.run_replay_lifecycle(None,None,None,None,'a'*40,root)
            report=json.loads((reports/'rust-lifecycle.json').read_text())
            self.assertFalse(report['cargo_started']);self.assertFalse(report['passed']);self.assertFalse(fixture.exists())


if __name__=='__main__':unittest.main()
