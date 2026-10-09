import copy
import gzip
import io
import json
from pathlib import Path
import shutil
import tempfile
import tarfile
import unittest
from unittest.mock import patch
from types import SimpleNamespace
import zipfile

import package_publication as package

SOURCE = 'a' * 40
REAL_HERE = package.HERE
REAL_REPO = package.REPO


def source_tar(entries):
    data = io.BytesIO()
    with tarfile.open(fileobj=data, mode='w', format=tarfile.PAX_FORMAT) as archive:
        root = tarfile.TarInfo(package.CT2_ROOT); root.type = tarfile.DIRTYPE
        root.mode = 0o755; root.mtime = 12345
        archive.addfile(root)
        for name, payload in sorted(entries.items()):
            info = tarfile.TarInfo(name); info.size = len(payload); info.mode = 0o644
            info.uid = 42; info.gid = 84; info.uname = 'upstream'; info.gname = 'sources'; info.mtime = 12345
            info.pax_headers = {'comment': 'retain this exact upstream metadata'}
            archive.addfile(info, io.BytesIO(payload))
    return gzip.compress(data.getvalue(), mtime=0)


class PackagingTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.addCleanup(patch.stopall)
        self.root = Path(self.temp.name)
        self.repo = self.root / 'repo'
        self.here = self.repo / 'scripts/asr-components/ct2-cpu'
        self.here.mkdir(parents=True)
        self.work = self.root / 'work'; self.work.mkdir()
        self.reports = self.root / 'reports'; self.reports.mkdir()
        self.cache = self.root / 'cache'; self.cache.mkdir()
        self.source = json.loads((REAL_HERE / 'sources.lock.json').read_text())
        self.notices = json.loads((REAL_HERE / 'notices.lock.json').read_text())
        for name in package.RECIPE_FILES:
            target = self.repo / name
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(REAL_REPO / name, target)
        for notice in self.notices['files']:
            target = self.here / notice['path']; target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(REAL_HERE / notice['path'], target)
        self.model_data = {name: ('synthetic fixture ' + name).encode() for name in package.MODEL_FIXTURES}
        self.model_pins = {name: (len(data), package.sha(data)) for name, data in self.model_data.items()}
        patch.object(package, 'MODEL_FIXTURES', self.model_pins).start()
        self.source_entries = {package.MODEL_PREFIX + name: data for name, data in self.model_data.items()}
        self.source_entries.update({package.CT2_ROOT + '/LICENSE': b'unchanged source license',
            package.CT2_ROOT + '/src/compile.cc': b'unchanged compiled source',
            package.MODEL_PREFIX + 'v1/aren-transliteration/source_vocabulary.txt': b'unchanged fixture vocabulary'})
        # Tiny synthetic fixtures exercise byte identity without downloading or executing anything.
        for item in self.source['sources'] + [x for x in self.source['build_wheels'] if x['name'] == 'pybind11']:
            data = source_tar(self.source_entries) if item['name'] == 'ctranslate2' else ('fixed source ' + item['name']).encode()
            item.update(bytes=len(data), sha256=package.sha(data))
            (self.cache / item['sha256']).write_bytes(data)
        self.write(self.here / 'sources.lock.json', self.source)
        self.provenance = {'schema': 1, 'luma_source_sha': SOURCE, 'variant': self.source['variant'],
                           'native_compile_flags': r'/Brepro /Z7 /experimental:deterministic /pathmap:<build-root>=C:\luma-ct2-build',
                           'native_link_flags': '/Brepro /INCREMENTAL:NO',
                           'build_strategy': copy.deepcopy(package.BUILD_STRATEGY),
                           'publication_authorized': False, 'upstream': self.source['sources'],
                           'build_wheels': self.source['build_wheels'], 'ct2_cmake': self.source['ct2_cmake'],
                           'onednn_cmake': self.source['onednn_cmake'], 'notices': self.notices}
        self.wheel = self.root / 'first' / package.WHEEL
        self.second = self.root / 'second' / package.WHEEL
        self.wheel.parent.mkdir(); self.second.parent.mkdir()
        self.entries = {'ctranslate2/__init__.py': b'__version__ = "4.8.2"\n',
                        'ctranslate2/ctranslate2.dll': b'MZ native library fixture',
                        'ctranslate2/_ext.cp312-win_amd64.pyd': b'MZ extension fixture',
                        'ctranslate2-4.8.2.dist-info/LUMA_CPU_BUILD.json': package.encoded(self.provenance)}
        for notice in self.notices['files']:
            self.entries['ctranslate2-4.8.2.dist-info/' + notice['path']] = (self.here / notice['path']).read_bytes()
        self.write_wheels()
        self.write(self.reports / 'provenance.json', self.provenance)
        self.write(self.reports / 'whole-runtime-native.json', {'passed': True, 'normal_and_delay_imports': True,
                   'blocked_dependencies': [], 'forbidden_files': [], 'files': [{'path': 'private.dll'}]})
        self.write(self.reports / 'inference.json', {'passed': True, 'isolated': True, 'relocated': True,
                   'offline_audit_enabled': True, 'inherited_python_path_ignored': True,
                   'system_python_used': False, 'host_crt_fallback_allowed': False,
                   'compute_types': ['float32', 'int8'], 'loaded_modules': [{'scope': 'private', 'path': 'msvcp140.dll'}],
                   'inference': {'cold_and_warm': True, 'cpu_only': True, 'model': 'SYSTRAN/faster-whisper-tiny',
                                 'segments': [{'start_ms': 1, 'end_ms': 10, 'text': 'country'}]}})
        self.write(self.reports / 'compiler-probe.json', {'passed': True, 'compiler_flags': self.provenance['native_compile_flags']})
        self.write(self.reports / 'private-crt-proof.json', {'public_redistribution_authorized': False,
                   'redistribution_grant_verified': False, 'original_files_unmodified': True,
                   'global_installation_performed': False, 'files': [{'private_filename': 'msvcp140.dll'}]})
        patch.object(package, 'HERE', self.here).start()
        patch.object(package, 'REPO', self.repo).start()
        self.git = patch.object(package.subprocess, 'run', side_effect=lambda args, **kw: SimpleNamespace(stdout=(self.repo / args[2].split(':', 1)[1]).read_bytes())).start()

    def write(self, path, value):
        path.write_bytes(package.encoded(value))

    def change(self, filename, fn):
        path = self.reports / filename
        value = json.loads(path.read_text()); fn(value); self.write(path, value)

    def write_wheels(self):
        for target in (self.wheel, self.second):
            if target.exists(): target.unlink()
            package.write_archive(target, self.entries)
        sha = package.sha(self.wheel.read_bytes())
        self.write(self.reports / 'result.json', {'source_sha': SOURCE, 'publication_authorized': False,
                   'passed': True, 'wheel_reproduced': True, 'native_inference_passed': True,
                   'wheel': {'filename': package.WHEEL, 'bytes': self.wheel.stat().st_size, 'sha256': sha}, 'second_sha256': sha})
        self.write_freshness(sha)
        self.write(self.reports / 'wheel-comparison.json', {'identical': True, 'binary_bytes_modified': False,
                   'different_members': [], 'first_sha256': sha, 'second_sha256': sha})

    def write_freshness(self, wheel_hash):
        for number in (1, 2):
            self.write(self.reports / f'build-{number}-freshness.json', {'schema': 1, 'build': number,
                'canonical_native_root': str(self.work / 'native-build'), 'fresh_native_root': True,
                'sources_reextracted': True, 'native_root_removed': True, 'native_object_cache_reused': False,
                'retained_wheel_sha256': wheel_hash, 'path_independence_claim': False, 'cross_machine_claim': False})

    def package(self, **kwargs):
        return package.package_publication(source_sha=SOURCE, wheel=self.wheel, second_wheel=self.second,
                    reports=self.reports, cache=self.cache, work=self.work, **kwargs)

    def test_exact_four_files_and_default_metadata_only_export(self):
        report = self.package()
        self.assertEqual(set(p.name for p in (self.work / 'publication-candidate').iterdir()),
                         {package.WHEEL, package.SOURCES, package.NOTICES, package.PROOF})
        self.assertEqual((self.reports / package.PROOF).read_bytes(), package.encoded(report))
        self.assertFalse(report['publication_authorized'])
        self.assertEqual(report['provenance']['build_strategy'], package.BUILD_STRATEGY)
        self.assertNotIn(str(self.work), package.encoded(report).decode())
        self.assertTrue(all(report['checks'].values()))
        self.assertEqual([a['name'] for a in report['assets']], sorted([package.WHEEL, package.SOURCES, package.NOTICES]))
        with zipfile.ZipFile(self.work / 'publication-candidate' / package.SOURCES) as z:
            self.assertEqual(len([n for n in z.namelist() if n.startswith('upstream/')]), 5)
            self.assertIn('luma/LICENSE', z.namelist())
            self.assertIn('SOURCE-EXPORTS.json', z.namelist())
            export = report['source_exports'][0]
            self.assertEqual(json.loads(z.read('SOURCE-EXPORTS.json')), report['source_exports'])
            self.assertEqual(package.pin(export['exported']['name'], z.read(export['exported']['name'])), export['exported'])
            self.assertFalse(export['original']['name'] in z.namelist())
            members, _ = package.source_members(z.read(export['exported']['name']))
            self.assertFalse(any(name.endswith('/model.bin') for name in members))
            self.assertIn(package.MODEL_PREFIX + 'v1/aren-transliteration/source_vocabulary.txt', members)
            self.assertIn('luma/scripts/asr-components/ct2-cpu/build_cpu.py', z.namelist())
            self.assertIn('luma/scripts/asr-components/fixtures.json', z.namelist())
            self.assertFalse(any(n.lower().endswith(('.dll', '.exe', '.pyd', '.bin', '.wav')) for n in z.namelist()))
            self.assertTrue(all(i.date_time == (2026, 1, 1, 0, 0, 0) for i in z.infolist()))
        with zipfile.ZipFile(self.work / 'publication-candidate' / package.NOTICES) as z:
            self.assertEqual(len(z.namelist()), len(self.notices['files']) + 2)

    def test_same_inputs_are_byte_deterministic_despite_paths_and_mtimes(self):
        self.package()
        first = {x.name: x.read_bytes() for x in (self.work / 'publication-candidate').iterdir()}
        self.work = self.root / 'another path with Unicode é'; self.work.mkdir()
        self.write_freshness(package.sha(self.wheel.read_bytes()))
        self.package()
        self.assertEqual(first, {x.name: x.read_bytes() for x in (self.work / 'publication-candidate').iterdir()})

    def test_explicit_export_only_copies_four_approved_files(self):
        output = self.root / 'approved-candidate'
        self.package(publication_output=output)
        self.assertEqual(len(list(output.iterdir())), 4)
        self.assertEqual((output / package.WHEEL).read_bytes(), self.wheel.read_bytes())

    def test_preexisting_candidate_is_never_overwritten(self):
        self.package()
        with self.assertRaisesRegex(ValueError, 'fresh'): self.package()

    def test_preexisting_export_is_never_overwritten(self):
        output = self.root / 'output'; output.mkdir(); (output / 'keep').write_text('untouched')
        with self.assertRaisesRegex(ValueError, 'fresh'): self.package(publication_output=output)
        self.assertEqual((output / 'keep').read_text(), 'untouched')

    def test_failed_or_incomplete_proofs_fail_before_candidate_creation(self):
        mutations = [
            ('result.json', lambda x: x.update(passed=False)),
            ('result.json', lambda x: x.update(source_sha='b' * 40)),
            ('result.json', lambda x: x.update(publication_authorized=True)),
            ('result.json', lambda x: x.update(wheel_reproduced=False)),
            ('result.json', lambda x: x.update(native_inference_passed=False)),
            ('wheel-comparison.json', lambda x: x.update(binary_bytes_modified=True)),
            ('whole-runtime-native.json', lambda x: x.update(blocked_dependencies=[{'name': 'missing.dll'}])),
            ('whole-runtime-native.json', lambda x: x.update(normal_and_delay_imports=False)),
            ('inference.json', lambda x: x.update(isolated=False)),
            ('inference.json', lambda x: x.update(system_python_used=True)),
            ('inference.json', lambda x: x['inference'].update(cold_and_warm=False)),
            ('inference.json', lambda x: x['inference'].update(segments=[])),
            ('inference.json', lambda x: x['loaded_modules'][0].update(scope='windows_os')),
            ('inference.json', lambda x: x['loaded_modules'][0].update(path='libiomp5md.dll')),
            ('compiler-probe.json', lambda x: x.update(passed=False)),
            ('private-crt-proof.json', lambda x: x.update(public_redistribution_authorized=True)),
            ('provenance.json', lambda x: x.update(publication_authorized=True)),
            ('provenance.json', lambda x: x['build_strategy'].update(path_independence_claim=True)),
            ('provenance.json', lambda x: x['build_strategy'].update(canonical_native_root='host-path')),
            ('build-1-freshness.json', lambda x: x.update(fresh_native_root=False)),
            ('build-2-freshness.json', lambda x: x.update(sources_reextracted=False)),
            ('build-2-freshness.json', lambda x: x.update(native_root_removed=False)),
            ('build-2-freshness.json', lambda x: x.update(native_object_cache_reused=True)),
            ('build-2-freshness.json', lambda x: x.update(retained_wheel_sha256='0' * 64)),
            ('build-2-freshness.json', lambda x: x.update(canonical_native_root='another-native-root')),
            ('build-2-freshness.json', lambda x: x.update(path_independence_claim=True)),
            ('build-2-freshness.json', lambda x: x.update(cross_machine_claim=True)),
        ]
        for name, fn in mutations:
            with self.subTest(name=name, fn=fn):
                original = (self.reports / name).read_bytes()
                self.change(name, fn)
                with self.assertRaises(ValueError): self.package()
                self.assertFalse((self.work / 'publication-candidate').exists())
                (self.reports / name).write_bytes(original)

    def test_canonical_native_objects_must_be_removed_before_packaging(self):
        native = self.work / 'native-build'; native.mkdir(); (native / 'stale.obj').write_bytes(b'object cache')
        with self.assertRaisesRegex(ValueError, 'not removed'): self.package()

    def test_missing_actual_freshness_report_is_not_replaced_by_a_summary_claim(self):
        (self.reports / 'build-2-freshness.json').unlink()
        with self.assertRaises(ValueError): self.package()

    def test_different_second_wheel_fails(self):
        self.second.write_bytes(self.second.read_bytes() + b'changed')
        with self.assertRaisesRegex(ValueError, 'repeatability'): self.package()

    def test_missing_or_changed_source_bytes_fail(self):
        item = self.source['sources'][0]
        (self.cache / item['sha256']).write_bytes(b'changed')
        with self.assertRaisesRegex(ValueError, 'source pin'): self.package()

    def test_notice_hash_drift_fails(self):
        (self.here / self.notices['files'][0]['path']).write_text('changed')
        with self.assertRaisesRegex(ValueError, 'Notice bytes'): self.package()

    def test_wheel_notice_missing_fails(self):
        self.entries['ctranslate2-4.8.2.dist-info/' + self.notices['files'][0]['path']] = b'changed'
        self.write_wheels()
        with self.assertRaisesRegex(ValueError, 'native notice'): self.package()

    def test_runtime_or_model_payload_never_enters_wheel(self):
        for name in ('ctranslate2/msvcp140.dll', 'ctranslate2/model.bin', 'python.exe', 'ctranslate2/avcodec.dll', 'ctranslate2-4.8.2.dist-info/model.bin'):
            with self.subTest(name=name):
                self.entries[name] = b'forbidden'; self.write_wheels()
                with self.assertRaises(ValueError): self.package()
                del self.entries[name]

    def test_source_symlink_is_rejected(self):
        item = self.source['sources'][0]; target = self.cache / item['sha256']; data = target.read_bytes()
        target.unlink(); other = self.root / 'outside'; other.write_bytes(data); target.symlink_to(other)
        with self.assertRaisesRegex(ValueError, 'symlink'): self.package()

    def test_parent_directory_symlink_is_rejected(self):
        target = self.repo / 'src-tauri/src/asr'
        moved = self.root / 'outside-source'; target.rename(moved); target.symlink_to(moved, target_is_directory=True)
        with self.assertRaisesRegex(ValueError, 'symlink'): self.package()

    def test_source_only_export_preserves_every_other_member_and_metadata(self):
        item = next(x for x in self.source['sources'] if x['name'] == 'ctranslate2')
        original = (self.cache / item['sha256']).read_bytes()
        exported, report = package.export_ct2_source(item, original, 'OFF')
        second, report2 = package.export_ct2_source(item, original, 'OFF')
        self.assertEqual(exported, second); self.assertEqual(report, report2)
        before, _ = package.source_members(gzip.decompress(original))
        after, _ = package.source_members(exported)
        expected = {package.MODEL_PREFIX + x for x in self.model_data}
        self.assertEqual(set(before) - set(after), expected)
        self.assertEqual(after, {name: member for name, member in before.items() if name not in expected})
        self.assertEqual(report['retained_member_count'], len(after))
        self.assertEqual(report['omitted_members'], [dict(name=package.MODEL_PREFIX + name, bytes=len(data), sha256=package.sha(data)) for name, data in sorted(self.model_data.items())])
        self.assertTrue(report['retained_member_records_preserved'])
        self.assertEqual(original, (self.cache / item['sha256']).read_bytes())

    def test_omission_requires_exact_original_pin_and_disabled_tests(self):
        item = next(x for x in self.source['sources'] if x['name'] == 'ctranslate2')
        original = (self.cache / item['sha256']).read_bytes()
        for selected, data, flag in ((item, original, 'ON'), (dict(item, sha256='0' * 64), original, 'OFF'),
                                     (item, original + b'drift', 'OFF')):
            with self.subTest(flag=flag), self.assertRaisesRegex(ValueError, 'Exact original'):
                package.export_ct2_source(selected, data, flag)

    def test_missing_changed_or_extra_model_payload_blocks_source_export(self):
        item = next(x for x in self.source['sources'] if x['name'] == 'ctranslate2')
        name = next(iter(self.model_data))
        mutations = [lambda entries: entries.pop(package.MODEL_PREFIX + name),
                     lambda entries: entries.update({package.MODEL_PREFIX + name: b'changed model'}),
                     lambda entries: entries.update({package.MODEL_PREFIX + 'unreviewed/model.bin': b'extra model'})]
        for change in mutations:
            entries = dict(self.source_entries); change(entries); original = source_tar(entries)
            selected = dict(item, bytes=len(original), sha256=package.sha(original))
            with self.subTest(change=change), self.assertRaisesRegex(ValueError, 'omission|model payload'):
                package.export_ct2_source(selected, original, 'OFF')

    def test_source_member_duplicate_link_and_traversal_fail_closed(self):
        for name, kind in ((package.CT2_ROOT + '/link', tarfile.SYMTYPE), ('../outside', tarfile.REGTYPE)):
            data = io.BytesIO()
            with tarfile.open(fileobj=data, mode='w') as archive:
                info = tarfile.TarInfo(name); info.type = kind; info.linkname = 'target' if kind == tarfile.SYMTYPE else ''
                archive.addfile(info, io.BytesIO())
            with self.subTest(name=name), self.assertRaises(ValueError): package.source_members(data.getvalue())
        data = io.BytesIO()
        with tarfile.open(fileobj=data, mode='w') as archive:
            for _ in range(2): archive.addfile(tarfile.TarInfo(package.CT2_ROOT + '/duplicate'), io.BytesIO())
        with self.assertRaisesRegex(ValueError, 'duplicated'): package.source_members(data.getvalue())

    def test_source_blob_preserves_git_bytes_across_windows_newlines(self):
        target = self.repo / 'LICENSE'
        target.write_bytes(b'line one\r\nline two\r\n')
        self.git.side_effect = lambda args, **kw: SimpleNamespace(stdout=b'line one\nline two\n')
        self.assertEqual(package.repository_bytes(SOURCE, 'LICENSE'), b'line one\nline two\n')
        target.write_bytes(b'changed source\r\n')
        with self.assertRaisesRegex(ValueError, 'exact source commit'):
            package.repository_bytes(SOURCE, 'LICENSE')

    def test_unsafe_archive_member_paths_are_rejected(self):
        for name in ('../secret', '/root/file', 'C:/file', 'a\\b', 'a//b', 'a/./b'):
            with self.subTest(name=name), self.assertRaises(ValueError): package.safe_name(name)


if __name__ == '__main__':
    unittest.main()
