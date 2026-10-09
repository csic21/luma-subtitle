import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

from smoke import terminate_idle_worker


class SmokeHarnessTests(unittest.TestCase):
    def test_idle_stdin_stays_open_until_terminated(self):
        with tempfile.TemporaryDirectory() as tmp:
            terminate_idle_worker([sys.executable, '-I', '-B', '-c', 'import sys; sys.stdin.readline()'], os.environ.copy(), Path(tmp))

    def test_early_exit_is_not_accepted_as_idle_termination(self):
        with tempfile.TemporaryDirectory() as tmp:
            with self.assertRaisesRegex(AssertionError, 'exited without EOF'):
                terminate_idle_worker([sys.executable, '-I', '-B', '-c', 'pass'], os.environ.copy(), Path(tmp))


if __name__ == '__main__': unittest.main()
