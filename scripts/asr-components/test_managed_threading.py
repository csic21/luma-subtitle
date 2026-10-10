"""Dependency-free managed-only threading policy and wheel-anchor regressions."""
import copy
import hashlib
import importlib.util
import io
import json
import os
from pathlib import Path
import sys
from types import SimpleNamespace
import unittest
from unittest.mock import patch
import zipfile

from fixture_paths import temporary_root
import windows_native_inventory as native

SOURCE = Path(__file__).resolve().parents[2] / 'src-tauri/src/asr/nagisa_compat.py'


def helper():
    spec = importlib.util.spec_from_file_location('managed_threading_fixture', SOURCE)
    module = importlib.util.module_from_spec(spec); spec.loader.exec_module(module)
    return module


class ManagedThreadingTests(unittest.TestCase):
    def setUp(self):
        self.root = self.enterContext(temporary_root()) / 'private é 测试'; self.root.mkdir()
        self.helper = helper()
        self.enterContext(patch.object(sys, 'platform', 'win32'))
        self.enterContext(patch.object(sys, 'prefix', str(self.root)))
        self.enterContext(patch.object(sys, 'executable', str(self.root/'python.exe')))
        self.enterContext(patch.object(self.helper._LumaPath, 'cwd', return_value=self.root))
        self.enterContext(patch.dict(os.environ, {}, clear=True))
        self.enterContext(patch.object(self.helper, '_luma_numba_origin', return_value=self.root))
        self.enterContext(patch.object(self.helper, '_luma_assert_no_tbb'))

    def numba(self, layer='workqueue'):
        site = self.root / 'Lib/site-packages'
        module = SimpleNamespace(__version__='0.68.0', __file__=str(site/'numba/__init__.py'),
            config=SimpleNamespace(THREADING_LAYER='workqueue', DISABLE_JIT=0, DISABLE_CUDA=1, NUMBA_NUM_THREADS=2,
                                   THREADING_LAYER_PRIORITY=['workqueue', 'omp', 'tbb']), threading_layer=lambda: layer)
        workqueue = SimpleNamespace(__file__=str(site/'numba/np/ufunc/workqueue.cp312-win_amd64.pyd'))
        self.enterContext(patch.dict(sys.modules, {'numba': module, 'numba.np.ufunc.workqueue': workqueue}))
        return module

    def test_inherited_jit_thread_and_backend_overrides_are_normalized_in_child(self):
        os.environ.update(NUMBA_DISABLE_JIT='1', NUMBA_NUM_THREADS='999', NUMBA_THREADING_LAYER='tbb',
                          NUMBA_THREADING_LAYER_PRIORITY='tbb omp workqueue', NUMBA_DEBUG='1', numba_unexpected='x')
        self.helper.luma_configure_numba_workqueue()
        self.assertEqual(dict(os.environ), self.helper._luma_numba_environment)
        self.numba()
        self.assertEqual(self.helper.luma_check_numba_workqueue(True), 'workqueue')
        self.assertEqual(self.helper.luma_configure_numba_workqueue(), 'workqueue')

    def test_preimported_or_changed_backend_is_fatal_and_cannot_recover(self):
        with patch.dict(sys.modules, {'numba': object()}):
            with self.assertRaises(self.helper.LumaManagedThreadingError): self.helper.luma_configure_numba_workqueue()
        with self.assertRaises(self.helper.LumaManagedThreadingError): self.helper.luma_configure_numba_workqueue()
        self.helper = helper()
        with patch.object(self.helper, '_luma_numba_origin', return_value=self.root), patch.object(self.helper, '_luma_assert_no_tbb'):
            self.helper.luma_configure_numba_workqueue(); module = self.numba('tbb')
            with self.assertRaises(self.helper.LumaManagedThreadingError): self.helper.luma_check_numba_workqueue()
            module.threading_layer = lambda: 'workqueue'
            with self.assertRaises(self.helper.LumaManagedThreadingError): self.helper.luma_check_numba_workqueue()
        self.assertFalse(issubclass(self.helper.LumaManagedThreadingError, Exception))

    def test_external_config_and_changed_jit_origin_are_rejected(self):
        (self.root/'.numba_config.yaml').write_text('disable_jit: true\n')
        with self.assertRaisesRegex(self.helper.LumaManagedThreadingError, 'configuration file'):
            self.helper.luma_configure_numba_workqueue()
        (self.root/'.numba_config.yaml').unlink(); self.helper = helper()
        with patch.object(self.helper, '_luma_numba_origin', return_value=self.root), patch.object(self.helper, '_luma_assert_no_tbb'):
            self.helper.luma_configure_numba_workqueue(); module = self.numba()
            module.config.DISABLE_JIT = 1
            with self.assertRaises(self.helper.LumaManagedThreadingError): self.helper.luma_check_numba_workqueue()

    def test_actual_worker_managed_gate_precedes_imports_and_manual_is_unchanged(self):
        source = SOURCE.with_name('worker.py').read_text(encoding='utf-8')
        namespace = {'__name__': 'worker_test'}; exec(compile(source, 'worker.py', 'exec'), namespace)
        calls = []
        namespace.update(luma_configure_numba_workqueue=lambda: calls.append('configure'),
                         luma_prepare_nagisa=lambda: calls.append('nagisa'),
                         luma_probe_numba_workqueue=lambda: calls.append('jit'),
                         luma_check_numba_workqueue=lambda **kwargs: calls.append(('check',kwargs)))
        torch = SimpleNamespace(version=SimpleNamespace(hip=None), float32='float32')
        qwen = SimpleNamespace(Qwen3ASRModel=SimpleNamespace(from_pretrained=lambda: None))
        def optional(name, package):
            calls.append(name); return torch if name == 'torch' else qwen
        namespace['optional_import'] = optional
        namespace['runtime']('qwen3-asr', 'cpu')
        self.assertEqual(calls, ['torch', 'qwen_asr'])
        calls.clear(); namespace['LUMA_MANAGED_QWEN_RUNTIME'] = True
        namespace['runtime']('qwen3-asr', 'cpu')
        self.assertEqual(calls, ['configure', 'nagisa', 'jit', 'torch', 'qwen_asr', ('check',{'require_initialized':True})])
        fatal = self.helper.LumaManagedThreadingError('conflicting threading')
        namespace['luma_configure_numba_workqueue'] = lambda: (_ for _ in ()).throw(fatal)
        namespace['configuration'] = lambda request: {'backend': 'qwen3-asr', 'engine': 'qwen3-asr', 'model_bytes': 1,
            'aligner_bytes': 1, 'requested_device': 'cpu'}
        with self.assertRaises(type(fatal)):
            namespace['serve'](io.StringIO(json.dumps({'id': 1, 'op': 'probe'})+'\n'), io.StringIO())
        calls.clear()
        namespace['luma_configure_numba_workqueue'] = lambda: calls.append('configure')
        def failing_jit():
            calls.append('jit'); raise fatal
        namespace['luma_probe_numba_workqueue'] = failing_jit
        with self.assertRaises(type(fatal)):
            namespace['serve'](io.StringIO(json.dumps({'id': 2, 'op': 'probe'})+'\n'), io.StringIO())
        self.assertEqual(calls,['configure','nagisa','jit'])

    def test_only_successful_real_probe_is_cached_and_rechecked(self):
        self.helper.luma_configure_numba_workqueue(); self.numba()
        proof = {'selected':'workqueue','numeric_passed':True}
        with patch.object(self.helper,'_luma_run_numba_workqueue_probe',return_value=proof) as run:
            first = self.helper.luma_probe_numba_workqueue()
            first['numeric_passed'] = False
            self.assertEqual(self.helper.luma_probe_numba_workqueue(),proof)
            self.assertEqual(run.call_count,1)
            os.environ['NUMBA_DISABLE_JIT'] = '1'
            with self.assertRaises(self.helper.LumaManagedThreadingError): self.helper.luma_probe_numba_workqueue()

    def test_probe_exception_is_fatal_and_does_not_cache_partial_success(self):
        self.helper.luma_configure_numba_workqueue()
        with patch.object(self.helper,'_luma_run_numba_workqueue_probe',side_effect=RuntimeError('JIT failed')):
            with self.assertRaises(self.helper.LumaManagedThreadingError): self.helper.luma_probe_numba_workqueue()
        self.assertIsNone(self.helper._luma_numba_proof)
        with self.assertRaises(self.helper.LumaManagedThreadingError): self.helper.luma_configure_numba_workqueue()

    def test_successful_cache_rejects_changed_module_identity(self):
        self.helper.luma_configure_numba_workqueue(); module = self.numba()
        with patch.object(self.helper,'_luma_run_numba_workqueue_probe',return_value={'selected':'workqueue'}):
            self.helper.luma_probe_numba_workqueue()
            with patch.dict(sys.modules,{'numba':copy.copy(module)}):
                with self.assertRaisesRegex(self.helper.LumaManagedThreadingError,'identity changed'):
                    self.helper.luma_probe_numba_workqueue()

    def test_successful_cache_rejects_changed_canonical_runtime_root(self):
        self.helper.luma_configure_numba_workqueue(); self.numba()
        with patch.object(self.helper,'_luma_run_numba_workqueue_probe',return_value={'selected':'workqueue'}):
            self.helper.luma_probe_numba_workqueue()
            with patch.object(sys,'prefix',str(self.root/'other')):
                with self.assertRaisesRegex(self.helper.LumaManagedThreadingError,'configuration changed'):
                    self.helper.luma_probe_numba_workqueue()


class OptionalPluginAnchorTests(unittest.TestCase):
    def setUp(self):
        self.root = self.enterContext(temporary_root()); self.cache = self.root/'cache'; self.cache.mkdir()
        self.runtime = self.root/'runtime'; self.runtime.mkdir()
        self.data = b'fixed native fixture'
        archive = self.root/'numba.whl'
        with zipfile.ZipFile(archive, 'w') as z: z.writestr(native.NUMBA_PLUGIN, self.data)
        content = archive.read_bytes()
        self.pin = dict(native.NUMBA_WHEEL, bytes=len(content), sha256=hashlib.sha256(content).hexdigest())
        self.archive = self.cache/self.pin['sha256']; self.archive.write_bytes(content)
        self.path = self.runtime/'Lib/site-packages'/native.NUMBA_PLUGIN
        self.path.parent.mkdir(parents=True); self.path.write_bytes(self.data)
        self.enterContext(patch.object(native, 'NUMBA_WHEEL', self.pin))

    def plugin(self): return native.inactive_numba_plugin(self.runtime, self.cache, self.pin)

    def test_complete_wheel_anchor_and_retained_member_hash_are_both_required(self):
        plugin = self.plugin()
        self.assertEqual(plugin['sha256'], hashlib.sha256(self.data).hexdigest()); self.assertTrue(plugin['retained'])
        for key, value in [('version', 'other'), ('url', 'https://other.invalid/x.whl'), ('sha256', '0'*64)]:
            with self.subTest(key=key), self.assertRaises(ValueError):
                native.inactive_numba_plugin(self.runtime, self.cache, dict(self.pin, **{key:value}))
        self.path.write_bytes(b'changed')
        with self.assertRaisesRegex(ValueError, 'Retained'): self.plugin()
        self.path.write_bytes(self.data); self.archive.write_bytes(b'x'*self.pin['bytes'])
        with self.assertRaisesRegex(ValueError, 'wheel hash'): self.plugin()

    def test_only_exact_inactive_edge_is_excluded_and_needs_real_jit_evidence(self):
        plugin = self.plugin()
        image = {key:plugin[key] for key in ('path','bytes','sha256')}
        image.update(normal=['tbb12.dll','kernel32.dll'], delay=[])
        self.assertFalse(native.closure([image])['passed'])
        report = native.closure([image],plugin)
        self.assertTrue(report['required_closure_passed']); self.assertFalse(report['full_tree_closure_passed'])
        self.assertFalse(report['passed'])
        with self.assertRaises(ValueError): native.validate_configured_closure(report,{})
        probe = {'schema':1,'numba':'0.68.0','configured':'workqueue','selected':'workqueue',
                 'parallel_jit_tested':True,'numeric_passed':True,'elements':1024,'private_workqueue':True,
                 'tbb_loaded':False,'checked_after_qwen_imports':True,'tbb_checked_before_jit':True,'tbb_checked_after_jit':True}
        for key, value in [('selected','tbb'),('parallel_jit_tested',False),('tbb_loaded',True),('numeric_passed',False)]:
            with self.subTest(key=key), self.assertRaises(ValueError):
                native.validate_configured_closure(copy.deepcopy(report),{'numba_threading':dict(probe,**{key:value})})
        native.validate_configured_closure(report,{'numba_threading':probe})
        self.assertTrue(report['passed']); self.assertFalse(report['full_tree_closure_passed'])
        for changes in ({'sha256':'f'*64},{'bytes':1},{'path':'other.pyd'},{'normal':[],'delay':['tbb12.dll']}):
            with self.subTest(changes=changes), self.assertRaises(ValueError): native.closure([dict(image,**changes)],plugin)
        for changes in ({'path':'elsewhere.pyd'},{'version':'0.69.0'},{'wheel':dict(self.pin,url='https://other.invalid')},
                        {'inactive_import':{'kind':'delay','name':'tbb12.dll'}},{'configured_backend':'tbb'},{'retained':False}):
            with self.subTest(descriptor=changes), self.assertRaisesRegex(ValueError,'descriptor'):
                native.closure([image],dict(plugin,**changes))
        extra = native.closure([dict(image,normal=image['normal']+['missing.dll'])],plugin)
        self.assertFalse(extra['required_closure_passed'])
        with self.assertRaises(ValueError): native.validate_configured_closure(extra,{'numba_threading':probe})
        with self.assertRaises(ValueError): native.closure([image,{'path':'tbb12.dll','normal':[],'delay':[]}],plugin)


class PrivateNumbaIdentityTests(unittest.TestCase):
    def setUp(self):
        self.root = self.enterContext(temporary_root()); self.helper = helper()
        self.site = self.root/'Lib/site-packages'; self.info = self.site/'numba-0.68.0.dist-info'
        self.info.mkdir(parents=True); (self.site/'numba').mkdir()
        (self.site/'numba/__init__.py').write_text('# fixture\n')
        (self.info/'METADATA').write_text('Metadata-Version: 2.1\nName: numba\nVersion: 0.68.0\n')
        flags = SimpleNamespace(**{name:getattr(sys.flags,name) for name in dir(sys.flags)
                                  if not name.startswith('_') and not callable(getattr(sys.flags,name))})
        flags.isolated = 1
        self.enterContext(patch.object(sys,'prefix',str(self.root)))
        self.enterContext(patch.object(sys,'executable',str(self.root/'python.exe')))
        self.enterContext(patch.object(sys,'flags',flags))

    def test_real_metadata_and_origin_validation_are_version_bounded(self):
        self.assertEqual(self.helper._luma_numba_origin(), self.root)
        metadata = self.info/'METADATA'; original = metadata.read_text()
        for value in (original.replace('0.68.0','0.69.0'),original.replace('Name: numba','Name: other')):
            metadata.write_text(value)
            with self.assertRaises(ValueError): self.helper._luma_numba_origin()
        metadata.write_text(original)
        with patch.object(self.helper._luma_machinery.PathFinder,'find_spec',return_value=SimpleNamespace(
                origin=str(self.root.parent/'outside.py'),loader=None)):
            with self.assertRaises(ValueError): self.helper._luma_numba_origin()
        (self.site/'numba/__init__.py').unlink()
        with self.assertRaises(ValueError): self.helper._luma_numba_origin()

    def test_tbb_module_and_any_loaded_tbb_family_are_rejected(self):
        for name in ('tbb12.dll','TBBMALLOC.DLL','C:\\outside\\tbbbind_2_5.dll','/elsewhere/libtbb.dll'):
            self.assertTrue(self.helper._luma_is_tbb_dll(name),name)
        for name in ('tbbpool.cp312-win_amd64.pyd','kernel32.dll','not-tbb.dll'):
            self.assertFalse(self.helper._luma_is_tbb_dll(name),name)
        with patch.dict(sys.modules,{'numba.np.ufunc.tbbpool':object()}):
            with self.assertRaisesRegex(ValueError,'TBB module'): self.helper._luma_assert_no_tbb()

    def test_classifier_source_anchor_matches_committed_lock(self):
        root = SOURCE.parents[3]/'scripts/asr-components'
        lock = json.loads((root/'locks/qwen3-asr-cpu-windows-x64.json').read_text())
        wheel = next(item for item in lock['wheels'] if item['name']=='numba')
        self.assertEqual({key:wheel[key] for key in native.NUMBA_WHEEL},native.NUMBA_WHEEL)


if __name__ == '__main__': unittest.main()
