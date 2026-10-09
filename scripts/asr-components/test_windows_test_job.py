import ctypes
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
        info._obj.process=20;info._obj.thread=21;info._obj.pid=20;return 1
    def AssignProcessToJobObject(self,*args):self.calls.append('assign');return self.failure!='assign'
    def ResumeThread(self,*args):self.calls.append('resume');return 1
    def WaitForSingleObject(self,*args):
        if self.failure=='cancel' and not self.terminated:raise KeyboardInterrupt()
        if self.failure=='timeout' and not self.terminated:return 258
        self.root_exited=True;return 0
    def GetExitCodeProcess(self,process,code):code._obj.value=0;return 1
    def TerminateProcess(self,*args):self.calls.append('terminate-process');self.terminated=True;return 1
    def TerminateJobObject(self,*args):self.calls.append('terminate-job');self.terminated=True;return 1
    def QueryInformationJobObject(self,job,kind,value,size,returned):
        if kind==3:
            self.calls.append('job-pids')
            pids=([20] if not self.root_exited else [22] if self.failure=='orphan' and not self.terminated else [])
            value._obj.assigned=len(pids);value._obj.count=len(pids)
            for index,pid in enumerate(pids):value._obj.pids[index]=pid
            return 1
        self.calls.append('accounting');value._obj.total_processes=3
        value._obj.active_processes=int(self.failure=='orphan' and not self.terminated);return 1
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

    def fake_run(self,failure=None):
        kernel=FakeKernel(failure)
        with temporary_root() as root:
            exe=root/'fixture.exe';exe.touch()
            with patch.object(j.sys,'platform','win32'),patch.object(j,'kernel_api',return_value=kernel), \
                 patch.dict(sys.modules,{'msvcrt':types.SimpleNamespace(get_osfhandle=lambda fd:fd)}), \
                 patch.object(j.os,'set_handle_inheritable',create=True), \
                 patch.object(j.c,'WinError',side_effect=lambda code:OSError('expected API failure'),create=True), \
                 patch.object(j.c,'get_last_error',return_value=18,create=True), \
                 patch.object(j.time,'monotonic',side_effect=iter(range(100))):
                try: result=j.run_owned_tree([str(exe)],cwd=root,timeout=.001)
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


if __name__=='__main__':unittest.main()
