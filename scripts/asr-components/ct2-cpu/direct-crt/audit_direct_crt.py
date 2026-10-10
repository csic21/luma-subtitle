#!/usr/bin/env python3
"""Report-only extraction of one hash-pinned official Microsoft CRT package.

This is an audit helper, not an app installer. It never executes the downloaded
installer or DLLs, accepts terms, registers software, or publishes binary files.
Only Windows' existing System32 expand.exe is invoked. Authenticode verification
is a separate native PowerShell step; this helper does not claim to provide it.
"""
from __future__ import annotations
import argparse
import ctypes
import hashlib
import json
import os
from pathlib import Path
import struct
import subprocess
import urllib.parse
import urllib.request

URL = 'https://download.visualstudio.microsoft.com/download/pr/73aabf2e-9532-4f68-99f7-3247081a619c/CC0FF0EB1DC3F5188AE6300FAEF32BF5BEEBA4BDD6E8E445A9184072096B713B/VC_redist.x64.exe'
INSTALLER = (25635768, 'cc0ff0eb1dc3f5188ae6300faef32bf5beeba4bdd6e8e445a9184072096b713b')
SLICES = {
    'ux.cab': (479744, 195988, '2f57cf2cd504bd100c9222bb123b9c3a40802cca73e89ca05c97885d91be3a78'),
    'attached.cab': (686152, 24939223, '468f1264d50b9e3b9d309ab9e13c02ff379fd95a2837383bf65c9616583013d6'),
}
MINIMUM = (987836, '640aa6c516c72444523b8fbe034db46ff4e118ed02705340e3ccb62d426ff040')
DLLS = {
    'msvcp140.dll': (557728, '0f885b509a685d2bbfa652fed26b5fb31d88fbdab0a978c641d1c7b8aa460aa9'),
    'msvcp140_1.dll': (35952, 'bfad5aef4c63a669e3c140655cdfdf395b6c979b400a447bd5dcb65ed8826c3d'),
}
LICENSES = {
    'license-en.rtf': ('u4', 9235, '8099dc3cf9502c335da829e5c755948a12e3e6de490eb492a99deb673d883d8b'),
    'license-zh-CN.rtf': ('u28', 18214, 'a808b4933ce3b3e0893504dbef43ebf90b8b567f94bd6481b6315ed9141e1b11'),
}


def require(condition: bool, message: str) -> None:
    if not condition:
        raise ValueError(message)


def identity(path: Path, expected: tuple[int, str]) -> dict:
    require(path.is_file() and not path.is_symlink(), f'Not a regular file: {path.name}')
    require(path.stat().st_size == expected[0], f'Unexpected size: {path.name}')
    actual = hashlib.sha256(path.read_bytes()).hexdigest()
    require(actual == expected[1], f'Unexpected SHA-256: {path.name}')
    return {'path': str(path.resolve()), 'bytes': expected[0], 'sha256': actual}


class MicrosoftOnlyRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        target = urllib.parse.urlsplit(newurl)
        require(target.scheme == 'https' and target.hostname == 'download.visualstudio.microsoft.com'
                and target.port in (None, 443) and not target.username and not target.password,
                'Download redirect left the fixed Microsoft HTTPS origin')
        return super().redirect_request(req, fp, code, msg, headers, newurl)


def download(path: Path) -> None:
    opener = urllib.request.build_opener(MicrosoftOnlyRedirect())
    with opener.open(URL, timeout=60) as response, path.open('xb') as output:
        require(response.status == 200, 'Unexpected download status')
        if response.headers.get('Content-Length'):
            require(int(response.headers['Content-Length']) == INSTALLER[0], 'Unexpected Content-Length')
        count = 0
        while chunk := response.read(1024 * 1024):
            count += len(chunk)
            require(count <= INSTALLER[0], 'Download exceeded pinned size')
            output.write(chunk)
    identity(path, INSTALLER)


def split_containers(installer: Path, root: Path) -> dict[str, Path]:
    # Whole-file hash is mandatory before interpreting any binary field. Exact
    # offsets are intentional: another package version must be separately audited.
    identity(installer, INSTALLER)
    data = installer.read_bytes()
    require(data[:2] == b'MZ' and data[280:284] == b'PE\0\0', 'Unexpected pinned PE header')
    require(data[447488:447488 + 8] == struct.pack('<II', 0x00f14300, 2), 'Unexpected Burn header')
    require(struct.unpack_from('<II', data, 447488 + 40) == (1, 2), 'Unexpected Burn CAB count')
    require(struct.unpack_from('<II', data, 447488 + 48) == (195988, 24939223), 'Unexpected Burn CAB sizes')
    result = {}
    for name, (offset, size, sha) in SLICES.items():
        payload = data[offset:offset + size]
        require(len(payload) == size and payload[:4] == b'MSCF', 'Invalid pinned CAB slice')
        require(hashlib.sha256(payload).hexdigest() == sha, 'Unexpected pinned CAB hash')
        path = root / name
        with path.open('xb') as output:
            output.write(payload)
        result[name] = path
    return result


def system_expand() -> Path:
    require(os.name == 'nt', 'Native extraction audit must run on Windows')
    kernel32 = ctypes.WinDLL('kernel32', use_last_error=True)
    get_system_directory = kernel32.GetSystemDirectoryW
    get_system_directory.argtypes = [ctypes.c_wchar_p, ctypes.c_uint]
    get_system_directory.restype = ctypes.c_uint
    buffer = ctypes.create_unicode_buffer(32768)
    length = get_system_directory(buffer, len(buffer))
    require(0 < length < len(buffer), 'Cannot locate the Windows system directory')
    executable = Path(buffer.value) / 'expand.exe'
    require(executable.is_file() and executable.is_absolute(), 'System expand.exe is unavailable')
    return executable


def extract_member(expand: Path, cabinet: Path, member: str, dest: Path, expected: tuple[int, str]) -> Path:
    require(member in {'a12', 'msvcp140.dll_amd64', 'msvcp140_1.dll_amd64', 'u4', 'u28'}, 'Unaudited member')
    require(not dest.exists(), 'Extraction directory already exists')
    dest.mkdir(mode=0o700)
    result = subprocess.run([str(expand), str(cabinet), '-F:' + member, str(dest)],
                            check=False, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                            timeout=120, shell=False, cwd=str(dest))
    require(result.returncode == 0, f'Native CAB extraction failed for {member}: exit {result.returncode}')
    require(sorted(item.name for item in dest.iterdir()) == [member], 'Unexpected extracted output')
    target = dest / member
    identity(target, expected)
    return target


def run(work_dir: Path) -> dict:
    expand = system_expand()
    require(work_dir.is_absolute(), 'Work directory must be absolute')
    require(not work_dir.exists(), 'Use a new private work directory')
    require(work_dir.parent.is_dir(), 'Work directory parent must exist')
    work_dir.mkdir(mode=0o700)
    root = work_dir.resolve(strict=True)
    installer = root / 'VC_redist.x64.exe'
    download(installer)
    containers = split_containers(installer, root)
    minimum_member = extract_member(expand, containers['attached.cab'], 'a12', root / 'minimum', MINIMUM)
    minimum = root / 'minimum-x64.cab'
    minimum_member.rename(minimum)
    files = [{'role': 'installer', **identity(installer, INSTALLER)}]
    for name, expected in DLLS.items():
        member = name + '_amd64'
        extracted = extract_member(expand, minimum, member, root / (name + '-extracted'), expected)
        target = root / name
        require(not target.exists(), 'Output already exists')
        extracted.rename(target)
        files.append({'role': name, **identity(target, expected)})
    notices = []
    for name, (member, size, sha) in LICENSES.items():
        extracted = extract_member(expand, containers['ux.cab'], member, root / (member + '-notice'), (size, sha))
        target = root / name
        extracted.rename(target)
        notices.append({'role': name, **identity(target, (size, sha))})
    return {
        'schema': 1, 'source_url': URL, 'work_root': str(root), 'files': files,
        'notices': notices, 'eula_id': 'Cpp_2015-2022_ENU.1033',
        'installer_executed': False, 'runtime_executed': False,
        'agreement_accepted': False, 'authenticode_verified': False,
        'extraction_method': 'Exact hash-pinned CAB slices; Windows system expand.exe with fixed members',
    }


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--work-dir', type=Path, required=True)
    parser.add_argument('--report', type=Path, required=True)
    args = parser.parse_args()
    require(not args.report.exists(), 'Report file already exists')
    report = run(args.work_dir)
    with args.report.open('x', encoding='utf-8') as output:
        json.dump(report, output, indent=2)
        output.write('\n')
    print(json.dumps({'report': str(args.report.resolve()), 'files_verified': len(report['files']),
                      'authenticode_verified': False}))


if __name__ == '__main__':
    main()
