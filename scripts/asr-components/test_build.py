import importlib.util
import io
import json
from pathlib import Path
import stat
import tarfile
import tempfile
import unittest
import zipfile

SPEC = importlib.util.spec_from_file_location('component_build', Path(__file__).with_name('build.py'))
b = importlib.util.module_from_spec(SPEC); SPEC.loader.exec_module(b)


class ComponentBuildTests(unittest.TestCase):
    def test_rejects_unsafe_archive_names(self):
        for value in ('../x', '/absolute', 'C:/x', 'a\\b', 'a/../b', 'a//b', '.', 'a/./b', 'x\x00'):
            with self.subTest(value=value), self.assertRaises(ValueError): b.safe_name(value)
        self.assertEqual(str(b.safe_name('lib/python3.12/a.py')), 'lib/python3.12/a.py')

    def test_deterministic_zip_regular_files(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp) / 'root'; root.mkdir(); p = root / 'python'; p.write_bytes(b'bytes'); p.chmod(0o755)
            a, c = Path(tmp) / 'a.zip', Path(tmp) / 'b.zip'
            b.archive_tree(root, a); p.touch(); b.archive_tree(root, c)
            self.assertEqual(a.read_bytes(), c.read_bytes())
            with zipfile.ZipFile(a) as z:
                i = z.infolist()[0]
                self.assertEqual(i.date_time, (2026, 1, 1, 0, 0, 0)); self.assertTrue(stat.S_ISREG(i.external_attr >> 16))
                self.assertEqual((i.external_attr >> 16) & 0o777, 0o755)

    def test_runtime_link_dereferenced(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp); arc = root / 'p.tar.gz'
            with tarfile.open(arc, 'w:gz') as tar:
                i = tarfile.TarInfo('python/bin/python3.12'); i.size = 4; i.mode = 0o755; tar.addfile(i, io.BytesIO(b'test'))
                link = tarfile.TarInfo('python/bin/python3'); link.type = tarfile.SYMTYPE; link.linkname = 'python3.12'; tar.addfile(link)
            b.unpack_runtime(arc, root / 'out')
            p = root / 'out/bin/python3'; self.assertFalse(p.is_symlink()); self.assertEqual(p.read_bytes(), b'test')

    def test_runtime_link_escape_rejected(self):
        with tempfile.TemporaryDirectory() as tmp:
            arc = Path(tmp) / 'p.tar.gz'
            with tarfile.open(arc, 'w:gz') as tar:
                link = tarfile.TarInfo('python/x'); link.type = tarfile.SYMTYPE; link.linkname = '../../escape'; tar.addfile(link)
            with self.assertRaises(ValueError): b.unpack_runtime(arc, Path(tmp) / 'out')

    def test_wheel_skips_scripts_and_preserves_licenses(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp); arc = root / 'w.whl'; site = root / 'out/Lib/site-packages'
            with zipfile.ZipFile(arc, 'w') as z:
                z.writestr('thing/__init__.py', 'x = 1\n'); z.writestr('thing-1.dist-info/licenses/LICENSE', 'test license')
                z.writestr('thing-1.data/scripts/launcher', '#!CI-path\n'); z.writestr('thing-1.data/data/share/a', 'a')
            b.install_wheel(arc, root / 'out', site)
            self.assertTrue((site / 'thing-1.dist-info/licenses/LICENSE').is_file())
            self.assertFalse(list((root / 'out').rglob('launcher'))); self.assertEqual((root / 'out/share/a').read_text(), 'a')

    def test_runtime_notice_locks(self):
        lock = json.loads((b.ROOT / 'licenses.lock.json').read_text())
        for entry in lock['files']:
            path = b.ROOT / entry['path']
            self.assertEqual(path.stat().st_size, entry['bytes']); self.assertEqual(b.sha256(path), entry['sha256'])

    def test_pack_ids_and_sources_are_fixed(self):
        config = json.loads((b.ROOT / 'packs.json').read_text()); self.assertEqual(len(config['packs']), 4)
        self.assertEqual(config['release_tag'], 'asr-components-' + config['version'])
        for p in config['packs']:
            self.assertEqual(p['min_os_version'], '14.0' if p['platform']=='macos-arm64' else '10.0')
            self.assertEqual(config['runtime'][p['platform']]['entrypoint'], 'bin/python3' if p['platform']=='macos-arm64' else 'python.exe')



class CompliancePackagingTests(unittest.TestCase):
    @staticmethod
    def pe_image(import_name=b'libiomp5md.dll', delay=False):
        import struct
        data = bytearray(4096); data[:2] = b'MZ'; struct.pack_into('<I', data, 0x3c, 0x80)
        data[0x80:0x84] = b'PE\0\0'; struct.pack_into('<H', data, 0x86, 1); struct.pack_into('<H', data, 0x94, 240)
        optional = 0x98; struct.pack_into('<H', data, optional, 0x20b)
        struct.pack_into('<II', data, optional + 112 + 8, 0x1000, 40)
        if delay: struct.pack_into('<II', data, optional + 112 + 13 * 8, 0x1100, 32)
        struct.pack_into('<IIII', data, optional + 240 + 8, 0x1000, 0x1000, 0x400, 0x400)
        struct.pack_into('<IIIII', data, 0x400, 0, 0, 0, 0x1080, 0)
        data[0x480:0x480 + len(import_name)] = import_name
        return bytes(data)

    def test_pe_import_parser_rejects_unreviewed_delay_loading(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / 'code.dll'; path.write_bytes(self.pe_image())
            self.assertEqual(b.pe_imports(path), ['libiomp5md.dll'])
            path.write_bytes(self.pe_image(delay=True))
            with self.assertRaisesRegex(ValueError, 'delayed'): b.pe_imports(path)

    def test_cpu_pruning_requires_exact_reviewed_bytes_and_no_cuda_import(self):
        from unittest.mock import patch
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp); payload = root / 'payload'; payload.mkdir()
            library = payload / 'code.dll'; library.write_bytes(self.pe_image())
            omitted = payload / 'cudnn64_9.dll'; omitted.write_bytes(b'unused')
            policy = {'verify_pe_imports': 'code.dll', 'native_library_sha256': b.sha256(library),
                      'files': [{'path': omitted.name, 'bytes': 6, 'sha256': b.sha256(omitted)}]}
            (root / 'pruning.json').write_text(json.dumps({'packs': {'test': policy}}))
            with patch.object(b, 'ROOT', root): b.prune_reviewed_files(payload, 'test')
            self.assertFalse(omitted.exists()); self.assertTrue(library.exists())
            self.assertTrue((payload / 'INSTALLATION_CHANGES.json').is_file())
            omitted.write_bytes(b'changed')
            with patch.object(b, 'ROOT', root), self.assertRaisesRegex(ValueError, 'differs'):
                b.prune_reviewed_files(payload, 'test')
            self.assertTrue(omitted.exists())
            library.write_bytes(self.pe_image(b'cudnn64_9.dll')); policy['native_library_sha256'] = b.sha256(library)
            (root / 'pruning.json').write_text(json.dumps({'packs': {'test': policy}}))
            with patch.object(b, 'ROOT', root), self.assertRaisesRegex(ValueError, 'CUDA'):
                b.prune_reviewed_files(payload, 'test')

    def test_supplemental_notices_preserve_locked_bytes(self):
        lock = json.loads((b.ROOT / 'supplemental-notices.lock.json').read_text())
        for entry in lock['files']:
            path = b.ROOT / entry['path']
            self.assertEqual(path.stat().st_size, entry['bytes']); self.assertEqual(b.sha256(path), entry['sha256'])
            b.safe_name(entry['destination']); self.assertTrue(entry['packs'])


if __name__ == '__main__': unittest.main()
