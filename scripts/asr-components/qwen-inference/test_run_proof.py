import copy
import importlib.util
from pathlib import Path
import sys
import unittest
import json
from unittest.mock import patch

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
                'purpose':'one-time-feature-branch-qwen-inference-proof','prior_native_evidence':{}}))
            def fail(*args,**kwargs):
                cache=output/'private-runtime-cache';cache.mkdir();(cache/'private-binary').write_bytes(b'disposable')
                raise RuntimeError('early native failure')
            argv=['proof','--source-sha','a'*40,'--pack',pack,'--output',str(output),'--request-copy',str(request)]
            with patch.object(sys,'argv',argv), patch.object(m.subprocess,'check_output',return_value='a'*40), patch.object(m,'owned',side_effect=fail), patch('builtins.print'):
                with self.assertRaisesRegex(RuntimeError,'early native failure'):m.main()
            report=json.loads((output/(pack+'.qwen-inference-proof.json')).read_text())
            self.assertEqual(report['outcome'],'failed');self.assertEqual(report['stage'],'native-readiness')
            self.assertFalse(report['inference']['weights_downloaded']);self.assertTrue(report['private_outputs_removed'])
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


if __name__ == '__main__': unittest.main()
