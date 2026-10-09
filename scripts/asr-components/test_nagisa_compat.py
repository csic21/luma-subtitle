"""Dependency-free tests for the one-use upstream initialization adapter."""
import importlib.util
import base64
import csv
import hashlib
import io
import json
from pathlib import Path
import sys
import unittest
from types import SimpleNamespace
from unittest.mock import patch
from fixture_paths import temporary_root

SOURCE = Path(__file__).resolve().parents[2] / 'src-tauri/src/asr/nagisa_compat.py'
WORKER = SOURCE.with_name('worker.py')


def load_helper():
    spec = importlib.util.spec_from_file_location('test_luma_nagisa_compat', SOURCE)
    helper = importlib.util.module_from_spec(spec); spec.loader.exec_module(helper)
    return helper


class NagisaCompatibilityTests(unittest.TestCase):
    def setUp(self):
        self.root = self.enterContext(temporary_root()) / 'runtime é 测试'; self.root.mkdir()
        self.package = self.root / 'nagisa'; self.package.mkdir(); (self.package / 'data').mkdir()
        (self.package / 'data/nagisa_v001.model').write_text('good', encoding='utf-8')
        (self.package / 'tagger.py').write_text('''from pathlib import Path
class Tagger:
    def __init__(self, vocabs=None, params=None, hp=None, single_word_list=None):
        assert (vocabs, params, hp) == ('nagisa_v001.dict', 'nagisa_v001.model', 'nagisa_v001.hp')
        if Path(params).read_text() == 'fail': raise RuntimeError('native initialization failed')
    def wakati(self, text): return [text]
    tagging = filter = extract = postagging = decode = wakati
ORIGINAL_INIT = Tagger.__init__
''', encoding='utf-8')
        self.initialization = '''from nagisa.tagger import Tagger
tagger = Tagger()
wakati = tagger.wakati
tagging = tagger.tagging
filter = tagger.filter
extract = tagger.extract
postagging = tagger.postagging
decode = tagger.decode
'''
        (self.package / '__init__.py').write_text(self.initialization, encoding='utf-8')
        self.helper = load_helper(); self.cwd = Path.cwd(); self.finders = tuple(sys.meta_path)
        self.addCleanup(self.remove_modules)

    def remove_modules(self):
        for name in list(sys.modules):
            if name == 'nagisa' or name.startswith('nagisa.'):
                del sys.modules[name]

    def initialize(self):
        with patch.object(sys, 'platform', 'win32'), patch.object(sys, 'path', [str(self.root)] + sys.path), patch.object(self.helper, '_luma_validate_nagisa', return_value=self.package):
            return self.helper.luma_prepare_nagisa()

    def assert_restored(self):
        self.assertEqual(Path.cwd(), self.cwd)
        self.assertEqual(tuple(sys.meta_path), self.finders)
        tagger = sys.modules.get('nagisa.tagger')
        if tagger is not None:
            self.assertIs(tagger.Tagger.__init__, tagger.ORIGINAL_INIT)
            self.assertIs(type(tagger.__loader__), self.helper._luma_machinery.SourceFileLoader)
            self.assertIs(tagger.__spec__.loader, tagger.__loader__)

    def test_one_use_preserves_api_and_restores_process_state(self):
        module = self.initialize()
        self.assertEqual(module.wakati('日本語'), ['日本語'])
        self.assertIs(self.initialize(), module)
        self.assert_restored()

    def test_partial_native_failure_is_fatal_and_poisoned(self):
        (self.package / 'data/nagisa_v001.model').write_text('fail', encoding='utf-8')
        with self.assertRaises(self.helper.LumaNagisaInitializationError): self.initialize()
        self.assert_restored()
        with self.assertRaises(self.helper.LumaNagisaInitializationError): self.initialize()
        self.assertFalse(issubclass(self.helper.LumaNagisaInitializationError, Exception))

    def test_legitimate_upstream_finder_is_preserved_and_luma_finder_removed(self):
        upstream = '''\nimport sys
class UpstreamFinder:
    def find_spec(self, fullname, path=None, target=None): return None
upstream_finder = UpstreamFinder()
sys.meta_path.append(upstream_finder)
'''
        (self.package / '__init__.py').write_text(self.initialization + upstream, encoding='utf-8')
        module = self.initialize()
        try:
            self.assertEqual(tuple(sys.meta_path), self.finders + (module.upstream_finder,))
            self.assertTrue(all(self.helper._luma_nagisa_restoration.values()))
            self.assertIs(module.Tagger.__init__, sys.modules['nagisa.tagger'].ORIGINAL_INIT)
        finally:
            sys.meta_path.remove(module.upstream_finder)
        self.assert_restored()

    def test_second_default_construction_and_explicit_arguments_fail_closed(self):
        for initialization in (self.initialization + '\nother = Tagger()\n', self.initialization.replace('Tagger()', "Tagger(params='override')")):
            with self.subTest(initialization=initialization):
                self.helper = load_helper(); self.remove_modules()
                (self.package / '__init__.py').write_text(initialization, encoding='utf-8')
                with self.assertRaises(self.helper.LumaNagisaInitializationError): self.initialize()
                self.assert_restored()

    def test_preexisting_partial_module_and_concurrency_fail_closed(self):
        sys.modules['nagisa.partial'] = object()
        with self.assertRaises(self.helper.LumaNagisaInitializationError): self.initialize()
        self.remove_modules(); self.helper = load_helper()
        with patch.object(self.helper._luma_threading, 'active_count', return_value=2):
            with self.assertRaises(self.helper.LumaNagisaInitializationError): self.initialize()
        self.assert_restored()

    def test_verification_failure_is_fatal_before_cwd_or_import(self):
        with patch.object(sys, 'platform', 'win32'), patch.object(self.helper, '_luma_validate_nagisa', side_effect=ValueError('RECORD mismatch')):
            with self.assertRaises(self.helper.LumaNagisaInitializationError): self.helper.luma_prepare_nagisa()
        self.assert_restored(); self.assertNotIn('nagisa', sys.modules)

    def test_fatal_adapter_failure_escapes_worker_request_catches(self):
        source = WORKER.read_text(encoding='utf-8')
        marker = '# LUMA_NAGISA_COMPAT_SOURCE'
        self.assertEqual(source.count(marker), 1)
        code = SOURCE.read_text(encoding='utf-8') + '\nLUMA_MANAGED_QWEN_RUNTIME = True\n'
        namespace = {'__name__': 'worker_test'}; exec(compile(source.replace(marker, code), str(WORKER), 'exec'), namespace)
        fatal = namespace['LumaNagisaInitializationError']('fatal fixture')
        namespace['configuration'] = lambda request: {'backend': 'qwen3-asr', 'engine': 'qwen3-asr', 'model_bytes': 1, 'aligner_bytes': 1, 'requested_device': 'cpu'}
        namespace['luma_configure_numba_workqueue'] = lambda: None
        namespace['luma_prepare_nagisa'] = lambda: (_ for _ in ()).throw(fatal)
        with self.assertRaises(type(fatal)):
            namespace['serve'](io.StringIO(json.dumps({'id': 1, 'op': 'probe'}) + '\n'), io.StringIO())


class NagisaValidationTests(unittest.TestCase):
    def setUp(self):
        self.root = self.enterContext(temporary_root()); self.site = self.root / 'Lib/site-packages'; self.site.mkdir(parents=True)
        self.helper = load_helper()
        self.original_find_spec = self.helper._luma_machinery.PathFinder.find_spec
        self.paths = {'nagisa': ['nagisa/__init__.py', 'nagisa/tagger.py', 'nagisa/model.py',
                                 'nagisa/data/nagisa_v001.dict', 'nagisa/data/nagisa_v001.model',
                                 'nagisa/data/nagisa_v001.hp', 'nagisa_utils.pyd'],
                      'dynet38': ['dynet.py', 'dynet_config.py', '_dynet.pyd']}
        self.infos = {}
        for name, version in [('nagisa', '0.2.11'), ('dynet38', '2.2')]:
            info = self.site / f'{name}-{version}.dist-info'; info.mkdir(); self.infos[name] = info
            (info / 'METADATA').write_text(f'Metadata-Version: 2.1\nName: {name}\nVersion: {version}\n', encoding='utf-8')
            for relative in self.paths[name]:
                path = self.site / relative; path.parent.mkdir(parents=True, exist_ok=True); path.write_bytes(b'fixture')
            self.write_record(name)

    def write_record(self, name):
        info = self.infos[name]
        with (info / 'RECORD').open('w', newline='', encoding='utf-8') as stream:
            writer = csv.writer(stream)
            for relative in self.paths[name] + [(info / 'METADATA').relative_to(self.site).as_posix()]:
                path = self.site / relative
                digest = base64.urlsafe_b64encode(hashlib.sha256(path.read_bytes()).digest()).rstrip(b'=').decode()
                writer.writerow([relative, 'sha256=' + digest, path.stat().st_size])
            writer.writerow([(info / 'RECORD').relative_to(self.site).as_posix(), '', ''])

    def spec(self, name, path, target=None):
        modules = {'nagisa': 'nagisa/__init__.py', 'dynet': 'dynet.py', 'dynet_config': 'dynet_config.py', '_dynet': '_dynet.pyd', 'nagisa_utils': 'nagisa_utils.pyd'}
        if name not in modules:
            return self.original_find_spec(name, path, target)
        relative = modules[name]
        origin = str(self.site / relative)
        cls = self.helper._luma_machinery.ExtensionFileLoader if name in ('_dynet', 'nagisa_utils') else self.helper._luma_machinery.SourceFileLoader
        return SimpleNamespace(origin=origin, loader=cls(name, origin))

    def validate(self, spec=None):
        flags = SimpleNamespace(**{name: getattr(sys.flags, name) for name in dir(sys.flags) if not name.startswith('_') and not callable(getattr(sys.flags, name))})
        flags.isolated = 1
        with patch.object(sys, 'prefix', str(self.root)), patch.object(sys, 'executable', str(self.root / 'python.exe')), patch.object(sys, 'flags', flags), patch.object(self.helper._luma_machinery.PathFinder, 'find_spec', side_effect=spec or self.spec):
            return self.helper._luma_validate_nagisa()

    def test_actual_record_validation_passes_exact_fixture(self):
        self.assertEqual(self.validate(), self.site / 'nagisa')

    def test_wrong_version_modified_and_missing_files_are_rejected(self):
        metadata = self.infos['nagisa'] / 'METADATA'; data = metadata.read_bytes()
        metadata.write_bytes(data.replace(b'0.2.11', b'0.3.0'))
        with self.assertRaises(ValueError): self.validate()
        metadata.write_bytes(data)
        model = self.site / 'nagisa/data/nagisa_v001.model'; model.write_bytes(b'modified')
        with self.assertRaises(ValueError): self.validate()
        model.unlink()
        with self.assertRaises(ValueError): self.validate()

    def test_record_escape_duplicate_and_size_are_rejected(self):
        record = self.infos['nagisa'] / 'RECORD'; original = record.read_text()
        for extra in ('../escape,sha256=invalid,1\n', original.splitlines()[0] + '\n', 'x' * (8 * 1024 * 1024 + 1)):
            with self.subTest(extra=extra[:30]):
                record.write_text(original + extra, encoding='utf-8')
                with self.assertRaises(ValueError): self.validate()
        record.write_text(original, encoding='utf-8')
        metadata = self.infos['nagisa'] / 'METADATA'; metadata.write_bytes(b'x' * (1024 * 1024 + 1))
        with self.assertRaises(ValueError): self.validate()

    def test_wrong_python_or_native_module_origin_is_rejected(self):
        for selected in ('nagisa', '_dynet', 'nagisa_utils'):
            def wrong(name, path, target=None):
                spec = self.spec(name, path, target)
                if name == selected: spec.origin = str(self.root / 'outside.py')
                return spec
            with self.subTest(selected=selected):
                with self.assertRaises(ValueError): self.validate(wrong)

    def test_python_module_omitted_from_record_is_rejected(self):
        self.paths['dynet38'].remove('dynet_config.py')
        self.write_record('dynet38')
        with self.assertRaises(ValueError): self.validate()


if __name__ == '__main__': unittest.main()
