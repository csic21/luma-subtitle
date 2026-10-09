import hashlib
import csv
import json
import os
from pathlib import Path
import unittest
from unittest.mock import patch
from fixture_paths import temporary_root

from assemble import local_file_uri, normalize_removed_records, offline_guard, remove_bootstrap_installer, wheel_requirements, PIP_VERSION, PYTHON_VERSION


class OfflineAssemblyTests(unittest.TestCase):
    def test_verified_wheel_becomes_hash_only_pinned_requirement(self):
        with temporary_root() as tmp:
            root = tmp; path = root / 'example-1.0-py3-none-any.whl'; path.write_bytes(b'example wheel')
            wheel = {'name': 'example', 'version': '1.0', 'filename': path.name,
                     'bytes': path.stat().st_size, 'sha256': hashlib.sha256(path.read_bytes()).hexdigest()}
            result = wheel_requirements({'wheels': [wheel]}, root)
            # macOS /var aliases and Windows short profile names canonicalize.
            self.assertEqual(result, 'example @ ' + path.resolve().as_uri() + ' --hash=sha256:' + wheel['sha256'] + '\n')
            self.assertIn('file:///', result); self.assertNotIn('http', result)
            # Require canonicalization before formatting, even on hosts whose
            # temporary path happens not to contain a short-name or symlink alias.
            with patch('assemble.local_file_uri', return_value='file:///canonical.whl') as uri:
                wheel_requirements({'wheels': [wheel]}, root)
                uri.assert_called_once_with(path.resolve())
            path.write_bytes(b'changed bytes')
            with self.assertRaisesRegex(ValueError, 'changed'):
                wheel_requirements({'wheels': [wheel]}, root)

    def test_unapproved_extra_wheels_and_option_injection_rejected(self):
        with temporary_root() as tmp:
            root = tmp; path = root / 'example-1.0-py3-none-any.whl'; path.write_bytes(b'wheel')
            wheel = {'name': 'example', 'version': '1.0', 'filename': path.name, 'bytes': 5,
                     'sha256': hashlib.sha256(b'wheel').hexdigest()}
            (root / 'surprise.whl').write_bytes(b'unapproved')
            with self.assertRaisesRegex(ValueError, 'exactly'):
                wheel_requirements({'wheels': [wheel]}, root)
            (root / 'surprise.whl').unlink()
            for key, value in [('name', '--extra-index-url'), ('name', 'x\n--index-url=bad'), ('version', '1.0\n--target=/bad'), ('filename', '../other.whl')]:
                with self.subTest(key=key, value=value), self.assertRaises(ValueError):
                    wheel_requirements({'wheels': [dict(wheel, **{key: value})]}, root)

    def test_guard_forbids_network_and_child_processes(self):
        for event in ('socket.connect', 'socket.getaddrinfo', 'socket.bind', 'subprocess.Popen', 'os.system', 'os.fork', 'os.posix_spawn', 'os.exec'):
            with self.subTest(event=event), self.assertRaises(RuntimeError): offline_guard(event, ())
        offline_guard('open', ('local-file', 'r', 0))

    def test_assembly_toolchain_is_explicit(self):
        self.assertEqual(PIP_VERSION, '26.2.1'); self.assertEqual(PYTHON_VERSION, '3.12.15')

    def test_bootstrap_vendor_provenance_is_removed_only_with_pinned_installer(self):
        with temporary_root() as tmp:
            site = tmp
            (site / 'pip').mkdir()
            bootstrap = site / f'pip-{PIP_VERSION}.dist-info'; bootstrap.mkdir()
            (bootstrap / 'direct_url.json').write_text('{"url":"file:///vendor-build/pip.whl"}')
            retained = site / 'engine-1.0.dist-info'; retained.mkdir()
            record = retained / 'direct_url.json'; record.write_text('{"url":"file:///verified-engine.whl"}')
            (site / 'pipeline').mkdir()
            remove_bootstrap_installer(site)
            self.assertFalse(bootstrap.exists()); self.assertFalse((site / 'pip').exists())
            self.assertEqual(list(site.glob('*.dist-info/direct_url.json')), [record])
            self.assertTrue((site / 'pipeline').is_dir())
            with self.assertRaisesRegex(RuntimeError, 'layout changed'):
                remove_bootstrap_installer(site)

    def test_extended_windows_local_drives_are_not_unc(self):
        self.assertEqual(local_file_uri(r'\\?\C:\private runtime\wheel.whl'), 'file:///C:/private%20runtime/wheel.whl')
        for path in (r'\\server\share\wheel.whl', r'\\?\UNC\server\share\wheel.whl', r'\\.\device'):
            with self.subTest(path=path), self.assertRaises(ValueError): local_file_uri(path)

    @unittest.skipUnless(os.name == 'nt', 'Native Windows filesystem regression')
    def test_native_extended_wheelhouse(self):
        with temporary_root() as tmp:
            root = Path('\\\\?\\' + str(tmp))
            path = root / 'example-1.0-py3-none-any.whl'; path.write_bytes(b'wheel')
            wheel = {'name': 'example', 'version': '1.0', 'filename': path.name,
                     'bytes': 5, 'sha256': hashlib.sha256(b'wheel').hexdigest()}
            result = wheel_requirements({'wheels': [wheel]}, root)
            self.assertIn('file:///', result); self.assertNotIn('%3F', result)

    def test_removed_generated_record_rows_do_not_retain_staging_hashes(self):
        records = []
        for staging_hash in ('first-stage-hash', 'second-stage-hash'):
            with temporary_root() as tmp:
                root = tmp; site = root / 'lib/python3.12/site-packages'
                metadata = site / 'example-1.0.dist-info'; metadata.mkdir(parents=True)
                record = metadata / 'RECORD'
                rows = [['../../../bin/example', staging_hash, '100'],
                        ['example-1.0.dist-info/direct_url.json', staging_hash, '200'],
                        ['example/module.py', 'sha256=retained-upstream-hash', '300'],
                        ['example-1.0.dist-info/RECORD', '', '']]
                with record.open('w', newline='', encoding='utf-8') as stream: csv.writer(stream).writerows(rows)
                removed = {root / 'bin/example', metadata / 'direct_url.json'}
                self.assertEqual(normalize_removed_records(site, root, removed), 2)
                records.append(record.read_bytes())
        self.assertEqual(*records)
        self.assertIn(b'retained-upstream-hash', records[0]); self.assertNotIn(b'stage-hash', records[0])


if __name__ == '__main__': unittest.main()
