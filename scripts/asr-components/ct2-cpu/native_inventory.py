#!/usr/bin/env python3
"""Bounded PE normal/delay import inventory; never repair or rename binaries."""
from __future__ import annotations
import hashlib
from pathlib import Path
import re
import struct

FORBIDDEN = re.compile(r'(?:cudnn|cublas|cudart|nvrtc|nvcuda|libiomp|libomp|vcomp|mkl|tbb)', re.I)
CRT = re.compile(r'^(?:msvcp\d|vcruntime\d|concrt\d|vcomp\d)', re.I)
# OS libraries are never copied from the host. API-set contracts are resolved by Windows.
OS_DLLS = frozenset('''advapi32.dll avicap32.dll avrt.dll bcrypt.dll bcryptprimitives.dll cabinet.dll cfgmgr32.dll
combase.dll comctl32.dll comdlg32.dll crypt32.dll cryptbase.dll cryptnet.dll cryptsp.dll
cryptui.dll d3d12.dll dbghelp.dll dnsapi.dll dwmapi.dll dxcore.dll dxgi.dll gdi32.dll
gdi32full.dll imagehlp.dll imm32.dll iphlpapi.dll kernel32.dll kernelbase.dll mpr.dll
msasn1.dll msi.dll msimg32.dll msvcrt.dll ncrypt.dll netapi32.dll normaliz.dll ntdll.dll ole32.dll
oleacc.dll oleaut32.dll pdh.dll powrprof.dll propsys.dll psapi.dll rpcrt4.dll secur32.dll setupapi.dll
shell32.dll shlwapi.dll sspicli.dll ucrtbase.dll user32.dll userenv.dll usp10.dll version.dll
wevtapi.dll win32u.dll winhttp.dll wininet.dll winmm.dll winspool.drv wintrust.dll
wldap32.dll ws2_32.dll wtsapi32.dll'''.split())


def digest(path):
    with Path(path).open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def pe_imports(path):
    """Return every normal and delayed DLL name, with architecture, or fail closed."""
    data = Path(path).read_bytes()
    def unpack(fmt, at):
        if at < 0 or at + struct.calcsize(fmt) > len(data):
            raise ValueError('Truncated PE structure')
        return struct.unpack_from(fmt, data, at)
    if data[:2] != b'MZ':
        raise ValueError('Expected PE image')
    pe, = unpack('<I', 0x3c)
    if data[pe:pe + 4] != b'PE\0\0':
        raise ValueError('Invalid PE signature')
    machine, sections = unpack('<HH', pe + 4)
    optional_size, = unpack('<H', pe + 20)
    optional = pe + 24
    magic, = unpack('<H', optional)
    if magic == 0x20b:
        image_base, = unpack('<Q', optional + 24)
        count, = unpack('<I', optional + 108)
        directories = optional + 112
    elif magic == 0x10b:
        image_base, = unpack('<I', optional + 28)
        count, = unpack('<I', optional + 92)
        directories = optional + 96
    else:
        raise ValueError('Unsupported PE optional header')
    if not 1 <= sections <= 96 or count > 32 or directories + count * 8 > optional + optional_size:
        raise ValueError('Invalid PE header bounds')
    table = optional + optional_size
    def offset(rva, length=1):
        for index in range(sections):
            virtual_size, address, raw_size, raw = unpack('<IIII', table + index * 40 + 8)
            if address <= rva and rva + length <= address + raw_size:
                at = raw + rva - address
                if at + length > len(data):
                    raise ValueError('PE section is truncated')
                return at
        raise ValueError('PE RVA is outside file-backed sections')
    def name_at(rva):
        at = offset(rva)
        end = data.find(b'\0', at, min(at + 260, len(data)))
        if end < 0:
            raise ValueError(f'Unterminated PE import name at RVA 0x{rva:x}')
        offset(rva, end - at + 1)
        raw_name = data[at:end]
        diagnostic = repr(raw_name[:96]) + (' (truncated)' if len(raw_name) > 96 else '')
        try:
            name = raw_name.decode('ascii').lower()
        except UnicodeDecodeError:
            raise ValueError(f'Non-ASCII PE import name at RVA 0x{rva:x}: {diagnostic}') from None
        if not re.fullmatch(r'[a-z0-9_.+-]+\.(dll|drv)', name):
            raise ValueError(f'Unsafe PE import name at RVA 0x{rva:x}: {diagnostic}')
        return name
    def directory(index, size, delayed):
        if count <= index:
            return []
        rva, length = unpack('<II', directories + index * 8)
        if not rva and not length:
            return []
        if not rva or length < size or length > len(data):
            raise ValueError('Invalid PE import directory')
        imports = []
        for position in range(min(length // size, 1024)):
            fields = unpack('<' + 'I' * (size // 4), offset(rva + position * size, size))
            if not any(fields):
                return sorted(set(imports))
            name_rva = fields[1] if delayed else fields[3]
            if delayed:
                if fields[0] not in (0, 1):
                    raise ValueError('Unsupported delay-import attributes')
                if fields[0] == 0:
                    name_rva -= image_base
            try:
                imports.append(name_at(name_rva))
            except ValueError as error:
                kind = 'delay' if delayed else 'normal'
                raise ValueError(f'{kind} import: {error}') from None
        raise ValueError('Unterminated PE import directory')
    return {'machine': hex(machine), 'normal': directory(1, 20, False),
            'delay': directory(13, 32, True)}


def inventory(root):
    root = Path(root)
    result = []
    for path in sorted(root.rglob('*')):
        if path.is_file() and path.suffix.lower() in {'.dll', '.pyd', '.exe'}:
            if path.is_symlink():
                raise ValueError('Native symlink is forbidden')
            relative = path.relative_to(root).as_posix()
            sha256 = digest(path)
            try:
                imports = pe_imports(path)
            except ValueError as error:
                identity = ascii(relative[:240]) + (' (truncated)' if len(relative) > 240 else '')
                raise ValueError(f'PE inventory failed for {identity} sha256={sha256}: {error}') from None
            result.append({'path': relative, 'bytes': path.stat().st_size,
                           'sha256': sha256, **imports})
    return result


def classify(name, by_name):
    if FORBIDDEN.search(name):
        return 'forbidden'
    if name in by_name:
        return 'private'
    if CRT.search(name):
        return 'missing_private_crt'
    if name.startswith(('api-ms-win-', 'ext-ms-win-')):
        return 'windows_api_set'
    if name in OS_DLLS:
        return 'windows_os'
    return 'unresolved'


def closure(files):
    by_name = {}
    for item in files:
        by_name.setdefault(Path(item['path']).name.lower(), []).append(item['path'])
    dependencies = []
    for item in files:
        for kind in ('normal', 'delay'):
            for name in item[kind]:
                dependencies.append({'from': item['path'], 'kind': kind, 'name': name,
                                     'resolution': classify(name, by_name),
                                     'private_candidates': by_name.get(name, [])})
    blocked = [x for x in dependencies if x['resolution'] in {'forbidden', 'missing_private_crt', 'unresolved'}]
    forbidden_files = [x['path'] for x in files if FORBIDDEN.search(Path(x['path']).name)]
    return {'schema': 1, 'normal_and_delay_imports': True, 'files': files,
            'dependencies': dependencies, 'blocked_dependencies': blocked,
            'forbidden_files': forbidden_files, 'passed': not blocked and not forbidden_files,
            'limitations': ['Static dependency inventory is not a loader test.',
                           'Private candidates still require an actual isolated import/inference test.',
                           'Windows OS/API-set dependencies are not redistributed.']}
