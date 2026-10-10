from pathlib import Path
import struct
import sys
import unittest
import zipfile

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from fixture_paths import temporary_root

import repro_diagnostics as diagnostic
from test_cpu_build import pe


class ReproDiagnosticsTests(unittest.TestCase):
    def test_native_header_sections_and_codeview_are_read_only(self):
        data = pe()
        struct.pack_into('<I', data, 0x88, 12345)
        # Debug directory at RVA 0x1200 / file 0x600; RSDS record at file 0x640.
        struct.pack_into('<II', data, 0x98 + 112 + 6 * 8, 0x1200, 28)
        payload = b'RSDS' + bytes(range(16)) + struct.pack('<I', 1) + b'C:\\build-one\\sample.pdb\0'
        struct.pack_into('<IIHHIIII', data, 0x600, 0, 67890, 0, 0, 2, len(payload), 0x1240, 0x640)
        data[0x640:0x640 + len(payload)] = payload
        before = bytes(data)
        report = diagnostic.pe_details(data)
        self.assertEqual(bytes(data), before)
        self.assertEqual(report['coff_timestamp_raw'], 12345)
        self.assertEqual(report['debug_records'][0]['timestamp_raw'], 67890)
        self.assertEqual(report['debug_records'][0]['pdb_path'], r'C:\build-one\sample.pdb')
        self.assertEqual(len(report['sections'][0]['sha256']), 64)

    def test_per_member_comparison_preserves_mismatch_without_binary_output(self):
        with temporary_root() as work:
            root = Path(work); paths = [root / 'one.whl', root / 'two.whl']
            for index, path in enumerate(paths):
                native = pe(); struct.pack_into('<I', native, 0x88, index)
                with zipfile.ZipFile(path, 'w') as archive:
                    archive.writestr('package/core.dll', native)
                    archive.writestr('package/version.py', 'unchanged')
            report = diagnostic.compare_wheels(*paths)
            self.assertFalse(report['identical'])
            self.assertFalse(report['binary_bytes_modified'])
            self.assertEqual(report['different_members'], ['package/core.dll'])
            self.assertEqual(report['first_members']['package/core.dll']['pe']['coff_timestamp_raw'], 0)
            self.assertEqual(report['second_members']['package/core.dll']['pe']['coff_timestamp_raw'], 1)

    def test_invalid_pe_diagnostic_bounds_fail_closed(self):
        with self.assertRaises(ValueError):
            diagnostic.pe_details(pe()[:100])
        data = pe(); struct.pack_into('<II', data, 0x98 + 112 + 6 * 8, 0x1200, 29)
        with self.assertRaises(ValueError):
            diagnostic.pe_details(data)
