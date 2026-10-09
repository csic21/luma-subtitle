import copy
import io
import json
from pathlib import Path
import subprocess
import sys
import unittest
from unittest.mock import Mock, patch
import warnings
import zipfile
sys.path.insert(0,str(Path(__file__).resolve().parents[1]))
from fixture_paths import temporary_root
import replay_driver as replay


def archive(entries):
    stream=io.BytesIO()
    with zipfile.ZipFile(stream,'w') as z:
        for name,data in entries:z.writestr(name,data)
    return stream.getvalue()


def fixture():
    provenance={'luma_source_sha':replay.PRODUCER};exports=[]
    sources=archive([('BUILD-PROVENANCE.json',json.dumps(provenance)),('SOURCE-EXPORTS.json',json.dumps(exports)),
        *[('luma/scripts/asr-components/ct2-cpu/'+name,b'# exact source') for name in ('build_cpu.py','prepare_toolchain.ps1','verify_runtime.py','sources.lock.json')]])
    payloads={replay.WHEEL:b'wheel',replay.SOURCES:sources,replay.NOTICES:b'notices'}
    pins={name:(len(data),replay.sha(data)) for name,data in payloads.items()}
    manifest={'schema_version':1,'kind':'ct2-cpu-diagnostic','publication_authorized':False,'installable':False,'inference_passed':False,
       'origin':{'repository':'csic21/luma-subtitle','run_id':replay.RUN_ID,'run_attempt':1,'job':'windows-cpu-proof',
         'head_sha':replay.PRODUCER,'source_sha':replay.PRODUCER,'ref':'refs/heads/feat/optional-asr-engines','event_name':'push',
         'workflow_ref':'csic21/luma-subtitle/.github/workflows/asr-ct2-cpu.yml@refs/heads/feat/optional-asr-engines'},
       'assets':[{'name':n,'bytes':pins[n][0],'sha256':pins[n][1]} for n in sorted(pins)],
       'verification':{'outcome':'timed_out','stage':'native-verifier','started':True,'timeout_seconds':900},
       'build_checks':{'compiler_probe':True,'executable_bytes_modified':False,'fresh_fixed_root_repeatability':True,'static_runtime_closure':True},
       'provenance':provenance,'source_exports':exports}
    payloads[replay.MANIFEST]=json.dumps(manifest).encode();pins[replay.MANIFEST]=(len(payloads[replay.MANIFEST]),replay.sha(payloads[replay.MANIFEST]))
    return payloads,pins,manifest


class ReplayDriverTests(unittest.TestCase):
    def test_four_files_and_original_recipe_are_verified_before_extraction(self):
        payloads,pins,manifest=fixture();data=archive(payloads.items())
        with temporary_root() as root,patch.object(replay,'PINS',pins),patch.object(replay,'ZIP_PIN',(len(data),replay.sha(data))):
            path=root/'diagnostic.zip';path.write_bytes(data)
            source,actual,recipes=replay.verify_extract(path,root/'work')
            self.assertEqual(actual,manifest);self.assertTrue((source/'scripts/asr-components/ct2-cpu/build_cpu.py').is_file())
            self.assertEqual(len(recipes),4)
            self.assertEqual((root/'work/artifact'/replay.WHEEL).read_bytes(),b'wheel')
            with self.assertRaises(ValueError):replay.verify_extract(path,root/'work')

    def test_zip_or_member_mismatch_does_not_create_work(self):
        payloads,pins,_=fixture()
        for kind in ('zip','member'):
            bad=dict(payloads);bad[replay.WHEEL]=b'evil!';data=archive(bad.items())
            zip_pin=(len(data),replay.sha(data)) if kind=='member' else (len(data),'a'*64)
            with self.subTest(kind=kind),temporary_root() as root,patch.object(replay,'PINS',pins),patch.object(replay,'ZIP_PIN',zip_pin):
                path=root/'bad.zip';path.write_bytes(data)
                with self.assertRaises(ValueError):replay.verify_extract(path,root/'work')
                self.assertFalse((root/'work').exists())

    def test_missing_duplicate_extra_and_unsafe_members_fail_closed(self):
        payloads,pins,_=fixture()
        variants=[list(payloads.items())[:-1],list(payloads.items())+[('extra',b'x')],
                  list(payloads.items())+[(replay.WHEEL,b'wheel')],list(payloads.items())+[('../escape',b'x')]]
        for entries in variants:
            with warnings.catch_warnings():
                warnings.simplefilter('ignore');data=archive(entries)
            with temporary_root() as root,patch.object(replay,'PINS',pins),patch.object(replay,'ZIP_PIN',(len(data),replay.sha(data))):
                path=root/'bad.zip';path.write_bytes(data)
                with self.assertRaises(ValueError):replay.verify_extract(path,root/'work')
                self.assertFalse((root/'work').exists())

    def test_archive_paths_symlinks_and_encryption_rejected(self):
        for name in ('/absolute','../escape','a/../b','a\\b','C:drive','a//b','a/'):
            with self.subTest(name=name),self.assertRaises(ValueError):replay.safe_member(zipfile.ZipInfo(name))
        for attr,flag in ((0o120777<<16,0),(0,1)):
            info=zipfile.ZipInfo('safe');info.external_attr=attr;info.flag_bits=flag
            with self.assertRaises(ValueError):replay.safe_member(info)

    def test_origin_and_original_failure_cannot_be_relabelled(self):
        _,pins,manifest=fixture()
        with patch.object(replay,'PINS',pins):
            replay.validate_manifest(manifest)
            for key,value in [('installable',True),('publication_authorized',True),('inference_passed',True),('assets',[]),('origin',{}),('verification',{})]:
                bad=copy.deepcopy(manifest);bad[key]=value
                with self.subTest(key=key),self.assertRaises(ValueError):replay.validate_manifest(bad)

    def test_private_environment_excludes_host_python_path_and_credentials(self):
        with patch.dict('os.environ',{'SYSTEMROOT':'C:/Windows','SECRET':'hidden','PATH':'host','PYTHONPATH':'host'},clear=True):
            result=replay.clean_environment(Path('/private/home'))
        self.assertEqual(result['SYSTEMROOT'],'C:/Windows');self.assertNotIn('SECRET',result)
        self.assertNotEqual(result['PATH'],'host');self.assertEqual(result['HF_HUB_OFFLINE'],'1')

    def test_normal_exit_and_passed_report_are_both_required(self):
        with temporary_root() as root:
            output=root/'case';output.mkdir();(output/'result.json').write_text('{"passed":true}')
            result=replay.run_case([sys.executable,'-B','-c','pass'],root,output,dict(__import__('os').environ),5)
            self.assertTrue(result['passed']);self.assertTrue(result['owned_child_reaped'])
            self.assertTrue(json.loads((root/'replay-child-drain.json').read_text())['drained'])
            output=root/'bad';output.mkdir();(output/'result.json').write_text('{"passed":true}')
            result=replay.run_case([sys.executable,'-B','-c','raise SystemExit(2)'],root,output,dict(__import__('os').environ),5)
            self.assertFalse(result['passed']);self.assertEqual(result['returncode'],2)

    def test_timeout_kills_and_reaps_without_converting_saved_success(self):
        with temporary_root() as root:
            output=root/'case';output.mkdir();(output/'result.json').write_text('{"passed":true}')
            result=replay.run_case([sys.executable,'-B','-c','import time;time.sleep(5)'],root,output,dict(__import__('os').environ),0.05)
            self.assertEqual(result['outcome'],'timed_out');self.assertFalse(result['passed']);self.assertTrue(result['owned_child_reaped'])

    def test_fast_exiting_child_cannot_bypass_output_bound(self):
        with temporary_root() as root:
            output=root/'case';output.mkdir();(output/'result.json').write_text('{"passed":true}')
            child=Mock(pid=123);child.poll.return_value=0;child.wait.return_value=0
            def launch(*args,**kwargs):
                kwargs['stdout'].write(b'x'*4_000_001);kwargs['stdout'].flush();return child
            with patch.object(replay.subprocess,'Popen',side_effect=launch):
                result=replay.run_case(['owned-child'],root,output,{},5)
            self.assertTrue(result['normal_exit']);self.assertFalse(result['passed'])
            self.assertEqual(result['outcome'],'output_bound_exceeded')
            self.assertEqual(result['observed_log_bytes'],4_000_001)
            self.assertEqual(result['log_bytes'],4_000_000)
            self.assertTrue(result['log_truncated']);child.wait.assert_called_once()

    def test_cancellation_always_kills_and_reaps_owned_child(self):
        child=Mock(pid=123);child.poll.return_value=None;child.wait.side_effect=[KeyboardInterrupt(),-9]
        with temporary_root() as root,patch.object(replay.subprocess,'Popen',return_value=child):
            with self.assertRaises(KeyboardInterrupt):replay.run_case(['owned-child'],root,root/'case',{},5)
        child.kill.assert_called_once();self.assertEqual(child.wait.call_count,2)

    def test_failed_reap_leaves_durable_undrained_checkpoint(self):
        child=Mock(pid=123);child.poll.return_value=None
        child.wait.side_effect=[subprocess.TimeoutExpired('owned-child',10)]
        with temporary_root() as root,patch.object(replay.subprocess,'Popen',return_value=child):
            with self.assertRaises(subprocess.TimeoutExpired):
                replay.run_case(['owned-child'],root,root/'default-unload',{},0)
            checkpoint=json.loads((root/'replay-child-drain.json').read_text())
            self.assertEqual(checkpoint,{'schema':1,'owner':'replay-driver','case':'default-unload','drained':False,'pid':123})
            child.kill.assert_called_once()

    def test_launch_failure_retains_conservative_checkpoint(self):
        with temporary_root() as root,patch.object(replay.subprocess,'Popen',side_effect=OSError('launch failed')):
            with self.assertRaises(OSError):replay.run_case(['owned-child'],root,root/'one-eof',{},1)
            self.assertEqual(json.loads((root/'replay-child-drain.json').read_text())['drained'],False)

    def test_source_only_replay_cannot_call_native_build_or_publication(self):
        source=Path(replay.__file__).read_text()
        self.assertNotIn('build_once(',source);self.assertNotIn('package_publication(',source)
        self.assertIn("cpu/'prepare_toolchain.ps1', '-Reports'",source)
        self.assertIn('deadline = time.monotonic() + 900',source)
        self.assertIn('timeout = min(120, deadline-time.monotonic())',source)
        self.assertEqual(len(replay.CASES),6)


if __name__=='__main__':unittest.main()
