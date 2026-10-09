import copy
import json
from pathlib import Path
import sys
import unittest
from unittest.mock import Mock
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from fixture_paths import temporary_root
import lifecycle_probe as probe


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
