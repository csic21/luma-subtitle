"""Shipped offline runtime/API self-test. Run with this pack's Python -I -B -u.

This deliberately does not download/load model weights or claim inference proof.
"""
from __future__ import annotations
import ctypes
import base64
import hashlib
import importlib
import importlib.metadata
import inspect
import json
import os
from pathlib import Path
import platform
import re
import site
import sys

# LUMA_NAGISA_COMPAT_SOURCE

ROOT = Path(__file__).resolve().parent


def platform_description():
    # platform.platform() may lazily query processor information via subprocess.
    # These fields use OS/runtime metadata without weakening the offline guard.
    return f'{sys.platform} {platform.release()}'


def optional_vc_runtime(filename):
    name = filename.lower()
    # msvcp_win.dll is an OS component on supported Windows 10, unlike the
    # numbered Visual C++ redistributables. Never equate the whole prefix with
    # optional app-local CRT libraries.
    return re.fullmatch(r'(?:msvcp|vcruntime|concrt|vcomp|vccorlib)[0-9][a-z0-9_]*\.dll', name) is not None


def data_file_diagnostic(path, root, record_sha256=None):
    """Read-only evidence for a bundled data file; never import/alter its package."""
    path = Path(path); root = Path(root).resolve()
    resolved = path.resolve()
    if not resolved.is_relative_to(root):
        raise ValueError('Bundled diagnostic path escaped the private runtime')
    result = {'relative_path': resolved.relative_to(root).as_posix(),
              'path_contains_non_ascii': not str(resolved).isascii(),
              'exists': path.exists(), 'regular_file': path.is_file() and not path.is_symlink(),
              'record_sha256_available': record_sha256 is not None, 'readable': False}
    if result['regular_file']:
        result['bytes'] = path.stat().st_size
        if result['bytes'] > 256 * 1024 * 1024:
            result['hash_skipped'] = 'diagnostic byte limit'
        else:
            with path.open('rb') as stream:
                digest = hashlib.file_digest(stream, 'sha256').digest()
            result.update(readable=True, sha256=digest.hex(),
                          record_sha256_match=base64.urlsafe_b64encode(digest).decode().rstrip('=') == record_sha256 if record_sha256 is not None else None)
    return result


def nagisa_data_diagnostic():
    distribution = importlib.metadata.distribution('nagisa')
    name = 'nagisa/data/nagisa_v001.model'
    entries = [entry for entry in distribution.files or () if entry.as_posix() == name]
    expected = entries[0].hash if len(entries) == 1 else None
    record_hash = expected.value if expected is not None and expected.mode == 'sha256' else None
    result = data_file_diagnostic(distribution.locate_file(name), ROOT, record_hash)
    result.update(schema=1, kind='bundled-nagisa-data-diagnostic', metadata_entry_present=len(entries) == 1,
                  python_filesystem_encoding=sys.getfilesystemencoding())
    if sys.platform == 'win32':
        kernel = ctypes.WinDLL('kernel32', use_last_error=True)
        kernel.GetACP.restype = ctypes.c_uint
        result['windows_ansi_code_page'] = kernel.GetACP()
    return result


def local_module(name):
    module = importlib.import_module(name)
    location = Path(module.__file__).resolve()
    if not location.is_relative_to(ROOT):
        raise AssertionError(f'{name} escaped the private component: {location}')
    return module


def loaded_native_libraries():
    if sys.platform == 'darwin':
        lib = ctypes.CDLL(None)
        lib._dyld_image_count.restype = ctypes.c_uint32
        lib._dyld_get_image_name.argtypes = [ctypes.c_uint32]
        lib._dyld_get_image_name.restype = ctypes.c_char_p
        paths = [lib._dyld_get_image_name(i).decode() for i in range(lib._dyld_image_count())]
        allowed = ('/usr/lib/', '/System/Library/')
    elif sys.platform == 'win32':
        from ctypes import wintypes
        psapi = ctypes.WinDLL('psapi', use_last_error=True)
        kernel = ctypes.WinDLL('kernel32', use_last_error=True)
        kernel.GetCurrentProcess.restype = wintypes.HANDLE
        psapi.EnumProcessModules.argtypes = [wintypes.HANDLE, ctypes.POINTER(wintypes.HMODULE), wintypes.DWORD, ctypes.POINTER(wintypes.DWORD)]
        psapi.GetModuleFileNameExW.argtypes = [wintypes.HANDLE, wintypes.HMODULE, wintypes.LPWSTR, wintypes.DWORD]
        process = kernel.GetCurrentProcess(); handles = (wintypes.HMODULE * 4096)(); needed = wintypes.DWORD()
        if not psapi.EnumProcessModules(process, handles, ctypes.sizeof(handles), ctypes.byref(needed)):
            raise ctypes.WinError(ctypes.get_last_error())
        if needed.value > ctypes.sizeof(handles):
            raise AssertionError('Native library enumeration overflow')
        paths = []
        for handle in handles[:needed.value // ctypes.sizeof(wintypes.HMODULE)]:
            buffer = ctypes.create_unicode_buffer(32768)
            if psapi.GetModuleFileNameExW(process, handle, buffer, len(buffer)):
                paths.append(buffer.value)
        allowed = (str(Path(os.environ['SystemRoot']).resolve()).lower() + '\\',)
    else:
        raise AssertionError('Only native Windows/macOS are supported')
    for path in paths:
        resolved = Path(path).resolve()
        test = str(resolved).lower() if sys.platform == 'win32' else str(resolved)
        if sys.platform == 'win32' and optional_vc_runtime(resolved.name) and not resolved.is_relative_to(ROOT):
            raise AssertionError(f'Optional Visual C++ runtime came from outside the private component: {path}')
        if not resolved.is_relative_to(ROOT) and not test.startswith(allowed):
            raise AssertionError(f'Loaded non-system library outside component: {path}')
    return len(paths)


def main():
    config = json.loads((ROOT / 'component.json').read_text(encoding='utf-8'))
    assert sys.flags.isolated and not site.ENABLE_USER_SITE
    assert Path(sys.executable).resolve().is_relative_to(ROOT)
    assert Path(sys.prefix).resolve() == ROOT
    assert sys.version.split()[0] == config['python_version']
    for path in sys.path:
        assert path and Path(path).resolve().is_relative_to(ROOT), f'External sys.path: {path}'
    for name in ('HF_HUB_OFFLINE', 'TRANSFORMERS_OFFLINE', 'HF_DATASETS_OFFLINE',
                 'HF_HUB_DISABLE_TELEMETRY', 'HF_HUB_DISABLE_IMPLICIT_TOKEN', 'DO_NOT_TRACK'):
        os.environ[name] = '1'
    os.environ['TOKENIZERS_PARALLELISM'] = 'false'
    sys.modules['vllm'] = None
    def audit(event, args):
        if event in {'socket.connect','socket.getaddrinfo','socket.bind','subprocess.Popen','os.system','os.posix_spawn','os.fork'}:
            raise RuntimeError(f'Offline self-test blocked {event}')
    sys.addaudithook(audit)
    numpy = local_module('numpy')
    assert numpy.asarray([1, 2, 3]).sum() == 6
    backend = config['backend']; device = 'cpu'; versions = {}; metal = None; metal_tested = False
    if backend == 'faster-whisper':
        assert not list(ROOT.rglob('cudnn*.dll')), 'CPU pack must omit unused cuDNN redistributables'
        fw = local_module('faster_whisper'); ct = local_module('ctranslate2')
        assert callable(fw.WhisperModel)
        assert 'int8' in ct.get_supported_compute_types('cpu')
        local_module('av'); local_module('onnxruntime'); local_module('tokenizers')
        packages = ['faster-whisper', 'ctranslate2', 'numpy']
    elif backend == 'mlx-whisper':
        mx = local_module('mlx.core'); mlx = local_module('mlx_whisper.transcribe')
        assert hasattr(mlx, 'ModelHolder')
        metal = bool(mx.metal.is_available())
        mx.set_default_device(mx.gpu if metal else mx.cpu)
        assert int(mx.sum(mx.array([1, 2, 3])).item()) == 6
        metal_tested = metal
        device = 'metal' if metal else 'cpu (runner has no Metal device)'
        packages = ['mlx-whisper', 'mlx', 'mlx-metal', 'torch', 'numpy']
        local_module('torch')
    elif backend == 'qwen3-asr':
        # A valid bundled file that Python can read but DyNet cannot open from
        # the Unicode relocation is materially different from missing data.
        # This is evidence only, never a path or dependency workaround.
        print(json.dumps(nagisa_data_diagnostic(), sort_keys=True), file=sys.stderr, flush=True)
        if sys.platform == 'win32':
            luma_prepare_nagisa()
        torch = local_module('torch')
        assert torch.version.cuda is None, 'CUDA libraries are outside this CPU pack'
        qwen = local_module('qwen_asr')
        for module in ('nagisa', 'dynet', 'soynlp', 'librosa', 'soundfile', 'transformers', 'qwen_omni_utils'):
            local_module(module)
        assert callable(qwen.Qwen3ASRModel.from_pretrained)
        assert 'forced_aligner' in inspect.signature(qwen.Qwen3ASRModel.from_pretrained).parameters
        assert torch.ones((2, 2), device='cpu').sum().item() == 4
        packages = ['qwen-asr', 'torch', 'transformers', 'nagisa', 'DyNet38', 'numpy']
    else:
        raise AssertionError('Unknown engine')
    for package in packages:
        versions[package] = importlib.metadata.version(package)
    assert importlib.util.find_spec('pip') is None, 'pip must not ship'
    libraries = loaded_native_libraries()
    print(json.dumps({'schema': 1, 'pack_id': config['id'], 'source_sha': config['source_sha'],
                      'python': sys.version.split()[0], 'platform': platform_description(), 'machine': platform.machine(),
                      'isolated': True, 'user_site': False, 'relocatable': True,
                      'private_native_libraries_checked': libraries, 'versions': versions,
                      'tested_device': device, 'inference_tested': False,
                      'metal_available': metal, 'metal_tested': metal_tested,
                      'scope': 'Offline import/API and tiny tensor operations; no model weights loaded.'}, ensure_ascii=False))


if __name__ == '__main__':
    main()
