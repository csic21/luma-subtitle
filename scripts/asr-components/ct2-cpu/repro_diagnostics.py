"""Read-only wheel/member/PE diagnostics. Never normalize or patch PE bytes."""
import hashlib
from pathlib import Path
import struct
import uuid
import zipfile


def sha(data):
    return hashlib.sha256(data).hexdigest()


def pe_details(data):
    data = bytes(data)
    def unpack(fmt, at):
        if at < 0 or at + struct.calcsize(fmt) > len(data):
            raise ValueError('Truncated PE diagnostic structure')
        return struct.unpack_from(fmt, data, at)
    if data[:2] != b'MZ':
        raise ValueError('Expected PE diagnostic input')
    pe, = unpack('<I', 0x3c)
    if data[pe:pe + 4] != b'PE\0\0':
        raise ValueError('Invalid PE diagnostic signature')
    count, timestamp = unpack('<HI', pe + 6)
    optional_size, = unpack('<H', pe + 20)
    optional = pe + 24
    magic, = unpack('<H', optional)
    if magic not in (0x10b, 0x20b) or not 1 <= count <= 96:
        raise ValueError('Invalid PE diagnostic header')
    directories = optional + (112 if magic == 0x20b else 96)
    directory_count, = unpack('<I', directories - 4)
    if directory_count > 32 or directories + directory_count * 8 > optional + optional_size:
        raise ValueError('Invalid PE diagnostic directory bounds')
    table = optional + optional_size
    sections = []
    for index in range(count):
        at = table + 40 * index
        name, virtual_size, address, size, raw = unpack('<8sIIII', at)
        if raw + size > len(data):
            raise ValueError('Truncated PE diagnostic section')
        sections.append({'name': name.rstrip(b'\0').decode('ascii', errors='replace'),
                         'rva': address, 'raw_offset': raw, 'bytes': size,
                         'sha256': sha(data[raw:raw + size])})
    def offset(rva, size):
        for section in sections:
            if section['rva'] <= rva and rva + size <= section['rva'] + section['bytes']:
                return section['raw_offset'] + rva - section['rva']
        raise ValueError('PE debug directory is outside file-backed sections')
    debug = []
    if directory_count > 6:
        rva, size = unpack('<II', directories + 6 * 8)
        if bool(rva) != bool(size) or size % 28 or size > 28 * 256:
            raise ValueError('Invalid PE debug directory')
        if size:
            base = offset(rva, size)
            for index in range(size // 28):
                _, stamp, major, minor, kind, length, _, raw = unpack('<IIHHIIII', base + index * 28)
                if raw + length > len(data) or length > 65536:
                    raise ValueError('Invalid PE debug record')
                payload = data[raw:raw + length]
                item = {'type': kind, 'timestamp_raw': stamp, 'bytes': length, 'sha256': sha(payload)}
                if kind == 2 and payload.startswith(b'RSDS') and len(payload) >= 25:
                    item['codeview_guid'] = str(uuid.UUID(bytes_le=payload[4:20]))
                    item['codeview_age'] = struct.unpack_from('<I', payload, 20)[0]
                    item['pdb_path'] = payload[24:].split(b'\0', 1)[0].decode('utf-8', errors='replace')
                debug.append(item)
    return {'coff_timestamp_raw': timestamp, 'sections': sections, 'debug_records': debug}


def wheel_members(path):
    result = {}
    with zipfile.ZipFile(path) as archive:
        if len(archive.infolist()) > 10000:
            raise ValueError('Wheel diagnostic member bound exceeded')
        for item in archive.infolist():
            if item.is_dir():
                continue
            if item.filename in result or item.file_size > 100_000_000:
                raise ValueError('Duplicate or oversized wheel diagnostic member')
            data = archive.read(item)
            record = {'bytes': len(data), 'sha256': sha(data), 'zip_timestamp': list(item.date_time),
                      'compression': item.compress_type, 'external_attributes': item.external_attr}
            if Path(item.filename).suffix.lower() in {'.dll', '.pyd', '.exe'}:
                record['pe'] = pe_details(data)
            result[item.filename] = record
    return result


def compare_wheels(first, second):
    left, right = wheel_members(first), wheel_members(second)
    return {'schema': 1, 'binary_bytes_modified': False,
            'first_sha256': sha(Path(first).read_bytes()), 'second_sha256': sha(Path(second).read_bytes()),
            'first_members': left, 'second_members': right,
            'different_members': [name for name in sorted(left.keys() | right.keys()) if left.get(name) != right.get(name)],
            'identical': Path(first).read_bytes() == Path(second).read_bytes()}
