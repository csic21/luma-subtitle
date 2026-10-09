import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

from smoke import terminate_idle_worker, clean_environment
from unittest.mock import patch


class SmokeHarnessTests(unittest.TestCase):
    def test_clean_environment_keeps_case_insensitive_windows_os_vars_only(self):
        with tempfile.TemporaryDirectory() as tmp, patch.dict(os.environ, {
            'SYSTEMROOT': 'C:\\Windows', 'Processor_Architecture': 'AMD64',
            'PROCESSOR_IDENTIFIER': 'Intel64 Family 6', 'PATH': 'unsafe-path',
            'PIP_INDEX_URL': 'https://invalid.example', 'SECRET_TOKEN': 'never-copy',
        }, clear=True):
            env = clean_environment(Path(tmp) / 'home')
        self.assertEqual(env['SYSTEMROOT'], 'C:\\Windows')
        self.assertEqual(env['Processor_Architecture'], 'AMD64')
        self.assertNotEqual(env['PATH'], 'unsafe-path')
        self.assertNotIn('PIP_INDEX_URL', env); self.assertNotIn('SECRET_TOKEN', env)

    def test_idle_stdin_stays_open_until_terminated(self):
        with tempfile.TemporaryDirectory() as tmp:
            terminate_idle_worker([sys.executable, '-I', '-B', '-c', 'import sys; sys.stdin.readline()'], os.environ.copy(), Path(tmp))

    def test_early_exit_is_not_accepted_as_idle_termination(self):
        with tempfile.TemporaryDirectory() as tmp:
            with self.assertRaisesRegex(AssertionError, 'exited without EOF'):
                terminate_idle_worker([sys.executable, '-I', '-B', '-c', 'pass'], os.environ.copy(), Path(tmp))


if __name__ == '__main__': unittest.main()
