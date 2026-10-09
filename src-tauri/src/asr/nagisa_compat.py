"""One-use initializer for the managed Windows Nagisa 0.2.11 runtime.

Embedded by the application, never imported from a user-selected helper path.
The upstream package executes unchanged. Only its eager, default Tagger call is
given the public API's relative file arguments while Python owns a Unicode cwd.
"""
import base64 as _luma_b64
import csv as _luma_csv
import hashlib as _luma_hashlib
import importlib as _luma_importlib
import importlib.machinery as _luma_machinery
import importlib.metadata as _luma_metadata
import inspect as _luma_inspect
import os as _luma_os
from pathlib import Path as _LumaPath, PurePosixPath as _LumaPurePath
import sys as _luma_sys
import threading as _luma_threading


class LumaNagisaInitializationError(BaseException):
    """Fatal: the owning process must exit after a partial native import."""


_luma_nagisa_state = 'new'
_luma_nagisa_module = None
_luma_nagisa_original_init = None
_luma_nagisa_restoration = None
_luma_numba_root = None
_luma_numba_failed = False
_luma_numba_proof = None
_luma_numba_module = None
_luma_numba_environment = {'NUMBA_THREADING_LAYER': 'workqueue',
                           'NUMBA_THREADING_LAYER_PRIORITY': 'workqueue omp tbb',
                           'NUMBA_DISABLE_JIT': '0', 'NUMBA_DISABLE_CUDA': '1',
                           'NUMBA_NUM_THREADS': '2'}


class LumaManagedThreadingError(BaseException):
    """Fatal: a managed worker must not continue with changed threading policy."""


def _luma_numba_origin():
    root = _LumaPath(_luma_sys.prefix).resolve()
    site = root / 'Lib' / 'site-packages'
    info = site / 'numba-0.68.0.dist-info'
    metadata = info / 'METADATA'
    if (not _luma_sys.flags.isolated or not _LumaPath(_luma_sys.executable).resolve().is_relative_to(root)
            or info.is_symlink() or metadata.is_symlink() or not metadata.is_file()
            or not metadata.resolve().is_relative_to(site) or not 0 < metadata.stat().st_size <= 1024 * 1024):
        raise ValueError('Managed threading requires the isolated private Numba distribution')
    dist = _luma_metadata.Distribution.at(info)
    expected = site / 'numba' / '__init__.py'
    spec = _luma_machinery.PathFinder.find_spec('numba', [str(site)])
    if (dist.version != '0.68.0' or dist.metadata['Name'].lower() != 'numba'
            or _LumaPath(dist.locate_file('')).resolve() != site
            or expected.is_symlink() or not expected.is_file()
            or spec is None or _LumaPath(spec.origin or '').resolve() != expected
            or type(spec.loader) is not _luma_machinery.SourceFileLoader):
        raise ValueError('Unsupported managed Numba version or origin')
    return root


def _luma_is_tbb_dll(path):
    name = str(path).replace('\\', '/').rsplit('/', 1)[-1].lower()
    return name.startswith(('tbb', 'libtbb')) and name.endswith('.dll')


def _luma_assert_no_tbb():
    if any(name == 'numba.np.ufunc.tbbpool' or name.startswith('tbb.') or name == 'tbb'
           for name in _luma_sys.modules):
        raise ValueError('Inactive TBB module was loaded')
    import ctypes
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
        raise ValueError('Native module enumeration overflow')
    for handle in handles[:needed.value // ctypes.sizeof(wintypes.HMODULE)]:
        path = ctypes.create_unicode_buffer(32768)
        if not psapi.GetModuleFileNameExW(process, handle, path, len(path)):
            raise ctypes.WinError(ctypes.get_last_error())
        if _luma_is_tbb_dll(path.value):
            raise ValueError('Inactive TBB native library was loaded: ' + path.value)


def luma_configure_numba_workqueue():
    """Managed Windows Qwen only; called before Numba or activity threads.

    Numba documents workqueue as its built-in backend. It is not reentrant:
    this worker serializes requests, and its heartbeat never calls Numba.
    No package bytes, global environment, or manual runtime are changed.
    """
    global _luma_numba_root, _luma_numba_failed
    try:
        if _luma_numba_failed:
            raise ValueError('Managed threading previously failed')
        if _luma_sys.platform != 'win32':
            raise ValueError('Managed workqueue policy is Windows-only')
        if _luma_numba_root is not None:
            return luma_check_numba_workqueue()
        if (_luma_threading.current_thread() is not _luma_threading.main_thread()
                or _luma_threading.active_count() != 1
                or any(name == 'numba' or name.startswith('numba.') for name in _luma_sys.modules)):
            raise ValueError('Managed Numba must be configured before imports and worker activity')
        _luma_numba_root = _luma_numba_origin()
        # Only this managed child is normalized. Inherited developer options
        # must not silently disable JIT or alter its parallel backend/limits.
        for name in tuple(_luma_os.environ):
            if name.upper().startswith('NUMBA_'):
                del _luma_os.environ[name]
        _luma_os.environ.update(_luma_numba_environment)
        if (_LumaPath.cwd() / '.numba_config.yaml').exists():
            raise ValueError('Managed Numba cannot use a working-directory configuration file')
        _luma_assert_no_tbb()
    except BaseException as exc:
        _luma_numba_failed = True
        raise LumaManagedThreadingError('Managed threading initialization failed; discard worker: ' + str(exc)) from exc


def luma_check_numba_workqueue(require_initialized=False):
    global _luma_numba_failed
    try:
        actual_env = {name: value for name, value in _luma_os.environ.items() if name.upper().startswith('NUMBA_')}
        if (_luma_numba_failed or _luma_numba_root is None or _LumaPath(_luma_sys.prefix).resolve() != _luma_numba_root
                or not _LumaPath(_luma_sys.executable).resolve().is_relative_to(_luma_numba_root)
                or actual_env != _luma_numba_environment or (_LumaPath.cwd() / '.numba_config.yaml').exists()):
            raise ValueError('Managed threading configuration changed')
        module = _luma_sys.modules.get('numba')
        if _luma_numba_module is not None and module is not _luma_numba_module:
            raise ValueError('Successfully initialized Numba module identity changed')
        layer = None
        if module is not None:
            site = _luma_numba_root / 'Lib' / 'site-packages'
            if (module.__version__ != '0.68.0' or _LumaPath(module.__file__).resolve() != site / 'numba' / '__init__.py'
                    or module.config.THREADING_LAYER != 'workqueue' or module.config.DISABLE_JIT or module.config.DISABLE_CUDA != 1
                    or module.config.NUMBA_NUM_THREADS != 2
                    or module.config.THREADING_LAYER_PRIORITY != ['workqueue', 'omp', 'tbb']):
                raise ValueError('Imported Numba identity or configured layer changed')
            try:
                layer = module.threading_layer()
            except ValueError:
                pass  # A valid configuration need not have compiled parallel code yet.
            if layer not in (None, 'workqueue'):
                raise ValueError('Numba initialized a conflicting threading layer')
            workqueue = _luma_sys.modules.get('numba.np.ufunc.workqueue')
            if workqueue is not None and _LumaPath(workqueue.__file__).resolve() != site / 'numba/np/ufunc/workqueue.cp312-win_amd64.pyd':
                raise ValueError('Workqueue came from outside the exact private module')
            if layer == 'workqueue' and workqueue is None:
                raise ValueError('Selected private workqueue module is absent')
        if require_initialized and layer != 'workqueue':
            raise ValueError('Real parallel compilation did not select workqueue')
        _luma_assert_no_tbb()
        return layer
    except BaseException as exc:
        _luma_numba_failed = True
        raise LumaManagedThreadingError('Managed threading verification failed; discard worker: ' + str(exc)) from exc


def _luma_run_numba_workqueue_probe():
    """Small real native JIT/reduction, not model inference or a speed claim."""
    luma_check_numba_workqueue()
    numba = _luma_importlib.import_module('numba')
    numpy = _luma_importlib.import_module('numpy')
    luma_check_numba_workqueue()

    @numba.njit(parallel=True, cache=False)
    def squares(values):
        total = 0.0
        for index in numba.prange(values.size):
            total += values[index] * values[index]
        return total

    result = squares(numpy.arange(1024, dtype=numpy.float64))
    if result != sum(index * index for index in range(1024)) or not squares.nopython_signatures:
        raise LumaManagedThreadingError('Managed workqueue numerical/JIT proof failed')
    layer = luma_check_numba_workqueue(require_initialized=True)
    return {'schema': 1, 'numba': '0.68.0', 'configured': 'workqueue', 'selected': layer,
            'parallel_jit_tested': True, 'numeric_passed': True, 'elements': 1024,
            'private_workqueue': True, 'tbb_loaded': False,
            'tbb_checked_before_jit': True, 'tbb_checked_after_jit': True,
            'scope': 'Serialized single-request worker; no nested or concurrent Numba parallel calls.'}


def luma_probe_numba_workqueue():
    """Actual worker and setup share a once-only, successful native JIT proof."""
    global _luma_numba_proof, _luma_numba_failed, _luma_numba_module
    try:
        if _luma_numba_proof is None:
            proof = _luma_run_numba_workqueue_probe()
            luma_check_numba_workqueue(require_initialized=True)
            _luma_numba_module = _luma_sys.modules['numba']
            _luma_numba_proof = proof
        else:
            luma_check_numba_workqueue(require_initialized=True)
        return dict(_luma_numba_proof)
    except BaseException as exc:
        _luma_numba_failed = True
        raise LumaManagedThreadingError('Managed threading JIT proof failed; discard worker: ' + str(exc)) from exc


def _luma_validate_nagisa():
    root = _LumaPath(_luma_sys.prefix).resolve()
    site = root / 'Lib' / 'site-packages'
    if not _luma_sys.flags.isolated or not _LumaPath(_luma_sys.executable).resolve().is_relative_to(root):
        raise ValueError('Nagisa compatibility requires the isolated private interpreter')
    required = {'nagisa/__init__.py', 'nagisa/tagger.py', 'nagisa/model.py',
                'nagisa/data/nagisa_v001.dict', 'nagisa/data/nagisa_v001.model',
                'nagisa/data/nagisa_v001.hp'}
    verified = set()
    for name, version in (('nagisa', '0.2.11'), ('dynet38', '2.2')):
        expected_info = f'{name}-{version}.dist-info'
        matches = [p for p in site.iterdir() if p.name.casefold() == expected_info]
        if len(matches) != 1 or matches[0].is_symlink() or not matches[0].is_dir():
            raise ValueError(f'Unsupported managed {name} distribution identity')
        info = matches[0]
        # Bound the actual files before importlib.metadata reads/parses them.
        for filename, bound in (('METADATA', 1024 * 1024), ('RECORD', 8 * 1024 * 1024)):
            path = info / filename
            if path.is_symlink() or not path.is_file() or not 0 < path.stat().st_size <= bound:
                raise ValueError('Missing or oversized package metadata')
        dist = _luma_metadata.Distribution.at(info)
        if dist.version != version or dist.metadata['Name'].lower() != name or _LumaPath(dist.locate_file('')).resolve() != site:
            raise ValueError(f'Unsupported managed {name} version or package origin')
        with (info / 'RECORD').open(encoding='utf-8', newline='') as stream:
            entries = []
            for row in _luma_csv.reader(stream):
                if len(entries) >= 4096 or len(row) != 3:
                    raise ValueError('Invalid or oversized package RECORD')
                entries.append(row)
        if not entries:
            raise ValueError('Empty package RECORD')
        names = set(); total = 0
        for relative, recorded_hash, recorded_size in entries:
            entry = _LumaPurePath(relative)
            if (relative in names or entry.is_absolute() or not relative
                    or any(part in ('', '.', '..') for part in relative.split('/'))
                    or any(char in relative for char in ('\\', ':', '\x00'))):
                raise ValueError('Unsafe or duplicate package RECORD path')
            names.add(relative)
            path = _LumaPath(dist.locate_file(entry))
            if path.is_symlink() or not path.is_file() or not path.resolve().is_relative_to(site):
                raise ValueError('Package RECORD file is missing or outside the private runtime')
            # RECORD itself is intentionally unhashed in the wheel format.
            if path == info / 'RECORD' and not recorded_hash and not recorded_size:
                continue
            size = path.stat().st_size; total += size
            if (total > 512 * 1024 * 1024 or not recorded_size.isdecimal()
                    or int(recorded_size) != size or not recorded_hash.startswith('sha256=')):
                raise ValueError('Package RECORD size/hash is missing or exceeds the bound')
            with path.open('rb') as stream:
                actual = _luma_b64.urlsafe_b64encode(_luma_hashlib.file_digest(stream, 'sha256').digest()).rstrip(b'=').decode('ascii')
            if 'sha256=' + actual != recorded_hash:
                raise ValueError(f'Package RECORD mismatch: {relative}')
            verified.add(path.resolve())
        if name == 'nagisa' and not required.issubset(names):
            raise ValueError('Unsupported Nagisa data layout')
    package = site / 'nagisa'
    for name, expected in (('nagisa', package / '__init__.py'),
                           ('dynet', site / 'dynet.py'), ('dynet_config', site / 'dynet_config.py')):
        spec = _luma_machinery.PathFinder.find_spec(name, [str(site)])
        if (spec is None or _LumaPath(spec.origin or '').resolve() != expected or expected not in verified
                or type(spec.loader) is not _luma_machinery.SourceFileLoader):
            raise ValueError(f'Unsupported managed module origin: {name}')
    for name in ('_dynet', 'nagisa_utils'):
        spec = _luma_machinery.PathFinder.find_spec(name, [str(site)])
        if spec is None or _LumaPath(spec.origin or '').resolve() not in verified or type(spec.loader) is not _luma_machinery.ExtensionFileLoader:
            raise ValueError(f'Unsupported managed native module origin: {name}')
    return package


def luma_prepare_nagisa():
    """Initialize once, retaining only the unmodified upstream public API."""
    global _luma_nagisa_state, _luma_nagisa_module, _luma_nagisa_original_init, _luma_nagisa_restoration
    if _luma_sys.platform != 'win32':
        return None
    if _luma_nagisa_state == 'ready':
        if (_luma_sys.modules.get('nagisa') is _luma_nagisa_module
                and _luma_nagisa_module.Tagger.__init__ is _luma_nagisa_original_init):
            return _luma_nagisa_module
        raise LumaNagisaInitializationError('Initialized Nagisa identity changed; discard this worker')
    if _luma_nagisa_state != 'new':
        raise LumaNagisaInitializationError('Nagisa initialization is already active or failed; discard this worker')
    _luma_nagisa_state = 'initializing'
    cwd = None; finder = None; tagger_class = None; original_init = None; finders_before = None
    try:
        if _luma_threading.current_thread() is not _luma_threading.main_thread() or _luma_threading.active_count() != 1:
            raise ValueError('Nagisa must initialize before worker activity threads')
        if any(name == 'nagisa' or name.startswith('nagisa.') for name in _luma_sys.modules):
            raise ValueError('Nagisa was imported before managed initialization')
        if any(name in _luma_sys.modules for name in ('dynet', '_dynet', 'dynet_config', 'nagisa_utils')):
            raise ValueError('DyNet was imported before managed initialization')
        package = _luma_validate_nagisa()
        calls = 0

        class Loader:
            def __init__(self, delegate):
                self.delegate = delegate

            def create_module(self, spec):
                return self.delegate.create_module(spec)

            def exec_module(self, module):
                nonlocal tagger_class, original_init
                try:
                    self.delegate.exec_module(module)
                finally:
                    # No loader wrapper survives initialization.
                    module.__loader__ = self.delegate
                    module.__spec__.loader = self.delegate
                tagger_class = module.Tagger
                original_init = tagger_class.__init__
                parameters = list(_luma_inspect.signature(original_init).parameters.values())
                if [p.name for p in parameters] != ['self', 'vocabs', 'params', 'hp', 'single_word_list'] or any(p.default is not None for p in parameters[1:]):
                    raise ValueError('Unsupported Nagisa Tagger constructor')

                def initialize(instance, *args, **kwargs):
                    nonlocal calls
                    calls += 1
                    if calls != 1 or args or kwargs or type(instance) is not tagger_class:
                        raise ValueError('Only one default eager Nagisa construction is supported')
                    return original_init(instance, vocabs='nagisa_v001.dict',
                                         params='nagisa_v001.model', hp='nagisa_v001.hp')

                tagger_class.__init__ = initialize

        class Finder:
            def find_spec(self, fullname, path=None, target=None):
                if fullname != 'nagisa.tagger':
                    return None
                spec = _luma_machinery.PathFinder.find_spec(fullname, path)
                if spec is None or _LumaPath(spec.origin or '').resolve() != package / 'tagger.py' or type(spec.loader) is not _luma_machinery.SourceFileLoader:
                    raise ValueError('Unexpected Nagisa Tagger loader or origin')
                spec.loader = Loader(spec.loader)
                return spec

        cwd = _LumaPath.cwd()
        finders_before = tuple(_luma_sys.meta_path)
        _luma_os.chdir(package / 'data')
        finder = Finder(); _luma_sys.meta_path.insert(0, finder)
        module = _luma_importlib.import_module('nagisa')
        if calls != 1 or module.Tagger is not tagger_class or type(module.tagger) is not tagger_class:
            raise ValueError('Unexpected Nagisa eager initialization or class identity')
        for name in ('wakati', 'tagging', 'filter', 'extract', 'postagging', 'decode'):
            if getattr(getattr(module, name, None), '__self__', None) is not module.tagger:
                raise ValueError('Unexpected Nagisa public API binding')
        _luma_nagisa_module = module
        _luma_nagisa_original_init = original_init
        _luma_nagisa_state = 'ready'
        return module
    except BaseException as exc:
        _luma_nagisa_state = 'failed'
        raise LumaNagisaInitializationError(f'Managed Nagisa initialization failed; discard this worker: {exc}') from exc
    finally:
        try:
            if original_init is not None:
                tagger_class.__init__ = original_init
            if finder is not None:
                _luma_sys.meta_path[:] = [item for item in _luma_sys.meta_path if item is not finder]
            if cwd is not None:
                _luma_os.chdir(cwd)
            _luma_nagisa_restoration = {
                'constructor_restored': original_init is None or tagger_class.__init__ is original_init,
                'finder_removed': finder is None or all(item is not finder for item in _luma_sys.meta_path),
                'cwd_restored': cwd is None or _LumaPath.cwd().resolve() == cwd.resolve(),
                'original_finders_retained': finders_before is None or tuple(
                    item for item in _luma_sys.meta_path if any(item is before for before in finders_before)
                ) == finders_before,
            }
            if not all(_luma_nagisa_restoration.values()):
                raise ValueError('Nagisa initialization state was not restored exactly')
        except BaseException as exc:
            _luma_nagisa_state = 'failed'
            raise LumaNagisaInitializationError('Nagisa restoration failed; discard this worker') from exc
