"""Fail-closed host-security classification; no Defender/security settings change."""
import copy
import ctypes
import io
import json
import os
from pathlib import Path, PureWindowsPath
import sys
import unittest
from contextlib import redirect_stderr
from types import SimpleNamespace
from unittest.mock import patch

import self_test as subject
from fixture_paths import temporary_root


PROGRAM_DATA = r'C:\ProgramData'
DEFENDER = PROGRAM_DATA + r'\Microsoft\Windows Defender\Platform\4.18.26080.4-0\MpOav.dll'


def valid_identity():
    # Synthetic metadata, deliberately no assumed current certificate/hash/CN.
    return {'authenticode_status': 0, 'signer_error': 0,
            'microsoft_root_policy_checked': True, 'microsoft_root_policy_error': 0,
            'microsoft_root_policy_flags': 0x40000,
            'certificate_chain': [{'organization': 'Microsoft Corporation', 'common_name': 'synthetic fixture'}],
            'version_identity': [{'OriginalFilename': 'MpOav.dll', 'CompanyName': 'Microsoft Corporation',
                                  'ProductName': 'Synthetic localized name'}]}


class DefenderIdentityTests(unittest.TestCase):
    def test_exact_platform_shape_accepts_case_changes_only(self):
        expected = PureWindowsPath(DEFENDER)
        self.assertEqual(subject.defender_platform_path(DEFENDER, PROGRAM_DATA), expected)
        self.assertEqual(subject.defender_platform_path(DEFENDER.lower(), PROGRAM_DATA), expected)
        self.assertEqual(subject.defender_platform_path(DEFENDER.replace('4.18.26080.4-0', '4.18.26090.1-1'), PROGRAM_DATA).name, 'MpOav.dll')

    def test_path_name_escapes_device_paths_and_extra_levels_rejected(self):
        bad = [DEFENDER.replace('MpOav.dll', name) for name in
               ('other.dll', 'msvcp140.dll', 'libiomp5md.dll', 'MpOav.dll.exe', 'MpOav.dll:payload')]
        bad += [DEFENDER.replace('ProgramData', 'ProgramData-other'),
                DEFENDER.replace('Windows Defender', 'Other Antivirus'),
                DEFENDER.replace('4.18.26080.4-0', 'current'),
                DEFENDER.replace('4.18.26080.4-0', '4.18.26080.4-0\\x64'),
                DEFENDER.replace('4.18.26080.4-0', '4.18.26080.4-0\\..\\4.18.26080.4-0'),
                DEFENDER.replace('4.18.26080.4-0', '4.18.26080.4-0.'),
                DEFENDER.replace('4.18.26080.4-0', '4.18.26080.4-0 '),
                DEFENDER.replace('\\MpOav', '\\ .\\MpOav'),
                DEFENDER.replace('\\MpOav', '\\.\\MpOav'),
                DEFENDER.replace('\\MpOav', '\\\\MpOav'),
                DEFENDER.replace('\\', '/'), '\\\\?\\' + DEFENDER,
                '\\\\server\\share\\MpOav.dll', DEFENDER[2:], DEFENDER + '\x00']
        for path in bad:
            with self.subTest(path=path), self.assertRaises(ValueError):
                subject.defender_platform_path(path, PROGRAM_DATA)

    def test_registry_value_never_expands_or_accepts_commands(self):
        for kind in (1, 2):
            self.assertEqual(subject.defender_registry_value(DEFENDER, kind), DEFENDER)
            self.assertEqual(subject.defender_registry_value('"' + DEFENDER + '"', kind), DEFENDER)
        for value, kind in [(DEFENDER, 4), (1, 1), ('%ProgramData%\\MpOav.dll', 2),
                            ('"' + DEFENDER, 1), ('"' + DEFENDER + '" /run', 1),
                            (DEFENDER + '\x00extra', 1), ('x' * 32768, 1)]:
            with self.subTest(value=value, kind=kind), self.assertRaises(ValueError):
                subject.defender_registry_value(value, kind)

    def test_registry_uses_exact_native_machine_keys(self):
        calls = []
        class Key:
            def __enter__(self): return self
            def __exit__(self, *args): pass
        def open_key(root, path, reserved, flags):
            calls.append((root, path, reserved, flags)); return Key()
        registry = SimpleNamespace(KEY_READ=0x20019, KEY_WOW64_64KEY=0x100,
            HKEY_LOCAL_MACHINE='HKLM', OpenKey=open_key, QueryValueEx=lambda key, value: ('"' + DEFENDER + '"', 1))
        with patch.dict(sys.modules, winreg=registry), redirect_stderr(io.StringIO()) as err:
            self.assertEqual(subject.defender_registered_path(), DEFENDER)
        diagnostic = json.loads(err.getvalue().split('=', 1)[1])
        self.assertEqual(diagnostic['raw_value'], '"' + DEFENDER + '"')
        self.assertEqual(diagnostic['registry_value_type'], 1)
        self.assertEqual(diagnostic['registry_view'], 'native-64')
        self.assertEqual(calls, [('HKLM', 'SOFTWARE\\Microsoft\\AMSI\\Providers\\' + subject.DEFENDER_AMSI_CLSID, 0, 0x20119),
            ('HKLM', 'SOFTWARE\\Classes\\CLSID\\' + subject.DEFENDER_AMSI_CLSID + '\\InprocServer32', 0, 0x20119)])

    def test_raw_registry_diagnostic_precedes_fail_closed_parsing(self):
        class Key:
            def __enter__(self): return self
            def __exit__(self, *args): pass
        value = '%ProgramData%\\Microsoft\\Windows Defender\\Platform\\4.18.26080.4-0\\MpOav.dll'
        registry = SimpleNamespace(KEY_READ=0x20019, KEY_WOW64_64KEY=0x100,
            HKEY_LOCAL_MACHINE='HKLM', OpenKey=lambda *args: Key(), QueryValueEx=lambda *args: (value, 2))
        with patch.dict(sys.modules, winreg=registry), redirect_stderr(io.StringIO()) as err, \
             patch.dict(os.environ, ProgramData=r'C:\attacker-controlled'):
            with self.assertRaisesRegex(ValueError, 'unexpanded reference'):
                subject.defender_registered_path()
        diagnostic = json.loads(err.getvalue().split('=', 1)[1])
        self.assertEqual(diagnostic['raw_value'], value)
        self.assertEqual(diagnostic['registry_value_type'], 2)
        self.assertFalse(diagnostic['raw_value_truncated'])
        self.assertNotIn('attacker-controlled', err.getvalue())
        for value, message in [(DEFENDER + '\x00suffix', 'NUL'), ('"' + DEFENDER, 'quotes')]:
            with self.assertRaisesRegex(ValueError, message): subject.defender_registry_value(value, 1)

    def test_registry_diagnostic_is_bounded_and_escapes_control_characters(self):
        value = '\x00' + '"%' + '\u6d4b' * 10000
        result = subject.defender_registry_diagnostic(value, 2)
        self.assertEqual(len(result['raw_value']), 512)
        self.assertEqual(result['raw_value_characters'], len(value))
        self.assertTrue(result['raw_value_truncated'])
        self.assertTrue(result['contains_nul']); self.assertTrue(result['contains_percent'])
        encoded = json.dumps(result, ensure_ascii=True)
        self.assertNotIn('\x00', encoded)
        self.assertLess(len(encoded), 4096)
        self.assertNotIn('raw_value', subject.defender_registry_diagnostic(b'unneeded-binary-data', 3))

    def test_every_reparse_ancestor_and_file_is_rejected(self):
        class Node:
            def __init__(self, attributes=0, parents=()): self.attributes=attributes; self.parents=parents
            def lstat(self): return SimpleNamespace(st_file_attributes=self.attributes)
            def is_file(self): return True
        subject.windows_no_reparse(Node(parents=(Node(), Node())))
        for path in (Node(0x400), Node(parents=(Node(0x400),)), Node(parents=(Node(), Node(0x400)))):
            with self.assertRaisesRegex(ValueError, 'reparse'):
                subject.windows_no_reparse(path)

    def test_authenticode_microsoft_root_and_publisher_all_required(self):
        subject.require_defender_identity(valid_identity())
        for field, values in {'authenticode_status': [1, 0x800B0100, 0x800B010C, None],
                              'signer_error': [1, None],
                              'microsoft_root_policy_checked': [False, 1, None],
                              'microsoft_root_policy_error': [0x800B0109, None],
                              'microsoft_root_policy_flags': [0, 0x10000, 0x20000, 0x50000]}.items():
            for value in values:
                evidence = valid_identity(); evidence[field] = value
                with self.subTest(field=field, value=value), self.assertRaises(ValueError):
                    subject.require_defender_identity(evidence)
        for publisher in ('Microsoft Corporation Evil', 'Microsoft', '', 'Example Corporation'):
            evidence = valid_identity(); evidence['certificate_chain'][0]['organization'] = publisher
            with self.assertRaisesRegex(ValueError, 'publisher'):
                subject.require_defender_identity(evidence)
        for field in ('certificate_chain', 'version_identity'):
            evidence = valid_identity(); evidence[field] = []
            with self.assertRaises(ValueError): subject.require_defender_identity(evidence)

    def test_renamed_signed_module_and_conflicting_resources_rejected(self):
        for key, value in [('OriginalFilename', 'msvcp140.dll'), ('OriginalFilename', 'other.dll'),
                           ('OriginalFilename', 'MpOav.dll '), ('CompanyName', 'Other Corporation')]:
            evidence = valid_identity(); evidence['version_identity'][0][key] = value
            with self.subTest(key=key, value=value), self.assertRaisesRegex(ValueError, 'resource identity'):
                subject.require_defender_identity(evidence)
            evidence['version_identity'].insert(0, copy.deepcopy(valid_identity()['version_identity'][0]))
            with self.assertRaisesRegex(ValueError, 'resource identity'): subject.require_defender_identity(evidence)

    def test_private_crt_openmp_never_reaches_host_security_exemption(self):
        with temporary_root() as tmp:
            private = tmp / 'private'; private.mkdir()
            outside = tmp / 'system'; outside.mkdir()
            with patch.object(subject, 'verified_defender_module') as verifier:
                for name in ('MSVCP140.dll', 'VCRUNTIME140_1.dll', 'concrt140.dll', 'vcomp140.dll', 'libiomp5md.dll', 'libomp.dll'):
                    path = outside / name; path.touch()
                    with self.subTest(name=name), self.assertRaisesRegex(AssertionError, 'CRT/OpenMP'):
                        subject.check_native_library(path, private, (str(outside).lower(),), windows=True)
                    local = private / name; local.touch()
                    self.assertIsNone(subject.check_native_library(local, private, (), windows=True))
                verifier.assert_not_called()

    def test_only_exact_external_mpoav_is_submitted_for_verification(self):
        with temporary_root() as tmp:
            private = tmp / 'private'; private.mkdir()
            expected = {'verified': True}
            with patch.object(subject, 'verified_defender_module', return_value=expected) as verifier:
                path = tmp / 'MpOav.dll'; path.touch()
                self.assertIs(subject.check_native_library(path, private, (), windows=True), expected)
                verifier.assert_called_once_with(path)
                for name in ('other.dll', 'MpOav.dll.exe', 'Defender.dll'):
                    path = tmp / name; path.touch()
                    with self.assertRaisesRegex(AssertionError, 'non-system'):
                        subject.check_native_library(path, private, (), windows=True)
                self.assertEqual(verifier.call_count, 1)
            with patch.object(subject, 'verified_defender_module', side_effect=AssertionError('untrusted')):
                with self.assertRaisesRegex(AssertionError, 'untrusted'):
                    subject.check_native_library(tmp / 'MpOav.dll', private, (), windows=True)

    def test_canonical_escape_is_rejected_before_trust(self):
        candidate = PureWindowsPath(DEFENDER)
        fake_path = SimpleNamespace(resolve=lambda strict: PureWindowsPath(r'C:\outside\MpOav.dll'))
        with patch.object(subject, 'windows_program_data', return_value=PROGRAM_DATA), \
             patch.object(subject, 'defender_registered_path', return_value=DEFENDER), \
             patch.object(subject, 'windows_no_reparse'), patch.object(subject, 'Path', return_value=fake_path), \
             patch.object(subject, 'windows_authenticode') as trust, redirect_stderr(io.StringIO()) as err:
            with self.assertRaisesRegex(AssertionError, 'canonical path changed'):
                subject.verified_defender_module(candidate)
            trust.assert_not_called()
            self.assertIn('"verified": false', err.getvalue())

    def test_registration_mismatch_is_rejected_before_file_or_trust(self):
        with patch.object(subject, 'windows_program_data', return_value=PROGRAM_DATA), \
             patch.object(subject, 'defender_registered_path', return_value=DEFENDER.replace('26080', '26090')), \
             patch.object(subject, 'windows_locked_file') as open_file, redirect_stderr(io.StringIO()):
            with self.assertRaisesRegex(AssertionError, 'registration does not match'):
                subject.verified_defender_module(DEFENDER)
            open_file.assert_not_called()

    def test_trust_loaded_libraries_are_reaudited_and_evidence_not_duplicated(self):
        with temporary_root() as tmp:
            private = tmp / 'private'; private.mkdir()
            provider = tmp / 'MpOav.dll'; provider.touch()
            native = private / 'native.dll'; native.touch()
            extra = private / 'trust-helper.dll'; extra.touch()
            snapshots = iter([[native, provider], [native, provider, extra], [native, provider, extra]])
            report = []; identity = {'verified': True}
            with patch.object(subject, 'verified_defender_module', return_value=identity) as verifier:
                self.assertEqual(subject.audit_windows_module_snapshots(lambda: next(snapshots), private, (), report), 3)
                verifier.assert_called_once_with(provider)
            self.assertEqual(report, [identity])
            unknown = tmp / 'unknown-injection.dll'; unknown.touch()
            snapshots = iter([[native], [native, unknown]])
            with self.assertRaisesRegex(AssertionError, 'non-system'):
                subject.audit_windows_module_snapshots(lambda: next(snapshots), private, (), [])

    def test_nonstabilizing_or_oversized_enumeration_fails_closed(self):
        with temporary_root() as tmp:
            paths = [tmp / name for name in ('one.dll', 'two.dll', 'three.dll')]
            for path in paths: path.touch()
            snapshots = iter([paths[:1], paths[:2], paths[:3]])
            with self.assertRaisesRegex(AssertionError, 'stabilize'):
                subject.audit_windows_module_snapshots(lambda: next(snapshots), tmp, (), [])
            for paths in ([], [tmp / 'one.dll'] * 4097):
                with self.assertRaisesRegex(AssertionError, 'bounds'):
                    subject.audit_windows_module_snapshots(lambda: paths, tmp, (), [])

    def test_native_enumeration_never_silently_omits_unreadable_modules(self):
        from ctypes import wintypes
        class Function:
            def __init__(self, call): self.call = call
            def __call__(self, *args): return self.call(*args)
        kernel = SimpleNamespace(GetCurrentProcess=Function(lambda: 123))
        def enumerate_one(process, handles, size, needed):
            needed._obj.value = ctypes.sizeof(wintypes.HMODULE); handles[0] = 123
            return True
        for path_length in (0, 32768):
            psapi = SimpleNamespace(EnumProcessModules=Function(enumerate_one),
                GetModuleFileNameExW=Function(lambda *args: path_length))
            with patch.object(ctypes, 'WinDLL', create=True, side_effect=lambda name, **kwargs: psapi if name == 'psapi' else kernel):
                with self.assertRaisesRegex(AssertionError, 'failed or was truncated'):
                    subject.windows_module_paths()
        def overflow(process, handles, size, needed):
            needed._obj.value = size + ctypes.sizeof(wintypes.HMODULE); return True
        psapi.EnumProcessModules = Function(overflow)
        with patch.object(ctypes, 'WinDLL', create=True, side_effect=lambda name, **kwargs: psapi if name == 'psapi' else kernel):
            with self.assertRaisesRegex(AssertionError, 'enumeration overflow'):
                subject.windows_module_paths()

    def test_trust_api_rejects_nonzero_and_closes_state_without_policy_fallback(self):
        class Function:
            def __init__(self, call): self.call = call
            def __call__(self, *args): return self.call(*args)
        encoded = ctypes.create_string_buffer(b'synthetic-certificate')
        certificate = subject.CertificateContext(1, ctypes.addressof(encoded), len(encoded.raw), None, None)
        prefix = subject.ProviderCertificatePrefix(ctypes.sizeof(subject.ProviderCertificatePrefix), ctypes.addressof(certificate))
        signer = subject.ProviderSigner(); signer.cbStruct = ctypes.sizeof(signer)
        signer.csCertChain = 1; signer.pChainContext = 123
        for trust_status, root_checked, root_error in ((0, True, 0), (1, True, 0), (0x800B010C, True, 0),
                                                       (0, False, 0), (0, True, 0x800B0109)):
            calls = []; policy_calls = []
            def verify(window, action, data):
                data = data._obj
                calls.append((data.dwStateAction, data.dwUIChoice, data.fdwRevocationChecks,
                              data.dwProvFlags, data.pFile.contents.hFile))
                data.hWVTStateData = 123
                return trust_status
            def attribute(certificate, kind, flags, oid, buffer, count):
                self.assertEqual((kind, flags), (3, 0))
                buffer.value = 'Microsoft Corporation' if oid == b'2.5.4.10' else 'Synthetic fixture signer'
                return len(buffer.value) + 1
            def policy(kind, chain, parameters, status):
                policy_calls.append((kind, parameters._obj.dwFlags))
                status._obj.dwError = root_error
                return root_checked
            trust = SimpleNamespace(WinVerifyTrust=Function(verify),
                WTHelperProvDataFromStateData=Function(lambda state: 123),
                WTHelperGetProvSignerFromChain=Function(lambda *args: ctypes.pointer(signer)),
                WTHelperGetProvCertFromChain=Function(lambda *args: ctypes.pointer(prefix)))
            crypt = SimpleNamespace(CertGetNameStringW=Function(attribute), CertVerifyCertificateChainPolicy=Function(policy))
            evidence = valid_identity()
            with patch.object(ctypes, 'WinDLL', create=True, side_effect=lambda name, **kwargs: trust if name == 'wintrust' else crypt):
                subject.windows_authenticode(DEFENDER, 777, evidence)
            self.assertEqual(calls, [(1, 2, 1, 0x2080, 777), (2, 2, 1, 0x2080, 777)])
            self.assertEqual(policy_calls, [(7, 0x40000)])
            self.assertEqual(evidence['authenticode_status'], trust_status)
            self.assertEqual(len(evidence['certificate_chain'][0]['der_sha256']), 64)
            if trust_status == 0 and root_checked and root_error == 0:
                subject.require_defender_identity(evidence)
            else:
                with self.assertRaises(ValueError): subject.require_defender_identity(evidence)
        trust.WTHelperGetProvSignerFromChain = Function(lambda *args: None)
        calls.clear()
        with patch.object(ctypes, 'WinDLL', create=True, side_effect=lambda name, **kwargs: trust if name == 'wintrust' else crypt):
            with self.assertRaisesRegex(ValueError, 'no usable provider signer'):
                subject.windows_authenticode(DEFENDER, 777, {})
        self.assertEqual([call[0] for call in calls], [1, 2])

    def test_opened_file_canonical_escape_is_rejected_before_trust(self):
        candidate = PureWindowsPath(DEFENDER)
        fake_path = SimpleNamespace(resolve=lambda strict: candidate)
        with patch.object(subject, 'windows_program_data', return_value=PROGRAM_DATA), \
             patch.object(subject, 'defender_registered_path', return_value=DEFENDER), \
             patch.object(subject, 'windows_no_reparse'), patch.object(subject, 'Path', return_value=fake_path), \
             patch.object(subject, 'windows_locked_file') as opened, \
             patch.object(subject, 'windows_authenticode') as trust, redirect_stderr(io.StringIO()):
            opened.return_value.__enter__.return_value = (None, 777, r'C:\outside\MpOav.dll')
            with self.assertRaisesRegex(AssertionError, 'outside the known platform'):
                subject.verified_defender_module(candidate)
            trust.assert_not_called()
            opened.return_value.__exit__.assert_called_once()

    @unittest.skipUnless(ctypes.sizeof(ctypes.c_void_p) == 8, 'x64 ABI assertions')
    def test_windows_x64_abi_layout_matches_sdk(self):
        self.assertEqual(ctypes.sizeof(subject.WindowsGuid), 16)
        self.assertEqual(ctypes.sizeof(subject.WinTrustFileInfo), 32)
        self.assertEqual(ctypes.sizeof(subject.WinTrustData), 88)
        self.assertEqual(subject.WinTrustData.pFile.offset, 40)
        self.assertEqual(subject.WinTrustData.hWVTStateData.offset, 56)
        self.assertEqual(ctypes.sizeof(subject.ProviderSigner), 64)
        self.assertEqual(subject.ProviderSigner.pChainContext.offset, 56)
        self.assertEqual(ctypes.sizeof(subject.ProviderCertificatePrefix), 16)
        self.assertEqual(ctypes.sizeof(subject.CertificateContext), 40)
        self.assertEqual(ctypes.sizeof(subject.ChainPolicyParameters), 16)
        self.assertEqual(ctypes.sizeof(subject.ChainPolicyStatus), 24)


@unittest.skipUnless(sys.platform == 'win32', 'native Windows Defender metadata proof')
class NativeDefenderIdentityTests(unittest.TestCase):
    def test_registered_module_identity_in_bounded_owned_child(self):
        # The child only reads the already installed provider's metadata. Never
        # loads/executes that DLL, changes registry/security, or stops Defender.
        from windows_crt_proof import helper_process
        with temporary_root() as tmp:
            source = str(Path(subject.__file__).resolve())
            command = [sys.executable, '-I', '-B', '-u', '-c',
                       "import json,runpy,sys; m=runpy.run_path(sys.argv[1]); "
                       "p=m['defender_registered_path'](); "
                       "r=m['verified_defender_module'](p); print(json.dumps(r,sort_keys=True))", source]
            code, out, err = helper_process(command, dict(os.environ), tmp)
            self.assertEqual(code, 0, (out + err)[-12000:])
            result = json.loads(out)
            self.assertIs(result['verified'], True)
            self.assertIs(result['inference_tested'], False)
            print('NATIVE_DEFENDER_IDENTITY=' + json.dumps(result, sort_keys=True, ensure_ascii=True), flush=True)


if __name__ == '__main__': unittest.main()
