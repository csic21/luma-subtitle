import io
import copy
import hashlib
import json
from pathlib import Path
import sys
import unittest
from unittest.mock import patch
from unittest.mock import Mock
from types import SimpleNamespace

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from fixture_paths import temporary_root
from verify_runtime import ProtocolCapture, StageJournal, record_failure, validate_loaded_paths
import verify_runtime as verifier


class RuntimeDiagnosticsTests(unittest.TestCase):
    def test_integrated_source_policy_requires_exact_built_and_installed_provenance(self):
        with temporary_root() as root:
            lock = json.loads(Path(verifier.__file__).with_name('sources.lock.json').read_text())
            provenance = {'schema': 1, 'variant': 'luma-cpu-seq-1', 'luma_source_sha': 'a' * 40,
                          'upstream': lock['sources'], 'ct2_cmake': lock['ct2_cmake'], 'onednn_cmake': lock['onednn_cmake']}
            proof = root / 'provenance.json'; proof.write_text(json.dumps(provenance))
            installed = root / 'Lib/site-packages/ctranslate2-4.8.2.dist-info/LUMA_CPU_BUILD.json'
            installed.parent.mkdir(parents=True); installed.write_text(json.dumps(provenance))
            worker_path = Path(__file__).resolve().parents[3] / 'src-tauri/src/asr/worker.py'
            def load():
                return verifier.load_source_proof_worker(worker_path, root, proof, hashlib.sha256(proof.read_bytes()).hexdigest(), 'a' * 40)
            with patch('sys.platform', 'win32'):
                worker, identity = load()
                self.assertEqual(worker['managed_ct2_cpu_options']('cpu'), {'cpu_threads': 1})
                self.assertEqual(worker['managed_ct2_cpu_options']('cuda'), {})
                self.assertFalse(identity['constructor_overridden']); self.assertFalse(identity['managed_receipt_selection_tested'])
                self.assertEqual(identity['selection'], 'source-build-provenance')
                for key, value in [('variant', 'upstream'), ('luma_source_sha', 'b' * 40), ('ct2_cmake', {}), ('upstream', [])]:
                    bad = copy.deepcopy(provenance); bad[key] = value; proof.write_text(json.dumps(bad)); installed.write_text(json.dumps(bad))
                    with self.subTest(key=key), self.assertRaisesRegex(ValueError, 'reviewed CPU build recipe'): load()
                proof.write_text(json.dumps(provenance)); installed.write_text('{}')
                with self.assertRaisesRegex(ValueError, 'Installed CPU wheel'): load()
                installed.write_text(json.dumps(provenance))
                with patch('sys.platform', 'darwin'), self.assertRaisesRegex(ValueError, 'not selected'): load()

    def test_source_proof_preparation_matches_production_rust_marker(self):
        source = (Path(__file__).resolve().parents[3] / 'src-tauri/src/asr/process.rs').read_text()
        self.assertIn('const CT2_CPU_POLICY_MARKER: &str = ' + json.dumps(verifier.CT2_CPU_POLICY_MARKER), source)
        self.assertIn('const CT2_CPU_POLICY_SOURCE: &str = ' + json.dumps(verifier.CT2_CPU_POLICY_SOURCE), source)
        self.assertNotIn('thread_override(', Path(verifier.__file__).read_text())

    def test_integrated_auditor_is_byte_identified_before_executing(self):
        with temporary_root() as root:
            path = root / 'auditor.py'; data = b'def verified_defender_module(path): return {}\n'; path.write_bytes(data)
            auditor, identity = verifier.load_host_auditor(path, hashlib.sha256(data).hexdigest(), 'a' * 40)
            self.assertTrue(callable(auditor.verified_defender_module)); self.assertEqual(identity['bytes'], len(data))
            path.write_text("raise AssertionError('must not execute')")
            with self.assertRaisesRegex(ValueError, 'bytes changed'):
                verifier.load_host_auditor(path, hashlib.sha256(data).hexdigest(), 'a' * 40)
            with self.assertRaises(ValueError): verifier.load_host_auditor(path, 'x', 'a' * 40)

    def test_integrated_defender_exception_preserves_every_other_path_gate(self):
        with temporary_root() as home:
            root, system = home / 'private', home / 'Windows'
            defender = str(home / 'ProgramData/Microsoft/Windows Defender/Platform/4.18.26080.4-0/MpOAV.dll')
            auditor = SimpleNamespace(verified_defender_module=Mock(return_value={'verified': True, 'kind': 'windows-defender-amsi'}))
            result = validate_loaded_paths([defender, str(root / 'msvcp140.dll')], root, system, auditor)
            self.assertEqual(result[0]['scope'], 'verified_host_security')
            for name in ('msvcp140.dll', 'libiomp5md.dll', 'cudnn64.dll', 'arbitrary.dll', 'MpOAV.dll.extra'):
                auditor.verified_defender_module.reset_mock()
                with self.subTest(name=name), self.assertRaises(RuntimeError):
                    validate_loaded_paths([defender, str(home / 'outside' / name)], root, system, auditor)
                auditor.verified_defender_module.assert_not_called()
            auditor.verified_defender_module.return_value = {'verified': False}
            with self.assertRaises(RuntimeError): validate_loaded_paths([defender], root, system, auditor)
            auditor.verified_defender_module.side_effect = AssertionError('untrusted registration/signature')
            with self.assertRaises(AssertionError): validate_loaded_paths([defender], root, system, auditor)

    def test_integrated_trust_dependencies_reach_strict_bounded_fixed_point(self):
        with temporary_root() as home:
            root, system = home / 'private', home / 'Windows'
            defender, dll = str(home / 'ProgramData/MpOAV.dll'), str(system / 'System32/cryptnet.dll')
            auditor = SimpleNamespace(verified_defender_module=Mock(return_value={'verified': True, 'kind': 'windows-defender-amsi'}))
            with patch.object(verifier, 'loaded_modules', side_effect=[[defender], [defender, dll], [defender, dll]]) as snapshot:
                self.assertEqual(len(verifier.audit_loaded_modules(root, system, auditor)), 2)
                self.assertEqual(snapshot.call_count, 3); auditor.verified_defender_module.assert_called_once()
            with patch.object(verifier, 'loaded_modules', side_effect=[[defender], [defender, str(home / 'outside/msvcp140.dll')]]):
                with self.assertRaisesRegex(RuntimeError, 'Host-global VC'): verifier.audit_loaded_modules(root, system, auditor)
            with patch.object(verifier, 'loaded_modules', side_effect=[[defender], [defender, dll], [defender]]):
                with self.assertRaisesRegex(RuntimeError, 'stabilize'): verifier.audit_loaded_modules(root, system, auditor)
            with patch.object(verifier, 'loaded_modules', return_value=[]):
                with self.assertRaisesRegex(RuntimeError, 'snapshot exceeds'): verifier.audit_loaded_modules(root, system, auditor)

    def test_only_exact_os_msvcp_win_exception_preserves_private_crt_gate(self):
        with temporary_root() as temporary:
            root, system = temporary / 'private', temporary / 'Windows'
            os_module = system / 'System32/msvcp_win.dll'
            self.assertEqual(validate_loaded_paths([str(os_module)], root, system)[0]['scope'], 'windows_os')
            for name in ('msvcp140.dll', 'msvcp140_1.dll', 'vcruntime140.dll', 'concrt140.dll', 'msvcp_win_extra.dll'):
                with self.subTest(name=name), self.assertRaisesRegex(RuntimeError, 'Host-global VC runtime'):
                    validate_loaded_paths([str(system / 'System32' / name)], root, system)
                self.assertEqual(validate_loaded_paths([str(root / name)], root, system)[0]['scope'], 'private')
            with self.assertRaisesRegex(RuntimeError, 'outside private runtime/Windows'):
                validate_loaded_paths([str(temporary / 'untrusted/msvcp_win.dll')], root, system)

    def test_late_diagnostic_failure_clears_previously_set_success(self):
        report = {'passed': True, 'inference': {'cold_and_warm': True}}
        record_failure(report, RuntimeError('diagnostic write failed'))
        self.assertFalse(report['passed'])
        self.assertEqual(report['error'], 'RuntimeError: diagnostic write failed')

    def test_stage_snapshot_is_retained_before_blocking_work(self):
        with temporary_root() as root, patch('sys.stdout', io.StringIO()):
            journal = StageJournal(root / 'stages.json')
            with journal.stage('import.backend'):
                saved = json.loads(journal.path.read_text())
                self.assertEqual(saved['events'][-1]['state'], 'started')
                self.assertEqual(saved['outer_timeout_seconds'], 900)
            self.assertEqual(json.loads(journal.path.read_text())['events'][-1]['state'], 'completed')
            self.assertFalse(journal.path.with_suffix('.json.tmp').exists())

    def test_stage_exception_does_not_hide_failure_or_expose_exception_text(self):
        with temporary_root() as root, patch('sys.stdout', io.StringIO()):
            journal = StageJournal(root / 'stages.json')
            with self.assertRaisesRegex(ValueError, 'sensitive error text'):
                with journal.stage('worker.serve'):
                    raise ValueError('sensitive error text')
            saved = journal.path.read_text()
            self.assertNotIn('sensitive error text', saved)
            self.assertEqual(json.loads(saved)['events'][-1]['details'], {'exception_type': 'ValueError'})

    def test_atomic_failure_preserves_previous_valid_snapshot(self):
        with temporary_root() as root, patch('sys.stdout', io.StringIO()):
            journal = StageJournal(root / 'stages.json'); journal.record('first', 'completed')
            before = journal.path.read_bytes()
            with patch.object(Path, 'replace', side_effect=OSError('fixture failure')):
                with self.assertRaises(OSError): journal.record('second', 'started')
            self.assertEqual(journal.path.read_bytes(), before)

    def test_event_and_byte_bounds_fail_closed_with_previous_snapshot(self):
        with temporary_root() as root, patch('sys.stdout', io.StringIO()):
            journal = StageJournal(root / 'stages.json'); journal.record('first', 'completed')
            before = journal.path.read_bytes()
            with patch.object(journal, 'MAX_EVENTS', 1):
                with self.assertRaisesRegex(RuntimeError, 'event bound'): journal.record('second', 'started')
            with patch.object(journal, 'MAX_BYTES', 10):
                with self.assertRaisesRegex(RuntimeError, 'byte bound'): journal.record('second', 'started')
            self.assertEqual(journal.path.read_bytes(), before)

    def test_incremental_protocol_capture_preserves_original_frames(self):
        with temporary_root() as root, patch('sys.stdout', io.StringIO()):
            journal = StageJournal(root / 'stages.json'); capture = ProtocolCapture(journal)
            value = json.dumps({'id': 'cold', 'event': 'progress', 'message': 'Loading local model', 'hidden': 'not journaled'}) + '\n'
            for chunk in (value[:13], value[13:]): capture.write(chunk)
            capture.flush()
            self.assertEqual(capture.getvalue(), value)
            saved = json.loads(journal.path.read_text())['events'][-1]
            self.assertEqual(saved['details']['message'], 'Loading local model')
            self.assertNotIn('hidden', saved['details'])
            self.assertEqual(saved['details']['id'], 'cold')

    def test_protocol_bounds_and_invalid_frames_are_rejected(self):
        with temporary_root() as root, patch('sys.stdout', io.StringIO()):
            journal = StageJournal(root / 'stages.json')
            capture = ProtocolCapture(journal)
            with patch.object(capture, 'MAX_BYTES', 3):
                with self.assertRaisesRegex(RuntimeError, 'output bound'): capture.write('long')
            with patch.object(capture, 'MAX_LINE_BYTES', 3):
                with self.assertRaisesRegex(RuntimeError, 'frame bound'): capture.write('long')
            with self.assertRaisesRegex(RuntimeError, 'protocol object'): capture.write('[]\n')

    def test_protocol_journal_is_ascii_safe_and_truncates_large_message(self):
        with temporary_root() as root, patch('sys.stdout', io.StringIO()) as output:
            journal = StageJournal(root / 'stages.json'); capture = ProtocolCapture(journal)
            capture.write(json.dumps({'id': 'cold', 'event': 'error', 'message': '测试\n' * 600}) + '\n')
            detail = json.loads(journal.path.read_text())['events'][0]['details']['message']
            self.assertEqual(len(detail), 512)
            self.assertTrue(output.getvalue().isascii())

    def test_traceback_is_one_shot_and_precedes_third_party_imports(self):
        import verify_runtime
        source = Path(verify_runtime.__file__).read_text()
        self.assertIn('faulthandler.dump_traceback_later(90, repeat=False, file=trace)', source)
        self.assertLess(source.index('faulthandler.dump_traceback_later'), source.index("with journal.stage('worker.module_load')"))
        self.assertIn('faulthandler.cancel_dump_traceback_later()', source)


if __name__ == '__main__':
    unittest.main()
