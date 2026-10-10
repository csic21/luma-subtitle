import os
import io
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from fixture_paths import temporary_root, windows_short_path_alias

import smoke
from smoke import terminate_idle_worker, clean_environment, diagnostic_json, managed_worker_runtime_probe
from unittest.mock import patch


class SmokeHarnessTests(unittest.TestCase):
    def exercise_owned_smoke_root(self, parent, alias):
        from test_own_cpu_recipe import SyntheticPublishedCpu
        import own_cpu_recipe as cpu
        from build import archive_tree, sha256
        from types import SimpleNamespace

        fixture = SyntheticPublishedCpu(parent / 'synthetic-reference')
        output = parent / 'manifest'; output.mkdir()
        archive = output / 'reference.zip'
        archive_tree(fixture.runtime, archive)
        files = [path for path in fixture.runtime.rglob('*') if path.is_file()]
        manifest = output / 'reference.json'
        manifest.write_text(json.dumps({'id': cpu.PACK_ID, 'platform': 'windows-x64',
            'backend': 'faster-whisper', 'entrypoint': 'python.exe',
            'installed_bytes': sum(path.stat().st_size for path in files), 'max_files': len(files),
            'archive': {'url': 'https://example.invalid/reference.zip',
                        'bytes': archive.stat().st_size, 'sha256': sha256(archive)}}), encoding='utf-8')
        roots = []

        class ReferenceReached(Exception): pass

        def inspect_reference(_manifest, root, _worker, _cache, _destination):
            roots.append(root.parent)
            self.assertEqual(root, root.resolve(strict=True))
            self.assertEqual(root.parent.parent, alias.resolve(strict=True))
            cpu.verify_installed_wheel(root, fixture.wheel, fixture.cache, fixture.provenance)
            # Canonicalizing the freshly owned root must not normalize unsafe
            # descendants or weaken the existing installed-byte identity guard.
            native = root / 'Lib/site-packages/ctranslate2/ctranslate2.dll'
            original = native.read_bytes(); native.write_bytes(b'x' * len(original))
            with self.assertRaisesRegex(ValueError, 'differs'):
                cpu.verify_installed_wheel(root, fixture.wheel, fixture.cache, fixture.provenance)
            native.write_bytes(original)
            original_lstat = Path.lstat
            def reparse(path, *args, **kwargs):
                info = original_lstat(path, *args, **kwargs)
                if path == native.parent:
                    return SimpleNamespace(st_mode=info.st_mode, st_file_attributes=0x400)
                return info
            with patch.object(Path, 'lstat', reparse), self.assertRaisesRegex(ValueError, 'reparse'):
                cpu.verify_installed_wheel(root, fixture.wheel, fixture.cache, fixture.provenance)
            raise ReferenceReached

        argv = ['smoke.py', '--manifest', str(manifest), '--worker', str(fixture.worker),
                '--cache', str(fixture.cache)]
        # Use the real smoke main and its real owned-root allocator. Stop before
        # engine subprocess/native execution; these package bytes are synthetic.
        with patch.object(tempfile, 'tempdir', str(alias)), patch.object(sys, 'argv', argv), \
             patch.object(smoke, 'prepare_cpu_smoke', side_effect=inspect_reference):
            with self.assertRaises(ReferenceReached): smoke.main()
        self.assertEqual(len(roots), 1)
        self.assertFalse(roots[0].exists(), 'Smoke must clean only its owned temporary child')
        self.assertTrue(alias.exists())
        self.assertTrue(fixture.runtime.exists())

    def test_smoke_canonicalizes_owned_root_before_deriving_runtime_paths(self):
        with temporary_root() as parent:
            actual = parent / 'long temporary parent'; actual.mkdir()
            alias = parent / 'alias'
            try: alias.symlink_to(actual, target_is_directory=True)
            except OSError as error: self.skipTest('Directory aliases unavailable: ' + str(error))
            self.exercise_owned_smoke_root(parent, alias)
            self.assertTrue(alias.is_symlink())

    @unittest.skipUnless(os.name == 'nt', 'Requires native Windows GetShortPathNameW')
    def test_actual_windows_short_temp_parent_reaches_strict_cpu_verifier(self):
        with temporary_root(prefix='Luma native smoke long parent ') as parent:
            try: alias = windows_short_path_alias(parent)
            except OSError as error: self.skipTest('Native short-path probe unavailable: ' + str(error))
            if alias is None: self.skipTest('This filesystem does not expose a distinct 8.3 alias')
            self.assertNotEqual(os.path.normcase(str(alias)), os.path.normcase(str(parent)))
            self.exercise_owned_smoke_root(parent, alias)

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

    def test_managed_runtime_probe_calls_actual_entrypoint_twice_in_owned_child(self):
        # A dependency-free stub proves the harness path; only native CI can
        # establish the real engine/JIT evidence represented by these fields.
        with temporary_root() as tmp:
            worker = tmp/'worker é 测试.py'
            worker.write_text('''import os, sys
from pathlib import Path
from types import SimpleNamespace
sys.prefix = str(Path(__file__).resolve().parent)
LUMA_MANAGED_QWEN_RUNTIME = True
_luma_numba_proof = None
calls = 0
module = SimpleNamespace(__file__=__file__)
def configure_offline(): pass
def offline_audit(event, args): pass
def luma_check_numba_workqueue(require_initialized=False):
    assert require_initialized and calls == 2
    return 'workqueue'
def runtime(backend, device):
    global calls, _luma_numba_proof
    print('legitimate upstream diagnostic')
    os.write(1,b'native stdout diagnostic\\n')
    calls += 1
    assert backend == 'qwen3-asr-transformers' and device == 'cpu' and calls <= 2
    if _luma_numba_proof is None:
        _luma_numba_proof = dict(selected='workqueue',numeric_passed=True,parallel_jit_tested=True)
    return dict(device='cpu',core=module,module=module)
''',encoding='utf-8')
            (tmp/'self_test.py').write_text('def loaded_native_libraries(host_security_modules):\n    host_security_modules.append(dict(kind="windows-defender-amsi", verified=True))\n    return 3\n')
            report = managed_worker_runtime_probe(sys.executable,worker,os.environ.copy(),tmp)
            self.assertTrue(report['successful_proof_reused']); self.assertFalse(report['inference'])
            self.assertEqual(report['private_native_libraries_checked'],3)
            self.assertEqual(report['verified_host_security_modules'],[dict(kind='windows-defender-amsi',verified=True)])
            self.assertEqual(len(report['embedded_worker_sha256']),64)
            with patch('windows_crt_proof.helper_process',return_value=(0,'{}\n{}\n','')):
                with self.assertRaisesRegex(ValueError,'exactly one'):
                    managed_worker_runtime_probe(sys.executable,worker,os.environ.copy(),tmp)


if __name__ == '__main__': unittest.main()
