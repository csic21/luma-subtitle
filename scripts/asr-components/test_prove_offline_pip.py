import ast
import copy
import hashlib
import json
from pathlib import Path
import unittest
from unittest.mock import patch
from fixture_paths import temporary_root
import prove_offline_pip as proof
import windows_test_job as job


class ProofTextEncodingTests(unittest.TestCase):
    def test_generated_utf8_reports_round_trip_under_cp1252_host_default(self):
        # U+5B50 contains UTF-8 byte0x90, which cp1252 cannot decode. Exercise the
        # actual orchestration reader, with only builds/children replaced by
        # tiny local fixtures. No engine, model or subprocess is involved.
        value={'label':'子 日本語 é 测试','path':'runtime 子/model 日本語'}
        with self.assertRaises(UnicodeDecodeError):json.dumps(value,ensure_ascii=False).encode('utf-8').decode('cp1252')
        with temporary_root(prefix='proof 子 ') as root:
            output=root/'output';output.mkdir();cache=root/'cache';cache.mkdir()
            candidates=root/'recipes.json';proof.dump(candidates,{'runtimes':[{'id':'fixture','label':value['label'],'recipe':{}}]})
            worker=root/'worker.py';worker.write_text('# source 子\n',encoding='utf-8')
            def build(pack,directory,cache,source,**kwargs):
                (directory/'staging'/pack).mkdir(parents=True)
                proof.dump(directory/'staging'/pack/'ASSEMBLY.json',value)
                proof.dump(directory/(pack+'.manifest.json'),value)
                return {'archive':{'sha256':'a'*64,'bytes':1}}
            def owned(command,**kwargs):proof.dump(output/'first/fixture.smoke.json',value)
            original=Path.read_text
            def cp1252_default(path,*args,**kwargs):
                if not args and 'encoding' not in kwargs:kwargs['encoding']='cp1252'
                return original(path,*args,**kwargs)
            argv=['proof','--pack','fixture','--source-sha','b'*40,'--output',str(output),'--cache',str(cache),
                  '--worker',str(worker),'--recipe-candidates',str(candidates)]
            with patch.object(proof.sys,'argv',argv),patch.object(proof,'build',side_effect=build), \
                 patch.object(proof,'owned',side_effect=owned),patch.object(proof,'prove_native_installer',return_value={'tested':False}), \
                 patch.object(Path,'read_text',cp1252_default),patch('builtins.print'):
                proof.main()
            report=json.loads((output/'fixture.offline-pip-proof.json').read_text(encoding='utf-8'))
            self.assertEqual(report['assembly'],value);self.assertEqual(report['runtime_smoke'],value)
            self.assertFalse((output/'first').exists());self.assertFalse((output/'second').exists())

    def test_adjacent_proof_text_io_always_names_its_encoding(self):
        for name in ('prove_offline_pip.py','build.py','smoke.py','windows_crt_proof.py','real_worker_fixture.py','resolve_locks.py'):
            source=Path(__file__).with_name(name).read_text(encoding='utf-8')
            for node in ast.walk(ast.parse(source)):
                if (isinstance(node,ast.Call) and isinstance(node.func,ast.Attribute)
                        and node.func.attr in ('read_text','write_text')):
                    encodings=[keyword.value for keyword in node.keywords if keyword.arg=='encoding']
                    self.assertEqual(len(encodings),1,f'{name}:{node.lineno} must not use the host code page')
                    self.assertIn(ast.literal_eval(encodings[0]),('utf-8','utf-8-sig'))

    def test_native_job_failure_keeps_partial_exact_source_metadata(self):
        with temporary_root() as root:
            output=root/'output';output.mkdir();cache=root/'cache';cache.mkdir()
            candidates=root/'recipes.json';proof.dump(candidates,{'runtimes':[{'id':'fixture','recipe':{}}]})
            worker=root/'worker.py';worker.write_text('# worker\n',encoding='utf-8')
            def build(pack,directory,*args,**kwargs):
                (directory/'staging'/pack).mkdir(parents=True)
                proof.dump(directory/'staging'/pack/'ASSEMBLY.json',{})
                proof.dump(directory/(pack+'.manifest.json'),{})
                return {'archive':{'sha256':'a'*64,'bytes':1}}
            def owned(*args,**kwargs):proof.dump(output/'first/fixture.smoke.json',{})
            def native(*args,progress=None,source_sha=None):
                self.assertEqual(source_sha,'b'*40)
                progress({'tested':True,'passed':False,'windows_owned_job':{'tree_drained':False,'job_start_pending':True}})
                checkpoint=json.loads((output/'fixture.offline-pip-proof.json').read_text(encoding='utf-8'))
                self.assertEqual(checkpoint['stage'],'native-installer-running')
                progress({'tested':True,'passed':False,'windows_owned_job':{'tree_drained':True,'exit_code':0,
                          'before_forced_termination':{'processes':[{'pid':123,'parent_pid':100}]}}})
                raise job.JobRunError('Owned Windows process tree did not drain',{'tree_drained':True})
            argv=['proof','--pack','fixture','--source-sha','b'*40,'--output',str(output),'--cache',str(cache),
                  '--worker',str(worker),'--recipe-candidates',str(candidates)]
            with patch.object(proof.sys,'argv',argv),patch.object(proof,'build',side_effect=build), \
                 patch.object(proof,'owned',side_effect=owned),patch.object(proof,'prove_native_installer',side_effect=native), \
                 patch('builtins.print'):
                with self.assertRaises(job.JobRunError):proof.main()
            report=json.loads((output/'fixture.offline-pip-proof.json').read_text(encoding='utf-8'))
            self.assertFalse(report['passed']);self.assertFalse(report['publication_authorized'])
            self.assertEqual(report['source_sha'],'b'*40);self.assertEqual(report['stage'],'native-installer')
            self.assertEqual(report['native_installer_test']['windows_owned_job']['before_forced_termination']['processes'][0]['pid'],123)


class WindowsInstallerJobTests(unittest.TestCase):
    def test_native_worker_lease_coverage_requires_complete_matching_lifecycle(self):
        fixture={'model_revision':'pinned','audio_sha256':'c'*64}
        lifecycle={'schema':1,'passed':True,'verifier_source_sha':'b'*40,'embedded_worker_sha256':'d'*64,
                   **fixture,'managed_receipt_policy_selection_tested':True,'cpu_thread_policy':'luma-cpu-seq-1:cpu_threads=1',
                   'global_managed_path_selection_tested':False,'injected_lease_retention_tested':True,
                   'cold_warm_srt_export_tested':True,'active_cancellation_recovery_tested':True,
                   'transitions':[{'operation':operation,'process_exited_when_lease_available':True,'script_removed':True}
                                  for operation in ('model-replacement','release-idle','legacy-release','active-cancellation','shutdown')]}
        with patch.object(proof,'sha256',return_value='d'*64):
            proof.validate_native_worker_lifecycle(lifecycle,fixture,'b'*40)
            for field,value in [('passed',False),('verifier_source_sha','e'*40),('embedded_worker_sha256','e'*64),
                                ('managed_receipt_policy_selection_tested',False),('cpu_thread_policy',None),
                                ('global_managed_path_selection_tested',True),('transitions',[])]:
                bad=copy.deepcopy(lifecycle);bad[field]=value
                with self.subTest(field=field),self.assertRaises(ValueError):proof.validate_native_worker_lifecycle(bad,fixture,'b'*40)
            bad=copy.deepcopy(lifecycle);bad['transitions'][0]['process_exited_when_lease_available']=False
            with self.assertRaises(ValueError):proof.validate_native_worker_lifecycle(bad,fixture,'b'*40)

    def exercise(self,root,job_result,*,raises=False,with_fixture=False,prepare_failure=False):
        output=root/'output';output.mkdir();cache=root/'cache';cache.mkdir()
        pin={'bytes':6,'sha256':hashlib.sha256(b'python').hexdigest()}
        (cache/pin['sha256']).write_bytes(b'python')
        candidates=root/'candidates.json';proof.dump(candidates,{'runtimes':[{'id':'fixture','recipe':{'python':pin,'wheels':[]}}]})
        cargo=root/'cargo.exe';cargo.touch();updates=[]
        def prepare(cache,directory):
            directory.mkdir();(directory/'partial-model').write_bytes(b'private fixture')
            if prepare_failure:raise RuntimeError('expected fixture preparation failure')
            return {'LUMA_ASR_TEST_OUTPUT':str(directory/'results')},{'fixture':True}
        def run(executable,**kwargs):
            self.assertEqual(executable,str(cargo.resolve()))
            self.assertEqual(kwargs['test_name'],'asr_components::tests::native_direct_recipe_installs_repairs_and_removes')
            self.assertEqual(kwargs['timeout'],1800);self.assertEqual(kwargs['output_limit'],8*1024*1024)
            self.assertEqual(updates[0]['windows_owned_job'],{'tree_drained':False,'job_start_pending':True})
            self.assertTrue(Path(kwargs['env']['LUMA_ASR_RECIPE_INPUTS']).is_dir())
            if raises:raise job.JobRunError('owned descendant survived',job_result)
            return job_result
        with patch.object(proof.sys,'platform','win32'),patch.object(proof.shutil,'which',return_value=str(cargo)), \
             patch.object(job,'run_owned_cargo_test',side_effect=run),patch('real_worker_fixture.final_cpu_recipe',return_value=with_fixture), \
             patch('real_worker_fixture.prepare',side_effect=prepare), \
             patch.object(proof,'owned',side_effect=AssertionError('must use Job')),patch('builtins.print'):
            try:
                result=proof.prove_native_installer('fixture',output,cache,candidates,progress=lambda value:updates.append(copy.deepcopy(value)))
                error=None
            except (job.JobRunError,RuntimeError) as caught:result=None;error=caught
        return output,result,error,updates

    def test_windows_cargo_success_has_job_evidence_and_cleans_inputs(self):
        with temporary_root() as root:
            output,result,error,updates=self.exercise(root,{'exit_code':0,'tree_drained':True,'natural_drain_completed':True,
                                                         'stdout':'test passed','stderr':''})
            self.assertIsNone(error);self.assertTrue(result['passed'])
            self.assertTrue(result['windows_owned_job']['natural_drain_completed'])
            self.assertNotIn('stdout',result['windows_owned_job']);self.assertEqual(list(output.iterdir()),[])
            self.assertTrue(updates[-1]['windows_owned_job']['tree_drained'])

    def test_windows_job_failure_is_strict_and_cleanup_requires_confirmed_drain(self):
        for drained in (False,True):
            with self.subTest(drained=drained),temporary_root() as root:
                output,result,error,updates=self.exercise(root,{'exit_code':0,'tree_drained':drained,'stdout':'','stderr':''},raises=True)
                self.assertIsInstance(error,job.JobRunError);self.assertIsNone(result)
                self.assertFalse(updates[-1]['passed'])
                self.assertEqual((output/'recipe inputs é 测试').exists(),not drained)
                self.assertEqual((output/'recipe-runtime.json').exists(),not drained)

    def test_windows_job_failure_gates_model_fixture_cleanup_on_confirmed_drain(self):
        for drained in (None,False,True):
            with self.subTest(drained=drained),temporary_root() as root:
                output,result,error,updates=self.exercise(root,{'exit_code':0,'tree_drained':drained,'stdout':'','stderr':''},
                                                         raises=True,with_fixture=True)
                self.assertIsInstance(error,job.JobRunError);self.assertIsNone(result)
                self.assertEqual((output/'real worker fixtures é 测试'/'partial-model').exists(),not drained)
                self.assertEqual((output/'recipe inputs é 测试').exists(),not drained)
                self.assertFalse(updates[-1]['passed'])

    def test_windows_fixture_preparation_failure_cleans_without_job_checkpoint(self):
        with temporary_root() as root:
            output,result,error,updates=self.exercise(root,{},with_fixture=True,prepare_failure=True)
            self.assertIsInstance(error,RuntimeError);self.assertIsNone(result);self.assertEqual(updates,[])
            self.assertEqual(list(output.iterdir()),[])

    def test_windows_nonzero_cargo_and_false_drain_cannot_pass(self):
        for code,drained in ((1,True),(0,False)):
            with self.subTest(code=code,drained=drained),temporary_root() as root:
                output,result,error,updates=self.exercise(root,{'exit_code':code,'tree_drained':drained,'stdout':'','stderr':''})
                self.assertIsInstance(error,RuntimeError);self.assertIsNone(result)
                self.assertEqual((output/'recipe inputs é 测试').exists(),not drained)


if __name__=='__main__':unittest.main()
