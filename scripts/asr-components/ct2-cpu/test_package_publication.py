import copy
from contextlib import nullcontext
import gzip
import io
import json
import os
from pathlib import Path
import shutil
import stat
import struct
import subprocess
import sys
import tempfile
import tarfile
import unittest
from unittest.mock import patch
from types import SimpleNamespace
import zipfile

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from fixture_paths import temporary_root, windows_short_path_alias

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
        self.root = self.enterContext(temporary_root())
        self.addCleanup(patch.stopall)
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
        worker = (self.repo / 'src-tauri/src/asr/worker.py').read_text(encoding='utf-8')
        prepared = worker.replace('# LUMA_MANAGED_CT2_CPU_POLICY', 'LUMA_MANAGED_CT2_CPU_POLICY = "luma-cpu-seq-1"', 1)
        identities = {}
        for key, path in (('host_auditor', 'scripts/asr-components/self_test.py'), ('verifier', 'scripts/asr-components/ct2-cpu/verify_runtime.py')):
            data = (self.repo / path).read_bytes()
            identities[key] = {'source_sha': SOURCE, 'repository_path': path, 'sha256': package.sha(data), 'bytes': len(data)}
        self.change('inference.json', lambda x: x.update(**identities,
            cpu_thread_policy={'source_sha': SOURCE, 'variant': 'luma-cpu-seq-1', 'cpu_threads': 1,
                'selection': 'source-build-provenance', 'managed_receipt_selection_tested': False,
                'constructor_overridden': False, 'performance_claim': False,
                'worker_sha256': package.sha(worker.encode()), 'prepared_worker_sha256': package.sha(prepared.encode()),
                'build_provenance_sha256': package.sha((self.reports / 'provenance.json').read_bytes())},
            source_worker_lifecycle=dict.fromkeys(('eof_after_inference', 'model_switch_after_inference', 'explicit_unload_after_inference'), True)))
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

    def test_windows_os_msvcp_win_is_distinct_from_private_numbered_crt(self):
        self.change('inference.json', lambda x: x['loaded_modules'].append(
            {'scope': 'windows_os', 'path': 'C:/Windows/System32/msvcp_win.dll'}))
        self.package()

    def test_integrated_policy_and_auditor_cannot_be_replaced_by_diagnostic_claims(self):
        for section, key, value in (
                ('host_auditor', 'source_sha', 'b' * 40), ('host_auditor', 'sha256', '0' * 64),
                ('host_auditor', 'bytes', 1), ('verifier', 'sha256', '0' * 64),
                ('cpu_thread_policy', 'cpu_threads', 0), ('cpu_thread_policy', 'cpu_threads', True),
                ('cpu_thread_policy', 'constructor_overridden', True),
                ('cpu_thread_policy', 'selection', 'diagnostic-one-thread'),
                ('cpu_thread_policy', 'managed_receipt_selection_tested', True),
                ('cpu_thread_policy', 'build_provenance_sha256', '0' * 64),
                ('source_worker_lifecycle', 'explicit_unload_after_inference', False)):
            original = (self.reports / 'inference.json').read_bytes()
            self.change('inference.json', lambda x: x[section].update({key: value}))
            with self.subTest(section=section, key=key, value=value), self.assertRaises(ValueError): self.package()
            (self.reports / 'inference.json').write_bytes(original)

    def test_only_complete_exact_defender_host_evidence_is_accepted(self):
        from test_defender_identity import valid_identity, DEFENDER
        from self_test import DEFENDER_AMSI_CLSID
        evidence = dict(valid_identity(), kind='windows-defender-amsi', verified=True, stage='verified',
            provider_clsid=DEFENDER_AMSI_CLSID, registry_path_matched=True, bytes=1234, sha256='a' * 64,
            path=DEFENDER, loaded_path=DEFENDER, registered_path=DEFENDER)
        record = {'scope': 'verified_host_security', 'path': DEFENDER, 'evidence': evidence}
        package.validate_defender_record(record)
        self.change('inference.json', lambda x: x['loaded_modules'].append(record))
        self.package()
        for key, value in [('verified', False), ('kind', 'other'), ('registry_path_matched', False),
                ('registered_path', DEFENDER.replace('4.18.26080.4-0', '4.18.26090.1-1')),
                ('sha256', 'bad'), ('bytes', 0), ('authenticode_status', 1), ('microsoft_root_policy_error', 1),
                ('provider_clsid', 'other'), ('stage', 'identity-policy')]:
            bad = copy.deepcopy(record); bad['evidence'][key] = value
            with self.subTest(key=key), self.assertRaises(ValueError): package.validate_defender_record(bad)
        for name in ('msvcp140.dll', 'libiomp5md.dll', 'other.dll', 'MpOav.dll.extra'):
            bad = copy.deepcopy(record); bad['path'] = bad['path'].replace('MpOav.dll', name)
            with self.subTest(name=name), self.assertRaises(ValueError): package.validate_defender_record(bad)
        for value in (DEFENDER.replace('\\MpOav', '\\.\\MpOav'), DEFENDER.replace('\\', '/'),
                      DEFENDER.replace('4.18.26080.4-0', 'current')):
            bad = copy.deepcopy(record); bad['path'] = value
            bad['evidence'].update(dict.fromkeys(('path', 'loaded_path', 'registered_path'), value))
            with self.subTest(path=value), self.assertRaises(ValueError): package.validate_defender_record(bad)

    def test_numbered_crt_and_lookalike_names_still_cannot_be_windows_os(self):
        for name in ('msvcp140.dll', 'msvcp140_1.dll', 'vcruntime140.dll', 'concrt140.dll', 'msvcp_win.dll.extra'):
            self.change('inference.json', lambda x: x.update(loaded_modules=[{'scope': 'windows_os', 'path': name}]))
            with self.subTest(name=name), self.assertRaisesRegex(ValueError, 'unsafe dependency'): self.package()

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


class DiagnosticPackagingTests(unittest.TestCase):
    setUp = PackagingTests.setUp
    write = PackagingTests.write
    change = PackagingTests.change
    write_wheels = PackagingTests.write_wheels
    write_freshness = PackagingTests.write_freshness
    package = PackagingTests.package
    def diagnostic_state(self, outcome='timed_out'):
        status = {'started': True, 'outcome': outcome}
        status.update(timeout_seconds=900) if outcome == 'timed_out' else status.update(returncode=1)
        self.change('result.json', lambda value: value.update(passed=False, native_inference_passed=False,
                                                             native_verifier=status))
        (self.reports / 'inference.json').unlink(missing_ok=True)
        return status

    def origin(self):
        return {'repository': 'csic21/luma-subtitle', 'run_id': 123, 'run_attempt': 1,
                'job': 'windows-cpu-proof', 'head_sha': SOURCE, 'source_sha': SOURCE,
                'ref': 'refs/heads/feat/optional-asr-engines', 'event_name': 'push',
                'workflow_ref': 'csic21/luma-subtitle/.github/workflows/asr-ct2-cpu.yml@refs/heads/feat/optional-asr-engines'}

    def diagnostic(self, **kwargs):
        return package.package_diagnostic(source_sha=SOURCE, wheel=self.wheel, second_wheel=self.second,
            reports=self.reports, cache=self.cache, work=self.work,
            diagnostic_output=kwargs.pop('diagnostic_output', self.root / 'diagnostic-export'),
            origin=kwargs.pop('origin', self.origin()), **kwargs)

    def test_diagnostic_exact_four_files_reuse_unchanged_source_notices_and_wheel(self):
        self.package()
        expected = {name: (self.work / 'publication-candidate' / name).read_bytes()
                    for name in (package.WHEEL, package.SOURCES, package.NOTICES)}
        self.diagnostic_state()
        report = self.diagnostic(); output = self.root / 'diagnostic-export'
        self.assertEqual({p.name for p in output.iterdir()}, {*expected, package.DIAGNOSTIC})
        for name, data in expected.items(): self.assertEqual((output / name).read_bytes(), data)
        self.assertEqual(report['kind'], 'ct2-cpu-diagnostic')
        self.assertEqual(report['purpose'], 'failed-native-verifier-debugging')
        self.assertEqual(report['retention_days'], 1)
        for field in ('publication_authorized', 'installable', 'inference_passed'): self.assertIs(report[field], False)
        self.assertEqual(report['origin']['source_sha'], SOURCE)
        self.assertEqual(report['verification']['outcome'], 'timed_out')
        self.assertEqual(report['provenance']['build_strategy'], package.BUILD_STRATEGY)
        self.assertNotIn(str(self.root), package.encoded(report).decode())
        self.assertNotIn('checks', report)
        self.assertNotIn('source_sha', report)
        for item in report['evidence']:
            self.assertEqual(item, package.pin(item['name'], (self.reports / item['name']).read_bytes()))
        self.assertEqual((output / package.DIAGNOSTIC).read_bytes(), package.encoded(report))
        self.assertFalse((output / package.PROOF).exists())
        with self.assertRaisesRegex(ValueError, 'successful exact native proof'): self.package()

    def test_diagnostic_failed_exit_may_have_a_failed_inference_report(self):
        self.diagnostic_state('failed')
        self.write(self.reports / 'inference.json', {'passed': False, 'error': 'bounded fixture error'})
        report = self.diagnostic()
        self.assertEqual(report['verification']['returncode'], 1)
        self.assertFalse((self.reports / package.PROOF).exists())

    def test_diagnostic_origin_rejects_other_runs_repositories_and_mutable_source(self):
        self.diagnostic_state()
        for key, value in [('repository', 'other/repo'), ('ref', 'refs/heads/main'), ('event_name', 'workflow_dispatch'),
                           ('job', 'other'), ('head_sha', 'b'*40), ('source_sha', 'main'),
                           ('workflow_ref', 'other.yml@refs/heads/feat/optional-asr-engines'),
                           ('run_id', True), ('run_attempt', 0), ('run_id', 2**53)]:
            with self.subTest(key=key, value=value), self.assertRaisesRegex(ValueError, 'origin'):
                self.diagnostic(origin=dict(self.origin(), **{key: value}))
        self.assertFalse((self.root / 'diagnostic-export').exists())

    def test_diagnostic_rejects_success_cancellation_earlier_failure_and_unknown_status(self):
        self.diagnostic_state(); baseline = (self.reports / 'result.json').read_bytes()
        mutations = [lambda x: x.update(passed=True), lambda x: x.update(native_inference_passed=True),
                     lambda x: x.pop('native_verifier'), lambda x: x['native_verifier'].update(started=False),
                     lambda x: x['native_verifier'].update(outcome='passed'),
                     lambda x: x['native_verifier'].update(outcome='cancelled'),
                     lambda x: x['native_verifier'].update(outcome='launch_error'),
                     lambda x: x['native_verifier'].update(timeout_seconds=1800)]
        for mutation in mutations:
            with self.subTest(mutation=mutation), self.assertRaises(ValueError):
                self.change('result.json', mutation); self.diagnostic()
            (self.reports / 'result.json').write_bytes(baseline)
        for code in (0, -1, -1073741510, 0xc000013a, True, 2**32, 130, 143):
            self.change('result.json', lambda x: x.update(native_verifier={'started': True, 'outcome': 'failed', 'returncode': code}))
            with self.subTest(code=code), self.assertRaises(ValueError): self.diagnostic()
        self.assertFalse((self.root / 'diagnostic-export').exists())

    def test_diagnostic_rejects_incomplete_native_build_or_static_closure(self):
        self.diagnostic_state()
        mutations = [('result.json', lambda x: x.update(wheel_reproduced=False)),
                     ('result.json', lambda x: x.update(source_sha='b'*40)),
                     ('compiler-probe.json', lambda x: x.update(passed=False)),
                     ('wheel-comparison.json', lambda x: x.update(identical=False)),
                     ('whole-runtime-native.json', lambda x: x.update(passed=False)),
                     ('whole-runtime-native.json', lambda x: x.update(blocked_dependencies=[{'name':'missing.dll'}])),
                     ('build-2-freshness.json', lambda x: x.update(native_object_cache_reused=True)),
                     ('private-crt-proof.json', lambda x: x.update(public_redistribution_authorized=True)),
                     ('provenance.json', lambda x: x.update(publication_authorized=True))]
        for name, mutation in mutations:
            original = (self.reports / name).read_bytes()
            with self.subTest(name=name), self.assertRaises(ValueError):
                self.change(name, mutation); self.diagnostic()
            (self.reports / name).write_bytes(original)
        self.assertFalse((self.root / 'diagnostic-export').exists())

    def test_diagnostic_rejects_contradictory_successful_inference(self):
        self.diagnostic_state(); self.write(self.reports / 'inference.json', {'passed': True})
        with self.assertRaisesRegex(ValueError, 'contradicts'): self.diagnostic()

    def test_diagnostic_keeps_source_notice_and_model_omission_guards(self):
        self.diagnostic_state()
        item = self.source['sources'][0]; path = self.cache / item['sha256']; original = path.read_bytes()
        path.write_bytes(b'changed source')
        with self.assertRaisesRegex(ValueError, 'source pin'): self.diagnostic()
        path.write_bytes(original)
        notice = self.here / self.notices['files'][0]['path']; original = notice.read_bytes(); notice.write_bytes(b'changed notice')
        with self.assertRaisesRegex(ValueError, 'Notice bytes'): self.diagnostic()
        notice.write_bytes(original)
        source_entries = dict(self.source_entries); source_entries[package.MODEL_PREFIX + 'new/model.bin'] = b'forbidden'
        original = source_tar(source_entries); item.update(bytes=len(original), sha256=package.sha(original))
        (self.cache / item['sha256']).write_bytes(original)
        self.write(self.here / 'sources.lock.json', self.source)
        self.write(self.reports / 'provenance.json', self.provenance)
        self.entries[package.DIST_INFO + 'LUMA_CPU_BUILD.json'] = package.encoded(self.provenance)
        self.write_wheels(); self.diagnostic_state()
        with self.assertRaisesRegex(ValueError, 'model payload'): self.diagnostic()

    def test_diagnostic_rejects_runtime_model_and_nested_third_party_wheel_payload(self):
        for name in ('python.exe', 'ctranslate2/msvcp140.dll', 'ctranslate2/model.bin',
                     'ctranslate2/torch.whl', 'ctranslate2/avcodec.dll', package.DIST_INFO + 'model.bin'):
            self.entries[name] = b'forbidden'; self.write_wheels(); self.diagnostic_state()
            with self.subTest(name=name), self.assertRaises(ValueError): self.diagnostic()
            del self.entries[name]
        self.assertFalse((self.root / 'diagnostic-export').exists())

    def test_diagnostic_never_overwrites_previous_output_or_symlink(self):
        self.diagnostic_state(); output = self.root / 'keep'; output.mkdir(); (output/'marker').write_text('keep')
        with self.assertRaisesRegex(ValueError, 'fresh'): self.diagnostic(diagnostic_output=output)
        self.assertEqual((output/'marker').read_text(), 'keep')
        alias = self.root / 'alias'; alias.symlink_to(output, target_is_directory=True)
        with self.assertRaisesRegex(ValueError, 'fresh'): self.diagnostic(diagnostic_output=alias)

    def test_partial_export_never_exposes_manifest_completion_marker(self):
        self.diagnostic_state(); output = self.root / 'diagnostic-export'
        def broken_copy(source, target):
            target.mkdir(); (target / package.WHEEL).write_bytes(self.wheel.read_bytes())
            raise OSError('simulated interrupted copy')
        with patch.object(package.shutil, 'copytree', side_effect=broken_copy):
            with self.assertRaisesRegex(OSError, 'interrupted'): self.diagnostic()
        self.assertFalse((output / package.DIAGNOSTIC).exists())
        self.assertFalse((output / package.PROOF).exists())

    def test_unexpected_file_or_changed_copy_blocks_manifest_marker(self):
        self.diagnostic_state(); output = self.root / 'diagnostic-export'; real_copy = shutil.copytree
        def changed_copy(source, target):
            real_copy(source, target); (target / package.WHEEL).write_bytes(b'changed')
        with patch.object(package.shutil, 'copytree', side_effect=changed_copy):
            with self.assertRaisesRegex(ValueError, 'changed during copying'): self.diagnostic()
        self.assertFalse((output / package.DIAGNOSTIC).exists())

    def test_extra_copy_file_blocks_manifest_marker(self):
        self.diagnostic_state(); output = self.root / 'diagnostic-export'; real_copy = shutil.copytree
        def extra_copy(source, target):
            real_copy(source, target); (target / 'runtime.dll').write_bytes(b'forbidden')
        with patch.object(package.shutil, 'copytree', side_effect=extra_copy):
            with self.assertRaisesRegex(ValueError, 'Unexpected diagnostic'): self.diagnostic()
        self.assertFalse((output / package.DIAGNOSTIC).exists())

    def test_failed_final_atomic_marker_activation_does_not_look_complete(self):
        self.diagnostic_state(); output = self.root / 'diagnostic-export'
        with patch.object(Path, 'replace', side_effect=OSError('simulated marker activation failure')):
            with self.assertRaisesRegex(OSError, 'marker activation'): self.diagnostic()
        self.assertFalse((output / package.DIAGNOSTIC).exists())
        self.assertTrue((output / '.diagnostic-manifest.pending').exists())

    def test_diagnostic_wheel_symlink_is_rejected(self):
        self.diagnostic_state(); self.wheel.unlink(); self.wheel.symlink_to(self.second)
        with self.assertRaisesRegex(ValueError, 'regular public input'): self.diagnostic()
        self.assertFalse((self.root / 'diagnostic-export').exists())


class ApprovedArtifactExtractionTests(unittest.TestCase):
    def setUp(self):
        self.root = self.enterContext(temporary_root())
        self.artifact = self.root / 'downloaded-actions.zip'
        self.approval = self.root / 'approval.json'
        self.output = self.root / 'approved-candidate'
        # These deliberately are not executable/valid wheel or source archives.
        # Extraction must treat all four approved files as opaque exact bytes.
        self.payloads = {package.WHEEL: b'opaque wheel bytes', package.SOURCES: b'opaque source bytes',
                         package.NOTICES: b'opaque notice bytes',
                         package.PROOF: package.encoded({'note': 'résumé 日本語'})}
        self.receipt = {'schema': 1, 'files': [package.pin(name, data) for name, data in self.payloads.items()]}
        self.write_artifact()

    def write_artifact(self, members=None, compression=zipfile.ZIP_STORED):
        with zipfile.ZipFile(self.artifact, 'w', compression=compression) as archive:
            for name, data in (self.payloads.items() if members is None else members):
                archive.writestr(name, data)
        self.repin_archive()

    def repin_archive(self):
        raw = self.artifact.read_bytes()
        self.receipt['archive'] = {'bytes': len(raw), 'sha256': package.sha(raw)}
        self.write_receipt()

    def write_receipt(self):
        self.approval.write_text(json.dumps(self.receipt, ensure_ascii=False), encoding='utf-8')

    def extract(self):
        return package.extract_approved_artifact(self.artifact, self.approval, self.output)

    def assert_rejected(self, error=ValueError):
        with self.assertRaises(error):
            self.extract()
        self.assertFalse(self.output.exists())

    def test_extracts_only_exact_four_files_without_interpreting_payloads(self):
        with patch.object(package.subprocess, 'run', side_effect=AssertionError('No execution permitted')):
            self.assertEqual(self.extract(), self.output)
        self.assertEqual({item.name: item.read_bytes() for item in self.output.iterdir()}, self.payloads)
        self.assertEqual(json.loads((self.output / package.PROOF).read_text(encoding='utf-8')),
                         {'note': 'résumé 日本語'})

    def test_accepts_standard_actions_deflated_zip(self):
        self.write_artifact(compression=zipfile.ZIP_DEFLATED)
        self.extract()
        self.assertEqual((self.output / package.WHEEL).read_bytes(), self.payloads[package.WHEEL])

    def test_cli_extracts_only_and_fails_closed_on_bad_approval(self):
        command = [sys.executable, '-B', str(REAL_HERE / 'package_publication.py'),
                   '--extract-approved-artifact', str(self.artifact), '--approval', str(self.approval),
                   '--output', str(self.output)]
        result = subprocess.run(command, capture_output=True, text=True, encoding='utf-8', timeout=30)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual({item.name for item in self.output.iterdir()}, set(self.payloads))
        shutil.rmtree(self.output)
        self.receipt['schema'] = 2; self.write_receipt()
        result = subprocess.run(command, capture_output=True, text=True, encoding='utf-8', timeout=30)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('Approved artifact extraction failed:', result.stderr)
        self.assertFalse(self.output.exists())

    def test_rejects_malformed_or_oversized_receipts(self):
        for raw in (b'not JSON', b'\xff', b'null', b'[]',
                    b'{"schema":1,"schema":1,"archive":{},"files":[]}',
                    b' ' * (package.MAX_APPROVAL_BYTES + 1)):
            with self.subTest(raw=raw[:100]):
                self.approval.write_bytes(raw)
                self.assert_rejected()

    def test_rejects_invalid_receipt_schema_and_file_pins(self):
        original = copy.deepcopy(self.receipt)
        mutations = [lambda x: x.update(schema=True), lambda x: x.update(extra=True),
                     lambda x: x.update(files=x['files'][:3]), lambda x: x.update(files={}),
                     lambda x: x['files'].__setitem__(0, None),
                     lambda x: x['files'].__setitem__(0, x['files'][1]),
                     lambda x: x['files'][0].update(name=package.WHEEL.upper()),
                     lambda x: x['files'][0].update(name='../' + package.WHEEL),
                     lambda x: x['files'][0].update(name=[]),
                     lambda x: x['files'][0].update(bytes=True),
                     lambda x: x['files'][0].update(bytes=0),
                     lambda x: x['files'][0].update(bytes=package.MAX_BYTES),
                     lambda x: x['files'][3].update(bytes=package.MAX_PROOF_BYTES + 1),
                     lambda x: x['files'][0].update(sha256='A' * 64),
                     lambda x: x['files'][0].update(sha256=None),
                     lambda x: x['files'][0].update(extra=True),
                     lambda x: x.update(archive=None),
                     lambda x: x['archive'].update(bytes=True),
                     lambda x: x['archive'].update(bytes=0),
                     lambda x: x['archive'].update(bytes=package.MAX_ARTIFACT_BYTES + 1),
                     lambda x: x['archive'].update(bytes=sum(p['bytes'] for p in x['files']) + package.MAX_ARTIFACT_OVERHEAD + 1),
                     lambda x: x['archive'].update(sha256=None),
                     lambda x: x['archive'].update(extra=True)]
        for mutation in mutations:
            with self.subTest(mutation=mutation):
                self.receipt = copy.deepcopy(original); mutation(self.receipt); self.write_receipt()
                self.assert_rejected()

    def test_rehashes_exact_archive_size_and_hash(self):
        original = self.artifact.read_bytes()
        for raw in (original + b'extra', original[:-1], b'X' + original[1:]):
            with self.subTest(size=len(raw)):
                self.artifact.write_bytes(raw)
                self.assert_rejected()

    def test_rejects_missing_duplicate_extra_case_and_path_members(self):
        members = list(self.payloads.items())
        alternatives = [members[:3], members + [('unexpected.dll', b'forbidden')],
                        members + [members[0]], members[1:] + [members[1]]]
        for name in (package.WHEEL.upper(), '../' + package.WHEEL, '/' + package.WHEEL,
                     'nested/' + package.WHEEL, 'nested\\' + package.WHEEL, package.WHEEL + '/',
                     'C:' + package.WHEEL):
            alternatives.append([(name, members[0][1])] + members[1:])
        for alternative in alternatives:
            with self.subTest(names=[name for name, _ in alternative]):
                with self.assertWarns(UserWarning) if len({n for n, _ in alternative}) != len(alternative) else nullcontext():
                    self.write_artifact(alternative)
                self.assert_rejected()

    def test_rejects_nonregular_modes_and_dos_directory_attributes(self):
        for mode, attributes in ((stat.S_IFLNK | 0o777, 0), (stat.S_IFDIR | 0o755, 0),
                                 (stat.S_IFIFO | 0o644, 0), (stat.S_IFREG | 0o644, 0x10)):
            info = zipfile.ZipInfo(package.WHEEL)
            info.create_system = 3; info.external_attr = (mode << 16) | attributes
            with self.subTest(mode=mode, attributes=attributes):
                self.write_artifact([(info, self.payloads[package.WHEEL]), *list(self.payloads.items())[1:]])
                self.assert_rejected()

    def test_rejects_encryption_and_unsupported_compression(self):
        for flag in (1, 0x40):
            self.write_artifact(); raw = bytearray(self.artifact.read_bytes())
            for signature, offset in ((b'PK\x03\x04', 6), (b'PK\x01\x02', 8)):
                struct.pack_into('<H', raw, raw.index(signature) + offset, flag)
            self.artifact.write_bytes(raw); self.repin_archive()
            with self.subTest(flag=flag): self.assert_rejected()
        self.write_artifact(compression=zipfile.ZIP_BZIP2)
        self.assert_rejected()

    def test_rejects_nul_truncated_member_names(self):
        self.write_artifact([(package.WHEEL + 'xhidden', self.payloads[package.WHEEL]),
                             *list(self.payloads.items())[1:]])
        self.artifact.write_bytes(self.artifact.read_bytes().replace(
            (package.WHEEL + 'xhidden').encode(), (package.WHEEL + '\x00hidden').encode()))
        self.repin_archive(); self.assert_rejected()

    def test_verifies_declared_member_sizes_and_actual_member_hashes_before_output(self):
        self.receipt['files'][0]['bytes'] += 1; self.write_receipt()
        self.assert_rejected()
        self.receipt['files'][0]['bytes'] -= 1
        changed = dict(self.payloads)
        changed[package.PROOF] = b'x' * len(changed[package.PROOF])
        self.write_artifact(changed.items())
        self.assert_rejected()

    def test_rejects_corrupt_and_truncated_zips_before_output(self):
        original = self.artifact.read_bytes()
        for raw in (original[:-22], original.replace(b'opaque wheel bytes', b'changed wheel data')):
            self.artifact.write_bytes(raw); self.repin_archive()
            with self.subTest(size=len(raw)):
                self.assert_rejected(zipfile.BadZipFile)

    def test_never_overwrites_existing_output_or_accepts_noncanonical_output(self):
        self.output.mkdir(); marker = self.output / 'keep'; marker.write_bytes(b'keep')
        with self.assertRaisesRegex(ValueError, 'fresh'): self.extract()
        self.assertEqual(marker.read_bytes(), b'keep')
        shutil.rmtree(self.output)
        self.output.write_bytes(b'keep existing file')
        with self.assertRaisesRegex(ValueError, 'fresh'): self.extract()
        self.assertEqual(self.output.read_bytes(), b'keep existing file')
        self.output.unlink()
        for output in (Path('relative-output'), self.root / 'missing' / '..' / 'candidate'):
            with self.subTest(output=output), self.assertRaisesRegex(ValueError, 'canonical'):
                package.extract_approved_artifact(self.artifact, self.approval, output)

    def test_rejects_nonfile_inputs(self):
        for target in (self.artifact, self.approval):
            original = target.read_bytes(); target.unlink(); target.mkdir()
            with self.subTest(target=target): self.assert_rejected()
            target.rmdir(); target.write_bytes(original)

    def test_rejects_input_and_output_symlinks(self):
        for original in (self.artifact, self.approval, self.output):
            alias = self.root / ('alias-' + original.name)
            try:
                alias.symlink_to(original, target_is_directory=original == self.output)
            except OSError as error:
                self.skipTest('Symlinks unavailable on this test host: ' + str(error))
            with self.subTest(original=original), self.assertRaises(ValueError):
                package.extract_approved_artifact(alias if original == self.artifact else self.artifact,
                    alias if original == self.approval else self.approval,
                    alias if original == self.output else self.output)
            self.assertFalse(self.output.exists())

    def test_rejects_symlinked_input_parent(self):
        alias = self.root / 'input-alias'
        try:
            alias.symlink_to(self.root, target_is_directory=True)
        except OSError as error:
            self.skipTest('Symlinks unavailable on this test host: ' + str(error))
        with self.assertRaises(ValueError):
            package.extract_approved_artifact(alias / self.artifact.name, self.approval, self.output)
        self.assertFalse(self.output.exists())

    def test_removes_partial_output_when_a_file_write_fails(self):
        real_open = Path.open
        def failing_open(path, *args, **kwargs):
            if path == self.output / package.SOURCES:
                self.assertTrue((self.output / package.WHEEL).exists())
                raise OSError('simulated write failure')
            return real_open(path, *args, **kwargs)
        with patch.object(Path, 'open', failing_open):
            with self.assertRaisesRegex(OSError, 'simulated write failure'): self.extract()
        self.assertFalse(self.output.exists())
        self.extract()
        self.assertEqual(len(list(self.output.iterdir())), 4)

    def test_failed_output_creation_does_not_remove_another_directory(self):
        real_mkdir = Path.mkdir
        def raced_mkdir(path, *args, **kwargs):
            real_mkdir(path, *args, **kwargs)
            if path == self.output:
                (path / 'keep').write_bytes(b'owned by another process')
                raise FileExistsError('simulated creation race')
        with patch.object(Path, 'mkdir', raced_mkdir):
            with self.assertRaises(FileExistsError): self.extract()
        self.assertEqual((self.output / 'keep').read_bytes(), b'owned by another process')


class TemporaryRootAliasTests(unittest.TestCase):
    def assert_packaging_through_parent_alias(self, alias, actual):
        # The real factory creates/owns the nested temporary child through the
        # alias; the shared helper must canonicalize it before fixture setup.
        real_temporary_directory = tempfile.TemporaryDirectory
        def through_alias(**kwargs):
            return real_temporary_directory(**dict(kwargs, dir=alias))
        case = PackagingTests('test_exact_four_files_and_default_metadata_only_export')
        result = unittest.TestResult()
        with patch.object(tempfile, 'TemporaryDirectory', side_effect=through_alias):
            case.run(result)
        self.assertEqual(case.root.parent, actual)
        self.assertEqual(result.errors, [])
        self.assertEqual(result.failures, [])
        self.assertEqual(result.testsRun, 1)
        self.assertTrue(result.wasSuccessful())
        self.assertFalse(case.root.exists(), 'Only the owned child should be cleaned')
        self.assertTrue(actual.exists())

    def test_packaging_fixture_canonicalizes_an_aliased_temp_root(self):
        with temporary_root() as root:
            actual = root / 'long temporary directory'; actual.mkdir()
            alias = root / 'SHORT~1'
            try:
                alias.symlink_to(actual, target_is_directory=True)
            except OSError as error:
                self.skipTest('Directory aliases unavailable on this test host: ' + str(error))
            self.assert_packaging_through_parent_alias(alias, actual)

    @unittest.skipUnless(os.name == 'nt', 'Requires native Windows GetShortPathNameW')
    def test_packaging_through_actual_windows_short_path_alias(self):
        with temporary_root(prefix='Luma native long packaging fixture ') as actual:
            try:
                alias = windows_short_path_alias(actual)
            except OSError as error:
                self.skipTest('Native short-path probe unavailable: ' + str(error))
            if alias is None:
                self.skipTest('This filesystem does not expose a distinct 8.3 alias')
            self.assert_packaging_through_parent_alias(alias, actual)


if __name__ == '__main__':
    unittest.main()
