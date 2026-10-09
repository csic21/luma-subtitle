"""Whole-runtime PE closure for reviewed CPU engines, including Qwen/Torch."""
import importlib.util
import hashlib
import zipfile
from pathlib import Path
import re

_path = Path(__file__).resolve().parent / 'ct2-cpu/native_inventory.py'
_spec = importlib.util.spec_from_file_location('luma_reviewed_pe_parser', _path)
_pe = importlib.util.module_from_spec(_spec); _spec.loader.exec_module(_pe)
GPU = re.compile(r'(?:cudnn|cublas|cudart|cufft|curand|cusolver|cusparse|nvrtc|nvcuda|nvjitlink|torch_cuda|c10_cuda)', re.I)
NUMBA_WHEEL = {
    'name': 'numba', 'version': '0.68.0', 'filename': 'numba-0.68.0-cp312-cp312-win_amd64.whl',
    'url': 'https://files.pythonhosted.org/packages/7e/2b/1b1f8b118cec28513665d8a53ff4f037d6c05720bd9e6f32f947c93c367f/numba-0.68.0-cp312-cp312-win_amd64.whl',
    'bytes': 2830891, 'sha256': '530961dc7e41ee358eca2b828baf7b645ce6fa466d778bb9dc73855dd103c4f7',
}
NUMBA_PLUGIN = 'numba/np/ufunc/tbbpool.cp312-win_amd64.pyd'


def inactive_numba_plugin(root, cache, wheel):
    """Compare retained bytes against the whole-hash-anchored official wheel.

    No network or installed RECORD is trusted as the source of the expected
    member digest. Native evidence records the actual derived digest/size.
    """
    if {key: wheel.get(key) for key in NUMBA_WHEEL} != NUMBA_WHEEL:
        raise ValueError('Unreviewed Numba source cannot classify an inactive plugin')
    archive = Path(cache) / NUMBA_WHEEL['sha256']
    if archive.is_symlink() or not archive.is_file() or archive.stat().st_size != NUMBA_WHEEL['bytes']:
        raise ValueError('Missing exact verified Numba wheel')
    if _pe.digest(archive) != NUMBA_WHEEL['sha256']:
        raise ValueError('Numba wheel hash mismatch')
    with zipfile.ZipFile(archive) as source:
        members = [item for item in source.infolist() if item.filename == NUMBA_PLUGIN]
        if len(members) != 1 or not 0 < members[0].file_size <= 1024 * 1024:
            raise ValueError('Unexpected Numba optional-plugin member')
        data = source.read(members[0])
    digest = hashlib.sha256(data).hexdigest()
    relative = 'Lib/site-packages/' + NUMBA_PLUGIN
    installed = Path(root) / relative
    if (installed.is_symlink() or not installed.is_file() or not installed.resolve().is_relative_to(Path(root).resolve())
            or installed.stat().st_size != len(data) or _pe.digest(installed) != digest):
        raise ValueError('Retained Numba optional-plugin bytes differ from the exact wheel')
    return {'package': 'numba', 'version': '0.68.0', 'path': relative, 'bytes': len(data),
            'sha256': digest, 'wheel': dict(NUMBA_WHEEL), 'retained': True,
            'inactive_import': {'kind': 'normal', 'name': 'tbb12.dll'}, 'configured_backend': 'workqueue'}


def closure(files, inactive_plugin=None):
    by_name = {}
    for item in files: by_name.setdefault(Path(item['path']).name.lower(), []).append(item['path'])
    if inactive_plugin is not None:
        fixed = {'package': 'numba', 'version': '0.68.0', 'path': 'Lib/site-packages/' + NUMBA_PLUGIN,
                 'wheel': NUMBA_WHEEL, 'retained': True,
                 'inactive_import': {'kind': 'normal', 'name': 'tbb12.dll'}, 'configured_backend': 'workqueue'}
        if (any(inactive_plugin.get(key) != value for key, value in fixed.items())
                or type(inactive_plugin.get('bytes')) is not int or not 0 < inactive_plugin['bytes'] <= 1024 * 1024
                or not re.fullmatch('[0-9a-f]{64}', str(inactive_plugin.get('sha256', '')))):
            raise ValueError('Unreviewed inactive-plugin descriptor')
        matches = [item for item in files if item['path'] == inactive_plugin['path']]
        if len(matches) != 1 or any(matches[0][key] != inactive_plugin[key] for key in ('bytes', 'sha256')):
            raise ValueError('Inactive plugin is not the verified retained PE member')
        if any(Path(item['path']).name.lower().startswith(('tbb', 'libtbb')) and item['path'].lower().endswith('.dll') for item in files):
            raise ValueError('Workqueue-only runtime unexpectedly contains TBB libraries')
    dependencies = []; inactive = []
    for item in files:
        for kind in ('normal', 'delay'):
            for name in item[kind]:
                if GPU.search(name): resolution = 'unreviewed_gpu'
                elif name in by_name: resolution = 'private'
                elif _pe.CRT.search(name): resolution = 'missing_private_crt'
                elif name.endswith('.dll') and name.startswith(('api-ms-win-', 'ext-ms-win-')): resolution = 'windows_api_set'
                elif name in _pe.OS_DLLS: resolution = 'windows_os'
                else: resolution = 'unresolved'
                if (resolution == 'unresolved' and inactive_plugin is not None
                        and item['path'] == inactive_plugin['path'] and kind == 'normal' and name == 'tbb12.dll'):
                    resolution = 'inactive_optional_plugin'
                    inactive.append(inactive_plugin)
                dependencies.append({'from': item['path'], 'kind': kind, 'name': name,
                                     'resolution': resolution, 'private_candidates': by_name.get(name, [])})
    blocked = [item for item in dependencies if item['resolution'] in ('unreviewed_gpu', 'missing_private_crt', 'unresolved')]
    gpu_files = [item['path'] for item in files if GPU.search(Path(item['path']).name)]
    if inactive_plugin is not None and len(inactive) != 1:
        raise ValueError('Expected exactly the reviewed inactive tbb12.dll import')
    required = not blocked and not gpu_files
    return {'schema': 1, 'normal_and_delay_imports': True, 'files': files, 'dependencies': dependencies,
            'blocked_dependencies': blocked, 'gpu_files': gpu_files,
            'inactive_optional_plugins': inactive, 'required_closure_passed': required,
            'full_tree_closure_passed': required and not inactive,
            'passed': required and not inactive,
            'scope': 'Entire EXE/DLL/PYD tree, including Torch/PyAV/DyNet; pinned private OpenMP/MKL is not treated as the CT2-only no-OpenMP variant.',
            'limitations': ['Static basename closure is not a loader test; actual isolated imports must separately verify every loaded native origin.',
                           'A retained inactive Numba TBB plugin does not have full dependency closure. The configured workqueue policy needs separate genuine numerical/JIT proof.']}


def inventory(root, inactive_plugin=None): return closure(_pe.inventory(root), inactive_plugin)


def validate_configured_closure(report, imports):
    if not report['required_closure_passed']:
        raise ValueError('Configured runtime has unresolved required native dependencies')
    if report['inactive_optional_plugins']:
        probe = imports.get('numba_threading') or {}
        expected = {'schema': 1, 'numba': '0.68.0', 'configured': 'workqueue', 'selected': 'workqueue',
                    'parallel_jit_tested': True, 'numeric_passed': True, 'elements': 1024,
                    'private_workqueue': True, 'tbb_loaded': False, 'checked_after_qwen_imports': True,
                    'tbb_checked_before_jit': True, 'tbb_checked_after_jit': True}
        if any(probe.get(key) != value for key, value in expected.items()):
            raise ValueError('Inactive plugin requires genuine matching workqueue/JIT/no-TBB proof')
        report['configured_threading_proof'] = probe
    report['passed'] = True
    return report
