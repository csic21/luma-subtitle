"""Structured native outcomes never turn a failed proof into a release pass."""
import copy
import json
from pathlib import Path
import subprocess
import sys
from types import SimpleNamespace
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from fixture_paths import temporary_root
import build_cpu as build

SHA = 'a' * 40


def failed_state(outcome='timed_out', **extra):
    return {'passed': False, 'wheel_reproduced': True, 'native_inference_passed': False,
            'native_verifier': dict(started=True, outcome=outcome, **extra)}


def environment():
    return {'CPU_DIAGNOSTIC_EXPORT': 'true', 'GITHUB_REPOSITORY': 'csic21/luma-subtitle',
            'GITHUB_JOB': 'windows-cpu-proof', 'GITHUB_SHA': SHA,
            'GITHUB_REF': 'refs/heads/feat/optional-asr-engines', 'GITHUB_EVENT_NAME': 'push',
            'GITHUB_WORKFLOW_REF': 'csic21/luma-subtitle/.github/workflows/asr-ct2-cpu.yml@refs/heads/feat/optional-asr-engines',
            'GITHUB_RUN_ID': '123', 'GITHUB_RUN_ATTEMPT': '1'}


class DiagnosticCaptureHookTests(unittest.TestCase):
    def test_outcomes_preserve_original_error_and_exact_bounded_state(self):
        cases = [(subprocess.TimeoutExpired('verifier', 900), {'started': True, 'outcome': 'timed_out', 'timeout_seconds': 900}),
                 (build.CommandFailed(7, 'failed'), {'started': True, 'outcome': 'failed', 'returncode': 7}),
                 (OSError('launch failed'), {'started': True, 'outcome': 'launch_error'})]
        for error, expected in cases:
            with self.subTest(error=type(error).__name__):
                status = {}
                with self.assertRaises(type(error)) as raised:
                    build.run_native_verifier(status, lambda: (_ for _ in ()).throw(error))
                self.assertIs(raised.exception, error)
                self.assertEqual(status['native_verifier'], expected)
        status = {}; build.run_native_verifier(status, lambda: None)
        self.assertEqual(status['native_verifier'], {'started': True, 'outcome': 'passed'})

    def test_cancellation_is_never_eligible(self):
        errors = [build.CommandFailed(code, 'cancelled') for code in (-1, -1073741510, 130, 143, 0xC000013A)]
        errors += [KeyboardInterrupt(), SystemExit(1)]
        for error in errors:
            with self.subTest(error=repr(error)):
                status = failed_state()
                with self.assertRaises(type(error)):
                    build.run_native_verifier(status, lambda: (_ for _ in ()).throw(error))
                self.assertEqual(status['native_verifier']['outcome'], 'cancelled')
                self.assertFalse(build.diagnostic_capture_allowed(status))

    def test_only_failed_verifier_with_equal_wheels_can_capture(self):
        state = failed_state(timeout_seconds=900)
        self.assertTrue(build.diagnostic_capture_allowed(state))
        self.assertTrue(build.diagnostic_capture_allowed(failed_state('failed', returncode=1)))
        for key, value in (('passed', True), ('native_inference_passed', True), ('wheel_reproduced', False), ('native_verifier', {})):
            changed = copy.deepcopy(state); changed[key] = value
            self.assertFalse(build.diagnostic_capture_allowed(changed))
        for code in (0, -1, 130, 143, 0xC000013A, 0x100000000, True, '1'):
            self.assertFalse(build.diagnostic_capture_allowed(failed_state('failed', returncode=code)))
        for timeout in (90, 901, '900'):
            self.assertFalse(build.diagnostic_capture_allowed(failed_state(timeout_seconds=timeout)))
        for outcome in ('running', 'passed', 'launch_error', 'cancelled'):
            self.assertFalse(build.diagnostic_capture_allowed(failed_state(outcome, returncode=1)))

    def test_origin_is_exact_request_context_without_extra_environment(self):
        env = environment(); env['UNRELATED_SECRET'] = 'must never be copied'
        actual = build.diagnostic_origin(SHA, env)
        self.assertEqual(set(actual), {'repository', 'run_id', 'run_attempt', 'job', 'head_sha', 'source_sha', 'ref', 'event_name', 'workflow_ref'})
        self.assertEqual((actual['run_id'], actual['run_attempt']), (123, 1))
        self.assertEqual(actual['source_sha'], SHA)
        for field, value in [('CPU_DIAGNOSTIC_EXPORT', 'false'), ('GITHUB_REPOSITORY', 'other/repo'),
                             ('GITHUB_JOB', 'other'), ('GITHUB_SHA', 'b' * 40),
                             ('GITHUB_REF', 'refs/heads/main'), ('GITHUB_EVENT_NAME', 'workflow_dispatch'),
                             ('GITHUB_WORKFLOW_REF', 'other'), ('GITHUB_RUN_ID', '0'),
                             ('GITHUB_RUN_ATTEMPT', '-1'), ('GITHUB_RUN_ID', '1\n'), ('GITHUB_RUN_ID', '1' * 17)]:
            changed = env | {field: value}
            with self.subTest(field=field, value=value), self.assertRaises(ValueError):
                build.diagnostic_origin(SHA, changed)

    def test_successful_capture_keeps_pinned_result_bytes_and_status_unchanged(self):
        with temporary_root() as root:
            args = SimpleNamespace(diagnostic_output=root/'output', reports=root, source_sha=SHA, cache=root/'cache', work=root/'work')
            status = failed_state(timeout_seconds=900); before = copy.deepcopy(status)
            origin = build.diagnostic_origin(SHA, environment()); seen = []
            def capture(**kwargs):
                seen.append((kwargs, (root/'result.json').read_bytes()))
            with patch('package_publication.package_diagnostic', side_effect=capture):
                build.capture_failed_verifier(args, status, root/'first.whl', root/'second.whl', origin)
            self.assertEqual(status, before)
            build.dump(root/'result.json', status)
            self.assertEqual((root/'result.json').read_bytes(), seen[0][1])
            self.assertEqual(seen[0][0]['origin'], origin)
            self.assertEqual(seen[0][0]['wheel'], root/'first.whl')
            self.assertFalse((root/'diagnostic-capture.json').exists())

    def test_capture_error_is_separate_and_never_relabels_original_failure(self):
        with temporary_root() as root:
            args = SimpleNamespace(diagnostic_output=root/'output', reports=root, source_sha=SHA, cache=root/'cache', work=root/'work')
            status = failed_state(timeout_seconds=900); before = copy.deepcopy(status)
            with patch('package_publication.package_diagnostic', side_effect=ValueError('gate rejected')):
                build.capture_failed_verifier(args, status, None, None, None)
            self.assertEqual(status, before)
            self.assertFalse(json.loads((root/'diagnostic-capture.json').read_text())['captured'])
            self.assertFalse((root/'output/diagnostic-manifest.json').exists())

    def test_no_opt_in_success_or_earlier_failure_never_calls_packager(self):
        with temporary_root() as root, patch('package_publication.package_diagnostic') as package:
            args = SimpleNamespace(diagnostic_output=None, reports=root)
            build.capture_failed_verifier(args, failed_state(timeout_seconds=900), None, None, None)
            args.diagnostic_output = root/'output'
            for state in ({}, {'passed': True}, failed_state('running')):
                build.capture_failed_verifier(args, state, None, None, None)
            package.assert_not_called()
            self.assertFalse((root/'result.json').exists())


if __name__ == '__main__':
    unittest.main()
