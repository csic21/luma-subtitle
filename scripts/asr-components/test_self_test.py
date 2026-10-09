import unittest
from unittest.mock import patch

from self_test import platform_description, optional_vc_runtime


class SelfTestReportingTests(unittest.TestCase):
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


if __name__ == '__main__': unittest.main()
