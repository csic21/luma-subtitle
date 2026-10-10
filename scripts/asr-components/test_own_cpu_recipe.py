"""Dependency-free synthetic proof tests. No fixture is a shipping pin."""
import copy
import hashlib
import io
import os
import stat
import struct
import sys
import json
from pathlib import Path
import shutil
import subprocess
import unittest
from unittest.mock import patch
import zipfile

from fixture_paths import temporary_root
import own_cpu_recipe as cpu
import generate_recipes as recipes
import measure_recipe_caps as caps
import build
import smoke

APP_SOURCE = 'b' * 40
COMPONENT_SOURCE = 'a' * 40


def encoded(value):
    return (json.dumps(value, indent=2, sort_keys=True) + '\n').encode('utf-8')


def pin(data):
    return {'bytes': len(data), 'sha256': hashlib.sha256(data).hexdigest()}


class SyntheticPublishedCpu:
    def __init__(self, directory):
        self.root = directory / 'repo/scripts/asr-components'
        self.root.mkdir(parents=True)
        self.cache = directory / 'cache'; self.cache.mkdir()
        self.runtime = directory / 'runtime'; self.runtime.mkdir()
        self.source = json.loads((cpu.ROOT / 'ct2-cpu/sources.lock.json').read_text(encoding='utf-8'))
        self.notices = json.loads((cpu.ROOT / 'ct2-cpu/notices.lock.json').read_text(encoding='utf-8'))
        self.python = json.loads((cpu.ROOT / 'packs.json').read_text(encoding='utf-8'))['runtime']['windows-x64']
        for name in ('ct2-cpu/sources.lock.json', 'ct2-cpu/notices.lock.json', 'ct2-cpu/verify_runtime.py',
                     'own_cpu_recipe.py', 'self_test.py', 'licenses.lock.json', 'engine-terms.lock.json', 'packs.json'):
            target = self.root / name; target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(cpu.ROOT / name, target)
        shutil.copytree(cpu.ROOT / 'licenses', self.root / 'licenses')
        shutil.copytree(cpu.ROOT / 'ct2-cpu/licenses', self.root / 'ct2-cpu/licenses')
        self.worker = self.root.parent.parent / 'src-tauri/src/asr/worker.py'
        self.worker.parent.mkdir(parents=True)
        shutil.copyfile(cpu.ROOT.parent.parent / 'src-tauri/src/asr/worker.py', self.worker)
        constants = subprocess.run(['node', '-e', "const p=require(process.argv[1]);process.stdout.write(JSON.stringify({exports:p.CT2_SOURCE_EXPORT,strategy:p.BUILD_STRATEGY,checks:p.CHECKS}));",
                                    str(cpu.ROOT / 'ct2-cpu/publish.cjs')], capture_output=True, text=True, check=True)
        constants = json.loads(constants.stdout)
        self.provenance = {'schema': 1, 'variant': 'luma-cpu-seq-1', 'luma_source_sha': COMPONENT_SOURCE,
            'publication_authorized': False, 'upstream': self.source['sources'], 'build_wheels': self.source['build_wheels'],
            'ct2_cmake': self.source['ct2_cmake'], 'onednn_cmake': self.source['onednn_cmake'],
            'notices': self.notices, 'source_date_epoch': self.source['source_date_epoch'],
            'build_strategy': constants['strategy'], 'python_runtime': self.python,
            'native_compile_flags': r'/Brepro /Z7 /experimental:deterministic /pathmap:<build-root>=C:\luma-ct2-build',
            'native_link_flags': '/Brepro /INCREMENTAL:NO',
            'toolchain': {**self.source['toolchain'], 'binary_hashes': {
                name: {'sha256': 'c' * 64} for name in ('cl.exe', 'link.exe', 'lib.exe', 'rc.exe', 'mt.exe')}}}
        members = {'ctranslate2/ctranslate2.dll': b'synthetic DLL, never execute',
                   'ctranslate2/_ext.cp312-win_amd64.pyd': b'synthetic PYD, never execute',
                   'ctranslate2/__init__.py': b'',
                   'ctranslate2-4.8.2.dist-info/LUMA_CPU_BUILD.json': encoded(self.provenance)}
        archive = directory / cpu.WHEEL
        with zipfile.ZipFile(archive, 'w') as wheel:
            for name, data in members.items():
                wheel.writestr(name, data)
                installed = self.runtime / 'Lib/site-packages' / name
                installed.parent.mkdir(parents=True, exist_ok=True); installed.write_bytes(data)
        self.wheel = dict(name='ctranslate2', version='4.8.2', filename=cpu.WHEEL, url=cpu.PREFIX + cpu.WHEEL,
                          **pin(archive.read_bytes()))
        shutil.copyfile(archive, self.cache / self.wheel['sha256'])
        self.proof = {'schema': 1, 'source_sha': COMPONENT_SOURCE, 'variant': 'luma-cpu-seq-1',
            'publication_authorized': False, 'provenance': self.provenance,
            'assets': sorted([{'name': cpu.WHEEL, **pin(archive.read_bytes())},
                             {'name': cpu.SOURCES, **pin(b'synthetic source')},
                             {'name': cpu.NOTICES, **pin(b'synthetic notices')}], key=lambda item: item['name']),
            'source_exports': [constants['exports']], 'checks': dict.fromkeys(constants['checks'], True),
            # Publication binds Git-blob LF bytes, not autocrlf checkout bytes.
            'locks': {key: {'name': key + '.lock.json', **pin((self.root / ('ct2-cpu/' + key + '.lock.json')).read_bytes().replace(b'\r\n', b'\n'))}
                      for key in ('sources', 'notices')}}
        self.lock = {'platform': 'windows-x64', 'python': '3.12', 'wheels': [self.wheel],
                     'cpu_component': {'source_sha': COMPONENT_SOURCE}}
        self.save_proof()

    def save_proof(self):
        raw = encoded(self.proof)
        self.lock['cpu_component']['proof'] = {'url': cpu.PREFIX + cpu.PROOF, **pin(raw)}
        self.path = self.cache / pin(raw)['sha256']; self.path.write_bytes(raw)
        (self.root / 'locks').mkdir(exist_ok=True)
        (self.root / 'locks' / (cpu.PACK_ID + '.json')).write_bytes(encoded(self.lock))

    def component(self):
        return cpu.published_component(self.lock, self.path, root=self.root)


class OwnCpuProofTests(unittest.TestCase):
    def setUp(self):
        self.directory = self.enterContext(temporary_root())
        self.fixture = SyntheticPublishedCpu(self.directory)

    def checkout_fixture(self, name, newline):
        original_copy = shutil.copyfile

        def copy_lock_with_checkout_newlines(source, target, *args, **kwargs):
            result = original_copy(source, target, *args, **kwargs)
            if Path(target).as_posix().endswith(('ct2-cpu/sources.lock.json', 'ct2-cpu/notices.lock.json')):
                path = Path(target)
                path.write_bytes(path.read_bytes().replace(b'\r\n', b'\n').replace(b'\n', newline))
            return result

        # Exercise fixture construction after checkout conversion, not merely
        # a converted lock after an LF proof was already constructed.
        with patch.object(shutil, 'copyfile', side_effect=copy_lock_with_checkout_newlines):
            return SyntheticPublishedCpu(self.directory / name)

    def test_synthetic_lock_identity_matches_git_blob_for_lf_and_crlf_checkouts(self):
        fixtures = [self.checkout_fixture('lf', b'\n'), self.checkout_fixture('crlf', b'\r\n')]
        self.assertEqual(fixtures[0].proof['locks'], fixtures[1].proof['locks'])
        for fixture in fixtures:
            with self.subTest(checkout=fixture.root):
                self.assertEqual(fixture.component()['component_source_sha'], COMPONENT_SOURCE)
                for key in ('sources', 'notices'):
                    data = (fixture.root / ('ct2-cpu/' + key + '.lock.json')).read_bytes()
                    self.assertEqual(fixture.proof['locks'][key],
                                     {'name': key + '.lock.json', **pin(data.replace(b'\r\n', b'\n'))})
        for key in ('sources', 'notices'):
            crlf = (fixtures[1].root / ('ct2-cpu/' + key + '.lock.json')).read_bytes()
            self.assertIn(b'\r\n', crlf)
            self.assertNotEqual(pin(crlf), pin(crlf.replace(b'\r\n', b'\n')))

    def test_lock_content_changes_still_fail_for_lf_and_crlf_checkouts(self):
        for name, newline in [('lf-tamper', b'\n'), ('crlf-tamper', b'\r\n')]:
            fixture = self.checkout_fixture(name, newline)
            for key in ('sources', 'notices'):
                with self.subTest(checkout=name, lock=key):
                    path = fixture.root / ('ct2-cpu/' + key + '.lock.json')
                    original = path.read_bytes()
                    altered = original.replace(b'"schema": 1', b'"schema": 2', 1)
                    self.assertNotEqual(original, altered)
                    self.assertEqual(len(original), len(altered))
                    path.write_bytes(altered)
                    try:
                        with self.assertRaisesRegex(ValueError, 'publication validator'):
                            fixture.component()
                    finally:
                        path.write_bytes(original)
                    self.assertEqual(fixture.component()['component_source_sha'], COMPONENT_SOURCE)

    def test_original_upstream_lock_is_not_a_shipping_cpu_pin(self):
        lock = copy.deepcopy(self.fixture.lock)
        lock['wheels'][0].update(filename='ctranslate2-4.8.2-cp312-cp312-win_amd64.whl',
                                 url='https://files.pythonhosted.org/packages/4e/23/e3b5322ff7368fcbed181ea4c209149416e7940b5b04971d5ee4084afe1a/ctranslate2-4.8.2-cp312-cp312-win_amd64.whl')
        with self.assertRaisesRegex(ValueError, 'own CPU wheel'):
            cpu.publication_pin(lock)
        (self.fixture.root / 'locks' / (cpu.PACK_ID + '.json')).write_bytes(encoded(lock))
        with patch.object(build, 'ROOT', self.fixture.root), patch.object(build, 'fetch', side_effect=AssertionError('must fail before fetch')):
            with self.assertRaisesRegex(ValueError, 'own CPU wheel'):
                build.build(cpu.PACK_ID, self.directory / 'output', self.directory / 'cache', APP_SOURCE, native=False)

    def test_exact_local_proof_uses_existing_release_validator(self):
        self.assertEqual(self.fixture.component()['component_source_sha'], COMPONENT_SOURCE)
        for change in ('native_inference', 'wheel_reproduced', 'whole_runtime_closure'):
            with self.subTest(change=change):
                self.fixture.proof['checks'][change] = False; self.fixture.save_proof()
                with self.assertRaisesRegex(ValueError, 'publication validator'):
                    self.fixture.component()
                self.fixture.proof['checks'][change] = True

    def test_missing_altered_wrong_wheel_and_source_fail_closed(self):
        original = copy.deepcopy(self.fixture.proof)
        with self.assertRaisesRegex(ValueError, 'required'):
            cpu.published_component(self.fixture.lock, None, root=self.fixture.root)
        self.fixture.path.write_bytes(b'altered')
        with self.assertRaisesRegex(ValueError, 'size-pinned'):
            self.fixture.component()
        for name in ('source', 'wheel', 'provenance', 'diagnostic'):
            self.fixture.proof = copy.deepcopy(original)
            if name == 'source': self.fixture.proof['source_sha'] = 'd' * 40
            elif name == 'wheel':
                next(item for item in self.fixture.proof['assets'] if item['name'] == cpu.WHEEL)['sha256'] = 'd' * 64
            elif name == 'provenance': self.fixture.proof['provenance']['luma_source_sha'] = 'd' * 40
            else:
                self.fixture.proof['kind'] = 'diagnostic'; self.fixture.proof['checks']['native_inference'] = False
            self.fixture.save_proof()
            with self.subTest(name=name), self.assertRaises(ValueError): self.fixture.component()

    def test_wheel_origin_and_proof_pin_have_no_generic_github_exception(self):
        for field, value in [('url', cpu.PREFIX + cpu.WHEEL + '?other=1'), ('filename', 'other.whl'),
                             ('version', '4.8.3'), ('bytes', True), ('sha256', 'MAIN')]:
            lock = copy.deepcopy(self.fixture.lock); lock['wheels'][0][field] = value
            with self.subTest(field=field), self.assertRaises(ValueError): cpu.publication_pin(lock)
        lock = copy.deepcopy(self.fixture.lock); lock['cpu_component']['proof']['url'] = 'https://github.com/other/proof.json'
        with self.assertRaises(ValueError): cpu.publication_pin(lock)

    def test_own_policy_requires_pinned_installed_bytes_and_current_worker(self):
        self.enterContext(patch('sys.platform', 'win32'))
        f = self.fixture; component = f.component()
        with patch.object(cpu, 'ROOT', f.root):
            output = self.directory / 'prepared.py'
            policy = cpu.prepare_published_worker(f.worker, f.runtime, f.cache, component, APP_SOURCE, output)
            self.assertEqual(policy['source_sha'], APP_SOURCE)
            self.assertEqual(policy['component_source_sha'], COMPONENT_SOURCE)
            self.assertFalse(policy['managed_receipt_selection_tested'])
            self.assertFalse(policy['global_managed_path_selection_tested'])
            self.assertIn('LUMA_MANAGED_CT2_CPU_POLICY = "luma-cpu-seq-1"', output.read_text(encoding='utf-8'))
            self.assertEqual(policy['prepared_worker_sha256'], build.sha256(output))
            original = f.runtime / 'Lib/site-packages/ctranslate2/ctranslate2.dll'
            original.write_bytes(b'changed DLL')
            with self.assertRaisesRegex(ValueError, 'differs'):
                cpu.prepare_published_worker(f.worker, f.runtime, f.cache, component, APP_SOURCE, self.directory / 'reject.py')
            self.assertFalse((self.directory / 'reject.py').exists())

    def test_reference_optional_source_requires_shipping_proof_default_stays_single_source(self):
        self.enterContext(patch('sys.platform', 'win32'))
        f = self.fixture
        with patch.object(cpu, 'ROOT', f.root): verifier = cpu.source_verifier()
        installed = f.runtime / 'Lib/site-packages/ctranslate2-4.8.2.dist-info/LUMA_CPU_BUILD.json'
        args = (f.worker, f.runtime, installed, build.sha256(installed))
        _, policy = verifier.load_source_proof_worker(*args, COMPONENT_SOURCE)
        self.assertNotIn('component_source_sha', policy)
        self.assertEqual(policy['source_sha'], COMPONENT_SOURCE)
        with self.assertRaisesRegex(ValueError, 'source proof|Source proof'):
            verifier.load_source_proof_worker(*args, APP_SOURCE)
        with self.assertRaisesRegex(ValueError, 'required'):
            verifier.load_source_proof_worker(*args, APP_SOURCE, component_source_sha=COMPONENT_SOURCE)
        _, policy = verifier.load_source_proof_worker(*args, APP_SOURCE, component_source_sha=COMPONENT_SOURCE, publication_proof=f.path)
        self.assertEqual(policy['source_sha'], APP_SOURCE); self.assertEqual(policy['component_source_sha'], COMPONENT_SOURCE)
        with self.assertRaisesRegex(ValueError, 'differs'):
            verifier.load_source_proof_worker(*args, APP_SOURCE, component_source_sha='d' * 40, publication_proof=f.path)

    def test_own_caps_require_exact_proof_and_actual_measurement(self):
        f = self.fixture; component = f.component()
        caps.validate_source(f.wheel, 'wheel', component)
        with self.assertRaisesRegex(ValueError, 'publication proof'): caps.validate_source(f.wheel, 'wheel')
        bad = dict(f.wheel, sha256='d' * 64)
        with self.assertRaisesRegex(ValueError, 'publication proof'): caps.validate_source(bad, 'wheel', component)
        for url in (cpu.PREFIX + 'other.whl', cpu.PREFIX + cpu.WHEEL + '#ignored', 'https://github.com/other/release/' + cpu.WHEEL):
            with self.assertRaises(ValueError): caps.validate_source(dict(f.wheel, url=url), 'wheel', component)
        config = {'packs': [{'id': cpu.PACK_ID, 'platform': 'windows-x64'}], 'runtime': {'windows-x64': f.python}}
        (f.root / 'packs.json').write_bytes(encoded(config))
        with patch.object(caps, 'ROOT', f.root), patch.object(caps, 'verified_input', side_effect=lambda item, cache, kind, component=None:
                self.directory / 'runtime.tar.gz' if kind == 'runtime' else caps.Path(cache) / item['sha256']), \
             patch.object(caps, 'runtime_measurements', return_value={'installed_bytes': 100, 'max_files': 2, 'normalized_links': 0}):
            result = caps.measure(f.cache, allow_unmeasured=True)
            self.assertEqual(result[cpu.PACK_ID]['wheels'][0]['count_kind'], 'measured')
            (f.cache / f.wheel['sha256']).unlink()
            with self.assertRaises(FileNotFoundError): caps.measure(f.cache, allow_unmeasured=True)

    def test_reference_smoke_refuses_manifest_mismatch_before_preparing_policy(self):
        self.enterContext(patch('sys.platform', 'win32'))
        f = self.fixture
        manifest = {'id': cpu.PACK_ID, 'platform': 'windows-x64', 'backend': 'faster-whisper', 'source_sha': APP_SOURCE}
        with patch.object(smoke, 'ROOT', f.root), patch.object(cpu, 'ROOT', f.root):
            with self.assertRaisesRegex(ValueError, 'manifest differs'):
                smoke.prepare_cpu_smoke(manifest, f.runtime, f.worker, f.cache, self.directory / 'reject.py')
            component = f.component(); manifest['component_provenance'] = {key: value for key, value in component.items() if key != 'proof'}
            _, policy = smoke.prepare_cpu_smoke(manifest, f.runtime, f.worker, f.cache, self.directory / 'accepted.py')
            self.assertEqual(policy['component_source_sha'], COMPONENT_SOURCE)
            self.assertEqual(policy['source_sha'], APP_SOURCE)


    def test_generator_preserves_other_wheels_python_terms_and_disabled_state(self):
        f = self.fixture
        original = json.loads((cpu.ROOT / 'locks' / (cpu.PACK_ID + '.json')).read_text(encoding='utf-8'))
        config = json.loads((cpu.ROOT / 'packs.json').read_text(encoding='utf-8'))
        catalog = json.loads((cpu.ROOT.parent.parent / 'src-tauri/resources/asr/catalog.json').read_text(encoding='utf-8'))
        catalog['runtimes'] = [item for item in catalog['runtimes'] if item['id'] == cpu.PACK_ID]
        bounds = json.loads((cpu.ROOT / 'recipe-caps.json').read_text(encoding='utf-8'))
        f.lock.update(original); f.lock['wheels'] = [f.wheel if item['name'] == 'ctranslate2' else item for item in original['wheels']]
        f.lock['cpu_component'] = {'source_sha': COMPONENT_SOURCE}
        f.save_proof()
        index = next(index for index, item in enumerate(bounds[cpu.PACK_ID]['wheels']) if item['name'] == 'ctranslate2')
        bounds[cpu.PACK_ID]['wheels'][index].update(f.wheel, count_kind='measured')
        terms = cpu.ROOT.parent.parent / 'src-tauri/resources/asr/terms'
        proprietary = json.loads((terms / 'sources.json').read_text(encoding='utf-8'))
        with patch.object(recipes, 'ROOT', f.root):
            result = recipes.generate(catalog, bounds, config, proprietary, terms, f.path)
            runtime = result['runtimes'][0]
            self.assertTrue(runtime['unavailable_reason']); self.assertEqual(result['status'], 'review-only-not-active')
            self.assertNotIn('component_provenance', runtime)  # Strict Rust Runtime schema is unchanged.
            self.assertEqual(result['component_provenance'][cpu.PACK_ID]['component_source_sha'], COMPONENT_SOURCE)
            self.assertEqual(len(runtime['recipe']['wheels']), 22)
            self.assertEqual(runtime['recipe']['python'], {key: bounds[cpu.PACK_ID]['runtime'][key] for key in recipes.PYTHON_FIELDS})
            for old, new in zip(original['wheels'], runtime['recipe']['wheels']):
                if old['name'] != 'ctranslate2':
                    self.assertEqual({key: old[key] for key in cpu.IDENTITY_FIELDS}, {key: new[key] for key in cpu.IDENTITY_FIELDS})
            disclosure = next(term for term in runtime['recipe']['terms'] if term['id'].endswith('-dependency-inventory'))
            self.assertEqual(disclosure['url'], 'https://pypi.org/project/faster-whisper/1.2.1/')
            self.assertEqual(runtime['recipe']['windows_crt'], 'msvc-14.44.35211-x64')
            self.assertTrue(any(term['id'] == 'microsoft-vc-runtime-en' for term in runtime['recipe']['terms']))
            with self.assertRaisesRegex(ValueError, 'required'):
                recipes.generate(catalog, bounds, config, proprietary, terms)
            bounds[cpu.PACK_ID]['wheels'][index]['count_kind'] = 'conservative_cap'
            with self.assertRaisesRegex(ValueError, 'measured'):
                recipes.generate(catalog, bounds, config, proprietary, terms, f.path)

    def test_reference_inference_keeps_both_sources_and_requires_cleanup_exit_evidence(self):
        f = self.fixture; component = f.component(); component['proof_path'] = f.path
        model = self.directory / 'model'; model.mkdir()
        switched = self.directory / 'model switch 子'; switched.mkdir()
        provenance = f.runtime / 'Lib/site-packages/ctranslate2-4.8.2.dist-info/LUMA_CPU_BUILD.json'
        report = {'passed': True, 'inference': {'cold_and_warm': True}, 'host_crt_fallback_allowed': False,
                  'offline_audit_enabled': True, 'isolated': True, 'system_python_used': False,
                  'inherited_python_path_ignored': True, 'loaded_modules': [{'scope': 'private', 'path': 'ctranslate2.dll'}],
                  'host_auditor': {'source_sha': APP_SOURCE, 'sha256': build.sha256(f.root / 'self_test.py')},
                  'verifier': {'source_sha': APP_SOURCE, 'sha256': build.sha256(f.root / 'ct2-cpu/verify_runtime.py')},
                  'cpu_thread_policy': {'source_sha': APP_SOURCE, 'component_source_sha': COMPONENT_SOURCE,
                                       'cpu_threads': 1, 'variant': 'luma-cpu-seq-1',
                                       'managed_receipt_selection_tested': False, 'constructor_overridden': False,
                                       'build_provenance_sha256': build.sha256(provenance),
                                       'worker_sha256': hashlib.sha256(f.worker.read_text(encoding='utf-8').encode('utf-8')).hexdigest()},
                  'source_worker_lifecycle': dict.fromkeys(('eof_after_inference', 'model_switch_after_inference', 'explicit_unload_after_inference'), True)}
        def child(command, env, cwd, **kwargs):
            self.assertEqual(command[1:7], ['-I', '-B', '-u', '-X', 'utf8', str(f.root / 'ct2-cpu/verify_runtime.py')])
            self.assertEqual(command[command.index('--source-sha') + 1], APP_SOURCE)
            self.assertEqual(command[command.index('--component-source-sha') + 1], COMPONENT_SOURCE)
            self.assertEqual(command[command.index('--publication-proof') + 1], str(f.path))
            self.assertEqual(kwargs['timeout'], 900)
            self.assertEqual(kwargs['output_limit'], 1_000_000)
            Path(command[command.index('--report') + 1]).write_bytes(encoded(report))
            return 0, '', ''
        args = (f.runtime / 'python.exe', f.worker, f.runtime, model, switched, self.directory / 'audio.wav', {}, self.directory,
                {'source_sha': APP_SOURCE}, component)
        with patch.object(smoke, 'ROOT', f.root), patch('windows_crt_proof.helper_process', side_effect=child):
            result = smoke.cpu_reference_inference(*args)
            self.assertTrue(result['normal_process_exit'])
            self.assertFalse(result['managed_receipt_selection_tested']); self.assertFalse(result['global_managed_path_selection_tested'])
            for key in report['source_worker_lifecycle']:
                report['source_worker_lifecycle'][key] = False
                with self.subTest(key=key), self.assertRaisesRegex(ValueError, 'incomplete'):
                    smoke.cpu_reference_inference(*args)
                report['source_worker_lifecycle'][key] = True
            report['loaded_modules'].append({'scope': 'private', 'path': 'libgomp-1.dll'})
            with self.assertRaisesRegex(ValueError, 'forbidden native'):
                smoke.cpu_reference_inference(*args)
            report['loaded_modules'].pop()
            report['cpu_thread_policy']['component_source_sha'] = APP_SOURCE
            with self.assertRaisesRegex(ValueError, 'source-mismatched'): smoke.cpu_reference_inference(*args)
        with patch.object(smoke, 'ROOT', f.root), patch('windows_crt_proof.helper_process', return_value=(9, '', 'failed cleanup')):
            with self.assertRaisesRegex(RuntimeError, 'cleanup failed'): smoke.cpu_reference_inference(*args)
        def oversized(command, *unused, **kwargs):
            Path(command[command.index('--report') + 1]).write_bytes(b'x' * 4_000_001)
            return 0, '', ''
        with patch.object(smoke, 'ROOT', f.root), patch('windows_crt_proof.helper_process', side_effect=oversized):
            with self.assertRaisesRegex(ValueError, 'metadata bound'): smoke.cpu_reference_inference(*args)



class WheelArchiveBoundaryTests(unittest.TestCase):
    def setUp(self):
        self.directory = self.enterContext(temporary_root())
        self.fixture = SyntheticPublishedCpu(self.directory)

    def wheel_bytes(self, extra):
        original = (self.fixture.cache / self.fixture.wheel['sha256']).read_bytes()
        output = io.BytesIO()
        with zipfile.ZipFile(io.BytesIO(original)) as source, zipfile.ZipFile(output, 'w') as target:
            for item in source.infolist(): target.writestr(item, source.read(item))
            for name, data in extra: target.writestr(name, data)
        return output.getvalue()

    def reject(self, raw):
        f = self.fixture; wheel = dict(f.wheel, **pin(raw))
        archive = f.cache / wheel['sha256']; archive.write_bytes(raw)
        with self.assertRaises((ValueError, zipfile.BadZipFile)):
            cpu.verify_installed_wheel(f.runtime, wheel, f.cache, f.provenance)
        with self.assertRaises((ValueError, zipfile.BadZipFile)):
            caps.wheel_measurements(archive)

    def test_actual_raw_nul_backslash_and_unsafe_ignored_members_are_rejected(self):
        for before, after in [(b'ctranslate2/evil.pyXsuffix', b'ctranslate2/evil.py\x00suffix'),
                              (b'ctranslate2/evil.py', b'ctranslate2\\evil.py'),
                              (b'ignored/evil.pyXsuffix', b'ignored/evil.py\x00suffix')]:
            raw = self.wheel_bytes([(before.decode('ascii'), b'evil')])
            self.assertEqual(raw.count(before), 2)
            # Patch actual local and central bytes; ZipInfo construction alone
            # would silently truncate a test's NUL before writing the fixture.
            raw = raw.replace(before, after)
            with self.subTest(raw=after): self.reject(raw)
        for name in ('../ignored.txt', '/absolute.txt', 'ignored/../../escape.py', 'ignored//empty.txt', 'ignored/./dot.txt', 'ignored:stream.txt'):
            with self.subTest(name=name): self.reject(self.wheel_bytes([(name, b'ignored')]))

    def test_ignored_local_header_name_cannot_differ_from_safe_central_name(self):
        name = b'ignored/safe.txt'; raw = self.wheel_bytes([(name.decode(), b'ignored')])
        replacement = b'../' + b'x' * (len(name) - 3)
        self.assertEqual(raw.count(name), 2)
        self.reject(raw.replace(name, replacement, 1))

    def test_case_prefix_and_windows_trailing_aliases_are_rejected(self):
        for extra in [[('CTRANSLATE2/__init__.py', b'')],
                      [('ctranslate2/__INIT__.py', b'')],
                      [('ignored', b'file'), ('ignored/nested.py', b'file')],
                      [('ignored/A.py', b''), ('IGNORED/b.py', b'')],
                      [('ignored/trailing. ', b'')]]:
            with self.subTest(extra=extra): self.reject(self.wheel_bytes(extra))

    def test_rust_destination_devices_controls_ascii_and_path_limits_are_preserved(self):
        for name in ('ignored/CON.txt', 'ctranslate2/NUL.py', 'ignored/CLOCK$.txt', 'ignored/COM0.py',
                     'ignored/lpt9.txt', 'ignored/x\x7fy.py', 'ignored/測試.txt', 'a/' * 32 + 'file.txt', 'x' * 2049):
            with self.subTest(name=name): self.reject(self.wheel_bytes([(name, b'ignored')]))

    def test_unknown_link_and_malformed_extra_metadata_cannot_hide_in_ignored_entries(self):
        for extra in (struct.pack('<HH', 0x000d, 4) + b'link', struct.pack('<HH', 0x756e, 0), b'x'):
            info = zipfile.ZipInfo('ignored.txt'); info.extra = extra
            with self.subTest(extra=extra): self.reject(self.wheel_bytes([(info, b'ignored')]))
        info = zipfile.ZipInfo('ignored.txt'); info.extra = struct.pack('<HH', 0x5455, 5) + b'\x01' + b'\x00' * 4
        raw = self.wheel_bytes([(info, b'ignored')])
        # Change only the local timestamp tag into a forbidden link tag.
        position = raw.index(b'ignored.txt') + len(b'ignored.txt')
        raw = raw[:position] + struct.pack('<H', 0x000d) + raw[position + 2:]
        self.reject(raw)

    def test_ignored_symlink_directory_mismatch_and_encryption_are_rejected(self):
        for mode, flags in [(stat.S_IFLNK | 0o777, 0), (stat.S_IFREG | 0o644, 0x10)]:
            item = zipfile.ZipInfo('ignored.txt'); item.external_attr = mode << 16 | flags
            self.reject(self.wheel_bytes([(item, b'ignored')]))
        raw = bytearray(self.wheel_bytes([('ignored.txt', b'ignored')]))
        # Encrypt only the ignored entry's local and central records.
        for signature, flag_offset, name_offset in ((b'PK\x03\x04', 6, 30), (b'PK\x01\x02', 8, 46)):
            start = 0
            while True:
                index = raw.find(signature, start)
                if index < 0: break
                if raw[index + name_offset:index + name_offset + 11] == b'ignored.txt':
                    bits = struct.unpack_from('<H', raw, index + flag_offset)[0]
                    struct.pack_into('<H', raw, index + flag_offset, bits | 1)
                start = index + 4
        self.reject(bytes(raw))

    def test_preflight_enforces_member_count_and_expansion_before_payload_reads(self):
        raw = self.wheel_bytes([('ignored.txt', b'ignored')])
        with zipfile.ZipFile(io.BytesIO(raw)) as archive:
            baseline = dict(max_files=2000, max_member_bytes=100_000_000, max_total_bytes=100_000_000)
            for field, value in [('max_files', 1), ('max_member_bytes', 1), ('max_total_bytes', 1)]:
                with self.subTest(field=field), patch.object(archive, 'open', side_effect=AssertionError('must reject before payload selection')):
                    with self.assertRaises(ValueError): cpu.reviewed_wheel_members(archive, **dict(baseline, **{field: value}))
        # The own-wheel entrypoint imposes the real 2,000-member source policy.
        raw = self.wheel_bytes([(f'ignored/{i}.txt', b'') for i in range(2001)])
        f = self.fixture; wheel = dict(f.wheel, **pin(raw)); (f.cache / wheel['sha256']).write_bytes(raw)
        with self.assertRaisesRegex(ValueError, 'member count'):
            cpu.verify_installed_wheel(f.runtime, wheel, f.cache, f.provenance)

    def test_legitimate_forward_slash_wheel_survives_windows_name_handling(self):
        raw = self.wheel_bytes([('ignored/', b''), ('ignored/nested.txt', b'')])
        with patch.object(zipfile.os, 'sep', '\\'), patch.object(zipfile.os, 'altsep', '/'):
            with zipfile.ZipFile(io.BytesIO(raw)) as archive:
                members = cpu.reviewed_wheel_members(archive, max_files=2000, max_member_bytes=100_000_000, max_total_bytes=100_000_000)
                self.assertTrue(all(item.orig_filename == item.filename for item in members))
                self.assertIn('ignored/nested.txt', [item.filename for item in members])

    def test_exact_installed_tree_rejects_shadow_package_and_all_generated_extras(self):
        f = self.fixture
        cpu.verify_installed_wheel(f.runtime, f.wheel, f.cache, f.provenance)
        for name in ('_ext/__init__.py', 'unexpected.py', '__pycache__/__init__.cpython-312.pyc'):
            extra = f.runtime / 'Lib/site-packages/ctranslate2' / name
            extra.parent.mkdir(parents=True, exist_ok=True); extra.write_bytes(b'not anchored')
            with self.subTest(name=name), self.assertRaisesRegex(ValueError, 'unexpected'):
                cpu.verify_installed_wheel(f.runtime, f.wheel, f.cache, f.provenance)
            extra.unlink()
            if extra.parent.name in ('_ext', '__pycache__'): extra.parent.rmdir()
        cpu.verify_installed_wheel(f.runtime, f.wheel, f.cache, f.provenance)

    def test_lstat_reparse_policy_rejects_cache_and_installed_ancestors(self):
        from types import SimpleNamespace
        f = self.fixture; original = Path.lstat
        for blocked in (f.cache, f.runtime / 'Lib/site-packages/ctranslate2'):
            def lstat(path, *args, **kwargs):
                info = original(path, *args, **kwargs)
                if path == blocked: return SimpleNamespace(st_mode=info.st_mode, st_file_attributes=0x400)
                return info
            with self.subTest(blocked=blocked), patch.object(Path, 'lstat', lstat):
                with self.assertRaisesRegex(ValueError, 'reparse'):
                    cpu.verify_installed_wheel(f.runtime, f.wheel, f.cache, f.provenance)


class BoundedReferenceChildTests(unittest.TestCase):
    def test_explicit_900_second_timeout_kills_and_reaps_owned_child(self):
        import windows_crt_proof as crt
        created = []; original = subprocess.Popen
        def launch(*args, **kwargs):
            child = original(*args, **kwargs); created.append(child); return child
        with temporary_root() as root, patch.object(crt.subprocess, 'Popen', side_effect=launch), \
             patch.object(crt.time, 'monotonic', side_effect=[0, 901]):
            with self.assertRaisesRegex(TimeoutError, 'timed out'):
                crt.helper_process([sys.executable, '-I', '-B', '-c', 'import time; time.sleep(60)'],
                                   None, root, timeout=900, output_limit=1_000_000)
        self.assertEqual(len(created), 1); self.assertIsNotNone(created[0].returncode)

    def test_combined_output_bound_and_interruption_both_kill_and_reap(self):
        import windows_crt_proof as crt
        for interruption in (False, True):
            created = []; original = subprocess.Popen
            def launch(*args, **kwargs):
                child = original(*args, **kwargs); created.append(child); return child
            with temporary_root() as root, patch.object(crt.subprocess, 'Popen', side_effect=launch):
                if interruption:
                    with patch.object(crt.time, 'sleep', side_effect=KeyboardInterrupt), self.assertRaises(KeyboardInterrupt):
                        crt.helper_process([sys.executable, '-I', '-B', '-c', 'import time; time.sleep(60)'],
                                           None, root, timeout=900, output_limit=1024)
                else:
                    command = [sys.executable, '-I', '-B', '-c', 'import os,time; os.write(1,b"x"*700); os.write(2,b"y"*700); time.sleep(60)']
                    with self.assertRaisesRegex(ValueError, 'output bound'):
                        crt.helper_process(command, None, root, timeout=900, output_limit=1024)
                self.assertEqual(list(root.iterdir()), [])
            self.assertEqual(len(created), 1); self.assertIsNotNone(created[0].returncode)

    def test_invalid_limits_never_launch_a_child(self):
        import windows_crt_proof as crt
        with patch.object(crt.subprocess, 'Popen', side_effect=AssertionError('must not launch')):
            for limits in ({'timeout': 901}, {'timeout': True}, {'output_limit': 0}, {'output_limit': 4_000_001}):
                with self.subTest(limits=limits), self.assertRaisesRegex(ValueError, 'limits'):
                    crt.helper_process([], None, None, **limits)


class StrictCpuClosureTests(unittest.TestCase):
    def test_own_cpu_rejects_forbidden_extras_and_normal_delay_imports(self):
        for name in ('cudnn64_9.dll', 'cufft64.dll', 'cusparse64.dll', 'cuda.dll', 'mkl_rt.dll',
                     'libiomp5md.dll', 'cupti64.dll', 'nvToolsExt64.dll', 'libgomp-1.dll', 'vcomp140.dll', 'libomp.dll', 'tbb12.dll'):
            for mode in ('file', 'normal', 'delay'):
                items = [{'path': name if mode == 'file' else 'test.pyd', 'normal': [], 'delay': []}]
                if mode != 'file':
                    items[0][mode] = [name]
                    items.append({'path': name, 'normal': [], 'delay': []})
                with self.subTest(name=name, mode=mode): self.assertFalse(cpu.strict_cpu_closure(items)['passed'])
        normal = [{'path': 'test.pyd', 'normal': ['msvcp140.dll', 'msvcp140_1.dll', 'kernel32.dll'], 'delay': []},
                  {'path': 'msvcp140.dll', 'normal': [], 'delay': []}, {'path': 'msvcp140_1.dll', 'normal': [], 'delay': []}]
        self.assertTrue(cpu.strict_cpu_closure(normal)['passed'])
        self.assertFalse(cpu.strict_cpu_closure(normal[:1])['passed'])

    def test_own_wheel_cannot_restore_obsolete_native_pruning(self):
        with temporary_root() as root:
            (root / 'pruning.json').write_bytes(encoded({'packs': {cpu.PACK_ID: {'files': []}}}))
            with patch.object(build, 'ROOT', root), self.assertRaisesRegex(ValueError, 'never undergo'):
                build.prune_reviewed_files(root, cpu.PACK_ID)
        self.assertNotIn(cpu.PACK_ID, json.loads((build.ROOT / 'pruning.json').read_text(encoding='utf-8'))['packs'])


if __name__ == '__main__': unittest.main()
