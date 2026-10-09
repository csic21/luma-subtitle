"""CI/private-assembly runtime/API self-test. Run with this pack's Python -I -B -u.

This deliberately does not download/load model weights or claim inference proof.
"""
from __future__ import annotations
import ctypes
from contextlib import contextmanager
import base64
import hashlib
import importlib
import importlib.metadata
import inspect
import json
import os
from pathlib import Path, PureWindowsPath
import platform
import re
import site
import sys
import uuid

# LUMA_NAGISA_COMPAT_SOURCE

ROOT = Path(__file__).resolve().parent


def platform_description():
    # platform.platform() may lazily query processor information via subprocess.
    # These fields use OS/runtime metadata without weakening the offline guard.
    return f'{sys.platform} {platform.release()}'


def optional_vc_runtime(filename):
    name = filename.lower()
    # msvcp_win.dll is an OS component on supported Windows 10, unlike the
    # numbered Visual C++ redistributables. Never equate the whole prefix with
    # optional app-local CRT libraries.
    return re.fullmatch(r'(?:msvcp|vcruntime|concrt|vcomp|vccorlib)[0-9][a-z0-9_]*\.dll', name) is not None


def private_openmp_runtime(filename):
    return re.fullmatch(r'(?:libiomp|libomp)[a-z0-9_.-]*\.dll', filename.lower()) is not None


# This is one host-security classification, not a signed-DLL or ProgramData
# allowlist. Microsoft documents this exact AMSI module/registration pattern:
# https://learn.microsoft.com/en-us/exchange/antispam-and-antimalware/amsi-integration-with-exchange
DEFENDER_AMSI_CLSID = '{2781761E-28E0-4109-99FE-B9D127C57AFE}'


class WindowsGuid(ctypes.Structure):
    _fields_ = [('data1', ctypes.c_uint32), ('data2', ctypes.c_uint16),
                ('data3', ctypes.c_uint16), ('data4', ctypes.c_ubyte * 8)]


def windows_guid(value):
    return WindowsGuid.from_buffer_copy(uuid.UUID(value).bytes_le)


def defender_platform_path(value, program_data):
    """Strict lexical check, before touching the candidate or invoking trust."""
    value = str(value)
    path = PureWindowsPath(value)
    base = PureWindowsPath(program_data) / 'Microsoft' / 'Windows Defender' / 'Platform'
    if (not re.match(r'^[A-Za-z]:\\', value) or '/' in value or '\x00' in value
            or any(part in {'', '.', '..'} or part.endswith((' ', '.')) or ':' in part
                   for part in value[3:].split('\\'))):
        raise ValueError('Defender candidate is not a canonical local DOS path')
    try:
        relative = path.relative_to(base)
    except ValueError as exc:
        raise ValueError('Defender candidate is outside the known platform directory') from exc
    if (len(relative.parts) != 2 or relative.name.lower() != 'mpoav.dll'
            or not re.fullmatch(r'4\.18\.[0-9]{1,6}\.[0-9]{1,6}-[0-9]{1,3}', relative.parts[0])):
        raise ValueError('Defender candidate has an unrecognized module/version path')
    return path


def defender_registry_value(value, value_type):
    # Real InprocServer32 values may include one pair of quotes. Never expand
    # environment variables, accept arguments or consult HKCU's merged HKCR.
    if value_type not in (1, 2) or not isinstance(value, str) or len(value) > 32767:
        raise ValueError('Defender registration is not a bounded registry string')
    if value.startswith('"') and value.endswith('"'):
        value = value[1:-1]
    if '\x00' in value:
        raise ValueError('Defender registration contains a NUL character')
    if '%' in value:
        raise ValueError('Defender registration contains an unexpanded reference')
    if '"' in value:
        raise ValueError('Defender registration contains unmatched/interior quotes')
    return value


def defender_registry_diagnostic(value, value_type):
    # Only this fixed public OS-module registration is inspected. JSON escapes
    # control characters; never expand tokens or include process environment.
    result = {'kind': 'windows-defender-amsi-registration', 'registry_hive': 'HKLM',
              'registry_view': 'native-64', 'provider_clsid': DEFENDER_AMSI_CLSID,
              'key': 'SOFTWARE\\Classes\\CLSID\\' + DEFENDER_AMSI_CLSID + '\\InprocServer32',
              'value_name': '(Default)', 'registry_value_type': value_type,
              'value_is_string': isinstance(value, str)}
    if isinstance(value, str):
        result.update(raw_value=value[:512], raw_value_characters=len(value),
                      raw_value_truncated=len(value) > 512,
                      contains_percent='%' in value, contains_nul='\x00' in value,
                      quote_count=value.count('"'))
    return result


def defender_registered_path():
    import winreg
    access = winreg.KEY_READ | winreg.KEY_WOW64_64KEY
    with winreg.OpenKey(winreg.HKEY_LOCAL_MACHINE,
                        'SOFTWARE\\Microsoft\\AMSI\\Providers\\' + DEFENDER_AMSI_CLSID, 0, access):
        pass
    with winreg.OpenKey(winreg.HKEY_LOCAL_MACHINE,
                        'SOFTWARE\\Classes\\CLSID\\' + DEFENDER_AMSI_CLSID + '\\InprocServer32', 0, access) as key:
        value, value_type = winreg.QueryValueEx(key, None)
    # Emit before validation: the native metadata test calls this function
    # before entering verified_defender_module's broader diagnostic scope.
    print('HOST_SECURITY_REGISTRATION_DIAGNOSTIC=' + json.dumps(
          defender_registry_diagnostic(value, value_type), sort_keys=True, ensure_ascii=True),
          file=sys.stderr, flush=True)
    return defender_registry_value(value, value_type)


def windows_program_data():
    # Ask the OS for the default known-folder location, never inherited
    # ProgramData/ALLUSERSPROFILE or a user-controlled folder redirection.
    shell = ctypes.WinDLL('shell32', winmode=0x00000800)
    ole = ctypes.WinDLL('ole32', winmode=0x00000800)
    shell.SHGetKnownFolderPath.argtypes = [ctypes.POINTER(WindowsGuid), ctypes.c_uint32,
                                         ctypes.c_void_p, ctypes.POINTER(ctypes.c_void_p)]
    shell.SHGetKnownFolderPath.restype = ctypes.c_int32
    ole.CoTaskMemFree.argtypes = [ctypes.c_void_p]; ole.CoTaskMemFree.restype = None
    folder = windows_guid('62ab5d82-fdc1-4dc3-a9dd-070d1d495d97')
    result = ctypes.c_void_p()
    try:
        status = shell.SHGetKnownFolderPath(ctypes.byref(folder), 0x400, None, ctypes.byref(result))
        if status != 0 or not result.value:
            raise ValueError(f'ProgramData known-folder query failed: 0x{status & 0xffffffff:08x}')
        return ctypes.wstring_at(result.value)
    finally:
        if result.value: ole.CoTaskMemFree(result)


def windows_no_reparse(path):
    for part in (path, *path.parents):
        if part.lstat().st_file_attributes & 0x400:  # FILE_ATTRIBUTE_REPARSE_POINT
            raise ValueError('Defender candidate traverses a reparse point')
    if not path.is_file():
        raise ValueError('Defender candidate is not a regular file')


@contextmanager
def windows_locked_file(path):
    """Hold the existing file against write/delete for metadata and trust checks."""
    import msvcrt
    kernel = ctypes.WinDLL('kernel32', use_last_error=True, winmode=0x00000800)
    kernel.CreateFileW.argtypes = [ctypes.c_wchar_p, ctypes.c_uint32, ctypes.c_uint32,
                                  ctypes.c_void_p, ctypes.c_uint32, ctypes.c_uint32, ctypes.c_void_p]
    kernel.CreateFileW.restype = ctypes.c_void_p
    kernel.CloseHandle.argtypes = [ctypes.c_void_p]; kernel.CloseHandle.restype = ctypes.c_int32
    kernel.GetFinalPathNameByHandleW.argtypes = [ctypes.c_void_p, ctypes.c_wchar_p, ctypes.c_uint32, ctypes.c_uint32]
    kernel.GetFinalPathNameByHandleW.restype = ctypes.c_uint32
    handle = kernel.CreateFileW(str(path), 0x80000000, 1, None, 3, 0x00200000, None)
    if handle == ctypes.c_void_p(-1).value:
        raise ctypes.WinError(ctypes.get_last_error())
    try:
        buffer = ctypes.create_unicode_buffer(32768)
        count = kernel.GetFinalPathNameByHandleW(handle, buffer, len(buffer), 0)
        if not count or count >= len(buffer):
            raise ValueError('Defender file handle has no bounded canonical path')
        canonical = buffer.value
        if canonical.startswith('\\\\?\\'):
            canonical = canonical[4:]
        # The stream takes handle ownership only after open_osfhandle succeeds.
        fd = msvcrt.open_osfhandle(handle, os.O_RDONLY | os.O_BINARY)
        handle = None
        with os.fdopen(fd, 'rb') as stream:
            yield stream, msvcrt.get_osfhandle(stream.fileno()), canonical
    finally:
        if handle is not None: kernel.CloseHandle(handle)


# Fixed-width Windows SDK ABI, including native pointer alignment. These live
# here because the proof script is copied into an isolated private interpreter.
# Official declarations: microsoft/win32metadata, WinTrust.h and wincrypt.h.
class WinTrustFileInfo(ctypes.Structure):
    _fields_ = [('cbStruct', ctypes.c_uint32), ('pcwszFilePath', ctypes.c_wchar_p),
                ('hFile', ctypes.c_void_p), ('pgKnownSubject', ctypes.c_void_p)]


class WinTrustData(ctypes.Structure):
    _fields_ = [('cbStruct', ctypes.c_uint32), ('pPolicyCallbackData', ctypes.c_void_p),
                ('pSIPClientData', ctypes.c_void_p), ('dwUIChoice', ctypes.c_uint32),
                ('fdwRevocationChecks', ctypes.c_uint32), ('dwUnionChoice', ctypes.c_uint32),
                ('pFile', ctypes.POINTER(WinTrustFileInfo)), ('dwStateAction', ctypes.c_uint32),
                ('hWVTStateData', ctypes.c_void_p), ('pwszURLReference', ctypes.c_wchar_p),
                ('dwProvFlags', ctypes.c_uint32), ('dwUIContext', ctypes.c_uint32),
                ('pSignatureSettings', ctypes.c_void_p)]


class ProviderSigner(ctypes.Structure):
    _fields_ = [('cbStruct', ctypes.c_uint32), ('verifyTime', ctypes.c_uint32 * 2),
                ('csCertChain', ctypes.c_uint32), ('pasCertChain', ctypes.c_void_p),
                ('dwSignerType', ctypes.c_uint32), ('psSigner', ctypes.c_void_p),
                ('dwError', ctypes.c_uint32), ('csCounterSigners', ctypes.c_uint32),
                ('pasCounterSigners', ctypes.c_void_p), ('pChainContext', ctypes.c_void_p)]


class ProviderCertificatePrefix(ctypes.Structure):
    # Only these documented leading fields are read; cbStruct is checked.
    _fields_ = [('cbStruct', ctypes.c_uint32), ('pCert', ctypes.c_void_p)]


class CertificateContext(ctypes.Structure):
    _fields_ = [('dwCertEncodingType', ctypes.c_uint32), ('pbCertEncoded', ctypes.c_void_p),
                ('cbCertEncoded', ctypes.c_uint32), ('pCertInfo', ctypes.c_void_p), ('hCertStore', ctypes.c_void_p)]


class ChainPolicyParameters(ctypes.Structure):
    _fields_ = [('cbSize', ctypes.c_uint32), ('dwFlags', ctypes.c_uint32), ('pvExtraPolicyPara', ctypes.c_void_p)]


class ChainPolicyStatus(ctypes.Structure):
    _fields_ = [('cbSize', ctypes.c_uint32), ('dwError', ctypes.c_uint32),
                ('lChainIndex', ctypes.c_int32), ('lElementIndex', ctypes.c_int32), ('pvExtraPolicyStatus', ctypes.c_void_p)]


def windows_authenticode(path, handle, evidence):
    """Metadata-only Authenticode inside the parent's killable proof child.

    Normal Windows chain/revocation retrieval is allowed; no Python networking,
    subprocess, certificate installation, custom network client or trust bypass.
    Never call this from the application's main process or transcription path.
    """
    trust = ctypes.WinDLL('wintrust', winmode=0x00000800)
    crypt = ctypes.WinDLL('crypt32', winmode=0x00000800)
    trust.WinVerifyTrust.argtypes = [ctypes.c_void_p, ctypes.POINTER(WindowsGuid), ctypes.POINTER(WinTrustData)]
    trust.WinVerifyTrust.restype = ctypes.c_int32
    trust.WTHelperProvDataFromStateData.argtypes = [ctypes.c_void_p]
    trust.WTHelperProvDataFromStateData.restype = ctypes.c_void_p
    trust.WTHelperGetProvSignerFromChain.argtypes = [ctypes.c_void_p, ctypes.c_uint32, ctypes.c_int32, ctypes.c_uint32]
    trust.WTHelperGetProvSignerFromChain.restype = ctypes.POINTER(ProviderSigner)
    trust.WTHelperGetProvCertFromChain.argtypes = [ctypes.POINTER(ProviderSigner), ctypes.c_uint32]
    trust.WTHelperGetProvCertFromChain.restype = ctypes.POINTER(ProviderCertificatePrefix)
    crypt.CertGetNameStringW.argtypes = [ctypes.c_void_p, ctypes.c_uint32, ctypes.c_uint32,
                                       ctypes.c_char_p, ctypes.c_wchar_p, ctypes.c_uint32]
    crypt.CertGetNameStringW.restype = ctypes.c_uint32
    crypt.CertVerifyCertificateChainPolicy.argtypes = [ctypes.c_void_p, ctypes.c_void_p,
        ctypes.POINTER(ChainPolicyParameters), ctypes.POINTER(ChainPolicyStatus)]
    crypt.CertVerifyCertificateChainPolicy.restype = ctypes.c_int32
    file_info = WinTrustFileInfo(ctypes.sizeof(WinTrustFileInfo), str(path), handle, None)
    data = WinTrustData(); data.cbStruct = ctypes.sizeof(data)
    data.dwUIChoice = 2; data.fdwRevocationChecks = 1; data.dwUnionChoice = 1
    data.pFile = ctypes.pointer(file_info); data.dwStateAction = 1
    # Check the whole non-root chain; the root is the trust anchor. Disable
    # obsolete signature hashes. No cache-only/revocation-ignore/test flags.
    data.dwProvFlags = 0x80 | 0x2000
    action = windows_guid('00aac56b-cd44-11d0-8cc2-00c04fc295ee')

    def certificate_attribute(certificate, oid):
        buffer = ctypes.create_unicode_buffer(256)
        count = crypt.CertGetNameStringW(certificate, 3, 0, oid, buffer, len(buffer))
        if not 1 < count < len(buffer):
            raise ValueError('Defender signer has a missing/oversized certificate identity')
        return buffer.value

    try:
        status = trust.WinVerifyTrust(ctypes.c_void_p(-1), ctypes.byref(action), ctypes.byref(data))
        evidence['authenticode_status'] = status & 0xffffffff
        provider = trust.WTHelperProvDataFromStateData(data.hWVTStateData) if data.hWVTStateData else None
        signer = trust.WTHelperGetProvSignerFromChain(provider, 0, False, 0) if provider else None
        if not signer or signer.contents.cbStruct < ctypes.sizeof(ProviderSigner):
            raise ValueError('Defender signature has no usable provider signer')
        evidence['signer_error'] = signer.contents.dwError
        if not 1 <= signer.contents.csCertChain <= 8 or not signer.contents.pChainContext:
            raise ValueError('Defender signature has no bounded signer chain')
        evidence['certificate_chain'] = []
        for index in range(signer.contents.csCertChain):
            item = trust.WTHelperGetProvCertFromChain(signer, index)
            if not item or item.contents.cbStruct < ctypes.sizeof(ProviderCertificatePrefix) or not item.contents.pCert:
                raise ValueError('Defender signature chain certificate is missing')
            certificate = ctypes.cast(item.contents.pCert, ctypes.POINTER(CertificateContext)).contents
            if not certificate.pbCertEncoded or not 0 < certificate.cbCertEncoded <= 65536:
                raise ValueError('Defender signature certificate exceeds metadata bound')
            evidence['certificate_chain'].append({
                'common_name': certificate_attribute(item.contents.pCert, b'2.5.4.3'),
                'organization': certificate_attribute(item.contents.pCert, b'2.5.4.10'),
                'der_sha256': hashlib.sha256(ctypes.string_at(certificate.pbCertEncoded, certificate.cbCertEncoded)).hexdigest()})
        parameters = ChainPolicyParameters(ctypes.sizeof(ChainPolicyParameters), 0x00040000, None)
        policy = ChainPolicyStatus(); policy.cbSize = ctypes.sizeof(policy)
        # Microsoft product-root keys only; test roots are never enabled and
        # flight roots are explicitly disabled. No application-root fallback.
        checked = crypt.CertVerifyCertificateChainPolicy(7, signer.contents.pChainContext,
                                                        ctypes.byref(parameters), ctypes.byref(policy))
        evidence.update(microsoft_root_policy_checked=bool(checked), microsoft_root_policy_error=policy.dwError,
                        microsoft_root_policy_flags=parameters.dwFlags, revocation='chain-excluding-root')
    finally:
        data.dwStateAction = 2
        trust.WinVerifyTrust(ctypes.c_void_p(-1), ctypes.byref(action), ctypes.byref(data))


def windows_version_identity(path):
    version = ctypes.WinDLL('version', use_last_error=True, winmode=0x00000800)
    version.GetFileVersionInfoSizeExW.argtypes = [ctypes.c_uint32, ctypes.c_wchar_p, ctypes.POINTER(ctypes.c_uint32)]
    version.GetFileVersionInfoSizeExW.restype = ctypes.c_uint32
    version.GetFileVersionInfoExW.argtypes = [ctypes.c_uint32, ctypes.c_wchar_p, ctypes.c_uint32, ctypes.c_uint32, ctypes.c_void_p]
    version.GetFileVersionInfoExW.restype = ctypes.c_int32
    version.VerQueryValueW.argtypes = [ctypes.c_void_p, ctypes.c_wchar_p, ctypes.POINTER(ctypes.c_void_p), ctypes.POINTER(ctypes.c_uint32)]
    version.VerQueryValueW.restype = ctypes.c_int32
    # Neither FILE_VER_GET_LOCALISED nor FILE_VER_GET_NEUTRAL: use the actual
    # binary's resource, not strings merged from a separate MUI file.
    unused = ctypes.c_uint32()
    size = version.GetFileVersionInfoSizeExW(0, str(path), ctypes.byref(unused))
    if not 0 < size <= 1024 * 1024:
        raise ValueError('Defender version resource is absent/oversized')
    buffer = ctypes.create_string_buffer(size)
    if not version.GetFileVersionInfoExW(0, str(path), 0, size, buffer):
        raise ctypes.WinError(ctypes.get_last_error())

    def query(key):
        value = ctypes.c_void_p(); count = ctypes.c_uint32()
        if not version.VerQueryValueW(buffer, key, ctypes.byref(value), ctypes.byref(count)) or not value.value:
            raise ValueError('Defender version resource is missing ' + key)
        if not ctypes.addressof(buffer) <= value.value < ctypes.addressof(buffer) + size:
            raise ValueError('Defender version resource pointer escaped its buffer')
        return value.value, count.value

    translations, count = query('\\VarFileInfo\\Translation')
    if not count or count % 4 or count > 16 or translations + count > ctypes.addressof(buffer) + size:
        raise ValueError('Defender version translations are malformed/oversized')
    words = (ctypes.c_uint16 * (count // 2)).from_address(translations)
    records = []
    for index in range(0, len(words), 2):
        result = {}
        for key in ('OriginalFilename', 'CompanyName', 'ProductName', 'FileVersion'):
            value, length = query(f'\\StringFileInfo\\{words[index]:04x}{words[index + 1]:04x}\\{key}')
            if not 1 < length <= 256 or value + length * 2 > ctypes.addressof(buffer) + size:
                raise ValueError('Defender version string is missing/oversized')
            result[key] = ctypes.wstring_at(value, length - 1)
        records.append(result)
    return records


def require_defender_identity(evidence):
    """Fail closed; diagnostics never authorize the classification."""
    if evidence.get('authenticode_status') != 0 or evidence.get('signer_error') != 0:
        raise ValueError('Defender Authenticode signature did not verify')
    if (evidence.get('microsoft_root_policy_checked') is not True
            or evidence.get('microsoft_root_policy_error') != 0
            or evidence.get('microsoft_root_policy_flags') != 0x00040000):
        raise ValueError('Defender signature does not satisfy the Microsoft product-root policy')
    chain = evidence.get('certificate_chain') or []
    if not chain or chain[0].get('organization') != 'Microsoft Corporation':
        raise ValueError('Defender signature publisher is not Microsoft Corporation')
    versions = evidence.get('version_identity') or []
    if not versions or any(item.get('OriginalFilename', '').lower() != 'mpoav.dll'
                           or item.get('CompanyName') != 'Microsoft Corporation' for item in versions):
        raise ValueError('Defender signed module resource identity does not match MpOav.dll')


def verified_defender_module(path):
    evidence = {'kind': 'windows-defender-amsi', 'verified': False, 'module': 'MpOav.dll',
                'provider_clsid': DEFENDER_AMSI_CLSID, 'inference_tested': False}
    try:
        evidence['stage'] = 'registered-platform-path'
        program_data = windows_program_data()
        evidence['loaded_path'] = str(path)[:1024]
        candidate = defender_platform_path(path, program_data)
        registered_path = defender_registered_path()
        evidence['registered_path'] = registered_path[:1024]
        registered = defender_platform_path(registered_path, program_data)
        if candidate != registered:
            raise ValueError('Defender registration does not match the loaded module path')
        evidence['path'] = str(candidate)
        evidence['registry_path_matched'] = True
        windows_no_reparse(Path(candidate))
        resolved = Path(candidate).resolve(strict=True)
        if PureWindowsPath(resolved) != candidate:
            raise ValueError('Defender loaded module canonical path changed')
        evidence['stage'] = 'locked-file-identity'
        with windows_locked_file(resolved) as (stream, handle, final_path):
            if defender_platform_path(final_path, program_data) != candidate:
                raise ValueError('Defender file handle canonical path differs from the loaded module')
            windows_no_reparse(Path(candidate))
            evidence['bytes'] = os.fstat(stream.fileno()).st_size
            if not 0 < evidence['bytes'] <= 32 * 1024 * 1024:
                raise ValueError('Defender module exceeds metadata inspection bound')
            evidence['sha256'] = hashlib.file_digest(stream, 'sha256').hexdigest()
            evidence['stage'] = 'authenticode-and-microsoft-root'
            windows_authenticode(resolved, handle, evidence)
            evidence['stage'] = 'version-resource'
            evidence['version_identity'] = windows_version_identity(resolved)
            evidence['stage'] = 'identity-policy'
            require_defender_identity(evidence)
        evidence['verified'] = True
        evidence['stage'] = 'verified'
        return evidence
    except (OSError, ValueError) as exc:
        evidence['failure'] = str(exc)[:512]
        evidence['error_type'] = type(exc).__name__
        raise AssertionError('Loaded host-security module could not be verified: ' + evidence['failure']) from exc
    finally:
        print('HOST_SECURITY_MODULE_DIAGNOSTIC=' + json.dumps(evidence, sort_keys=True, ensure_ascii=True),
              file=sys.stderr, flush=True)


def data_file_diagnostic(path, root, record_sha256=None):
    """Read-only evidence for a bundled data file; never import/alter its package."""
    path = Path(path); root = Path(root).resolve()
    resolved = path.resolve()
    if not resolved.is_relative_to(root):
        raise ValueError('Bundled diagnostic path escaped the private runtime')
    result = {'relative_path': resolved.relative_to(root).as_posix(),
              'path_contains_non_ascii': not str(resolved).isascii(),
              'exists': path.exists(), 'regular_file': path.is_file() and not path.is_symlink(),
              'record_sha256_available': record_sha256 is not None, 'readable': False}
    if result['regular_file']:
        result['bytes'] = path.stat().st_size
        if result['bytes'] > 256 * 1024 * 1024:
            result['hash_skipped'] = 'diagnostic byte limit'
        else:
            with path.open('rb') as stream:
                digest = hashlib.file_digest(stream, 'sha256').digest()
            result.update(readable=True, sha256=digest.hex(),
                          record_sha256_match=base64.urlsafe_b64encode(digest).decode().rstrip('=') == record_sha256 if record_sha256 is not None else None)
    return result


def nagisa_data_diagnostic():
    distribution = importlib.metadata.distribution('nagisa')
    name = 'nagisa/data/nagisa_v001.model'
    entries = [entry for entry in distribution.files or () if entry.as_posix() == name]
    expected = entries[0].hash if len(entries) == 1 else None
    record_hash = expected.value if expected is not None and expected.mode == 'sha256' else None
    result = data_file_diagnostic(distribution.locate_file(name), ROOT, record_hash)
    result.update(schema=1, kind='bundled-nagisa-data-diagnostic', metadata_entry_present=len(entries) == 1,
                  python_filesystem_encoding=sys.getfilesystemencoding())
    if sys.platform == 'win32':
        kernel = ctypes.WinDLL('kernel32', use_last_error=True)
        kernel.GetACP.restype = ctypes.c_uint
        result['windows_ansi_code_page'] = kernel.GetACP()
    return result


def local_module(name):
    module = importlib.import_module(name)
    location = Path(module.__file__).resolve()
    if not location.is_relative_to(ROOT):
        raise AssertionError(f'{name} escaped the private component: {location}')
    return module


def check_native_library(path, root, allowed, *, windows):
    resolved = Path(path).resolve()
    private = resolved.is_relative_to(root)
    test = str(resolved).lower() if windows else str(resolved)
    # This precedes both the OS-directory and host-security classifications.
    if windows and (optional_vc_runtime(resolved.name) or private_openmp_runtime(resolved.name)) and not private:
        raise AssertionError(f'Optional CRT/OpenMP runtime came from outside the private component: {path}')
    if not private and not test.startswith(allowed):
        if windows and resolved.name.lower() == 'mpoav.dll':
            return verified_defender_module(path)
        raise AssertionError(f'Loaded non-system library outside component: {path}')
    return None


def windows_module_paths():
    from ctypes import wintypes
    psapi = ctypes.WinDLL('psapi', use_last_error=True, winmode=0x00000800)
    kernel = ctypes.WinDLL('kernel32', use_last_error=True, winmode=0x00000800)
    kernel.GetCurrentProcess.restype = wintypes.HANDLE
    psapi.EnumProcessModules.restype = wintypes.BOOL
    psapi.EnumProcessModules.argtypes = [wintypes.HANDLE, ctypes.POINTER(wintypes.HMODULE), wintypes.DWORD, ctypes.POINTER(wintypes.DWORD)]
    psapi.GetModuleFileNameExW.restype = wintypes.DWORD
    psapi.GetModuleFileNameExW.argtypes = [wintypes.HANDLE, wintypes.HMODULE, wintypes.LPWSTR, wintypes.DWORD]
    process = kernel.GetCurrentProcess(); handles = (wintypes.HMODULE * 4096)(); needed = wintypes.DWORD()
    if not psapi.EnumProcessModules(process, handles, ctypes.sizeof(handles), ctypes.byref(needed)):
        raise ctypes.WinError(ctypes.get_last_error())
    if not needed.value or needed.value > ctypes.sizeof(handles) or needed.value % ctypes.sizeof(wintypes.HMODULE):
        raise AssertionError('Native library enumeration overflow or invalid module count')
    paths = []
    for handle in handles[:needed.value // ctypes.sizeof(wintypes.HMODULE)]:
        buffer = ctypes.create_unicode_buffer(32768)
        count = psapi.GetModuleFileNameExW(process, handle, buffer, len(buffer))
        if not count or count >= len(buffer):
            raise AssertionError('Native library path enumeration failed or was truncated')
        paths.append(buffer.value)
    return paths


def audit_windows_module_snapshots(enumerate_paths, root, allowed, host_security_modules):
    # Trust APIs can load more OS libraries. Re-audit new paths until a second
    # snapshot agrees, with a hard three-round bound and per-invocation cache.
    checked = set(); previous = None
    for _ in range(3):
        paths = enumerate_paths()
        if not 0 < len(paths) <= 4096:
            raise AssertionError('Native library snapshot exceeds bounds')
        current = frozenset(str(path).lower() for path in paths)
        for path in paths:
            identity = str(path).lower()
            if identity not in checked:
                evidence = check_native_library(path, root, allowed, windows=True)
                if evidence is not None: host_security_modules.append(evidence)
                checked.add(identity)
        if current == previous: return len(current)
        previous = current
    raise AssertionError('Native library snapshot did not stabilize within three rounds')


def loaded_native_libraries(host_security_modules=None):
    if host_security_modules is None: host_security_modules = []
    if sys.platform == 'darwin':
        lib = ctypes.CDLL(None)
        lib._dyld_image_count.restype = ctypes.c_uint32
        lib._dyld_get_image_name.argtypes = [ctypes.c_uint32]
        lib._dyld_get_image_name.restype = ctypes.c_char_p
        paths = [lib._dyld_get_image_name(i).decode() for i in range(lib._dyld_image_count())]
        for path in paths:
            check_native_library(path, ROOT, ('/usr/lib/', '/System/Library/'), windows=False)
        return len(paths)
    if sys.platform == 'win32':
        allowed = (str(Path(os.environ['SystemRoot']).resolve()).lower() + '\\',)
        return audit_windows_module_snapshots(windows_module_paths, ROOT, allowed, host_security_modules)
    raise AssertionError('Only native Windows/macOS are supported')


def main():
    config = json.loads((ROOT / 'component.json').read_text(encoding='utf-8'))
    assert sys.flags.isolated and not site.ENABLE_USER_SITE
    assert Path(sys.executable).resolve().is_relative_to(ROOT)
    assert Path(sys.prefix).resolve() == ROOT
    assert sys.version.split()[0] == config['python_version']
    for path in sys.path:
        assert path and Path(path).resolve().is_relative_to(ROOT), f'External sys.path: {path}'
    for name in ('HF_HUB_OFFLINE', 'TRANSFORMERS_OFFLINE', 'HF_DATASETS_OFFLINE',
                 'HF_HUB_DISABLE_TELEMETRY', 'HF_HUB_DISABLE_IMPLICIT_TOKEN', 'DO_NOT_TRACK'):
        os.environ[name] = '1'
    os.environ['TOKENIZERS_PARALLELISM'] = 'false'
    sys.modules['vllm'] = None
    def audit(event, args):
        if event in {'socket.connect','socket.getaddrinfo','socket.bind','subprocess.Popen','os.system','os.posix_spawn','os.fork'}:
            raise RuntimeError(f'Offline self-test blocked {event}')
    sys.addaudithook(audit)
    numpy = local_module('numpy')
    assert numpy.asarray([1, 2, 3]).sum() == 6
    backend = config['backend']; device = 'cpu'; versions = {}; metal = None; metal_tested = False; numba_threading = None
    if backend == 'faster-whisper':
        assert not list(ROOT.rglob('cudnn*.dll')), 'CPU pack must omit unused cuDNN redistributables'
        fw = local_module('faster_whisper'); ct = local_module('ctranslate2')
        assert callable(fw.WhisperModel)
        assert 'int8' in ct.get_supported_compute_types('cpu')
        local_module('av'); local_module('onnxruntime'); local_module('tokenizers')
        packages = ['faster-whisper', 'ctranslate2', 'numpy']
    elif backend == 'mlx-whisper':
        mx = local_module('mlx.core'); mlx = local_module('mlx_whisper.transcribe')
        assert hasattr(mlx, 'ModelHolder')
        metal = bool(mx.metal.is_available())
        mx.set_default_device(mx.gpu if metal else mx.cpu)
        assert int(mx.sum(mx.array([1, 2, 3])).item()) == 6
        metal_tested = metal
        device = 'metal' if metal else 'cpu (runner has no Metal device)'
        packages = ['mlx-whisper', 'mlx', 'mlx-metal', 'torch', 'numpy']
        local_module('torch')
    elif backend == 'qwen3-asr':
        # A valid bundled file that Python can read but DyNet cannot open from
        # the Unicode relocation is materially different from missing data.
        # This is evidence only, never a path or dependency workaround.
        print(json.dumps(nagisa_data_diagnostic(), sort_keys=True), file=sys.stderr, flush=True)
        if sys.platform == 'win32':
            luma_configure_numba_workqueue()
            luma_prepare_nagisa()
            numba_threading = luma_probe_numba_workqueue()
        torch = local_module('torch')
        assert torch.version.cuda is None, 'CUDA libraries are outside this CPU pack'
        qwen = local_module('qwen_asr')
        for module in ('nagisa', 'dynet', 'soynlp', 'librosa', 'soundfile', 'transformers', 'qwen_omni_utils'):
            local_module(module)
        assert callable(qwen.Qwen3ASRModel.from_pretrained)
        assert 'forced_aligner' in inspect.signature(qwen.Qwen3ASRModel.from_pretrained).parameters
        assert torch.ones((2, 2), device='cpu').sum().item() == 4
        if sys.platform == 'win32':
            assert luma_check_numba_workqueue(require_initialized=True) == 'workqueue'
            numba_threading['checked_after_qwen_imports'] = True
        packages = ['qwen-asr', 'torch', 'transformers', 'nagisa', 'DyNet38', 'numpy']
    else:
        raise AssertionError('Unknown engine')
    for package in packages:
        versions[package] = importlib.metadata.version(package)
    assert importlib.util.find_spec('pip') is None, 'pip must not ship'
    host_security_modules = []
    libraries = loaded_native_libraries(host_security_modules)
    print(json.dumps({'schema': 1, 'pack_id': config['id'], 'source_sha': config['source_sha'],
                      'python': sys.version.split()[0], 'platform': platform_description(), 'machine': platform.machine(),
                      'isolated': True, 'user_site': False, 'relocatable': True,
                      'private_native_libraries_checked': libraries, 'versions': versions,
                      'verified_host_security_modules': host_security_modules,
                      'tested_device': device, 'inference_tested': False,
                      'metal_available': metal, 'metal_tested': metal_tested,
                      'numba_threading': numba_threading,
                      'scope': 'Offline model import/API and tiny tensor operations; host-security Authenticode may use normal Windows revocation retrieval; no model weights loaded.'}, ensure_ascii=False))


if __name__ == '__main__':
    main()
