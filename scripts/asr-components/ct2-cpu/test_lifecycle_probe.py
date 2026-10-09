import copy
import hashlib
import json
from pathlib import Path
import sys
import unittest
from unittest.mock import Mock, patch
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from fixture_paths import temporary_root
import lifecycle_probe as probe
import verify_runtime as original_policy
from types import SimpleNamespace


def frame():
    return {'id':'cold','event':'result','device':'cpu','backend':'faster-whisper','reused':False,
            'segments':[{'start_ms':0,'end_ms':10999,'text':'ask your country'}],
            'load_seconds':0.1,'inference_seconds':0.2}


class LifecycleProbeTests(unittest.TestCase):
    def test_real_result_content_time_and_reuse_assertions_stay_strict(self):
        self.assertEqual(probe.validate_result(frame(),False)['inference_seconds'],0.2)
        for key,value in [('event','error'),('device','cuda'),('backend','other'),('reused',True),
                          ('segments',[]),('load_seconds',-1),('inference_seconds',float('nan'))]:
            bad=frame();bad[key]=value
            with self.subTest(key=key),self.assertRaises(ValueError):probe.validate_result(bad,False)
        bad=frame();bad['segments'][0]['end_ms']=12000
        with self.assertRaises(ValueError):probe.validate_result(bad,False)

    def test_supported_thread_argument_wrapper_preserves_all_other_arguments(self):
        calls=[]
        def constructor(path,*,cpu_threads=0,**kwargs):
            calls.append((path,cpu_threads,kwargs));return 'actual-model'
        journal=Mock();wrapped=probe.thread_override(constructor,journal)
        self.assertEqual(wrapped('original-model',device='cpu',num_workers=1),'actual-model')
        self.assertEqual(calls,[('original-model',1,{'device':'cpu','num_workers':1})])
        with self.assertRaises(ValueError):wrapped('model',cpu_threads=4)
        with self.assertRaises(ValueError):probe.thread_override(lambda path:None,journal)
        self.assertEqual(len(calls),1)

    def test_retained_factory_returns_actual_worker_and_does_not_unload_it(self):
        class Worker:
            def __init__(self):self.model=object()
        holder=[];factory=probe.retained_factory(Worker,holder,Mock())
        first=factory();second=factory()
        self.assertIs(first,second);self.assertIs(type(first),Worker);self.assertEqual(len(holder),1)
        self.assertIsNotNone(first.model)

    def test_atomic_json_retains_measured_timings_without_performance_claim(self):
        with temporary_root() as root:
            path=root/'result.json';probe.write_json(path,{'results':[probe.validate_result(frame(),False)],'performance_claim':False})
            self.assertFalse(json.loads(path.read_text())['performance_claim'])
            self.assertFalse(path.with_suffix('.tmp').exists())

    def test_only_verified_defender_is_added_to_original_loaded_module_policy(self):
        with temporary_root() as home:
            root,system=home/'private',home/'Windows'
            defender=str(home/'ProgramData/Microsoft/Windows Defender/Platform/4.18.26080.4-0/MpOAV.dll')
            auditor=SimpleNamespace(verified_defender_module=Mock(return_value={'verified':True,'kind':'windows-defender-amsi'}))
            paths=[str(root/'msvcp140.dll'),str(system/'System32/msvcp_win.dll'),defender]
            result=probe.diagnostic_loaded_paths(paths,root,system,original_policy,auditor)
            self.assertEqual([r['scope'] for r in result],['private','windows_os','verified_host_security'])
            auditor.verified_defender_module.assert_called_once_with(defender)
            for name in ('msvcp140.dll','libiomp5md.dll','cudnn64.dll','arbitrary.dll','MpOAV.dll.extra'):
                auditor.verified_defender_module.reset_mock()
                with self.subTest(name=name),self.assertRaises(RuntimeError):
                    probe.diagnostic_loaded_paths([defender,str(home/'outside'/name)],root,system,original_policy,auditor)
                auditor.verified_defender_module.assert_not_called()
            auditor.verified_defender_module.return_value={'verified':False,'kind':'windows-defender-amsi'}
            with self.assertRaises(RuntimeError):probe.diagnostic_loaded_paths([defender],root,system,original_policy,auditor)
            auditor.verified_defender_module.side_effect=AssertionError('untrusted registration/signature')
            with self.assertRaises(AssertionError):probe.diagnostic_loaded_paths([defender],root,system,original_policy,auditor)

    def test_trust_loaded_modules_are_reaudited_to_bounded_fixed_point(self):
        with temporary_root() as home:
            root,system=home/'private',home/'Windows'
            defender=str(home/'ProgramData/MpOAV.dll');dll=str(system/'System32/cryptnet.dll')
            auditor=SimpleNamespace(verified_defender_module=Mock(return_value={'verified':True,'kind':'windows-defender-amsi'}))
            original=SimpleNamespace(validate_loaded_paths=original_policy.validate_loaded_paths,
                                     loaded_modules=Mock(side_effect=[[defender],[defender,dll],[defender,dll]]))
            result=probe.audit_diagnostic_modules(root,system,original,auditor)
            self.assertEqual(len(result),2);self.assertEqual(original.loaded_modules.call_count,3)
            auditor.verified_defender_module.assert_called_once()
            external=str(home/'outside/msvcp140.dll')
            original.loaded_modules=Mock(side_effect=[[defender],[defender,external]])
            with self.assertRaisesRegex(RuntimeError,'Host-global VC'):
                probe.audit_diagnostic_modules(root,system,original,auditor)
            original.loaded_modules=Mock(side_effect=[[defender],[defender,dll],[defender]])
            with self.assertRaisesRegex(RuntimeError,'stabilize'):
                probe.audit_diagnostic_modules(root,system,original,auditor)
            original.loaded_modules=Mock(return_value=[])
            with self.assertRaisesRegex(RuntimeError,'snapshot exceeds'):
                probe.audit_diagnostic_modules(root,system,original,auditor)

    def test_auditor_hash_is_checked_before_import_and_source_record_is_separate(self):
        with temporary_root() as root:
            path=root/'auditor.py';data=b'def verified_defender_module(path): return {}\n';path.write_bytes(data)
            module,identity=probe.load_host_auditor(path,hashlib.sha256(data).hexdigest(),'b'*40)
            self.assertTrue(callable(module.verified_defender_module))
            self.assertEqual(identity['source_sha'],'b'*40);self.assertEqual(identity['bytes'],len(data))
            path.write_text("raise RuntimeError('must not execute')",encoding='ascii')
            with self.assertRaisesRegex(ValueError,'bytes changed'):
                probe.load_host_auditor(path,hashlib.sha256(data).hexdigest(),'b'*40)
            with self.assertRaisesRegex(ValueError,'identity'):
                probe.load_host_auditor(path,'not-a-hash','b'*40)

    def test_cleanup_and_normal_exit_are_not_bypassed(self):
        source=Path(probe.__file__).read_text()
        self.assertIn('holder[0].unload()',source)
        self.assertIn("with journal.stage('worker.model_switch'):",source)
        self.assertIn("if args.case != 'eof':",source)
        self.assertIn("worker_class.unload.__globals__['gc']",source)
        self.assertNotIn('os._exit(',source)
        self.assertIn('faulthandler.dump_traceback_later(90, repeat=False, file=trace)',source)
        self.assertIn("'normal_exit_observed_by_parent': False",source)


if __name__=='__main__':unittest.main()
