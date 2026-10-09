"""CI-only Windows process-tree ownership for bounded Rust lifecycle proofs.

No shipping runtime imports this module. The child starts suspended, joins a
kill-on-close Job before any instruction runs, and inherits only three explicit
stdio handles. Every exit path drains the Job before callers may remove files.
"""
import ctypes as c
import json
import os
from pathlib import Path, PureWindowsPath
import subprocess
import sys
import tempfile
import time

U32=c.c_uint32; U16=c.c_uint16; I64=c.c_int64; SIZE=c.c_size_t; HANDLE=c.c_void_p
MAX_JOB_PIDS=4096
MAX_DIAGNOSTIC_RECORDS=4096
MAX_DIAGNOSTIC_BYTES=64*1024
MAX_IMAGE_CHARS=1024


class JobPids(c.Structure):
    _fields_=[('assigned',U32),('count',U32),('pids',SIZE*MAX_JOB_PIDS)]


class ProcessEntry(c.Structure):
    # WCHAR is always 16-bit, including when layout tests run on Linux.
    _fields_=[('size',U32),('usage',U32),('pid',U32),('heap',SIZE),('module',U32),
              ('threads',U32),('parent_pid',U32),('priority',c.c_int32),('flags',U32),
              ('name',U16*260)]


def command_category(executable):
    """A label only, never an allowlist or an assertion of trusted origin."""
    return {'cargo.exe':'cargo','rustc.exe':'rust-compiler','rustup.exe':'rustup',
            'cl.exe':'msvc-compiler-name','link.exe':'msvc-linker-name',
            'vctip.exe':'msvc-telemetry-name','mspdbsrv.exe':'msvc-pdb-server-name',
            'python.exe':'python-interpreter','pythonw.exe':'python-interpreter',
            'git.exe':'git','cmd.exe':'command-shell','powershell.exe':'powershell',
            'pwsh.exe':'powershell'}.get(PureWindowsPath(executable).name.lower(),'other-executable')


def owned_process_snapshot(kernel,job):
    """Read only owned PIDs and fixed metadata; never read argv or environment.

    The OS PID list includes nested Jobs. Toolhelp is used only for the parent
    IDs of that list; names/details of unrelated host processes are discarded.
    Snapshots are diagnostic and cannot authorize a process to outlive its Job.
    """
    errors=[];pids=JobPids()
    ok=kernel.QueryInformationJobObject(job,3,c.byref(pids),c.sizeof(pids),None)
    code=0 if ok else c.get_last_error()
    if not ok and code!=234: # ERROR_MORE_DATA permits a bounded partial list.
        return {'processes':[],'truncated':False,'errors':[{'api':'job-pids','code':code}]}
    wanted=set(int(pid) for pid in pids.pids[:min(pids.count,MAX_JOB_PIDS)] if pid)
    truncated=not ok or pids.assigned>MAX_JOB_PIDS or pids.count>MAX_JOB_PIDS
    if not wanted:return {'processes':[],'truncated':truncated,'errors':errors}
    parents={};snapshot_time=time.time_ns()//100+116444736000000000
    snapshot=kernel.CreateToolhelp32Snapshot(2,0)
    if snapshot in (None,0,HANDLE(-1).value):
        errors.append({'api':'process-snapshot','code':c.get_last_error()})
    else:
        try:
            entry=ProcessEntry();entry.size=c.sizeof(entry)
            available=kernel.Process32FirstW(snapshot,c.byref(entry));count=0
            while available and count<16384:
                if entry.pid in wanted:parents[int(entry.pid)]=int(entry.parent_pid)
                count+=1;available=kernel.Process32NextW(snapshot,c.byref(entry))
            if available:truncated=True
            elif c.get_last_error()!=18: # ERROR_NO_MORE_FILES
                errors.append({'api':'process-enumeration','code':c.get_last_error()})
        finally:kernel.CloseHandle(snapshot)
    records=[]
    for pid in sorted(wanted):
        record={'pid':pid,'parent_pid':parents.get(pid)};records.append(record)
        process=kernel.OpenProcess(0x1000,False,pid) # QUERY_LIMITED_INFORMATION
        if not process:
            record['query_error']={'api':'open-process','code':c.get_last_error()};continue
        try:
            owned=c.c_int()
            if not kernel.IsProcessInJob(process,job,c.byref(owned)):
                record['query_error']={'api':'is-process-in-job','code':c.get_last_error()};continue
            if not owned.value:
                record['parent_pid']=None;record['identity_status']='pid-no-longer-owned';continue
            record['job_membership_verified']=True
            created=c.c_uint64();exited=c.c_uint64();system=c.c_uint64();user=c.c_uint64()
            if kernel.GetProcessTimes(process,c.byref(created),c.byref(exited),c.byref(system),c.byref(user)):
                record['created_filetime']=created.value
                record['observed_filetime']=time.time_ns()//100+116444736000000000
                record['exited_filetime']=exited.value or None
                if created.value>snapshot_time:
                    record['parent_pid']=None;record['parent_status']='created-after-parent-snapshot'
            else:record['times_error']={'api':'process-times','code':c.get_last_error()}
            buffer=c.create_unicode_buffer(MAX_IMAGE_CHARS);size=U32(MAX_IMAGE_CHARS)
            if not kernel.QueryFullProcessImageNameW(process,0,buffer,c.byref(size)):
                record['image_error']={'api':'process-image','code':c.get_last_error()};continue
            try:
                executable=str(Path(buffer.value).resolve(strict=True))
                if len(executable)>MAX_IMAGE_CHARS:raise ValueError('image bound')
                record['canonical_executable']=executable
                record['command_category']=command_category(executable)
            except (OSError,ValueError,RuntimeError):
                record['image_error']={'api':'canonical-image','code':None}
        finally:kernel.CloseHandle(process)
    return {'processes':records,'truncated':truncated,'errors':errors}


class JobDiagnostics:
    """Bounded sampled ancestry, with creation-time guards against PID reuse."""
    def __init__(self,kernel,job,root_pid):
        self.kernel=kernel;self.job=job;self.root_pid=root_pid
        self.history={};self.latest=None;self.errors=[];self.truncated=False

    def sample(self):
        try:current=owned_process_snapshot(self.kernel,self.job)
        except Exception as error:
            # Do not copy arbitrary exception text, which could contain data.
            if len(self.errors)<8:self.errors.append({'api':'diagnostic-snapshot','error_type':type(error).__name__})
            return
        self.latest=current;self.truncated|=current['truncated']
        for record in current['processes']:
            key=(record['pid'],record.get('created_filetime'))
            if key not in self.history and len(self.history)>=MAX_DIAGNOSTIC_RECORDS:
                self.truncated=True;continue
            self.history[key]=record.copy()
        self.errors=(self.errors+current['errors'])[:8]

    def report(self,phase):
        report={'schema':1,'phase':phase,'root_pid':self.root_pid,
                'scope':'owned Job processes only; sampled ancestry may have gaps',
                'raw_command_lines_collected':False,'environment_collected':False,
                'category_is_trust_decision':False,'truncated':self.truncated,
                'errors':list(self.errors),'processes':[]}
        for process in (self.latest or {}).get('processes',[]):
            item=process.copy();item['parent_chain']=[];node=process;seen={node['pid']}
            while node['pid']!=self.root_pid and len(item['parent_chain'])<16:
                parent_pid=node.get('parent_pid');created=node.get('created_filetime')
                if not parent_pid or not created or parent_pid in seen:break
                # A parent must have been observed alive after this child was
                # created. Earlier generations of a reused PID are not evidence.
                candidates=[p for p in self.history.values() if p['pid']==parent_pid
                            and p.get('job_membership_verified') and p.get('created_filetime',created)>=1
                            and p.get('created_filetime',created)<created
                            and p.get('observed_filetime',0)>=created
                            and (p.get('exited_filetime') is None or p['exited_filetime']>=created)]
                if len(candidates)!=1:break
                node=candidates[0];seen.add(node['pid']);item['parent_chain'].append(node.copy())
            item['parent_chain_reaches_root']=node['pid']==self.root_pid
            if not item['parent_chain_reaches_root']:item['unresolved_parent_pid']=node.get('parent_pid')
            report['processes'].append(item)
            if len(json.dumps(report,ensure_ascii=True).encode('utf-8'))>MAX_DIAGNOSTIC_BYTES-1024:
                report['processes'].pop();report['truncated']=True;break
        return report


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
        'CreateToolhelp32Snapshot':([U32,U32],HANDLE),
        'Process32FirstW':([HANDLE,c.POINTER(ProcessEntry)],c.c_int),
        'Process32NextW':([HANDLE,c.POINTER(ProcessEntry)],c.c_int),
        'OpenProcess':([U32,c.c_int,U32],HANDLE),
        'IsProcessInJob':([HANDLE,HANDLE,c.POINTER(c.c_int)],c.c_int),
        'GetProcessTimes':([HANDLE,HANDLE,HANDLE,HANDLE,HANDLE],c.c_int),
        'QueryFullProcessImageNameW':([HANDLE,U32,c.c_wchar_p,c.POINTER(U32)],c.c_int),
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
    job=None;info=ProcessInfo();assigned=False;attributes=None;attribute_buffer=None;attributes_ready=False;diagnostics=None
    result={'schema':1,'exit_code':None,'job_assigned_before_resume':False,'tree_drained':False,
            'natural_drain_completed':False,'termination_requested':False,
            'total_processes':0,'timeout_seconds':timeout,'output_limit_bytes':output_limit}
    reason=None;failure=None
    def require(ok):
        if not ok: raise c.WinError(c.get_last_error())
    def accounting():
        value=Accounting();require(kernel.QueryInformationJobObject(job,1,c.byref(value),c.sizeof(value),None));return value
    def observe(phase=None):
        # Neither collecting nor serializing optional evidence may interrupt
        # termination/reaping. API failures never change the ownership policy.
        try:
            diagnostics.sample()
            if phase:return diagnostics.report(phase)
        except Exception as error:
            if phase:return {'schema':1,'phase':phase,'root_pid':info.pid,'processes':[],
                             'truncated':True,'errors':[{'api':'diagnostic-report','error_type':type(error).__name__}]}
    def drain(terminate):
        # A failed assignment leaves a suspended child outside our Job. It must
        # be explicitly terminated/reaped; it was never allowed to run.
        if info.process and not assigned:
            require(kernel.TerminateProcess(info.process,1))
        if assigned and terminate:
            if diagnostics:
                result['before_forced_termination']=observe('before-forced-termination')
            result['termination_requested']=True
            require(kernel.TerminateJobObject(job,1))
        if info.process:
            state=kernel.WaitForSingleObject(info.process,10_000)
            if state!=0: raise RuntimeError('Owned root process did not terminate')
        deadline=time.monotonic()+10
        while job:
            state=accounting();result['total_processes']=state.total_processes
            if state.active_processes==0:
                result['tree_drained']=True
                if assigned and not terminate:result['natural_drain_completed']=True
                return
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
            result['root_pid']=info.pid
            diagnostics=JobDiagnostics(kernel,job,info.pid);observe()
            if kernel.ResumeThread(info.thread)==0xffffffff: raise c.WinError(c.get_last_error())
            kernel.CloseHandle(info.thread);info.thread=None
            started=time.monotonic();next_sample=started
            while True:
                state=kernel.WaitForSingleObject(info.process,50)
                if state not in (0,258): raise RuntimeError('Owned process wait failed')
                if os.fstat(stdout.fileno()).st_size+os.fstat(stderr.fileno()).st_size>output_limit:
                    reason='output_limit';raise RuntimeError('Owned proof exceeded its output bound')
                if state==0: break
                now=time.monotonic()
                if now>=next_sample:
                    observe();next_sample=now+.25
                if now-started>=timeout:
                    reason='timeout';raise TimeoutError('Owned proof exceeded its time bound')
            code=U32();require(kernel.GetExitCodeProcess(info.process,c.byref(code)));result['exit_code']=code.value
            # A root process exiting is not proof its descendants have exited.
            result['after_root_exit']=observe('after-root-exit')
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
