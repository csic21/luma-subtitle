"""CI-only Windows process-tree ownership for bounded Rust lifecycle proofs.

No shipping runtime imports this module. The child starts suspended, joins a
kill-on-close Job before any instruction runs, and inherits only three explicit
stdio handles. Every exit path drains the Job before callers may remove files.
"""
import ctypes as c
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import time

U32=c.c_uint32; U16=c.c_uint16; I64=c.c_int64; SIZE=c.c_size_t; HANDLE=c.c_void_p


class BasicLimits(c.Structure):
    _fields_=[('process_time',I64),('job_time',I64),('flags',U32),('min_ws',SIZE),('max_ws',SIZE),
              ('active_limit',U32),('affinity',SIZE),('priority',U32),('scheduling',U32)]


class IoCounters(c.Structure):
    _fields_=[(name,c.c_uint64) for name in ('read_ops','write_ops','other_ops','read_bytes','write_bytes','other_bytes')]


class ExtendedLimits(c.Structure):
    _fields_=[('basic',BasicLimits),('io',IoCounters),('process_memory',SIZE),('job_memory',SIZE),
              ('peak_process_memory',SIZE),('peak_job_memory',SIZE)]


class Accounting(c.Structure):
    _fields_=[('user_time',I64),('kernel_time',I64),('period_user',I64),('period_kernel',I64),
              ('page_faults',U32),('total_processes',U32),('active_processes',U32),('terminated_processes',U32)]


class Startup(c.Structure):
    _fields_=[('cb',U32),('reserved',c.c_wchar_p),('desktop',c.c_wchar_p),('title',c.c_wchar_p),
              ('x',U32),('y',U32),('x_size',U32),('y_size',U32),('x_chars',U32),('y_chars',U32),
              ('fill',U32),('flags',U32),('show',U16),('reserved_size',U16),('reserved_bytes',HANDLE),
              ('stdin',HANDLE),('stdout',HANDLE),('stderr',HANDLE)]


class StartupEx(c.Structure):
    _fields_=[('base',Startup),('attributes',HANDLE)]


class ProcessInfo(c.Structure):
    _fields_=[('process',HANDLE),('thread',HANDLE),('pid',U32),('tid',U32)]


def command_line(command):
    if not command or any(not isinstance(arg,str) or '\0' in arg for arg in command):
        raise ValueError('Expected explicit NUL-free executable/arguments')
    executable=Path(command[0])
    if not executable.is_absolute() or not executable.is_file() or executable.suffix.lower()!='.exe':
        raise ValueError('Windows proof requires an absolute existing executable, never a shell command')
    line=subprocess.list2cmdline(command)
    if len(line.encode('utf-16-le'))//2>=32767: raise ValueError('Windows command line exceeds the OS bound')
    return line


def environment_block(env):
    if env is None: return None
    if any(not isinstance(key,str) or not key or '=' in key or '\0' in key
           or not isinstance(value,str) or '\0' in value for key,value in env.items()):
        raise ValueError('Invalid private child environment')
    if len({key.upper() for key in env})!=len(env): raise ValueError('Ambiguous Windows environment keys')
    return c.create_unicode_buffer('\0'.join(key+'='+env[key] for key in sorted(env,key=str.upper))+'\0\0')


class JobRunError(RuntimeError):
    def __init__(self,message,result):
        super().__init__(message);self.result=result


def kernel_api():
    kernel=c.WinDLL('kernel32',use_last_error=True)
    definitions={
        'CreateJobObjectW':([HANDLE,c.c_wchar_p],HANDLE),
        'SetInformationJobObject':([HANDLE,c.c_int,HANDLE,U32],c.c_int),
        'QueryInformationJobObject':([HANDLE,c.c_int,HANDLE,U32,HANDLE],c.c_int),
        'AssignProcessToJobObject':([HANDLE,HANDLE],c.c_int),
        'TerminateJobObject':([HANDLE,U32],c.c_int),
        'CreateProcessW':([c.c_wchar_p,c.c_wchar_p,HANDLE,HANDLE,c.c_int,U32,HANDLE,c.c_wchar_p,c.POINTER(StartupEx),c.POINTER(ProcessInfo)],c.c_int),
        'InitializeProcThreadAttributeList':([HANDLE,U32,U32,c.POINTER(SIZE)],c.c_int),
        'UpdateProcThreadAttribute':([HANDLE,U32,SIZE,HANDLE,SIZE,HANDLE,HANDLE],c.c_int),
        'DeleteProcThreadAttributeList':([HANDLE],None),
        'ResumeThread':([HANDLE],U32),
        'TerminateProcess':([HANDLE,U32],c.c_int),
        'WaitForSingleObject':([HANDLE,U32],U32),
        'GetExitCodeProcess':([HANDLE,c.POINTER(U32)],c.c_int),
        'CloseHandle':([HANDLE],c.c_int),
    }
    for name,(args,result) in definitions.items():
        function=getattr(kernel,name);function.argtypes=args;function.restype=result
    return kernel


def run_owned_tree(command,*,cwd,env=None,timeout=1800,output_limit=8*1024*1024):
    if sys.platform!='win32': raise RuntimeError('The native proof Job supervisor is Windows-only')
    if not 0<timeout<=1800 or not 0<output_limit<=8*1024*1024: raise ValueError('Unbounded proof process request')
    command=[str(arg) for arg in command];line=command_line(command)
    cwd=Path(cwd).resolve(strict=True)
    if not cwd.is_dir(): raise ValueError('Proof cwd must be an existing private directory')
    block=environment_block(env);kernel=kernel_api()
    job=None;info=ProcessInfo();assigned=False;attributes=None;attribute_buffer=None;attributes_ready=False
    result={'schema':1,'exit_code':None,'job_assigned_before_resume':False,'tree_drained':False,
            'total_processes':0,'timeout_seconds':timeout,'output_limit_bytes':output_limit}
    reason=None;failure=None
    def require(ok):
        if not ok: raise c.WinError(c.get_last_error())
    def accounting():
        value=Accounting();require(kernel.QueryInformationJobObject(job,1,c.byref(value),c.sizeof(value),None));return value
    def drain(terminate):
        # A failed assignment leaves a suspended child outside our Job. It must
        # be explicitly terminated/reaped; it was never allowed to run.
        if info.process and not assigned:
            require(kernel.TerminateProcess(info.process,1))
        if assigned and terminate: require(kernel.TerminateJobObject(job,1))
        if info.process:
            state=kernel.WaitForSingleObject(info.process,10_000)
            if state!=0: raise RuntimeError('Owned root process did not terminate')
        deadline=time.monotonic()+10
        while job:
            state=accounting();result['total_processes']=state.total_processes
            if state.active_processes==0:
                result['tree_drained']=True;return
            if time.monotonic()>=deadline: raise RuntimeError('Owned Windows process tree did not drain')
            time.sleep(.01)
        result['tree_drained']=True
    with tempfile.TemporaryFile(dir=cwd) as stdout,tempfile.TemporaryFile(dir=cwd) as stderr,open(os.devnull,'rb') as stdin:
        try:
            import msvcrt
            job=kernel.CreateJobObjectW(None,None);require(job)
            limits=ExtendedLimits();limits.basic.flags=0x2000 # JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE
            require(kernel.SetInformationJobObject(job,9,c.byref(limits),c.sizeof(limits)))
            handles=[msvcrt.get_osfhandle(stream.fileno()) for stream in (stdin,stdout,stderr)]
            size=SIZE();kernel.InitializeProcThreadAttributeList(None,1,0,c.byref(size))
            if not 0<size.value<=65536: raise RuntimeError('Unexpected process attribute-list size')
            attribute_buffer=c.create_string_buffer(size.value);attributes=c.cast(attribute_buffer,HANDLE)
            require(kernel.InitializeProcThreadAttributeList(attributes,1,0,c.byref(size)))
            attributes_ready=True
            inherited=(HANDLE*3)(*handles)
            startup=StartupEx();startup.base.cb=c.sizeof(startup);startup.base.flags=0x100
            startup.base.stdin,startup.base.stdout,startup.base.stderr=handles;startup.attributes=attributes
            flags=0x4|0x400|0x80000|0x08000000 # suspended, Unicode env, explicit handle list, no window
            try:
                for handle in handles: os.set_handle_inheritable(handle,True)
                require(kernel.UpdateProcThreadAttribute(attributes,0,0x20002,c.byref(inherited),c.sizeof(inherited),None,None))
                require(kernel.CreateProcessW(command[0],c.create_unicode_buffer(line),None,None,True,flags,
                                              c.cast(block,HANDLE) if block is not None else None,str(cwd),c.byref(startup),c.byref(info)))
            finally:
                for handle in handles: os.set_handle_inheritable(handle,False)
            require(kernel.AssignProcessToJobObject(job,info.process));assigned=True
            result['job_assigned_before_resume']=True
            if kernel.ResumeThread(info.thread)==0xffffffff: raise c.WinError(c.get_last_error())
            kernel.CloseHandle(info.thread);info.thread=None
            started=time.monotonic()
            while True:
                state=kernel.WaitForSingleObject(info.process,50)
                if state not in (0,258): raise RuntimeError('Owned process wait failed')
                if os.fstat(stdout.fileno()).st_size+os.fstat(stderr.fileno()).st_size>output_limit:
                    reason='output_limit';raise RuntimeError('Owned proof exceeded its output bound')
                if state==0: break
                if time.monotonic()-started>=timeout:
                    reason='timeout';raise TimeoutError('Owned proof exceeded its time bound')
            code=U32();require(kernel.GetExitCodeProcess(info.process,c.byref(code)));result['exit_code']=code.value
            # A root process exiting is not proof its descendants have exited.
            drain(False)
        except BaseException as error:
            failure=error;result['reason']=reason or type(error).__name__
            try: drain(True)
            except BaseException as cleanup:
                result['cleanup_error']=str(cleanup);result['tree_drained']=False
        finally:
            if attributes_ready: kernel.DeleteProcThreadAttributeList(attributes)
            for handle in (info.thread,info.process,job):
                if handle: kernel.CloseHandle(handle)
            stdout.seek(0);stderr.seek(0)
            out=stdout.read(output_limit);err=stderr.read(max(0,output_limit-len(out)))
            result.update(stdout=out.decode('utf-8',errors='replace'),stderr=err.decode('utf-8',errors='replace'))
    if failure is not None: raise JobRunError(str(failure),result) from failure
    return result


def native_tree_smoke(directory):
    """Genuine tiny child/grandchild proof, run before invoking Cargo."""
    directory=Path(directory).resolve(strict=True)
    grandchild='import time; print("grandchild-ready",flush=True); time.sleep(60)'
    child=f'import subprocess,sys,time; subprocess.Popen([sys.executable,"-I","-B","-c",{grandchild!r}]); time.sleep(60)'
    parent=f'import subprocess,sys,time; subprocess.Popen([sys.executable,"-I","-B","-c",{child!r}]); time.sleep(60)'
    try:
        run_owned_tree([str(Path(sys.executable).resolve()),'-I','-B','-u','-c',parent],cwd=directory,timeout=10,output_limit=4096)
        raise AssertionError('Tree fixture unexpectedly completed')
    except JobRunError as error:
        report=error.result
        if 'grandchild-ready' not in report['stdout'] and report.get('tree_drained'):
            raise JobRunError('Native tree smoke startup was inconclusive; the owned tree was drained',report) from error
        if (report.get('reason')!='timeout' or not report['tree_drained']
                or not report['job_assigned_before_resume'] or report['total_processes']<3
                or 'grandchild-ready' not in report['stdout']):
            raise
        return {key:report[key] for key in ('schema','reason','tree_drained','job_assigned_before_resume','total_processes')}
