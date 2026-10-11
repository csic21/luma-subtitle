import importlib.util
import json
import pathlib
import struct
import tempfile
import unittest
from unittest import mock

MODULE = pathlib.Path(__file__).with_name('run.py')
spec = importlib.util.spec_from_file_location('native_install_runner', MODULE)
runner = importlib.util.module_from_spec(spec)
spec.loader.exec_module(runner)


class ControllerTests(unittest.TestCase):
    def test_fixed_fixtures_match_checked_in_hashes(self):
        self.assertEqual(len(runner.check_fixtures()), 3)

    def test_fixture_bytes_are_not_subject_to_checkout_line_endings(self):
        attributes = (runner.REPO / '.gitattributes').read_text()
        self.assertIn('src-tauri/native-install-regression/fixtures/** -text', attributes)

    def test_pe_stack_reserve_and_commit_are_read_at_real_offsets(self):
        with tempfile.TemporaryDirectory() as temp:
            path = pathlib.Path(temp) / 'fixture.exe'
            image = bytearray(512)
            image[:2] = b'MZ'
            struct.pack_into('<I', image, 0x3c, 128)
            image[128:132] = b'PE\0\0'
            struct.pack_into('<H', image, 132, 0x8664)
            struct.pack_into('<H', image, 152, 0x20b)
            struct.pack_into('<QQ', image, 224, 1048576, 4096)
            path.write_bytes(image)
            self.assertEqual(runner.pe_stack(path), {'SizeOfStackReserve': 1048576,
                'SizeOfStackCommit': 4096, 'machine': '0x8664'})

    def test_non_pe_is_not_reported_as_windows_evidence(self):
        with tempfile.TemporaryDirectory() as temp:
            path = pathlib.Path(temp) / 'fixture'
            path.write_bytes(b'not a PE')
            self.assertIsNone(runner.pe_stack(path))

    def test_diagnostic_marker_cannot_be_distributed(self):
        with tempfile.TemporaryDirectory() as temp:
            root = pathlib.Path(temp)
            path = root / 'fixture'
            path.write_bytes(b'prefix' + runner.MARKER + b'suffix')
            with self.assertRaisesRegex(RuntimeError, 'REFUSING DISTRIBUTION'):
                runner.verify_production(path, root / 'report')

    def test_clean_production_verification_records_hash(self):
        with tempfile.TemporaryDirectory() as temp:
            root = pathlib.Path(temp)
            path = root / 'fixture'
            path.write_bytes(b'ordinary binary fixture')
            runner.verify_production(path, root / 'report')
            report = json.loads((root / 'report/production-binary.json').read_text())
            self.assertEqual(report['sha256'], runner.digest(path))
            self.assertTrue(report['diagnostic_marker_absent'])

    def test_candidate_ab_subset_is_also_run_on_fixed_variant(self):
        self.assertTrue(set(runner.AB_CASES).issubset(runner.COMMON_CASES))
        self.assertIn('metrics', runner.AB_CASES)
        self.assertIn('model-cached', runner.AB_CASES)
        self.assertIn('engine-cold', runner.AB_CASES)

    def run_fake_child(self, exit_code, report):
        with tempfile.TemporaryDirectory() as temp:
            root = pathlib.Path(temp)
            def fake_popen(args, **kwargs):
                if report is not None:
                    pathlib.Path(args[-1]).write_text(json.dumps(report))
                process = mock.Mock()
                process.wait.return_value = exit_code
                return process
            with mock.patch.object(runner.subprocess, 'Popen', side_effect=fake_popen):
                return runner.run_case(root / 'fake-executable', 'fixture', 'heap', root, 5)

    def test_zero_os_exit_cannot_hide_failed_native_report(self):
        result = self.run_fake_child(0, {'passed': False, 'error': 'assertion failed'})
        self.assertFalse(result['passed'])
        self.assertEqual(result['exit_code'], 0)
        self.assertEqual(result['assertion_error'], 'assertion failed')

    def test_zero_os_exit_without_report_is_not_a_pass(self):
        self.assertFalse(self.run_fake_child(0, None)['passed'])

    def test_native_fault_cannot_be_hidden_by_a_success_report(self):
        result = self.run_fake_child(-1073741571, {'passed': True})
        self.assertFalse(result['passed'])
        self.assertEqual(result['exit_code_hex'], '0xC00000FD')

    def test_both_successful_exit_and_native_report_are_required(self):
        self.assertTrue(self.run_fake_child(0, {'passed': True})['passed'])


if __name__ == '__main__':
    unittest.main()
