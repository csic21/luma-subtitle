import unittest
from unittest.mock import patch

from self_test import platform_description


class SelfTestReportingTests(unittest.TestCase):
    def test_platform_report_never_uses_processor_subprocess(self):
        with patch('platform.platform', side_effect=AssertionError('processor probing forbidden')), patch('subprocess.Popen', side_effect=AssertionError('child forbidden')):
            description = platform_description()
        self.assertIsInstance(description, str)
        self.assertGreater(len(description), 3)


if __name__ == '__main__': unittest.main()
