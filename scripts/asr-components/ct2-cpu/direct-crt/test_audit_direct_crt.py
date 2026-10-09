from __future__ import annotations
import hashlib
import importlib.util
import json
from pathlib import Path
import struct
import tempfile
import unittest
from unittest.mock import patch
import urllib.request

HERE = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location('audit_direct_crt', HERE / 'audit_direct_crt.py')
m = importlib.util.module_from_spec(spec)
spec.loader.exec_module(m)


def pin(data):
    return len(data), hashlib.sha256(data).hexdigest()


class AuditTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.root = Path(self.tmp.name)

    def tearDown(self):
        self.tmp.cleanup()

    def test_lock_agrees_with_helper(self):
        lock = json.loads((HERE / 'package.lock.json').read_text())
        self.assertEqual(lock['source_url'], m.URL)
        self.assertEqual((lock['installer']['bytes'], lock['installer']['sha256']), m.INSTALLER)
        for n, (offset, size, sha) in m.SLICES.items():
            self.assertEqual(lock['containers'][n], {'offset': offset, 'bytes': size, 'sha256': sha})
        self.assertEqual((lock['minimum_cab']['bytes'], lock['minimum_cab']['sha256']), m.MINIMUM)
        for n, (size, sha) in m.DLLS.items():
            self.assertEqual(lock['dlls'][n], {'member': n + '_amd64', 'bytes': size, 'sha256': sha})
        for n, (member, size, sha) in m.LICENSES.items():
            self.assertEqual({key: lock['notices'][n][key] for key in ['parent', 'member', 'bytes', 'sha256']}, {'parent': 'ux.cab', 'member': member, 'bytes': size, 'sha256': sha})
        self.assertFalse(lock['redistribution_permission_established'])
        self.assertIn('/3956ea1dc1086c62fa6d2d05484952caee010646/', lock['source_manifest'])
        self.assertEqual(lock['source_manifest_provenance']['blob_sha1'], '6cdab74037d685cf44e9f811043cb9fc3a44cb80')
        self.assertEqual(lock['source_manifest_provenance']['sha256'], 'de0d58bf1227acee24c6a920f31fc0f3f922d263725ccfec2bc5fe7553875f60')
        self.assertEqual(lock['notices']['license-en.rtf']['eula_id'], 'Cpp_2015-2022_ENU.1033')
        self.assertEqual(lock['notices']['license-zh-CN.rtf']['eula_id'], 'Cpp_2015-2022_CHS.2052')

    def test_identity_rejects_size(self):
        f = self.root / 'payload'; f.write_bytes(b'x')
        with self.assertRaisesRegex(ValueError, 'size'): m.identity(f, pin(b'xx'))

    def test_identity_rejects_hash(self):
        f = self.root / 'payload'; f.write_bytes(b'x')
        with self.assertRaisesRegex(ValueError, 'SHA'): m.identity(f, pin(b'y'))

    def test_identity_accepts_exact_bytes(self):
        f = self.root / 'payload'; f.write_bytes(b'x')
        self.assertEqual(m.identity(f, pin(b'x'))['sha256'], pin(b'x')[1])

    def test_full_hash_gate_precedes_parsing(self):
        f = self.root / 'bad.exe'; f.write_bytes(b'MZ')
        with self.assertRaisesRegex(ValueError, 'size'): m.split_containers(f, self.root)
        self.assertEqual(list(self.root.iterdir()), [f])

    def test_valid_slices_are_independently_hash_checked(self):
        data = bytearray(447600)
        data[:2] = b'MZ'; data[280:284] = b'PE\0\0'
        struct.pack_into('<II', data, 447488, 0x00f14300, 2)
        struct.pack_into('<IIII', data, 447528, 1, 2, 195988, 24939223)
        data[1000:1008] = b'MSCFtest'
        f = self.root / 'fixture'; f.write_bytes(data)
        with patch.object(m, 'INSTALLER', pin(data)), patch.object(m, 'SLICES', {'ux.cab': (1000, 8, pin(b'MSCFtest')[1])}):
            out = m.split_containers(f, self.root)
        self.assertEqual(out['ux.cab'].read_bytes(), b'MSCFtest')

    def test_bad_slice_rejected(self):
        data = bytearray(447600)
        data[:2] = b'MZ'; data[280:284] = b'PE\0\0'
        struct.pack_into('<II', data, 447488, 0x00f14300, 2)
        struct.pack_into('<IIII', data, 447528, 1, 2, 195988, 24939223)
        data[1000:1008] = b'MSCFtest'
        f = self.root / 'fixture'; f.write_bytes(data)
        with patch.object(m, 'INSTALLER', pin(data)), patch.object(m, 'SLICES', {'ux.cab': (1000, 8, '0' * 64)}):
            with self.assertRaisesRegex(ValueError, 'CAB hash'): m.split_containers(f, self.root)
        self.assertFalse((self.root / 'ux.cab').exists())

    def test_redirect_rejects_other_origins_and_credentials(self):
        req = urllib.request.Request(m.URL)
        for url in ['http://download.visualstudio.microsoft.com/a', 'https://evil.invalid/a', 'https://user@download.visualstudio.microsoft.com/a', 'https://download.visualstudio.microsoft.com:8443/a']:
            with self.subTest(url=url), self.assertRaisesRegex(ValueError, 'origin'):
                m.MicrosoftOnlyRedirect().redirect_request(req, None, 302, '', {}, url)

    def test_redirect_allows_exact_microsoft_https_host(self):
        req = urllib.request.Request(m.URL)
        url = 'https://download.visualstudio.microsoft.com/test'
        out = m.MicrosoftOnlyRedirect().redirect_request(req, None, 302, '', {}, url)
        self.assertEqual(out.full_url, url)

    def test_member_allowlist(self):
        with self.assertRaisesRegex(ValueError, 'Unaudited'):
            m.extract_member(self.root / 'expand.exe', self.root / 'cab', '../evil', self.root / 'out', pin(b'x'))

    def test_existing_destination_rejected(self):
        dest = self.root / 'out'; dest.mkdir()
        with self.assertRaisesRegex(ValueError, 'already exists'):
            m.extract_member(self.root / 'expand.exe', self.root / 'cab', 'a12', dest, pin(b'x'))

    def test_expander_failure_rejected(self):
        with patch.object(m.subprocess, 'run') as run:
            run.return_value.returncode = 1
            with self.assertRaisesRegex(ValueError, 'failed'):
                m.extract_member(self.root / 'expand.exe', self.root / 'cab', 'a12', self.root / 'out', pin(b'x'))

    def test_expander_unexpected_output_rejected(self):
        dest = self.root / 'out'
        def fake(*args, **kwargs):
            (dest / 'a12').write_bytes(b'x'); (dest / 'unexpected').write_bytes(b'x')
            return m.subprocess.CompletedProcess([], 0)
        with patch.object(m.subprocess, 'run', side_effect=fake):
            with self.assertRaisesRegex(ValueError, 'Unexpected extracted'):
                m.extract_member(self.root / 'expand.exe', self.root / 'cab', 'a12', dest, pin(b'x'))

    def test_expander_exact_output_and_argument_list(self):
        dest = self.root / 'space path'
        def fake(args, **kwargs):
            self.assertEqual(args, [str(self.root / 'expand.exe'), str(self.root / 'source.cab'), '-F:a12', str(dest)])
            self.assertFalse(kwargs['shell']); self.assertEqual(kwargs['timeout'], 120)
            (dest / 'a12').write_bytes(b'x')
            return m.subprocess.CompletedProcess(args, 0)
        with patch.object(m.subprocess, 'run', side_effect=fake):
            self.assertEqual(m.extract_member(self.root / 'expand.exe', self.root / 'source.cab', 'a12', dest, pin(b'x')).read_bytes(), b'x')


if __name__ == '__main__':
    unittest.main()
