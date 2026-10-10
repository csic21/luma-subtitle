#!/usr/bin/env python3
"""Executed by the fresh, relocated PBS interpreter, never the build interpreter."""
import argparse
from contextlib import contextmanager
import ctypes
from ctypes import wintypes
import importlib
import importlib.metadata
import importlib.util
import faulthandler
import hashlib
import io
import json
import os
from pathlib import Path
import re
import sys
import time

CT2_CPU_POLICY_MARKER = '# LUMA_MANAGED_CT2_CPU_POLICY'
CT2_CPU_POLICY_SOURCE = 'LUMA_MANAGED_CT2_CPU_POLICY = "luma-cpu-seq-1"'


def checked_source(path, expected_hash):
    path = Path(path)
    if (not re.fullmatch('[a-f0-9]{64}', expected_hash) or path.is_symlink()
            or not path.is_file() or not 0 < path.stat().st_size < 1_000_000):
        raise ValueError('Invalid integrated proof source identity')
    data = path.read_bytes()
    if hashlib.sha256(data).hexdigest() != expected_hash:
        raise ValueError('Integrated proof source bytes changed')
    return data


def load_host_auditor(path, expected_hash, source_sha):
    if not re.fullmatch('[a-f0-9]{40}', source_sha):
        raise ValueError('Exact integrated verifier source commit required')
    data = checked_source(path, expected_hash)
    spec = importlib.util.spec_from_file_location('luma_integrated_host_auditor', path)
    auditor = importlib.util.module_from_spec(spec)
    exec(compile(data, str(path), 'exec'), auditor.__dict__)
    if not callable(getattr(auditor, 'verified_defender_module', None)):
        raise ValueError('Reviewed Defender classifier is unavailable')
    return auditor, {'source_sha': source_sha, 'repository_path': 'scripts/asr-components/self_test.py',
                     'bytes': len(data), 'sha256': expected_hash,
                     'scope': 'Exact registered Microsoft Defender AMSI identity only'}


def load_source_proof_worker(path, root, provenance_path, provenance_hash, source_sha, *, component_source_sha=None, publication_proof=None):
    """Select the production source policy for a locally built exact variant.

    This is a source-build proof, not evidence of app-global managed selection.
    The separate native installer test matches its actual active receipt through
    the production Rust selector. No constructor wrapper/A-B override is used.
    """
    if not re.fullmatch('[a-f0-9]{40}', source_sha):
        raise ValueError('Exact source-build commit required')
    component_sha = source_sha if component_source_sha is None else component_source_sha
    if not isinstance(component_sha, str) or not re.fullmatch('[a-f0-9]{40}', component_sha):
        raise ValueError('Exact immutable component source commit required')
    provenance = json.loads(checked_source(provenance_path, provenance_hash))
    if component_source_sha is not None:
        helper_path = Path(__file__).resolve().parents[1] / 'own_cpu_recipe.py'
        spec = importlib.util.spec_from_file_location('luma_pinned_cpu_reference', helper_path)
        helper = importlib.util.module_from_spec(spec); spec.loader.exec_module(helper)
        shipping_lock = json.loads((helper_path.parent / 'locks' / (helper.PACK_ID + '.json')).read_text(encoding='utf-8'))
        component = helper.pinned_component(shipping_lock, publication_proof)
        if component['component_source_sha'] != component_sha or component['proof']['provenance'] != provenance:
            raise ValueError('Reference component source/provenance differs from the pinned published wheel')
    elif publication_proof is not None:
        raise ValueError('Publication proof requires an explicit immutable component source')
    lock = json.loads(Path(__file__).with_name('sources.lock.json').read_text(encoding='utf-8'))
    if (provenance.get('schema') != 1 or provenance.get('variant') != 'luma-cpu-seq-1'
            or provenance.get('luma_source_sha') != component_sha or lock.get('variant') != 'luma-cpu-seq-1'
            or lock.get('wheel_build_tag') != '1lumacpu'
            or provenance.get('upstream') != lock['sources']
            or provenance.get('ct2_cmake') != lock['ct2_cmake']
            or provenance.get('onednn_cmake') != lock['onednn_cmake']):
        raise ValueError('Source proof does not identify the reviewed CPU build recipe')
    installed = root / 'Lib/site-packages/ctranslate2-4.8.2.dist-info/LUMA_CPU_BUILD.json'
    if (installed.is_symlink() or not installed.resolve(strict=True).is_relative_to(root)
            or not 0 < installed.stat().st_size < 1_000_000
            or json.loads(installed.read_text(encoding='utf-8')) != provenance):
        raise ValueError('Installed CPU wheel differs from its source-build provenance')
    source = Path(path).read_text(encoding='utf-8')
    if source.count(CT2_CPU_POLICY_MARKER) != 1:
        raise ValueError('Managed CPU runtime policy marker is missing or ambiguous')
    prepared = source.replace(CT2_CPU_POLICY_MARKER, CT2_CPU_POLICY_SOURCE, 1)
    worker = {'__name__': 'luma_source_proof_worker', '__file__': str(path)}
    exec(compile(prepared, str(path), 'exec'), worker)
    if worker['managed_ct2_cpu_options']('cpu') != {'cpu_threads': 1}:
        raise ValueError('Reviewed managed CPU constructor policy was not selected')
    identity = {'source_sha': source_sha, 'variant': 'luma-cpu-seq-1', 'cpu_threads': 1,
                'selection': 'source-build-provenance', 'managed_receipt_selection_tested': False,
                'constructor_overridden': False, 'worker_sha256': hashlib.sha256(source.encode()).hexdigest(),
                'prepared_worker_sha256': hashlib.sha256(prepared.encode()).hexdigest(),
                'build_provenance_sha256': provenance_hash, 'performance_claim': False}
    if component_source_sha is not None:
        identity['component_source_sha'] = component_sha
    return worker, identity


class StageJournal:
    """Small atomic metadata snapshots survive an outer forced termination."""
    MAX_EVENTS = 256
    MAX_BYTES = 256_000

    def __init__(self, path):
        self.path = Path(path)
        self.started = time.monotonic()
        self.events = []

    def record(self, stage, state, details=None):
        if len(self.events) >= self.MAX_EVENTS or len(stage) > 128 or len(state) > 32:
            raise RuntimeError('Verifier diagnostic event bound exceeded')
        event = {'stage': stage, 'state': state, 'elapsed_seconds': round(time.monotonic() - self.started, 6)}
        if details:
            event['details'] = details
        events = self.events + [event]
        payload = json.dumps({'schema': 1, 'publication_authorized': False,
                              'outer_timeout_seconds': 900, 'traceback_snapshot_after_seconds': 90,
                              'events': events}, indent=2, sort_keys=True, ensure_ascii=True) + '\n'
        if len(payload.encode('utf-8')) > self.MAX_BYTES:
            raise RuntimeError('Verifier diagnostic byte bound exceeded')
        temporary = self.path.with_suffix(self.path.suffix + '.tmp')
        temporary.write_text(payload, encoding='utf-8', newline='\n')
        temporary.replace(self.path)
        self.events = events
        print('LUMA_CT2_STAGE ' + json.dumps(event, sort_keys=True, ensure_ascii=True), flush=True)

    @contextmanager
    def stage(self, name):
        self.record(name, 'started')
        try:
            yield
        except BaseException as error:
            self.record(name, 'failed', {'exception_type': type(error).__name__})
            raise
        else:
            self.record(name, 'completed')


class ProtocolCapture(io.StringIO):
    """Keep original frames for assertions, while journaling bounded progress."""
    MAX_BYTES = 1_000_000
    MAX_LINE_BYTES = 128_000

    def __init__(self, journal):
        super().__init__()
        self.journal = journal
        self.total = 0
        self.pending = ''

    def write(self, value):
        size = len(value.encode('utf-8'))
        if self.total + size > self.MAX_BYTES:
            raise RuntimeError('Verifier protocol output bound exceeded')
        pending = self.pending + value
        lines = pending.split('\n')
        if any(len(line.encode('utf-8')) > self.MAX_LINE_BYTES for line in lines):
            raise RuntimeError('Verifier protocol frame bound exceeded')
        for line in lines[:-1]:
            if not line:
                continue
            frame = json.loads(line)
            if not isinstance(frame, dict):
                raise RuntimeError('Verifier expected protocol object')
            details = {}
            for key in ('id', 'event', 'backend', 'device', 'ready', 'reused', 'code', 'message'):
                item = frame.get(key)
                if isinstance(item, bool):
                    details[key] = item
                elif isinstance(item, str):
                    details[key] = item[:512 if key == 'message' else 128]
            if isinstance(frame.get('segments'), list):
                details['segment_count'] = len(frame['segments'])
            self.journal.record('worker.protocol', 'frame', details)
        self.pending = lines[-1]
        self.total += size
        return super().write(value)


def record_failure(report, error):
    report.update(passed=False, error=f'{type(error).__name__}: {error}')


def validate_loaded_paths(paths, root, system, auditor=None):
    forbidden = ('cudnn', 'cublas', 'cudart', 'nvrtc', 'nvcuda', 'iomp', 'libomp', 'vcomp', 'mkl', 'tbb')
    resolved = {}
    security = []
    for name in paths:
        path = Path(name); base = path.name.lower()
        if any(word in base for word in forbidden):
            raise RuntimeError('Unexpected GPU/OpenMP/MKL native module: ' + name)
        # Match the shared runtime policy: this exact Windows OS component is
        # not one of the optional numbered Visual C++ redistributable DLLs.
        if base != 'msvcp_win.dll' and base.startswith(('msvcp', 'vcruntime', 'concrt', 'vcomp', 'vccorlib')) and not path.is_relative_to(root):
            raise RuntimeError('Host-global VC runtime leaked into clean proof: ' + name)
        if not path.is_relative_to(root) and not path.is_relative_to(system):
            if base == 'mpoav.dll' and auditor is not None:
                security.append(name)
                continue
            raise RuntimeError('Unexpected native module outside private runtime/Windows: ' + name)
        resolved[name.lower()] = {'path': str(path.relative_to(root)) if path.is_relative_to(root) else str(path),
                                 'scope': 'private' if path.is_relative_to(root) else 'windows_os'}
    # Check every ordinary dependency before attempting the sole host-security
    # exemption. Signature verification never blesses arbitrary CRT/GPU modules.
    for name in security:
        evidence = auditor.verified_defender_module(name)
        if evidence.get('verified') is not True or evidence.get('kind') != 'windows-defender-amsi':
            raise RuntimeError('Defender classification did not produce verified identity')
        resolved[name.lower()] = {'path': name, 'scope': 'verified_host_security', 'evidence': evidence}
    return [resolved[name.lower()] for name in paths]


def audit_loaded_modules(root, system, auditor):
    # Trust APIs can themselves load DLLs. Audit their additions until the same
    # bounded three-round fixed point as the shared managed runtime self-test.
    checked = {}; previous = None
    for _ in range(3):
        paths = loaded_modules()
        if not 0 < len(paths) <= 4096:
            raise RuntimeError('Native module snapshot exceeds bounds')
        current = frozenset(name.lower() for name in paths)
        new = [name for name in paths if name.lower() not in checked]
        records = validate_loaded_paths(new, root, system, auditor)
        checked.update((name.lower(), record) for name, record in zip(new, records))
        if current == previous:
            return [checked[name] for name in sorted(checked)]
        previous = current
    raise RuntimeError('Native module snapshot did not stabilize within three rounds')


def loaded_modules():
    psapi = ctypes.WinDLL('psapi', use_last_error=True)
    kernel = ctypes.WinDLL('kernel32', use_last_error=True)
    kernel.GetCurrentProcess.restype = wintypes.HANDLE
    process = kernel.GetCurrentProcess()
    enum = psapi.EnumProcessModules
    enum.argtypes = [wintypes.HANDLE, ctypes.POINTER(wintypes.HMODULE), wintypes.DWORD, ctypes.POINTER(wintypes.DWORD)]
    enum.restype = wintypes.BOOL
    getname = psapi.GetModuleFileNameExW
    getname.argtypes = [wintypes.HANDLE, wintypes.HMODULE, wintypes.LPWSTR, wintypes.DWORD]
    getname.restype = wintypes.DWORD
    modules = (wintypes.HMODULE * 4096)(); needed = wintypes.DWORD()
    if not enum(process, modules, ctypes.sizeof(modules), ctypes.byref(needed)) or needed.value > ctypes.sizeof(modules):
        raise RuntimeError('Cannot inventory loaded native modules')
    result = []
    for handle in modules[:needed.value // ctypes.sizeof(wintypes.HMODULE)]:
        buffer = ctypes.create_unicode_buffer(32768)
        length = getname(process, handle, buffer, len(buffer))
        if not length or length >= len(buffer):
            raise RuntimeError('Cannot resolve loaded native module')
        result.append(str(Path(buffer.value).resolve()))
    return sorted(set(result), key=str.lower)


def main():
    p = argparse.ArgumentParser()
    for arg in ('root', 'model', 'switch-model', 'audio', 'worker', 'report', 'host-auditor', 'build-provenance'):
        p.add_argument('--' + arg, required=True, type=Path)
    for arg in ('host-auditor-sha256', 'build-provenance-sha256', 'source-sha'):
        p.add_argument('--' + arg, required=True)
    p.add_argument('--publication-proof', type=Path, help='Required with --component-source-sha; bytes are checked against the committed shipping lock.')
    p.add_argument('--component-source-sha', help='Immutable component source for later app-source verification; defaults to --source-sha for native builds.')
    a = p.parse_args(); root = a.root.resolve()
    report = {'schema': 1, 'passed': False, 'isolated': bool(sys.flags.isolated),
              'relocated': True, 'system_python_used': False, 'host_crt_fallback_allowed': False}
    journal = StageJournal(a.report.with_name('inference-stages.json'))
    journal.record('verifier.bootstrap', 'started')
    trace = a.report.with_name('inference-traceback.log').open('w', encoding='utf-8')
    trace.write('One diagnostic stack snapshot after 90 seconds; the outer proof timeout remains 900 seconds.\n')
    trace.flush()
    faulthandler.dump_traceback_later(90, repeat=False, file=trace)
    try:
        if sys.platform != 'win32' or sys.version.split()[0] != '3.12.15' or not sys.flags.isolated or not sys.flags.dont_write_bytecode:
            raise RuntimeError('Expected isolated pinned native Windows interpreter')
        if Path(sys.prefix).resolve() != root or not Path(sys.executable).resolve().is_relative_to(root):
            raise RuntimeError('Verifier must run inside the fresh private interpreter')
        journal.record('verifier.bootstrap', 'completed')
        with journal.stage('host_auditor.module_load'):
            auditor, auditor_identity = load_host_auditor(a.host_auditor, a.host_auditor_sha256, a.source_sha)
            report['host_auditor'] = auditor_identity
            report['verifier'] = {'source_sha': a.source_sha,
                'repository_path': 'scripts/asr-components/ct2-cpu/verify_runtime.py',
                'sha256': hashlib.sha256(Path(__file__).read_bytes()).hexdigest()}
        with journal.stage('worker.module_load'):
            worker, report['cpu_thread_policy'] = load_source_proof_worker(
                a.worker, root, a.build_provenance, a.build_provenance_sha256, a.source_sha,
                component_source_sha=a.component_source_sha, publication_proof=a.publication_proof)
        with journal.stage('worker.offline_audit'):
            worker['configure_offline'](); sys.addaudithook(worker['offline_audit'])
        imports = {}
        for name in ('ctranslate2', 'faster_whisper', 'av', 'numpy', 'onnxruntime', 'tokenizers'):
            with journal.stage('import.' + name):
                module = importlib.import_module(name)
            origin = Path(module.__file__).resolve()
            if not origin.is_relative_to(root):
                raise RuntimeError('Imported package escaped private runtime: ' + name)
            imports[name] = str(origin.relative_to(root))
        import ctranslate2
        if ctranslate2.__version__ != '4.8.2' or importlib.metadata.version('faster-whisper') != '1.2.1':
            raise RuntimeError('Unreviewed inference package version')
        with journal.stage('device.cuda_count'):
            cuda_count = ctranslate2.get_cuda_device_count()
        if cuda_count != 0:
            raise RuntimeError('CPU build unexpectedly reports CUDA')
        with journal.stage('device.cpu_compute_types'):
            compute = sorted(ctranslate2.get_supported_compute_types('cpu'))
        if not {'float32', 'int8'}.issubset(compute):
            raise RuntimeError('Supported CPU float32/int8 GEMM backends were not built')
        common = {'engine': 'whisper-accelerated', 'device': 'cpu', 'model_path': str(a.model),
                  'audio_path': str(a.audio), 'language': 'en'}
        if a.model.resolve(strict=True) == a.switch_model.resolve(strict=True):
            raise RuntimeError('Model switch proof requires distinct fixture directories')
        requests = [dict(common, id='probe', op='probe'), dict(common, id='cold', op='transcribe'),
                    dict(common, id='warm', op='transcribe'),
                    dict(common, id='switch', op='transcribe', model_path=str(a.switch_model))]
        output = ProtocolCapture(journal)
        started = time.perf_counter()
        with journal.stage('worker.serve'):
            worker['serve'](io.StringIO(''.join(json.dumps(x) + '\n' for x in requests)), output)
        journal.record('protocol.validation', 'started')
        frames = [json.loads(x) for x in output.getvalue().splitlines()]
        results = {f['id']: f for f in frames if f.get('event') in {'probe', 'result', 'error'}}
        if not results['probe']['ready']:
            raise RuntimeError('Private worker probe failed: ' + str(results))
        with journal.stage('worker.explicit_unload'):
            owned = worker['Worker']()
            results['unload'] = owned.handle(dict(common, id='unload', op='transcribe'), lambda event: None)
            owned.unload()
            if owned.model is not None or owned.key is not None:
                raise RuntimeError('Explicit model unload did not clear owned model state')
        elapsed = time.perf_counter() - started
        for key in ('cold', 'warm', 'switch', 'unload'):
            result = results[key]
            if result['event'] != 'result' or result['device'] != 'cpu' or 'country' not in ' '.join(s['text'] for s in result['segments']).lower():
                raise RuntimeError('Tiny inference did not produce expected speech: ' + str(result))
            previous = 0
            for segment in result['segments']:
                if not previous <= segment['start_ms'] < segment['end_ms'] <= 11001:
                    raise RuntimeError('Invalid Tiny segment timestamps')
                previous = segment['end_ms']
        if any(results[key]['reused'] for key in ('cold', 'switch', 'unload')) or not results['warm']['reused']:
            raise RuntimeError('Cold/warm model reuse contract failed')
        journal.record('protocol.validation', 'completed')
        journal.record('loaded_modules.validation', 'started')
        system = Path(next(v for k, v in os.environ.items() if k.upper() == 'SYSTEMROOT')).resolve()
        resolved = audit_loaded_modules(root, system, auditor)
        journal.record('loaded_modules.validation', 'completed')
        report.update(passed=True, imports=imports, compute_types=compute, loaded_modules=resolved,
                      source_worker_lifecycle={'eof_after_inference': True,
                          'model_switch_after_inference': True, 'explicit_unload_after_inference': True},
                      inference={'model': 'SYSTRAN/faster-whisper-tiny', 'cold_and_warm': True,
                                 'segments': results['cold']['segments'], 'cpu_only': True,
                                 'total_seconds_observed': elapsed, 'performance_claim': False},
                      offline_audit_enabled=True, inherited_python_path_ignored=True)
        journal.record('verification', 'completed', {'passed': True})
    except BaseException as error:
        record_failure(report, error)
        journal.record('verification', 'failed', {'exception_type': type(error).__name__})
        raise
    finally:
        faulthandler.cancel_dump_traceback_later()
        trace.close()
        a.report.write_text(json.dumps(report, indent=2, sort_keys=True, ensure_ascii=False) + '\n', encoding='utf-8')


if __name__ == '__main__':
    main()
