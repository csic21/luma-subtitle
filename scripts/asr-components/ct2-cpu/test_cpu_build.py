import base64
import copy
import csv
import hashlib
import io
import json
import os
import subprocess
from pathlib import Path
import struct
import tarfile
import tempfile
import unittest
import zipfile

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
    def read(self, data):
        with tempfile.TemporaryDirectory() as work:
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
            with tempfile.TemporaryDirectory() as work:
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
        with tempfile.TemporaryDirectory() as work:
            root = Path(work); archive = root / 'source.tgz'
            with tarfile.open(archive, 'w:gz') as tar:
                member = tarfile.TarInfo('source/file'); member.size = 2; tar.addfile(member, io.BytesIO(b'ok'))
            build.extract_source(archive, root / 'out')
            self.assertEqual((root / 'out/file').read_bytes(), b'ok')

    def test_canonical_wheel_is_repeatable_and_preserves_payload(self):
        with tempfile.TemporaryDirectory() as work:
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
        with tempfile.TemporaryDirectory(prefix='luma quoting test ') as work:
            batch = Path(work) / 'fake developer environment.cmd'
            batch.write_text('@echo off\nset LUMA_CT2_QUOTING_TEST=passed\nexit /b 0\n', encoding='ascii')
            result = subprocess.run(['pwsh', '-NoProfile', '-NonInteractive', '-File',
                            str(build.HERE / 'test_cmd_environment.ps1'), '-Helper',
                            str(build.HERE / 'cmd_environment.ps1'), '-DeveloperBatch', str(batch)],
                           check=False, capture_output=True, text=True, timeout=90)
            safe = [line for line in result.stderr.splitlines() if line.startswith('CT2_QUOTING_FAILURE ')]
            self.assertEqual(result.returncode, 0, '\n'.join(safe) or 'Native quoting regression failed without safe diagnostic')

    def test_workflow_publishes_no_binaries(self):
        workflow = (build.HERE.parents[2] / '.github/workflows/asr-ct2-cpu.yml').read_text()
        self.assertNotIn('contents: write', workflow)
        self.assertNotIn('packages: write', workflow)
        paths = workflow.split('          path: |')[1]
        self.assertNotIn('.whl', paths)
        self.assertNotIn('.dll', paths)
        self.assertNotIn('**', paths)


if __name__ == '__main__':
    unittest.main()
