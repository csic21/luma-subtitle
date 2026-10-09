#!/usr/bin/env python3
"""Executed by the fresh, relocated PBS interpreter, never the build interpreter."""
import argparse
import ctypes
from ctypes import wintypes
import importlib
import importlib.metadata
import io
import json
import os
from pathlib import Path
import runpy
import sys
import time


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
    for arg in ('root', 'model', 'audio', 'worker', 'report'):
        p.add_argument('--' + arg, required=True, type=Path)
    a = p.parse_args(); root = a.root.resolve()
    report = {'schema': 1, 'passed': False, 'isolated': bool(sys.flags.isolated),
              'relocated': True, 'system_python_used': False, 'host_crt_fallback_allowed': False}
    try:
        if sys.platform != 'win32' or sys.version.split()[0] != '3.12.15' or not sys.flags.isolated or not sys.flags.dont_write_bytecode:
            raise RuntimeError('Expected isolated pinned native Windows interpreter')
        if Path(sys.prefix).resolve() != root or not Path(sys.executable).resolve().is_relative_to(root):
            raise RuntimeError('Verifier must run inside the fresh private interpreter')
        worker = runpy.run_path(str(a.worker))
        worker['configure_offline'](); sys.addaudithook(worker['offline_audit'])
        imports = {}
        for name in ('ctranslate2', 'faster_whisper', 'av', 'numpy', 'onnxruntime', 'tokenizers'):
            module = importlib.import_module(name)
            origin = Path(module.__file__).resolve()
            if not origin.is_relative_to(root):
                raise RuntimeError('Imported package escaped private runtime: ' + name)
            imports[name] = str(origin.relative_to(root))
        import ctranslate2
        if ctranslate2.__version__ != '4.8.2' or importlib.metadata.version('faster-whisper') != '1.2.1':
            raise RuntimeError('Unreviewed inference package version')
        if ctranslate2.get_cuda_device_count() != 0:
            raise RuntimeError('CPU build unexpectedly reports CUDA')
        compute = sorted(ctranslate2.get_supported_compute_types('cpu'))
        if not {'float32', 'int8'}.issubset(compute):
            raise RuntimeError('Supported CPU float32/int8 GEMM backends were not built')
        common = {'engine': 'whisper-accelerated', 'device': 'cpu', 'model_path': str(a.model),
                  'audio_path': str(a.audio), 'language': 'en'}
        requests = [dict(common, id='probe', op='probe'), dict(common, id='cold', op='transcribe'),
                    dict(common, id='warm', op='transcribe')]
        output = io.StringIO()
        started = time.perf_counter()
        worker['serve'](io.StringIO(''.join(json.dumps(x) + '\n' for x in requests)), output)
        elapsed = time.perf_counter() - started
        frames = [json.loads(x) for x in output.getvalue().splitlines()]
        results = {f['id']: f for f in frames if f.get('event') in {'probe', 'result', 'error'}}
        if not results['probe']['ready']:
            raise RuntimeError('Private worker probe failed: ' + str(results))
        for key in ('cold', 'warm'):
            result = results[key]
            if result['event'] != 'result' or result['device'] != 'cpu' or 'country' not in ' '.join(s['text'] for s in result['segments']).lower():
                raise RuntimeError('Tiny inference did not produce expected speech: ' + str(result))
            previous = 0
            for segment in result['segments']:
                if not previous <= segment['start_ms'] < segment['end_ms'] <= 11001:
                    raise RuntimeError('Invalid Tiny segment timestamps')
                previous = segment['end_ms']
        if results['cold']['reused'] or not results['warm']['reused']:
            raise RuntimeError('Cold/warm model reuse contract failed')
        paths = loaded_modules()
        system = Path(next(v for k, v in os.environ.items() if k.upper() == 'SYSTEMROOT')).resolve()
        forbidden = ('cudnn', 'cublas', 'cudart', 'nvrtc', 'nvcuda', 'iomp', 'libomp', 'vcomp', 'mkl', 'tbb')
        resolved = []
        for name in paths:
            path = Path(name); base = path.name.lower()
            if any(word in base for word in forbidden):
                raise RuntimeError('Unexpected GPU/OpenMP/MKL native module: ' + name)
            if base.startswith(('msvcp', 'vcruntime', 'concrt', 'vcomp', 'vccorlib')) and not path.is_relative_to(root):
                raise RuntimeError('Host-global VC runtime leaked into clean proof: ' + name)
            if not path.is_relative_to(root) and not path.is_relative_to(system):
                raise RuntimeError('Unexpected native module outside private runtime/Windows: ' + name)
            resolved.append({'path': str(path.relative_to(root)) if path.is_relative_to(root) else str(path),
                             'scope': 'private' if path.is_relative_to(root) else 'windows_os'})
        report.update(passed=True, imports=imports, compute_types=compute, loaded_modules=resolved,
                      inference={'model': 'SYSTRAN/faster-whisper-tiny', 'cold_and_warm': True,
                                 'segments': results['cold']['segments'], 'cpu_only': True,
                                 'total_seconds_observed': elapsed, 'performance_claim': False},
                      offline_audit_enabled=True, inherited_python_path_ignored=True)
    except BaseException as error:
        report['error'] = f'{type(error).__name__}: {error}'
        raise
    finally:
        a.report.write_text(json.dumps(report, indent=2, sort_keys=True, ensure_ascii=False) + '\n', encoding='utf-8')


if __name__ == '__main__':
    main()
