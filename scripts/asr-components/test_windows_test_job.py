import ctypes
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
        self.failure=failure; self.calls=[]; self.terminated=False
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
        assert flags&4 and flags&0x80000 and inherit
        assert startup._obj.base.cb==ctypes.sizeof(j.StartupEx)
        if self.failure=='launch':return 0
        info._obj.process=20;info._obj.thread=21;return 1
    def AssignProcessToJobObject(self,*args):self.calls.append('assign');return self.failure!='assign'
    def ResumeThread(self,*args):self.calls.append('resume');return 1
    def WaitForSingleObject(self,*args):
        if self.failure=='cancel' and not self.terminated:raise KeyboardInterrupt()
        return 258 if self.failure=='timeout' and not self.terminated else 0
    def GetExitCodeProcess(self,process,code):code._obj.value=0;return 1
    def TerminateProcess(self,*args):self.calls.append('terminate-process');self.terminated=True;return 1
    def TerminateJobObject(self,*args):self.calls.append('terminate-job');self.terminated=True;return 1
    def QueryInformationJobObject(self,job,kind,value,size,returned):
        self.calls.append('accounting');value._obj.total_processes=3
        value._obj.active_processes=int(self.failure=='orphan' and not self.terminated);return 1
    def CloseHandle(self,handle):self.calls.append(('close',handle));return 1


class WindowsJobTests(unittest.TestCase):
    def test_x64_win32_structure_layouts(self):
        self.assertEqual(ctypes.sizeof(ctypes.c_void_p),8)
        for structure,size in [(j.BasicLimits,64),(j.IoCounters,48),(j.ExtendedLimits,144),
                               (j.Accounting,48),(j.Startup,104),(j.StartupEx,112),(j.ProcessInfo,24)]:
            self.assertEqual(ctypes.sizeof(structure),size)
        self.assertEqual(j.Startup.stdin.offset,80)
        self.assertEqual(j.Accounting.active_processes.offset,40)

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
                 patch.object(j.c,'get_last_error',return_value=1,create=True), \
                 patch.object(j.time,'monotonic',side_effect=iter(range(100))):
                try: result=j.run_owned_tree([str(exe)],cwd=root,timeout=.001)
                except j.JobRunError as error:result=error.result
        return kernel,result

    def test_assignment_precedes_execution_and_all_handles_close(self):
        kernel,result=self.fake_run()
        self.assertTrue(result['tree_drained']);self.assertTrue(result['job_assigned_before_resume'])
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
