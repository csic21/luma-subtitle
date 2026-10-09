import ctypes
import copy
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import types
import unittest
from unittest.mock import patch
from fixture_paths import temporary_root
import windows_test_job as j


class FakeKernel:
    def __init__(self, failure=None):
        self.failure=failure; self.calls=[]; self.terminated=False;self.root_exited=False
    def CreateJobObjectW(self,*args):self.calls.append('job');return 10
    def SetInformationJobObject(self,handle,kind,pointer,size):
        assert kind==9 and pointer._obj.basic.flags==0x2000;return 1
    def InitializeProcThreadAttributeList(self,pointer,count,flags,size):
        size._obj.value=64
        return 0 if pointer is None or self.failure=='attributes' else 1
    def UpdateProcThreadAttribute(self,*args):return 1
    def DeleteProcThreadAttributeList(self,*args):self.calls.append('delete-attributes')
    def CreateProcessW(self,exe,line,ps,ts,inherit,flags,env,cwd,startup,info):
        self.calls.append('create')
        self.exe=exe
        assert flags&4 and flags&0x80000 and inherit
        assert startup._obj.base.cb==ctypes.sizeof(j.StartupEx)
        if self.failure=='launch':return 0
        if self.failure=='utf8':os.write(startup._obj.base.stdout,b'\xff')
        info._obj.process=20;info._obj.thread=21;info._obj.pid=20;return 1
    def AssignProcessToJobObject(self,*args):self.calls.append('assign');return self.failure!='assign'
    def ResumeThread(self,*args):self.calls.append('resume');return 1
    def WaitForSingleObject(self,*args):
        if self.failure=='cancel' and not self.terminated:raise KeyboardInterrupt()
        if self.failure=='timeout' and not self.terminated:return 258
        self.root_exited=True;return 0
    def GetExitCodeProcess(self,process,code):code._obj.value=101 if self.failure=='orphan-failed-root' else 0;return 1
    def TerminateProcess(self,*args):self.calls.append('terminate-process');self.terminated=True;return 1
    def TerminateJobObject(self,*args):self.calls.append('terminate-job');self.terminated=True;return 1
    def QueryInformationJobObject(self,job,kind,value,size,returned):
        if kind==3:
            self.calls.append('job-pids')
            pids=([20] if not self.root_exited else [22] if self.failure in ('orphan','orphan-failed-root','undrained') and not self.terminated else [])
            value._obj.assigned=len(pids);value._obj.count=len(pids)
            for index,pid in enumerate(pids):value._obj.pids[index]=pid
            return 1
        self.calls.append('accounting');value._obj.total_processes=3
        value._obj.active_processes=int(self.failure=='undrained' or self.failure in ('orphan','orphan-failed-root') and not self.terminated);return 1
    def CreateToolhelp32Snapshot(self,*args):return 30
    def Process32FirstW(self,snapshot,entry):
        self.snapshot_index=0;entry._obj.pid=20;entry._obj.parent_pid=1;return 1
    def Process32NextW(self,snapshot,entry):
        self.snapshot_index+=1
        if self.snapshot_index==1:entry._obj.pid=22;entry._obj.parent_pid=20;return 1
        if self.snapshot_index==2:entry._obj.pid=999;entry._obj.parent_pid=998;return 1
        return 0
    def OpenProcess(self,access,inherit,pid):
        assert access==0x1000 and not inherit
        self.calls.append(('open',pid));return pid+100
    def IsProcessInJob(self,process,job,value):value._obj.value=1;return 1
    def GetProcessTimes(self,process,created,exited,kernel,user):
        created._obj.value=100 if process==120 else 200;return 1
    def QueryFullProcessImageNameW(self,process,flags,buffer,size):
        buffer.value=self.exe;size._obj.value=len(self.exe);return 1
    def CloseHandle(self,handle):self.calls.append(('close',handle));return 1


class WindowsJobTests(unittest.TestCase):
    def test_x64_win32_structure_layouts(self):
        self.assertEqual(ctypes.sizeof(ctypes.c_void_p),8)
        for structure,size in [(j.BasicLimits,64),(j.IoCounters,48),(j.ExtendedLimits,144),
                               (j.Accounting,48),(j.Startup,104),(j.StartupEx,112),(j.ProcessInfo,24)]:
            self.assertEqual(ctypes.sizeof(structure),size)
        self.assertEqual(j.Startup.stdin.offset,80)
        self.assertEqual(j.Accounting.active_processes.offset,40)
        self.assertEqual(ctypes.sizeof(j.ProcessEntry),568)
        self.assertEqual(j.ProcessEntry.parent_pid.offset,32)
        self.assertEqual(j.JobPids.pids.offset,8)

    def test_explicit_executable_quoting_and_utf16_bound(self):
        with temporary_root() as root:
            exe=root/'test executable.exe';exe.touch()
            args=[str(exe),'','space é 测试','quote"inside','trailing slash\\']
            self.assertEqual(j.command_line(args),subprocess.list2cmdline(args))
            for bad in [[],['relative.exe'],[str(exe),'nul\0argument'],[str(exe),'😀'*17000]]:
                with self.assertRaises(ValueError):j.command_line(bad)

    def test_environment_rejects_case_aliases_and_nul(self):
        self.assertIsNone(j.environment_block(None))
        self.assertEqual(j.environment_block({'Z':'last','a':'first'}).value,'a=first')
        for env in [{'PATH':'a','Path':'b'},{'bad=name':'value'},{'A':'\0'}]:
            with self.assertRaises(ValueError):j.environment_block(env)

    def fake_run(self,failure=None,*,build=False):
        kernel=FakeKernel(failure)
        with temporary_root() as root:
            exe=root/('cargo.exe' if build else 'fixture.exe');exe.touch()
            with patch.object(j.sys,'platform','win32'),patch.object(j,'kernel_api',return_value=kernel), \
                 patch.dict(sys.modules,{'msvcrt':types.SimpleNamespace(get_osfhandle=lambda fd:fd)}), \
                 patch.object(j.os,'set_handle_inheritable',create=True), \
                 patch.object(j.c,'WinError',side_effect=lambda code:OSError('expected API failure'),create=True), \
                 patch.object(j.c,'get_last_error',return_value=18,create=True), \
                 patch.object(j.time,'monotonic',side_effect=iter(range(100))):
                command=[str(exe),*(['test','--no-run','--message-format=json'] if build else [])]
                try: result=j.run_owned_tree(command,cwd=root,timeout=.001,_build_only=build)
                except j.JobRunError as error:result=error.result
        return kernel,result

    def test_assignment_precedes_execution_and_all_handles_close(self):
        kernel,result=self.fake_run()
        self.assertTrue(result['tree_drained']);self.assertTrue(result['job_assigned_before_resume'])
        self.assertTrue(result['natural_drain_completed']);self.assertFalse(result['termination_requested'])
        self.assertEqual(result['after_root_exit']['processes'],[])
        self.assertLess(kernel.calls.index('assign'),kernel.calls.index('resume'))
        for handle in (10,20,21):self.assertEqual(kernel.calls.count(('close',handle)),1)

    def test_assignment_failure_kills_suspended_root_without_resuming(self):
        kernel,result=self.fake_run('assign')
        self.assertNotIn('resume',kernel.calls);self.assertIn('terminate-process',kernel.calls)
        self.assertFalse(result['job_assigned_before_resume']);self.assertTrue(result['tree_drained'])

    def test_timeout_and_cancellation_terminate_and_drain_whole_tree(self):
        for failure in ('timeout','cancel'):
            kernel,result=self.fake_run(failure)
            self.assertIn('terminate-job',kernel.calls);self.assertTrue(result['tree_drained'])
            self.assertEqual(result['total_processes'],3)

    def test_partial_attribute_or_launch_failure_is_closed(self):
        for failure in ('attributes','launch'):
            kernel,result=self.fake_run(failure)
            self.assertNotIn('resume',kernel.calls);self.assertTrue(result['tree_drained'])
            self.assertEqual('delete-attributes' in kernel.calls,failure=='launch')

    def test_root_exit_with_live_descendant_is_failure_then_killed_and_drained(self):
        kernel,result=self.fake_run('orphan')
        self.assertEqual(result['exit_code'],0)
        self.assertIn('reason',result)
        self.assertIn('terminate-job',kernel.calls)
        self.assertTrue(result['tree_drained'])
        self.assertFalse(result['natural_drain_completed']);self.assertTrue(result['termination_requested'])
        report=result['before_forced_termination']
        self.assertEqual(report['phase'],'before-forced-termination')
        self.assertEqual(len(report['processes']),1)
        survivor=report['processes'][0]
        self.assertEqual((survivor['pid'],survivor['parent_pid']),(22,20))
        self.assertTrue(survivor['parent_chain_reaches_root'])
        self.assertEqual(survivor['parent_chain'][0]['pid'],20)
        self.assertIn('canonical_executable',survivor)
        self.assertNotIn(('open',999),kernel.calls)
        self.assertLess(kernel.calls.index('job-pids'),kernel.calls.index('terminate-job'))

    def test_successful_compile_only_root_explicitly_reaps_all_build_helpers(self):
        kernel,result=self.fake_run('orphan',build=True)
        self.assertEqual(result['exit_code'],0);self.assertTrue(result['tree_drained'])
        self.assertNotIn('reason',result);self.assertTrue(result['build_helper_cleanup_completed'])
        self.assertEqual(result['drain_policy'],'terminate-build-helpers')
        self.assertTrue(result['termination_requested']);self.assertFalse(result['natural_drain_completed'])
        self.assertIn('terminate-job',kernel.calls)

    def test_compile_only_failure_or_undrained_helpers_cannot_pass(self):
        for failure in ('orphan-failed-root','undrained'):
            _,result=self.fake_run(failure,build=True)
            self.assertIn('reason',result);self.assertNotIn('build_helper_cleanup_completed',result)
        self.assertFalse(result['tree_drained'])

    def test_compile_only_output_must_be_strict_utf8(self):
        _,result=self.fake_run('utf8',build=True)
        self.assertTrue(result['tree_drained']);self.assertEqual(result['reason'],'invalid-cargo-utf8')

    def test_runtime_command_cannot_request_build_helper_cleanup(self):
        with temporary_root() as root,patch.object(j.sys,'platform','win32'):
            exe=root/'cargo.exe';exe.touch()
            for command in ([str(exe),'test'],[str(exe),'test','--no-run','--message-format=json','--','--ignored']):
                with self.assertRaisesRegex(ValueError,'compile-only'):j.run_owned_tree(command,cwd=root,_build_only=True)

    def test_diagnostic_categories_never_authorize_or_collect_arguments(self):
        self.assertEqual(j.command_category(r'C:\private\vctip.exe'),'msvc-telemetry-name')
        self.assertEqual(j.command_category(r'C:\private\secret-data.exe'),'other-executable')
        _,result=self.fake_run('orphan')
        report=result['before_forced_termination']
        self.assertFalse(report['category_is_trust_decision'])
        self.assertFalse(report['raw_command_lines_collected'])
        self.assertFalse(report['environment_collected'])
        self.assertEqual(set(report['processes'][0]),{
            'pid','parent_pid','job_membership_verified','created_filetime','observed_filetime',
            'exited_filetime','canonical_executable','command_category','parent_chain','parent_chain_reaches_root'})

    def test_diagnostic_failure_cannot_skip_forced_job_cleanup(self):
        with patch.object(j,'owned_process_snapshot',side_effect=RuntimeError('do not publish secret')):
            kernel,result=self.fake_run('orphan')
        self.assertTrue(result['tree_drained']);self.assertTrue(result['termination_requested'])
        self.assertIn('terminate-job',kernel.calls)
        self.assertNotIn('do not publish secret',json.dumps(result))
        self.assertEqual(result['before_forced_termination']['errors'][0]['error_type'],'RuntimeError')

    def test_diagnostic_serialization_failure_cannot_skip_forced_job_cleanup(self):
        with patch.object(j.JobDiagnostics,'report',side_effect=ValueError('do not publish secret')):
            kernel,result=self.fake_run('orphan')
        self.assertTrue(result['tree_drained']);self.assertTrue(result['termination_requested'])
        self.assertIn('terminate-job',kernel.calls)
        self.assertNotIn('do not publish secret',json.dumps(result))
        self.assertEqual(result['before_forced_termination']['errors'][0]['api'],'diagnostic-report')

    def test_pid_reuse_and_missing_parent_do_not_fabricate_ancestry(self):
        def record(pid,parent,created,observed):
            return {'pid':pid,'parent_pid':parent,'created_filetime':created,'observed_filetime':observed,
                    'job_membership_verified':True,'exited_filetime':None}
        tracker=j.JobDiagnostics(None,None,20)
        for parent in (record(20,1,300,500),record(20,1,100,150)):
            tracker.history={(20,parent['created_filetime']):parent}
            tracker.latest={'processes':[record(22,20,200,600)]}
            survivor=tracker.report('test')['processes'][0]
            self.assertFalse(survivor['parent_chain_reaches_root'])
            self.assertEqual(survivor['parent_chain'],[])
            self.assertEqual(survivor['unresolved_parent_pid'],20)

    def test_history_and_serialized_reports_are_bounded(self):
        tracker=j.JobDiagnostics(None,None,1)
        records=[{'pid':pid,'parent_pid':1,'created_filetime':pid,
                  'canonical_executable':'é'*j.MAX_IMAGE_CHARS} for pid in range(2,80)]
        with patch.object(j,'owned_process_snapshot',return_value={'processes':records,'errors':[],'truncated':False}), \
             patch.object(j,'MAX_DIAGNOSTIC_RECORDS',2):tracker.sample()
        self.assertEqual(len(tracker.history),2)
        report=tracker.report('test')
        self.assertTrue(report['truncated']);self.assertLess(len(report['processes']),len(records))
        self.assertLessEqual(len(json.dumps(report,ensure_ascii=True).encode()),j.MAX_DIAGNOSTIC_BYTES)

    @unittest.skipUnless(sys.platform=='win32','genuine native Windows Job Object proof')
    def test_native_child_grandchild_and_output_bound(self):
        with temporary_root(prefix='luma job é 测试 ') as root:
            self.assertTrue(j.native_tree_smoke(root)['tree_drained'])
            with self.assertRaises(j.JobRunError) as caught:
                j.run_owned_tree([str(Path(sys.executable).resolve()),'-I','-B','-u','-c',
                                  'print("x"*8192,flush=True); import time; time.sleep(30)'],
                                 cwd=root,timeout=10,output_limit=1024)
            self.assertEqual(caught.exception.result['reason'],'output_limit')
            self.assertTrue(caught.exception.result['tree_drained'])
            self.assertLessEqual(len(caught.exception.result['stdout']),1024)


class TwoPhaseCargoTests(unittest.TestCase):
    TEST='asr_components::tests::native_direct_recipe_installs_repairs_and_removes'

    def fixture(self,root):
        package=root/'src-tauri';(package/'src').mkdir(parents=True)
        manifest=package/'Cargo.toml';manifest.write_text('[package]\nname="luma-subtitle"\n',encoding='utf-8')
        (package/'src/main.rs').write_text('// reviewed fixture\n',encoding='utf-8')
        target=package/'target';directory=target/'x86_64-pc-windows-msvc/debug/deps';directory.mkdir(parents=True)
        executable=directory/'luma_subtitle-0123456789abcdef.exe';executable.write_bytes(b'MZreviewed test binary')
        cargo=root/'cargo.exe';cargo.touch()
        artifact={'reason':'compiler-artifact','manifest_path':str(manifest),
                  'target':{'name':'luma-subtitle','kind':['bin'],'crate_types':['bin'],'test':True,'src_path':str(package/'src/main.rs')},
                  'profile':{'test':True},'executable':str(executable),'filenames':[str(executable)],'fresh':False}
        return cargo,manifest,target,executable,artifact

    def messages(self,artifact):return json.dumps(artifact)+'\n'+json.dumps({'reason':'build-finished','success':True})+'\n'

    def test_cargo_json_accepts_only_one_exact_native_test_artifact(self):
        with temporary_root() as root:
            _,manifest,target,executable,artifact=self.fixture(root)
            self.assertEqual(j.select_test_artifact(self.messages(artifact),manifest,target),executable)
            mutations=[('manifest_path',str(root/'Cargo.toml')),('executable',str(root/'outside.exe'))]
            for key,value in mutations:
                bad=copy.deepcopy(artifact);bad[key]=value
                with self.subTest(key=key),self.assertRaises((ValueError,FileNotFoundError)):
                    j.select_test_artifact(self.messages(bad),manifest,target)
            for key,value in [('name','other'),('kind',['lib']),('test',False),('src_path',str(root/'main.rs'))]:
                bad=copy.deepcopy(artifact);bad['target'][key]=value
                with self.subTest(target=key),self.assertRaises(ValueError):j.select_test_artifact(self.messages(bad),manifest,target)
            for output in ('not JSON\n',json.dumps(artifact)+'\n'+self.messages(artifact),
                           self.messages(artifact)+json.dumps({'reason':'build-finished','success':True}),
                           self.messages(artifact).replace('"success": true','"success": false')):
                with self.assertRaises(ValueError):j.select_test_artifact(output,manifest,target)

    def test_binary_hash_and_file_identity_are_checked(self):
        with temporary_root() as root:
            *_,executable,artifact=self.fixture(root)
            with executable.open('rb') as stream:
                before=j.binary_identity(stream)
                self.assertEqual(before['sha256'],hashlib.sha256(executable.read_bytes()).hexdigest())
                self.assertGreater(before['file_id'],0)
            executable.write_bytes(b'not PE')
            with executable.open('rb') as stream,self.assertRaisesRegex(ValueError,'Windows executable'):j.binary_identity(stream)

    def run_fixture(self,root,*,build_error=None,runtime_error=None,mutate=None,build_override=None,runtime_override=None,lock_error=False):
        cargo,manifest,target,executable,artifact=self.fixture(root);calls=[]
        rustc=root/'rustc.exe';rustc.touch();libdir=root/'rustlib';libdir.mkdir()
        def run(command,**kwargs):
            calls.append((command,kwargs))
            self.assertEqual(kwargs['cwd'],manifest.parent)
            self.assertGreater(kwargs['timeout'],0);self.assertLessEqual(kwargs['timeout'],1800)
            if '--print' in command:
                self.assertEqual(command,[str(rustc),'--print','target-libdir','--target','x86_64-pc-windows-msvc'])
                self.assertFalse(kwargs['_build_only'])
                return {'exit_code':0,'tree_drained':True,'natural_drain_completed':True,
                        'termination_requested':False,'stdout':str(libdir)+'\n','stderr':''}
            if kwargs['_build_only']:
                self.assertIn('--no-run',command);self.assertIn('--message-format=json',command)
                self.assertNotIn(self.TEST,command)
                if build_error:raise j.JobRunError('compile failed',build_error)
                if mutate:mutate(artifact,manifest,executable)
                if build_override is not None:return build_override
                return {'exit_code':0,'tree_drained':True,'termination_requested':True,
                        'natural_drain_completed':False,'build_helper_cleanup_completed':True,
                        'stdout':self.messages(artifact),'stderr':'bounded compiler stderr\n'}
            self.assertEqual(command,[str(executable),self.TEST,'--exact','--ignored','--nocapture'])
            self.assertFalse(kwargs['_build_only'])
            self.assertTrue(kwargs['env']['PATH'].startswith(str(executable.parent)+';'))
            self.assertIn(str(libdir),kwargs['env']['PATH'])
            self.assertEqual(kwargs['env']['CARGO_MANIFEST_DIR'],str(manifest.parent))
            if runtime_error:raise j.JobRunError('runtime failed',runtime_error)
            if runtime_override is not None:return runtime_override
            return {'exit_code':0,'tree_drained':True,'termination_requested':False,'natural_drain_completed':True,
                    'stdout':'running 1 test\ntest result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 4 filtered out; finished in 1s\n','stderr':''}
        with patch.object(j.sys,'platform','win32'),patch.object(j,'run_owned_tree',side_effect=run), \
             patch.object(j,'locked_test_binary',side_effect=PermissionError('expected lock failure') if lock_error else lambda path:path.open('rb')):
            try:
                result=j.run_owned_cargo_test(str(cargo),manifest=manifest,test_name=self.TEST,
                                             env={'LUMA_ASR_TEST_VERIFIER_SOURCE_SHA':'a'*40,'RUSTC':str(rustc)})
                error=None
            except j.JobRunError as caught:result=caught.result;error=caught
        return calls,result,error

    def test_compile_cleanup_precedes_fresh_strict_runtime_job(self):
        with temporary_root() as root:
            calls,result,error=self.run_fixture(root)
            self.assertIsNone(error);self.assertEqual(len(calls),3);self.assertTrue(result['passed'])
            self.assertTrue(result['build']['build_helper_cleanup_completed'])
            self.assertTrue(result['build']['termination_requested']);self.assertFalse(result['build']['natural_drain_completed'])
            self.assertTrue(result['test']['natural_drain_completed']);self.assertFalse(result['test']['termination_requested'])
            self.assertEqual(result['source_sha'],'a'*40);self.assertTrue(result['artifact']['deny_write_delete_held'])
            self.assertNotIn('compiler-artifact',result['stdout']);self.assertNotIn('stdout',result['build'])

    def test_failed_or_undrained_build_never_starts_runtime(self):
        for drained in (True,False):
            with self.subTest(drained=drained),temporary_root() as root:
                calls,result,error=self.run_fixture(root,build_error={'exit_code':101,'tree_drained':drained,'stdout':'','stderr':'failure'})
                self.assertIsInstance(error,j.JobRunError);self.assertEqual(len(calls),2)
                self.assertFalse(result['test_started']);self.assertEqual(result['tree_drained'],drained)

    def test_runtime_orphans_remain_failure_after_successful_build_cleanup(self):
        with temporary_root() as root:
            calls,result,error=self.run_fixture(root,runtime_error={'exit_code':0,'tree_drained':True,
                    'natural_drain_completed':False,'termination_requested':True,'stdout':'partial','stderr':''})
            self.assertIsInstance(error,j.JobRunError);self.assertEqual(len(calls),3);self.assertFalse(result['passed'])
            self.assertTrue(result['test_started']);self.assertTrue(result['tree_drained'])
            self.assertEqual(result['stage'],'test');self.assertEqual(result['stdout'],'partial')

    def test_changed_source_or_mismatched_artifact_never_starts_runtime(self):
        mutations=[lambda artifact,manifest,exe:artifact['target'].update(name='another'),
                   lambda artifact,manifest,exe:manifest.write_text('changed',encoding='utf-8')]
        for mutate in mutations:
            with temporary_root() as root:
                calls,result,error=self.run_fixture(root,mutate=mutate)
                self.assertIsInstance(error,j.JobRunError);self.assertEqual(len(calls),2)
                self.assertTrue(result['tree_drained']);self.assertFalse(result['test_started'])

    def test_raw_build_script_environment_is_never_rendered(self):
        output=json.dumps({'reason':'build-script-executed','env':[['TOKEN','not-for-report']]})+'\n'
        output+=json.dumps({'reason':'compiler-message','message':{'rendered':'reviewed compiler warning\n'}})
        self.assertEqual(j.compiler_diagnostics(output,4096),'reviewed compiler warning\n')

    def test_missing_or_false_build_drain_and_missing_helper_cleanup_block_runtime(self):
        for override in ({'exit_code':0},{'exit_code':0,'tree_drained':False},
                         {'exit_code':0,'tree_drained':True,'build_helper_cleanup_completed':False}):
            with temporary_root() as root:
                calls,result,error=self.run_fixture(root,build_override=override)
                self.assertIsInstance(error,j.JobRunError);self.assertEqual(len(calls),2)
                self.assertFalse(result['test_started']);self.assertFalse(result['passed'])

    def test_runtime_exit_zero_cannot_hide_forced_drain_or_zero_matching_tests(self):
        baseline={'exit_code':0,'tree_drained':True,'natural_drain_completed':True,
                  'termination_requested':False,'stdout':'test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured;\n','stderr':''}
        for field,value in (('natural_drain_completed',False),('termination_requested',True),
                            ('stdout','test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured;\n')):
            with temporary_root() as root:
                override={**baseline,field:value};calls,result,error=self.run_fixture(root,runtime_override=override)
                self.assertIsInstance(error,j.JobRunError);self.assertEqual(len(calls),3);self.assertFalse(result['passed'])

    def test_artifact_lock_and_identity_failure_prevent_launch(self):
        with temporary_root() as root:
            calls,result,error=self.run_fixture(root,lock_error=True)
            self.assertIsInstance(error,j.JobRunError);self.assertEqual(len(calls),2);self.assertFalse(result['test_started'])
        original=j.binary_identity;count=0
        def changed(stream):
            nonlocal count
            count+=1;result=original(stream)
            if count==2:result['sha256']='0'*64
            return result
        with temporary_root() as root,patch.object(j,'binary_identity',side_effect=changed):
            calls,result,error=self.run_fixture(root)
            self.assertIsInstance(error,j.JobRunError);self.assertEqual(len(calls),2);self.assertFalse(result['test_started'])

    def test_cargo_dll_paths_use_only_canonical_target_searches_and_compiler_libdir(self):
        with temporary_root() as root:
            _,manifest,target,executable,artifact=self.fixture(root)
            native=target/'native';native.mkdir();libdir=root/'rustlib';libdir.mkdir()
            output=self.messages(artifact)+json.dumps({'reason':'build-script-executed',
                    'linked_paths':['native='+str(native),str(root/'outside')],'env':[['TOKEN','do-not-export']]})
            environment,search=j.cargo_runtime_environment(output,target,libdir,{'Path':'original-system-path','KEEP':'yes'})
            self.assertEqual(search,[str(native),str(executable.parent),str(executable.parent.parent),str(libdir)])
            self.assertNotIn('Path',environment);self.assertNotIn('TOKEN',environment)
            self.assertEqual(environment['KEEP'],'yes');self.assertTrue(environment['PATH'].endswith(';original-system-path'))

    @unittest.skipUnless(sys.platform=='win32','native Windows deny-write/delete sharing')
    def test_native_binary_lock_denies_replacement_and_write(self):
        with temporary_root() as root:
            executable=root/'fixture.exe';executable.write_bytes(b'MZfixture')
            with j.locked_test_binary(executable) as stream:
                self.assertEqual(j.binary_identity(stream)['bytes'],9)
                with self.assertRaises(OSError):executable.write_bytes(b'MZchanged')
                with self.assertRaises(OSError):executable.unlink()


if __name__=='__main__':unittest.main()
