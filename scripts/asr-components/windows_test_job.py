"""CI-only Windows process-tree ownership for bounded Rust lifecycle proofs.

No shipping runtime imports this module. The child starts suspended, joins a
kill-on-close Job before any instruction runs, and inherits only three explicit
stdio handles. Every exit path drains the Job before callers may remove files.
"""
import ctypes as c
from contextlib import contextmanager
import hashlib
import json
import os
from pathlib import Path, PureWindowsPath
import re
import shutil
import stat
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


def run_owned_tree(command,*,cwd,env=None,timeout=1800,output_limit=8*1024*1024,_build_only=False):
    if sys.platform!='win32': raise RuntimeError('The native proof Job supervisor is Windows-only')
    if not 0<timeout<=1800 or not 0<output_limit<=8*1024*1024: raise ValueError('Unbounded proof process request')
    command=[str(arg) for arg in command];line=command_line(command)
    if _build_only and (Path(command[0]).name.lower()!='cargo.exe' or command[1:2]!=['test']
                        or '--no-run' not in command or '--message-format=json' not in command or '--' in command):
        raise ValueError('Build-helper cleanup is restricted to compile-only Cargo invocations')
    cwd=Path(cwd).resolve(strict=True)
    if not cwd.is_dir(): raise ValueError('Proof cwd must be an existing private directory')
    block=environment_block(env);kernel=kernel_api()
    job=None;info=ProcessInfo();assigned=False;attributes=None;attribute_buffer=None;attributes_ready=False;diagnostics=None
    result={'schema':1,'exit_code':None,'job_assigned_before_resume':False,'tree_drained':False,
            'natural_drain_completed':False,'termination_requested':False,
            'drain_policy':'terminate-build-helpers' if _build_only else 'natural',
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
            if _build_only and code.value==0:
                # Compile-only Cargo has finished. Every remaining process is
                # owned build work, not a runtime test descendant. Kill/reap
                # the entire Job without any process-name exception.
                drain(True);result['build_helper_cleanup_completed']=True
            else:drain(False)
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
            try:text=out.decode('utf-8',errors='strict' if _build_only else 'replace')
            except UnicodeDecodeError as error:
                failure=failure or error;result['reason']='invalid-cargo-utf8';text=''
            result.update(stdout=text,stderr=err.decode('utf-8',errors='replace'))
    if failure is not None: raise JobRunError(str(failure),result) from failure
    return result


def canonical_proof_path(path,owner,*,file=False):
    path=Path(path);owner=Path(owner)
    if not path.is_absolute() or not path.is_relative_to(owner) or path.resolve(strict=True)!=path:
        raise ValueError('Cargo proof path is not canonical and owned')
    for part in (path,*path.parents):
        info=part.lstat()
        if stat.S_ISLNK(info.st_mode) or getattr(info,'st_file_attributes',0)&0x400:
            raise ValueError('Cargo proof path contains a link or reparse point')
        if part==owner:break
    if not (path.is_file() if file else path.is_dir()):raise ValueError('Cargo proof path has the wrong type')
    return path


def select_test_artifact(output,manifest,target_dir):
    """Accept one Cargo-reported binary unit-test artifact, never a file glob."""
    if len(output.encode('utf-8'))>8*1024*1024:raise ValueError('Cargo JSON exceeds its bound')
    artifacts=[];finished=[]
    for line in output.splitlines():
        if not line.strip():continue
        message=json.loads(line)
        if not isinstance(message,dict):raise ValueError('Cargo emitted a non-object JSON message')
        if message.get('reason')=='build-finished':finished.append(message.get('success'))
        if (message.get('reason')=='compiler-artifact' and message.get('profile',{}).get('test') is True
                and message.get('executable') is not None):artifacts.append(message)
    if finished!=[True] or len(artifacts)!=1:raise ValueError('Cargo must report one successful build and one test executable')
    artifact=artifacts[0];target=artifact.get('target',{})
    manifest=Path(manifest);source=manifest.parent/'src/main.rs'
    if (target.get('name')!='luma-subtitle' or target.get('kind')!=['bin']
            or target.get('crate_types')!=['bin'] or target.get('test') is not True
            or Path(artifact.get('manifest_path',''))!=manifest or Path(target.get('src_path',''))!=source):
        raise ValueError('Cargo test artifact does not match the exact Luma manifest/source target')
    canonical_proof_path(manifest,manifest.parent,file=True);canonical_proof_path(source,manifest.parent,file=True)
    executable=canonical_proof_path(artifact['executable'],target_dir,file=True)
    expected_parent=target_dir/'x86_64-pc-windows-msvc/debug/deps'
    if (executable.parent!=expected_parent or not re.fullmatch(r'luma_subtitle-[a-f0-9]{16}\.exe',executable.name)
            or [Path(name) for name in artifact.get('filenames',[])].count(executable)!=1):
        raise ValueError('Cargo executable is not the emitted native test binary under the fixed target root')
    return executable


@contextmanager
def locked_test_binary(executable):
    """Keep Windows deny-write/delete sharing across verification and launch."""
    import msvcrt
    kernel=c.WinDLL('kernel32',use_last_error=True)
    create=kernel.CreateFileW;create.argtypes=[c.c_wchar_p,U32,U32,HANDLE,U32,U32,HANDLE];create.restype=HANDLE
    close=kernel.CloseHandle;close.argtypes=[HANDLE];close.restype=c.c_int
    handle=create(str(executable),0x80000000,1,None,3,0x00200000,None)
    if handle in (None,HANDLE(-1).value):raise c.WinError(c.get_last_error())
    try:fd=msvcrt.open_osfhandle(handle,os.O_RDONLY|os.O_BINARY)
    except BaseException:
        close(handle);raise
    with os.fdopen(fd,'rb') as stream:yield stream


def binary_identity(stream):
    before=os.fstat(stream.fileno())
    if not stat.S_ISREG(before.st_mode) or not 0<before.st_size<=512*1024*1024:
        raise ValueError('Cargo test binary is not a bounded regular file')
    stream.seek(0);digest=hashlib.sha256()
    if stream.read(2)!=b'MZ':raise ValueError('Cargo test artifact is not a Windows executable')
    stream.seek(0)
    for chunk in iter(lambda:stream.read(1024*1024),b''):digest.update(chunk)
    after=os.fstat(stream.fileno())
    identity=lambda value:(value.st_dev,value.st_ino,value.st_size,value.st_mtime_ns)
    if identity(before)!=identity(after):raise ValueError('Cargo test binary changed during hashing')
    return {'volume':before.st_dev,'file_id':before.st_ino,'bytes':before.st_size,
            'mtime_ns':before.st_mtime_ns,'sha256':digest.hexdigest()}


def compiler_diagnostics(output,limit):
    """Render only compiler diagnostics, never Cargo build-script env records."""
    rendered=[];size=0
    for line in output.splitlines():
        try:message=json.loads(line)
        except (ValueError,TypeError):continue
        if not isinstance(message,dict) or message.get('reason')!='compiler-message':continue
        detail=message.get('message',{})
        if not isinstance(detail,dict) or not isinstance(detail.get('rendered'),str):continue
        text=detail['rendered'];size+=len(text.encode('utf-8'))
        if size>limit:break
        rendered.append(text)
    return ''.join(rendered)


def cargo_runtime_environment(output,target,libdir,environment):
    """Mirror Cargo's documented Windows DLL paths, without exporting env."""
    search=[]
    for line in output.splitlines():
        if not line.strip():continue
        message=json.loads(line)
        if message.get('reason')!='build-script-executed':continue
        for value in message.get('linked_paths',[]):
            if not isinstance(value,str) or len(value)>MAX_IMAGE_CHARS:raise ValueError('Invalid Cargo library search path')
            if value.startswith(('native=','framework=','dependency=','crate=','all=')):value=value.split('=',1)[1]
            path=Path(value)
            # Cargo itself excludes build-script paths outside the target tree.
            if not path.is_absolute() or not path.is_relative_to(target):continue
            path=canonical_proof_path(path,target)
            if path not in search:search.append(path)
            if len(search)>256:raise ValueError('Cargo library path list exceeds its bound')
    for path in (target/'x86_64-pc-windows-msvc/debug/deps',target/'x86_64-pc-windows-msvc/debug',libdir):
        if path not in search:search.append(path)
    result=dict(environment);path_keys=[key for key in result if key.upper()=='PATH']
    if len(path_keys)>1:raise ValueError('Ambiguous Windows PATH keys')
    original=result.pop(path_keys[0]) if path_keys else ''
    result['PATH']=';'.join(map(str,search))+(';' + original if original else '')
    return result,[str(path) for path in search]


def run_owned_cargo_test(cargo,*,manifest,test_name,env=None,timeout=1800,output_limit=8*1024*1024):
    """Separate owned compile helpers from a freshly owned strict runtime test."""
    if sys.platform!='win32':raise RuntimeError('The two-phase Cargo proof is Windows-only')
    if not 0<timeout<=1800 or not 0<output_limit<=8*1024*1024:raise ValueError('Unbounded Cargo proof request')
    if test_name not in ('asr_components::tests::native_direct_recipe_installs_repairs_and_removes',
                         'asr::process::tests::real_optional_worker_transcribes_exports_reuses_and_cancels'):
        raise ValueError('Unexpected exact native proof test')
    manifest=Path(manifest);package=manifest.parent.resolve(strict=True)
    manifest=canonical_proof_path(manifest,package,file=True)
    if manifest.name!='Cargo.toml':raise ValueError('Expected the package Cargo manifest')
    source=canonical_proof_path(package/'src/main.rs',package,file=True)
    source_sha=(env or {}).get('LUMA_ASR_TEST_VERIFIER_SOURCE_SHA','')
    if not re.fullmatch('[a-f0-9]{40}',source_sha):raise ValueError('Exact verifier source SHA is required')
    def source_identity():
        identity={}
        for key,path in (('manifest_sha256',manifest),('target_source_sha256',source)):
            if path.stat().st_size>4*1024*1024:raise ValueError('Cargo source metadata exceeds its bound')
            identity[key]=hashlib.sha256(path.read_bytes()).hexdigest()
        return identity
    source_before=source_identity()
    target=package/'target';target.mkdir(exist_ok=True);canonical_proof_path(target,package)
    cargo=str(Path(cargo).resolve(strict=True));deadline=time.monotonic()+timeout
    environment=dict(env or os.environ)
    rustc=environment.get('RUSTC') or shutil.which('rustc',path=next((value for key,value in environment.items() if key.upper()=='PATH'),None))
    if not rustc:raise ValueError('Native Rust compiler is missing')
    rustc=str(Path(rustc).resolve(strict=True));environment['RUSTC']=rustc
    result={'schema':1,'kind':'two-phase-cargo-test','stage':'toolchain-query','exit_code':None,'tree_drained':True,
            'test_name':test_name,'test_started':False,'stdout':'','stderr':'','timeout_seconds':timeout,
            'manifest':str(manifest),'source':str(source),'source_sha':source_sha,'source_identity':source_before,
            'target_dir':str(target)}
    def remaining():
        value=deadline-time.monotonic()
        if value<=0:raise TimeoutError('Two-phase Cargo proof exceeded its shared time bound')
        return min(value,1800)
    def invoke(command,*,phase,environment):
        build=phase=='build';report=None
        budget=remaining();result['tree_drained']=False
        try:report=run_owned_tree(command,cwd=package,env=environment,timeout=budget,
                                  output_limit=4096 if phase=='toolchain_query' else output_limit,_build_only=build)
        except JobRunError as error:report=error.result;raise
        finally:
            if report is not None:
                result[phase]={key:value for key,value in report.items() if key not in ('stdout','stderr')}
                result['tree_drained']=report.get('tree_drained') is True
                result['exit_code']=report.get('exit_code')
                # Cargo JSON may include build-script environment assignments.
                # Do not publish it. Keep only runtime stdout and bounded stderr.
                if phase=='test':result['stdout']=report.get('stdout','')[:output_limit]
                detail=compiler_diagnostics(report.get('stdout',''),output_limit) if build else ''
                remaining_bytes=max(0,output_limit-len(result['stdout'].encode('utf-8')))
                result['stderr']=(result['stderr']+report.get('stderr','')+detail).encode('utf-8')[:remaining_bytes].decode('utf-8',errors='ignore')
        if report.get('exit_code')!=0 or report.get('tree_drained') is not True:
            raise RuntimeError('Cargo build failed' if build else 'Exact native test failed')
        return report
    try:
        toolchain=invoke([rustc,'--print','target-libdir','--target','x86_64-pc-windows-msvc'],phase='toolchain_query',environment=environment)
        if toolchain.get('natural_drain_completed') is not True:raise RuntimeError('Rust compiler query did not drain naturally')
        paths=toolchain['stdout'].splitlines()
        if len(paths)!=1 or len(paths[0])>MAX_IMAGE_CHARS:raise ValueError('Unexpected Rust target library path')
        libdir=Path(paths[0]);canonical_proof_path(libdir,libdir)
        result['compiler']={'executable':rustc,'target_libdir':str(libdir)}
        result['stage']='build'
        command=[cargo,'test','--manifest-path',str(manifest),'--locked','--bin','luma-subtitle',
                 '--no-run','--message-format=json','--target','x86_64-pc-windows-msvc','--target-dir',str(target)]
        build=invoke(command,phase='build',environment=environment)
        if build.get('build_helper_cleanup_completed') is not True:raise RuntimeError('Owned build helpers were not confirmed reaped')
        result['stage']='artifact-verification'
        if source_identity()!=source_before:raise ValueError('Cargo manifest or target source changed while building')
        executable=select_test_artifact(build['stdout'],manifest,target)
        runtime_env,search=cargo_runtime_environment(build['stdout'],target,libdir,environment)
        runtime_env.update(CARGO_MANIFEST_DIR=str(package),CARGO_MANIFEST_PATH=str(manifest))
        result['runtime_dll_search_paths']=search
        with locked_test_binary(executable) as stream:
            identity=binary_identity(stream)
            result['artifact']={'path':str(executable),**identity,'deny_write_delete_held':True}
            canonical_proof_path(executable,target,file=True)
            named=executable.stat()
            if (named.st_dev,named.st_ino)!=(identity['volume'],identity['file_id']):
                raise ValueError('Locked Cargo artifact no longer matches the launch path')
            if binary_identity(stream)!=identity:raise ValueError('Cargo test binary changed before launch')
            if source_identity()!=source_before:raise ValueError('Cargo manifest or target source changed before launch')
            result['stage']='test';result['test_started']=True
            runtime=invoke([str(executable),test_name,'--exact','--ignored','--nocapture'],phase='test',environment=runtime_env)
            if runtime.get('natural_drain_completed') is not True or runtime.get('termination_requested') is not False:
                raise RuntimeError('Runtime test descendants did not drain naturally')
            if not re.search(r'(?m)^test result: ok\. 1 passed; 0 failed; 0 ignored; 0 measured;',runtime['stdout']):
                raise RuntimeError('Expected exactly one successful native test')
            if binary_identity(stream)!=identity:raise ValueError('Cargo test binary changed during execution')
        result['stage']='complete';result['passed']=True
    except BaseException as error:
        result['passed']=False
        raise JobRunError(str(error),result) from error
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
