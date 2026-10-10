import copy
import importlib.util
from pathlib import Path
import sys
import unittest
import json
from unittest.mock import patch, Mock

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent))
from fixture_paths import temporary_root
sys.path.insert(0, str(HERE))
spec = importlib.util.spec_from_file_location('qwen_run_proof', HERE / 'run_proof.py')
m = importlib.util.module_from_spec(spec); spec.loader.exec_module(m)


def fixture(windows=False):
    pack = 'qwen3-asr-cpu-' + ('windows-x64' if windows else 'macos-arm64')
    return pack, {'schema': 1, 'source_sha': 'a'*40, 'pack_id': pack,
                  'method': 'private-offline-pip', 'reproducible': True,
                  'native_installer_test': {'passed': True},
                  'runtime_smoke': {'relocated': True, 'imports': {'tested_device': 'cpu', 'isolated': True, 'user_site': False},
                                    'system_python_used': False, 'system_packages_used': False,
                                    'worker_test': {'passed': True}, 'windows_native_inventory': {'passed': True},
                                    'nagisa_unicode': {'japanese_tokens_match': True}},
                  'app_helper_test': {'passed': True}}


class Readiness(unittest.TestCase):
    def test_exact_native_proof_is_required(self):
        for windows in (False, True):
            pack, report = fixture(windows)
            m.validate_ready(report, pack, 'a'*40)
            for field, value in [('source_sha','b'*40), ('reproducible', False), ('method','unverified')]:
                bad=copy.deepcopy(report); bad[field]=value
                with self.assertRaises(ValueError): m.validate_ready(bad, pack, 'a'*40)
            bad=copy.deepcopy(report); bad['native_installer_test']['passed']=False
            with self.assertRaises(ValueError): m.validate_ready(bad, pack, 'a'*40)

    def test_early_native_failure_is_reported_and_cache_removed(self):
        pack,_=fixture()
        with temporary_root() as tmp:
            root=tmp;output=root/'fresh';request=root/'request.json'
            request.write_text(json.dumps({'source_sha':'a'*40,'pack_id':pack,
                'selector':'full-inference','purpose':'one-time-feature-branch-qwen-inference-proof','prior_native_evidence':{}}),encoding='utf-8')
            def fail(*args,**kwargs):
                cache=output/'private-runtime-cache';cache.mkdir();(cache/'private-binary').write_bytes(b'disposable')
                raise RuntimeError('early native failure')
            argv=['proof','--source-sha','a'*40,'--pack',pack,'--output',str(output),'--request-copy',str(request)]
            with patch.object(sys,'argv',argv), patch.object(m.subprocess,'check_output',return_value='a'*40), \
                 patch.object(m,'prepare_cpu_proof',return_value=None),patch.object(m,'owned',side_effect=fail), patch('builtins.print'):
                with self.assertRaisesRegex(RuntimeError,'early native failure'):m.main()
            report=json.loads((output/(pack+'.qwen-inference-proof.json')).read_text(encoding='utf-8'))
            self.assertEqual(report['outcome'],'failed');self.assertEqual(report['stage'],'native-readiness')
            self.assertFalse(report['inference']['weights_downloaded']);self.assertTrue(report['private_outputs_removed'])
            self.assertFalse((output/'private-runtime-cache').exists())

    def test_exact_local_cpu_proof_is_prepared_before_all_recipe_candidates(self):
        import prepare_cpu_proof as metadata
        from test_own_cpu_recipe import SyntheticPublishedCpu
        pack,_=fixture()
        with temporary_root() as root:
            cpu=SyntheticPublishedCpu(root);output=root/'fresh';request=root/'request.json'
            request.write_text(json.dumps({'source_sha':'a'*40,'pack_id':pack,
                'selector':'full-inference','purpose':'one-time-feature-branch-qwen-inference-proof','prior_native_evidence':{}}),encoding='utf-8')
            def generation(command,**kwargs):
                self.assertEqual(Path(command[2]).name,'generate_recipes.py')
                proof_path=Path(command[command.index('--cpu-publication-proof')+1])
                self.assertEqual(proof_path,output/'private-runtime-cache'/cpu.path.name)
                self.assertEqual(proof_path.read_bytes(),cpu.path.read_bytes())
                raise RuntimeError('stop before native build')
            argv=['proof','--source-sha','a'*40,'--pack',pack,'--output',str(output),'--request-copy',str(request),
                  '--cpu-publication-proof',str(cpu.path)]
            with patch.object(sys,'argv',argv),patch.object(m.subprocess,'check_output',return_value='a'*40), \
                 patch.object(metadata,'ROOT',cpu.root),patch.object(metadata,'fetch',side_effect=AssertionError('offline proof must not download')), \
                 patch.object(m,'owned',side_effect=generation) as owned,patch('builtins.print'):
                with self.assertRaisesRegex(RuntimeError,'stop before native build'):m.main()
            owned.assert_called_once()
            report=json.loads((output/(pack+'.qwen-inference-proof.json')).read_text(encoding='utf-8'))
            self.assertEqual(report['outcome'],'failed');self.assertTrue(report['private_outputs_removed'])
            self.assertFalse(report['inference']['weights_downloaded'])
            self.assertFalse((output/'private-runtime-cache').exists())

    def test_windows_requires_private_closure_unicode_and_real_app_helper(self):
        pack, report=fixture(True)
        for section, field in [('windows_native_inventory','passed'), ('nagisa_unicode','japanese_tokens_match')]:
            bad=copy.deepcopy(report); bad['runtime_smoke'][section][field]=False
            with self.assertRaises(ValueError): m.validate_ready(bad, pack, 'a'*40)
        report['app_helper_test']['passed']=False
        with self.assertRaises(ValueError): m.validate_ready(report, pack, 'a'*40)

    def test_windows_cleanup_retains_inputs_until_owned_job_drain_confirmed(self):
        pack,_=fixture(True)
        for job in ({'tree_drained':False},{'job_start_pending':True},{},None,{'tree_drained':'true'}):
            with self.subTest(job=job),temporary_root() as output:
                native=output/'native-readiness';native.mkdir()
                private=native/'recipe inputs é 测试';private.mkdir();(private/'binary').write_bytes(b'private')
                cache=output/'private-runtime-cache';cache.mkdir()
                receipt=native/(pack+'.offline-pip-proof.json')
                receipt.write_text(json.dumps({'native_installer_test':{'windows_owned_job':job}}),encoding='utf-8')
                self.assertTrue(m.cleanup_private_outputs(output,pack))
                self.assertTrue(private.exists());self.assertTrue(cache.exists());self.assertTrue(receipt.exists())

    def test_windows_cleanup_allows_prelaunch_failure_or_confirmed_drain(self):
        pack,_=fixture(True)
        for native_state in (None,{'tested':False,'passed':False},{'windows_owned_job':{'tree_drained':True}}):
            with self.subTest(native_state=native_state),temporary_root() as output:
                native=output/'native-readiness';native.mkdir()
                private=native/'recipe inputs é 测试';private.mkdir()
                cache=output/'private-runtime-cache';cache.mkdir()
                receipt=native/(pack+'.offline-pip-proof.json')
                if native_state is not None:receipt.write_text(json.dumps({'native_installer_test':native_state}),encoding='utf-8')
                self.assertEqual(m.cleanup_private_outputs(output,pack),[])
                self.assertFalse(private.exists());self.assertFalse(cache.exists())
                self.assertEqual(receipt.exists(),native_state is not None)

    def test_windows_cleanup_unreadable_checkpoint_is_fail_closed(self):
        pack,_=fixture(True)
        for value in ('malformed','[]','x'*(2*1024*1024+1)):
            with temporary_root() as output:
                native=output/'native-readiness';native.mkdir()
                receipt=native/(pack+'.offline-pip-proof.json');receipt.write_text(value,encoding='utf-8')
                cache=output/'private-runtime-cache';cache.mkdir()
                self.assertTrue(m.cleanup_private_outputs(output,pack));self.assertTrue(cache.exists())


class DiagnosticRouting(unittest.TestCase):
    def request(self, path, pack, selector='readme-diagnostic'):
        import fixture as download
        value={'source_sha':'a'*40,'pack_id':pack,'purpose':'one-time-feature-branch-qwen-inference-proof',
               'prior_native_evidence':{},'selector':selector}
        if selector=='readme-diagnostic':value['diagnostic']=dict(download.README_REQUEST)
        path.write_text(json.dumps(value),encoding='utf-8')

    def test_readme_main_cannot_fall_through_to_preload_runtime_weights_audio_or_inference(self):
        import fixture as download
        pack,_=fixture(windows=True)
        for error in (None,download.FixtureDownloadError({**download.unconfirmed_child_error('child-reap-unconfirmed',None),'child_reap_confirmed':False})):
            with self.subTest(error=error is not None),temporary_root() as root:
                output=root/'fresh';request=root/'request.json';self.request(request,pack)
                argv=['proof','--source-sha','a'*40,'--pack',pack,'--output',str(output),'--request-copy',str(request)]
                blockers={name:Mock(side_effect=AssertionError('README must not call '+name)) for name in ('prepare_cpu_proof','owned','build','model_smoke','bounded_worker_requests')}
                with patch.object(sys,'argv',argv),patch.object(m.subprocess,'check_output',return_value='a'*40), \
                     patch.multiple(m,**blockers),patch.object(download,'download_child',side_effect=error) as child,patch('builtins.print'):
                    if error:
                        with self.assertRaisesRegex(RuntimeError,'Bounded README diagnostic failed'):m.main()
                    else:m.main()
                child.assert_called_once();self.assertEqual(child.call_args.kwargs['selector'],'readme-diagnostic')
                for blocker in blockers.values():blocker.assert_not_called()
                report=json.loads((output/(pack+'.qwen-inference-proof.json')).read_text(encoding='utf-8'))
                self.assertEqual(report['selector'],'readme-diagnostic');self.assertEqual(report['stage'],'readme-download-diagnostic')
                self.assertIsNone(report['fresh_native_proof_sha256']);self.assertFalse(report['inference']['tested']);self.assertFalse(report['inference']['weights_downloaded'])
                self.assertLessEqual(len(json.dumps(report).encode('utf-8')),8192)
                if error:
                    self.assertEqual(report['outcome'],'failed');self.assertFalse(report['private_outputs_removed'])
                    self.assertFalse(report['diagnostic']['fixture_cache_removed'])
                    self.assertIn('Owned downloader child reap is unconfirmed',report['cleanup_errors'][0])
                    self.assertTrue(list(output.glob('qwen-readme-diagnostic-*/readme-request.json')))
                else:
                    self.assertEqual(report['outcome'],'passed-readme-only');self.assertTrue(report['private_outputs_removed'])
                    self.assertEqual([x.name for x in output.iterdir()],[pack+'.qwen-inference-proof.json'])

    def test_unknown_or_absent_selector_fails_before_output_or_any_execution(self):
        pack,_=fixture()
        for selector in ('unknown','',None):
            with self.subTest(selector=selector),temporary_root() as root:
                output=root/'fresh';request=root/'request.json';self.request(request,pack,selector)
                argv=['proof','--source-sha','a'*40,'--pack',pack,'--output',str(output),'--request-copy',str(request)]
                with patch.object(sys,'argv',argv),patch.object(m.subprocess,'check_output',return_value='a'*40), \
                     patch.object(m,'run_readme_diagnostic') as diagnostic,patch.object(m,'owned') as owned,self.assertRaises(ValueError):m.main()
                diagnostic.assert_not_called();owned.assert_not_called();self.assertFalse(output.exists())

    def test_both_downloader_sections_block_output_cleanup_when_reap_is_unconfirmed(self):
        pack,_=fixture()
        for section in ('inference','diagnostic'):
            for reaped in (False,True):
                with self.subTest(section=section,reaped=reaped),temporary_root() as output:
                    private=output/'private-runtime-cache';private.mkdir();(private/'owned-input').write_text('retain until reaped',encoding='utf-8')
                    report={section:{'download_failure':{'child_reap_confirmed':reaped}}}
                    errors=m.cleanup_private_outputs(output,pack,report)
                    self.assertEqual(bool(errors),not reaped);self.assertEqual(private.exists(),not reaped)


if __name__ == '__main__': unittest.main()
