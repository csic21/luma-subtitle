"""Rejected PE input diagnostics remain bounded and do not change acceptance."""
import hashlib
from pathlib import Path
import sys
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from fixture_paths import temporary_root
import native_inventory as native
from test_cpu_build import pe


class NativeInventoryErrorTests(unittest.TestCase):
    def rejection(self, data):
        with temporary_root() as root:
            path = root / 'Lib/problem.pyd'; path.parent.mkdir()
            path.write_bytes(data)
            with self.assertRaises(ValueError) as caught:
                native.inventory(root)
            message = str(caught.exception)
            self.assertNotIn(str(root), message)
            self.assertIn("'Lib/problem.pyd'", message)
            self.assertIn('sha256=' + hashlib.sha256(data).hexdigest(), message)
            self.assertLess(len(message), 900)
            self.assertTrue(message.isascii())
            return message

    def test_rejected_path_retains_relative_file_raw_name_kind_and_rva(self):
        message = self.rejection(pe(normal='../evil.dll'))
        self.assertIn('normal import: Unsafe PE import name at RVA 0x1100', message)
        self.assertIn("b'../evil.dll'", message)

    def test_delay_extensionless_name_is_still_rejected(self):
        message = self.rejection(pe(delayed='Extensionless'))
        self.assertIn('delay import: Unsafe PE import name at RVA 0x1180', message)
        self.assertIn("b'Extensionless'", message)

    def test_raw_non_ascii_and_control_bytes_are_escaped(self):
        data = pe(); data[0x500:0x507] = b'\xffx.dll\0'
        self.assertIn("b'\\xffx.dll'", self.rejection(data))
        message = self.rejection(pe(normal='line\nfeed.dll'))
        self.assertIn(r'line\nfeed.dll', message)
        self.assertNotIn('\n', message)

    def test_long_rejected_name_is_truncated_without_accepting_it(self):
        message = self.rejection(pe(normal='A' * 110 + '/bad.dll'))
        self.assertIn('(truncated)', message)
        self.assertNotIn('A' * 97, message)


if __name__ == '__main__':
    unittest.main()
