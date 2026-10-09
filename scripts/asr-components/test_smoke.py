import os
import io
import json
from pathlib import Path
import subprocess
import sys
import unittest
from fixture_paths import temporary_root

from smoke import terminate_idle_worker, clean_environment, diagnostic_json
from unittest.mock import patch


class SmokeHarnessTests(unittest.TestCase):
    def test_unicode_evidence_survives_windows_legacy_log_encoding(self):
        evidence = {'words': ['Python', 'で', '簡単', 'に', '使える', 'ツール', 'です'],
                    'cwd_before': 'runtime é 测试', 'finder_removed': True}
        for value in (evidence, {'smoke': {'nagisa_unicode': evidence},
                                 'path': 'C:/runtime é 测试/final-report.json'}):
            with self.subTest(final_report='smoke' in value):
                buffer = io.BytesIO(); stream = io.TextIOWrapper(buffer, encoding='cp1252')
                stream.write(diagnostic_json(value, indent=2)); stream.flush()
                self.assertEqual(json.loads(buffer.getvalue().decode('cp1252')), value)
                stream.close()

    def test_clean_environment_keeps_case_insensitive_windows_os_vars_only(self):
        with temporary_root() as tmp, patch.dict(os.environ, {
            'SYSTEMROOT': 'C:\\Windows', 'Processor_Architecture': 'AMD64',
            'PROCESSOR_IDENTIFIER': 'Intel64 Family 6', 'PATH': 'unsafe-path',
            'PIP_INDEX_URL': 'https://invalid.example', 'SECRET_TOKEN': 'never-copy',
        }, clear=True):
            env = clean_environment(tmp / 'home')
        self.assertEqual(env['SYSTEMROOT'], 'C:\\Windows')
        # Windows os.environ uppercases keys even when the fixture supplies
        # mixed case. The preserved OS value, not spelling, is the contract.
        self.assertEqual({key.upper(): value for key, value in env.items()}['PROCESSOR_ARCHITECTURE'], 'AMD64')
        self.assertNotEqual(env['PATH'], 'unsafe-path')
        self.assertNotIn('PIP_INDEX_URL', env); self.assertNotIn('SECRET_TOKEN', env)

    def test_idle_stdin_stays_open_until_terminated(self):
        with temporary_root() as tmp:
            terminate_idle_worker([sys.executable, '-I', '-B', '-c', 'import sys; sys.stdin.readline()'], os.environ.copy(), tmp)

    def test_early_exit_is_not_accepted_as_idle_termination(self):
        with temporary_root() as tmp:
            with self.assertRaisesRegex(AssertionError, 'exited without EOF'):
                terminate_idle_worker([sys.executable, '-I', '-B', '-c', 'pass'], os.environ.copy(), tmp)


if __name__ == '__main__': unittest.main()
