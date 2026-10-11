import importlib.util
import json
import pathlib
import struct
import tempfile
import unittest

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


if __name__ == '__main__':
    unittest.main()
