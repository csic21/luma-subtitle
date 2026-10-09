"""Whole-runtime PE closure for reviewed CPU engines, including Qwen/Torch."""
import importlib.util
from pathlib import Path
import re

_path = Path(__file__).resolve().parent / 'ct2-cpu/native_inventory.py'
_spec = importlib.util.spec_from_file_location('luma_reviewed_pe_parser', _path)
_pe = importlib.util.module_from_spec(_spec); _spec.loader.exec_module(_pe)
GPU = re.compile(r'(?:cudnn|cublas|cudart|cufft|curand|cusolver|cusparse|nvrtc|nvcuda|nvjitlink|torch_cuda|c10_cuda)', re.I)


def closure(files):
    by_name = {}
    for item in files: by_name.setdefault(Path(item['path']).name.lower(), []).append(item['path'])
    dependencies = []
    for item in files:
        for kind in ('normal', 'delay'):
            for name in item[kind]:
                if GPU.search(name): resolution = 'unreviewed_gpu'
                elif name in by_name: resolution = 'private'
                elif _pe.CRT.search(name): resolution = 'missing_private_crt'
                elif name.startswith(('api-ms-win-', 'ext-ms-win-')): resolution = 'windows_api_set'
                elif name in _pe.OS_DLLS: resolution = 'windows_os'
                else: resolution = 'unresolved'
                dependencies.append({'from': item['path'], 'kind': kind, 'name': name,
                                     'resolution': resolution, 'private_candidates': by_name.get(name, [])})
    blocked = [item for item in dependencies if item['resolution'] in ('unreviewed_gpu', 'missing_private_crt', 'unresolved')]
    gpu_files = [item['path'] for item in files if GPU.search(Path(item['path']).name)]
    return {'schema': 1, 'normal_and_delay_imports': True, 'files': files, 'dependencies': dependencies,
            'blocked_dependencies': blocked, 'gpu_files': gpu_files, 'passed': not blocked and not gpu_files,
            'scope': 'Entire EXE/DLL/PYD tree, including Torch/PyAV/DyNet; pinned private OpenMP/MKL is not treated as the CT2-only no-OpenMP variant.',
            'limitations': ['Static basename closure is not a loader test; actual isolated imports must separately verify every loaded native origin.']}


def inventory(root): return closure(_pe.inventory(root))
