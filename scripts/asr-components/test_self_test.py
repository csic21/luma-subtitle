import unittest
import base64
import hashlib
from pathlib import Path
from unittest.mock import patch
from fixture_paths import temporary_root

from self_test import platform_description, optional_vc_runtime, private_openmp_runtime, data_file_diagnostic


class SelfTestReportingTests(unittest.TestCase):
    def test_bundled_data_diagnostics_distinguish_missing_and_unicode_file(self):
        with temporary_root() as tmp:
            root = tmp; path = root / 'private runtime 测试' / 'nagisa.model'
            expected = base64.urlsafe_b64encode(hashlib.sha256(b'fixture-data').digest()).decode().rstrip('=')
            missing = data_file_diagnostic(path, root, expected)
            self.assertFalse(missing['exists']); self.assertFalse(missing['readable'])
            path.parent.mkdir(); path.write_bytes(b'fixture-data')
            present = data_file_diagnostic(path, root, expected)
            self.assertTrue(present['path_contains_non_ascii']); self.assertTrue(present['readable'])
            self.assertTrue(present['record_sha256_match']); self.assertEqual(present['bytes'], 12)
            self.assertFalse(data_file_diagnostic(path, root, 'incorrect-hash')['record_sha256_match'])
            with self.assertRaisesRegex(ValueError, 'escaped'):
                data_file_diagnostic(root.parent / 'outside', root)

    def test_optional_visual_cpp_dlls_are_not_misclassified_as_windows_os(self):
        for name in ['MSVCP140.dll', 'MSVCP140_ATOMIC_WAIT.dll', 'VCRUNTIME140.dll', 'vcruntime140_1.dll', 'concrt140.dll', 'vcomp140.dll', 'vccorlib140.dll']:
            self.assertTrue(optional_vc_runtime(name), name)
        for name in ['kernel32.dll', 'api-ms-win-crt-runtime-l1-1-0.dll', 'ucrtbase.dll', 'ntdll.dll', 'msvcp_win.dll', 'msvcrt.dll']:
            self.assertFalse(optional_vc_runtime(name), name)

    def test_platform_report_never_uses_processor_subprocess(self):
        with patch('platform.platform', side_effect=AssertionError('processor probing forbidden')), patch('subprocess.Popen', side_effect=AssertionError('child forbidden')):
            description = platform_description()
        self.assertIsInstance(description, str)
        self.assertGreater(len(description), 3)

    def test_openmp_cannot_hide_under_systemroot_allowlist(self):
        for name in ('libiomp5md.dll', 'libomp.dll', 'LIBOMP140.X86_64.DLL'):
            self.assertTrue(private_openmp_runtime(name))
        for name in ('kernel32.dll', 'combase.dll', 'msvcp_win.dll'):
            self.assertFalse(private_openmp_runtime(name))


if __name__ == '__main__': unittest.main()
