import io
import json
from pathlib import Path
import sys
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from fixture_paths import temporary_root
from verify_runtime import ProtocolCapture, StageJournal, record_failure, validate_loaded_paths


class RuntimeDiagnosticsTests(unittest.TestCase):
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
