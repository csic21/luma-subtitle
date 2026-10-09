import base64
import copy
import csv
import hashlib
import io
import json
import os
import re
import subprocess
from pathlib import Path
import struct
import tarfile
import sys
import unittest
from unittest.mock import patch
from types import SimpleNamespace
import zipfile

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from fixture_paths import temporary_root

import build_cpu as build
import native_inventory as native


def pe(normal='kernel32.dll', delayed='msvcp140_1.dll', pe32=False):
    data = bytearray(2048); data[:2] = b'MZ'; struct.pack_into('<I', data, 0x3c, 0x80)
    data[0x80:0x84] = b'PE\0\0'
    optional, size = 0x98, 224 if pe32 else 240
    struct.pack_into('<HH', data, 0x84, 0x14c if pe32 else 0x8664, 1)
    struct.pack_into('<H', data, 0x94, size)
    struct.pack_into('<H', data, optional, 0x10b if pe32 else 0x20b)
    struct.pack_into('<I' if pe32 else '<Q', data, optional + (28 if pe32 else 24), 0x400000)
    struct.pack_into('<I', data, optional + (92 if pe32 else 108), 16)
    directories = optional + (96 if pe32 else 112)
    struct.pack_into('<II', data, directories + 8, 0x1000, 40)
    struct.pack_into('<II', data, directories + 13 * 8, 0x1080, 64)
    struct.pack_into('<IIII', data, optional + size + 8, 1024, 0x1000, 1024, 0x400)
    struct.pack_into('<IIIII', data, 0x400, 1, 0, 0, 0x1100, 1)
    struct.pack_into('<IIIIIIII', data, 0x480, 1, 0x1180, 0, 1, 1, 0, 0, 0)
    data[0x500:0x500 + len(normal) + 1] = normal.encode() + b'\0'
    data[0x580:0x580 + len(delayed) + 1] = delayed.encode() + b'\0'
    return data


class NativeTests(unittest.TestCase):
    def test_full_source_proof_uses_integrated_auditor_policy_and_distinct_model(self):
        import inspect
        source = inspect.getsource(build.private_proof)
        for argument in ('--source-sha', '--host-auditor', '--host-auditor-sha256', '--build-provenance', '--build-provenance-sha256', '--switch-model'):
            self.assertIn(argument, source)
        self.assertIn('shutil.copytree(model, switch_model)', source)
        self.assertNotIn('lifecycle_probe.py', source)
        self.assertNotIn('thread_override', source)

    def test_fixed_native_root_is_fresh_and_first_output_survives_cleanup(self):
        with temporary_root() as directory:
            work = Path(directory)
            first = work / 'build-1'; first.mkdir()
            wheel = first / 'result.whl'; wheel.write_bytes(b'exact retained wheel')
            native = build.prepare_native_root(work)
            (native / 'stale.obj').write_bytes(b'must not be reused')
            with self.assertRaisesRegex(ValueError, 'must be absent'):
                build.prepare_native_root(work)
            build.retire_native_root(work, native, wheel)
            self.assertEqual(wheel.read_bytes(), b'exact retained wheel')
            second = build.prepare_native_root(work)
            self.assertEqual(second, native)
            self.assertEqual(list(second.iterdir()), [])
            self.assertEqual(wheel.read_bytes(), b'exact retained wheel')

    def test_fixed_native_cleanup_rejects_wrong_root_or_unretained_output(self):
        with temporary_root() as directory:
            work = Path(directory); native = build.prepare_native_root(work)
            (native / 'output.whl').write_bytes(b'not safely retained')
            with self.assertRaisesRegex(ValueError, 'Unsafe native cleanup'):
                build.retire_native_root(work, native, native / 'output.whl')
            first = work / 'build-1'; first.mkdir(); wheel = first / 'result.whl'; wheel.write_bytes(b'ok')
            other = work / 'other'; other.mkdir()
            with self.assertRaisesRegex(ValueError, 'Unsafe native cleanup'):
                build.retire_native_root(work, other, wheel)
            self.assertTrue((native / 'output.whl').is_file())

    def test_unicode_failure_status_survives_host_cp1252_console(self):
        status = {'passed': False, 'error': 'RuntimeError: Relocated private Python é 测试/failed.dll'}
        raw = io.BytesIO()
        with io.TextIOWrapper(raw, encoding='cp1252', errors='strict') as console:
            with patch('sys.stdout', console):
                build.print_host_status(status)
            self.assertEqual(json.loads(raw.getvalue().decode('cp1252')), status)
            self.assertTrue(raw.getvalue().isascii())
        with temporary_root() as work:
            path = Path(work) / 'result.json'
            build.dump(path, status)
            self.assertIn('测试'.encode('utf-8'), path.read_bytes())
            self.assertEqual(build.load(path), status)

    def read(self, data):
        with temporary_root() as work:
            path = Path(work) / 'test.dll'; path.write_bytes(data)
            return native.pe_imports(path)

    def test_normal_and_delayed_imports(self):
        result = self.read(pe())
        self.assertEqual(result, {'machine': '0x8664', 'normal': ['kernel32.dll'], 'delay': ['msvcp140_1.dll']})

    def test_32_bit_launcher_inventory(self):
        self.assertEqual(self.read(pe(pe32=True))['machine'], '0x14c')

    def test_old_delay_virtual_address_form(self):
        data = pe(pe32=True)
        struct.pack_into('<II', data, 0x480, 0, 0x401180)
        self.assertEqual(self.read(data)['delay'], ['msvcp140_1.dll'])

    def test_truncated_and_invalid_rva_rejected(self):
        for data in (pe()[:100], bytearray(100)):
            with self.assertRaises(ValueError):
                self.read(data)
        data = pe(); struct.pack_into('<I', data, 0x40c, 0xffff0000)
        with self.assertRaises(ValueError):
            self.read(data)

    def test_unknown_delay_attributes_rejected(self):
        data = pe(); struct.pack_into('<I', data, 0x480, 2)
        with self.assertRaises(ValueError):
            self.read(data)

    def test_path_import_rejected(self):
        with self.assertRaises(ValueError):
            self.read(pe(normal='../evil.dll'))

    def test_renamed_crt_is_not_a_plain_crt_replacement(self):
        files = [dict(path='numpy.libs/msvcp140-1234.dll', normal=[], delay=[], machine='0x8664'),
                 dict(path='ctranslate2/core.dll', normal=['msvcp140.dll'], delay=['msvcp140_1.dll'], machine='0x8664')]
        report = native.closure(files)
        self.assertFalse(report['passed'])
        self.assertEqual({x['resolution'] for x in report['blocked_dependencies']}, {'missing_private_crt'})

    def test_gpu_openmp_rejected_even_without_importers(self):
        self.assertFalse(native.closure([dict(path='cudnn64_9.dll', normal=[], delay=[])])['passed'])
        self.assertEqual(native.classify('libiomp5md.dll', {'libiomp5md.dll': ['libiomp5md.dll']}), 'forbidden')

    def test_whitelisted_os_dependencies_only(self):
        self.assertEqual(native.classify('ole32.dll', {}), 'windows_os')
        self.assertEqual(native.classify('api-ms-win-crt-runtime-l1-1-0.dll', {}), 'windows_api_set')
        self.assertEqual(native.classify('random_global.dll', {}), 'unresolved')
        self.assertEqual(native.classify('msvcp140.dll', {}), 'missing_private_crt')


class SourceTests(unittest.TestCase):
    def test_source_and_license_locks(self):
        build.validate_lock(build.load(build.HERE / 'sources.lock.json'))
        lock, files = build.notice_files()
        self.assertGreaterEqual(len(files), 10)
        self.assertTrue(any(x['component'] == 'thread-pool' for x in lock['files']))
        self.assertTrue(any(x['component'] == 'pybind11' for x in lock['files']))

    def test_deterministic_mapping_flag_is_explicit_and_process_local(self):
        original = {'UNCHANGED': 'value'}
        result = build.deterministic_environment(Path('private-build'), original, {'source_date_epoch': 1})
        self.assertEqual(original, {'UNCHANGED': 'value'})
        self.assertIn('/experimental:deterministic', result['_CL_'])
        self.assertIn('/pathmap:private-build=C:\\luma-ct2-build', result['_CL_'])
        self.assertEqual(result['_LINK_'], '/Brepro /INCREMENTAL:NO')

    def test_path_mapping_probe_rejects_ignored_compiler_flag(self):
        for diagnostic, valid in [('const char* source_file = "C:\\\\luma-ct2-build\\\\probe.cpp";\n', True),
                                  ("cl : warning D9007 : '/pathmap:' option ignored\n", False)]:
            with temporary_root() as work:
                root = Path(work); reports = root / 'reports'; reports.mkdir()
                args = SimpleNamespace(work=root, reports=reports)
                def fake_command(*args, **kwargs):
                    kwargs['logfile'].write_text(diagnostic)
                with patch.object(build, 'command', side_effect=fake_command):
                    if valid:
                        build.probe_path_mapping(args, {}, {'source_date_epoch': 1})
                        self.assertTrue((reports / 'compiler-probe.json').is_file())
                    else:
                        with self.assertRaisesRegex(RuntimeError, 'did not apply'):
                            build.probe_path_mapping(args, {}, {'source_date_epoch': 1})

    def test_functional_proof_runs_on_mismatch_and_final_gate_still_fails(self):
        calls = []
        status = {'wheel_reproduced': False, 'native_inference_passed': False}
        with self.assertRaisesRegex(RuntimeError, 'Independent builds differ'):
            build.enforce_independent_proofs(status, lambda: calls.append('exact first wheel'))
        self.assertEqual(calls, ['exact first wheel'])
        self.assertTrue(status['native_inference_passed'])
        self.assertIn('reproducibility_error', status)

    def test_both_independent_proof_failures_are_retained(self):
        def fail():
            raise ValueError('functional fixture failure')
        status = {'wheel_reproduced': False, 'native_inference_passed': False}
        with self.assertRaisesRegex(RuntimeError, 'Independent builds differ'):
            build.enforce_independent_proofs(status, fail)
        self.assertEqual(status['functional_error'], 'ValueError: functional fixture failure')
        self.assertFalse(status['native_inference_passed'])
        with self.assertRaisesRegex(ValueError, 'functional fixture failure'):
            build.enforce_independent_proofs({'wheel_reproduced': True}, fail)

    def test_build_tool_scripts_scheme_requires_exact_private_payload(self):
        with temporary_root() as work:
            root = Path(work); archive = root / 'ninja.whl'
            member = 'ninja-1.11.1.4.data/scripts/ninja.exe'
            target = root / 'Scripts/ninja.exe'
            with zipfile.ZipFile(archive, 'w') as wheel:
                wheel.writestr(member, b'PINNED EXE BYTES')
            with self.assertRaises(ValueError):
                build.verified_tool_payload(root, archive, member, 'Scripts/ninja.exe')
            target.parent.mkdir(); target.write_bytes(b'PINNED EXE BYTES')
            self.assertEqual(build.verified_tool_payload(root, archive, member, 'Scripts/ninja.exe'), target)
            target.write_bytes(b'WRONG EXE')
            with self.assertRaises(ValueError):
                build.verified_tool_payload(root, archive, member, 'Scripts/ninja.exe')

    def test_source_version_accepts_upstream_docstring_without_execution(self):
        build.validate_source_version('"""Version information."""\n\n__version__ = "4.8.2"\n')
        build.validate_source_version("# metadata\r\n__version__ = '4.8.2'\r\n")
        for source in ('__version__ = "4.8.1"', '__version__ = str("4.8.2")',
                       '__version__ = "4.8.2"\nprint("not allowed")',
                       '__version__ = "4.8.2"\n__version__ = "4.8.2"',
                       'other = __version__ = "4.8.2"', 'invalid python !'):
            with self.assertRaises(ValueError):
                build.validate_source_version(source)

    def test_pinned_speech_convolution_source_requires_both_primitives(self):
        source = build.HERE / 'fixtures/ctranslate2'
        options = build.load(build.HERE / 'sources.lock.json')['onednn_cmake']
        inventory = build.validate_primitive_coverage(source, options)
        self.assertEqual(inventory['direct_cpp_primitives'], ['CONVOLUTION', 'REORDER'])
        self.assertEqual(inventory['independent_post_op_primitives'], [])
        self.assertEqual(len(inventory['low_level_gemm_apis']), 3)
        for selected in ('MATMUL', 'MATMUL;CONVOLUTION', 'MATMUL;REORDER'):
            with self.assertRaisesRegex(ValueError, 'primitive is missing'):
                build.validate_primitive_coverage(source, dict(options, DNNL_ENABLE_PRIMITIVE=selected))
        with temporary_root() as directory:
            root = Path(directory)
            for path in source.rglob('*.cc'):
                target = root / path.relative_to(source); target.parent.mkdir(parents=True, exist_ok=True)
                target.write_bytes(path.read_bytes())
            (root / 'src/new.cc').write_text('dnnl::eltwise_forward extra;', encoding='utf-8')
            with self.assertRaisesRegex(ValueError, 'source/API inventory changed'):
                build.validate_primitive_coverage(root, options)

    def test_gpu_or_openmp_lock_cannot_be_enabled(self):
        original = build.load(build.HERE / 'sources.lock.json')
        for field in ('WITH_CUDA', 'WITH_CUDNN', 'WITH_MKL'):
            changed = copy.deepcopy(original); changed['ct2_cmake'][field] = 'ON'
            with self.assertRaises(ValueError):
                build.validate_lock(changed)
        changed = copy.deepcopy(original); changed['onednn_cmake']['DNNL_CPU_RUNTIME'] = 'OMP'
        with self.assertRaises(ValueError):
            build.validate_lock(changed)

    def test_source_path_escape_rejected(self):
        for name in ('../x', '/x', 'a//b', 'a/./b', 'C:/x', 'a\\b'):
            with self.assertRaises(ValueError):
                build.safe_path(name)

    def test_source_archive_links_and_duplicates_rejected(self):
        for bad in ('symlink', 'duplicate'):
            with temporary_root() as work:
                root = Path(work); archive = root / 'source.tgz'
                with tarfile.open(archive, 'w:gz') as tar:
                    member = tarfile.TarInfo('source/file')
                    if bad == 'symlink':
                        member.type = tarfile.SYMTYPE; member.linkname = '../../other'; tar.addfile(member)
                    else:
                        member.size = 1; tar.addfile(member, io.BytesIO(b'a')); tar.addfile(member, io.BytesIO(b'b'))
                with self.assertRaises(ValueError):
                    build.extract_source(archive, root / 'out')

    def test_source_archive_extracts_regular_files(self):
        with temporary_root() as work:
            root = Path(work); archive = root / 'source.tgz'
            with tarfile.open(archive, 'w:gz') as tar:
                member = tarfile.TarInfo('source/file'); member.size = 2; tar.addfile(member, io.BytesIO(b'ok'))
            build.extract_source(archive, root / 'out')
            self.assertEqual((root / 'out/file').read_bytes(), b'ok')

    def test_canonical_wheel_is_repeatable_and_preserves_payload(self):
        with temporary_root() as work:
            root = Path(work); original = root / 'original.whl'
            info = 'ctranslate2-4.8.2.dist-info'
            with zipfile.ZipFile(original, 'w') as archive:
                archive.writestr(info + '/METADATA', 'Name: ctranslate2\nVersion: 4.8.2\nClassifier: Environment :: GPU :: NVIDIA CUDA :: 12\n')
                archive.writestr(info + '/WHEEL', 'Build: 1lumacpu\nTag: cp312-cp312-win_amd64\n')
                archive.writestr(info + '/RECORD', '')
                archive.writestr('ctranslate2/_ext.cp312-win_amd64.pyd', b'UNCHANGED BINARY')
            license_file = root / 'Lib/site-packages/pybind11-2.11.1.dist-info/LICENSE'
            license_file.parent.mkdir(parents=True)
            notice = next(i for i in build.load(build.HERE / 'notices.lock.json')['files'] if i['component'] == 'pybind11')
            license_file.write_bytes((build.HERE / notice['path']).read_bytes())
            for name in ('one.whl', 'two.whl'):
                build.canonical_wheel(original, root / name, {'variant': 'test'}, root)
            self.assertEqual(build.digest(root / 'one.whl'), build.digest(root / 'two.whl'))
            with zipfile.ZipFile(root / 'one.whl') as archive:
                self.assertEqual(archive.read('ctranslate2/_ext.cp312-win_amd64.pyd'), b'UNCHANGED BINARY')
                self.assertNotIn(b'GPU', archive.read(info + '/METADATA'))
                for name, hashed, size in csv.reader(io.StringIO(archive.read(info + '/RECORD').decode())):
                    if not hashed:
                        self.assertEqual(name, info + '/RECORD'); continue
                    data = archive.read(name)
                    expected = base64.urlsafe_b64encode(hashlib.sha256(data).digest()).rstrip(b'=').decode()
                    self.assertEqual(hashed, 'sha256=' + expected)
                    self.assertEqual(size, str(len(data)))

    def test_developer_shell_uses_batch_and_memory_only_environment(self):
        script = (build.HERE / 'cmd_environment.ps1').read_text()
        self.assertIn('if ($Value -cmatch ', script)
        self.assertIn('if ($argument -cnotmatch ', script)
        self.assertIn('call "{0}" {1} >nul', script)
        self.assertIn("@('/d', '/c', 'call', $batch)", script)
        self.assertIn('$start.UseShellExecute = $false', script)
        self.assertIn('$start.RedirectStandardOutput = $true', script)
        self.assertIn('$process.StandardOutput.ReadToEndAsync()', script)
        self.assertIn("[Environment]::SetEnvironmentVariable($Matches[1], $Matches[2], 'Process')", script)
        self.assertIn('} finally {', script)
        self.assertIn('Remove-Item -LiteralPath $batch -Force', script)
        self.assertNotIn('&& set', script)
        self.assertNotIn('set >', script)
        self.assertNotIn('$env:GITHUB_ENV', script)
        self.assertNotIn('Write-Output $lines', script)

    @unittest.skipUnless(os.name == 'nt', 'Actual CMD/PowerShell quoting is a native Windows check')
    def test_native_developer_shell_quotes_spaces_and_rejects_injection(self):
        with temporary_root(prefix='luma quoting test ') as work:
            batch = Path(work) / 'fake developer environment.cmd'
            batch.write_text('@echo off\nset LUMA_CT2_QUOTING_TEST=passed\nexit /b 0\n', encoding='ascii')
            result = subprocess.run(['pwsh', '-NoProfile', '-NonInteractive', '-File',
                            str(build.HERE / 'test_cmd_environment.ps1'), '-Helper',
                            str(build.HERE / 'cmd_environment.ps1'), '-DeveloperBatch', str(batch)],
                           check=False, capture_output=True, text=True, timeout=90)
            safe = [line for line in result.stderr.splitlines() if line.startswith('CT2_QUOTING_FAILURE ')]
            self.assertEqual(result.returncode, 0, '\n'.join(safe) or 'Native quoting regression failed without safe diagnostic')

    def test_direct_crt_signatures_and_manual_proof_concurrency_are_bounded(self):
        script = (build.HERE / 'verify_direct_crt_signatures.ps1').read_text()
        self.assertIn('Get-AuthenticodeSignature -LiteralPath $path', script)
        self.assertIn('[System.Management.Automation.SignatureStatus]::Valid', script)
        self.assertIn('[Security.Cryptography.SHA256]::HashData($signer.RawData)', script)
        self.assertIn('signer_certificate_der_sha256=$signerDerSha256', script)
        self.assertIn("'cc0ff0eb1dc3f5188ae6300faef32bf5beeba4bdd6e8e445a9184072096b713b'", script)
        self.assertIn('installer_executed=$false', script)
        self.assertNotIn('Start-Process', script)
        self.assertNotIn('Invoke-Expression', script)
        workflow = (build.HERE.parents[2] / '.github/workflows/asr-ct2-cpu.yml').read_text()
        self.assertIn("cancel-in-progress: ${{ github.event_name == 'pull_request' }}", workflow)
        self.assertIn("format('ct2-cpu-proof-{0}-{1}', inputs.source_sha || github.sha, github.run_id)", workflow)
        self.assertIn('windows-direct-crt-signatures:', workflow)
        self.assertIn('path: dist/direct-crt-reports/*.json', workflow)

    def test_workflow_defaults_to_metadata_and_guards_exact_candidates(self):
        workflow = (build.HERE.parents[2] / '.github/workflows/asr-ct2-cpu.yml').read_text()
        self.assertNotIn('contents: write', workflow)
        self.assertNotIn('packages: write', workflow)
        blocks = re.findall(r'(?m)^          path: (?:\|\n((?:            [^\n]+\n)+)|([^\n]+))', workflow)
        self.assertEqual(len(blocks), 4)
        paths = [line.strip() for block in blocks for text in block for line in text.splitlines() if line.strip()]
        self.assertEqual(set(paths), {'dist/ct2-cpu-reports/*.json', 'dist/ct2-cpu-reports/*.log',
                                     'dist/ct2-cpu-reports/*-CMakeCache.txt', 'dist/direct-crt-reports/*.json',
                                     'dist/ct2-cpu-candidate/ctranslate2-4.8.2-1lumacpu-cp312-cp312-win_amd64.whl',
                                     'dist/ct2-cpu-candidate/luma-ct2-cpu-4.8.2-1-sources.zip',
                                     'dist/ct2-cpu-candidate/luma-ct2-cpu-4.8.2-1-notices.zip',
                                     'dist/ct2-cpu-candidate/publication-proof.json',
                                     'dist/ct2-cpu-diagnostic/ctranslate2-4.8.2-1lumacpu-cp312-cp312-win_amd64.whl',
                                     'dist/ct2-cpu-diagnostic/luma-ct2-cpu-4.8.2-1-sources.zip',
                                     'dist/ct2-cpu-diagnostic/luma-ct2-cpu-4.8.2-1-notices.zip',
                                     'dist/ct2-cpu-diagnostic/diagnostic-manifest.json'})
        self.assertIn("if: success() && !cancelled() && env.CPU_CANDIDATE_EXPORT == 'true'", workflow)
        self.assertIn("needs.request.outputs.success_artifact == 'cpu-wheel-source-notices-proof-14-days'", workflow)
        self.assertIn("github.event_name == 'push'", workflow)
        self.assertIn("github.ref == 'refs/heads/feat/optional-asr-engines'", workflow)
        self.assertNotIn('publication:', workflow.split('permissions:', 1)[0])
        self.assertIn('name: ct2-cpu-candidate-${{ env.SOURCE_SHA }}-${{ github.run_id }}-${{ github.run_attempt }}', workflow)
        self.assertNotIn('candidate_artifact_id:', workflow.split('permissions:', 1)[0])
        self.assertIn('        id: candidate-upload', workflow)
        for name in ('ct2-cpu-proof', 'direct-crt-proof'):
            self.assertIn("name: " + name + "-${{ env.SOURCE_SHA }}-${{ github.run_id }}-${{ github.run_attempt }}", workflow)
        self.assertNotIn('inputs.publication', workflow)
        self.assertIn('needs: [source-tests, request, windows-direct-crt-signatures]', workflow)


if __name__ == '__main__':
    unittest.main()
