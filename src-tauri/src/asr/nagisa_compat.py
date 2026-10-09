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
