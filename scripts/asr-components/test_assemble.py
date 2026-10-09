import hashlib
import csv
import json
import os
from pathlib import Path
from types import SimpleNamespace
import unittest
from unittest.mock import patch
from fixture_paths import temporary_root

from assemble import local_file_uri, local_windows_disk_path, normalize_removed_records, offline_guard, remove_bootstrap_installer, require_private_interpreter, within_private_root, wheel_requirements, PIP_VERSION, PYTHON_VERSION


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

    def test_private_interpreter_requires_every_isolation_flag_and_file_identity(self):
        with temporary_root(prefix='Assembly identity é 测试 ') as tmp:
            root = tmp / 'private'; root.mkdir(); executable = root / 'python.exe'; executable.write_bytes(b'fixture')
            external = tmp / 'private-other'; external.mkdir(); outside = external / 'python.exe'; outside.write_bytes(b'fixture')
            flags = SimpleNamespace(isolated=1, no_site=1, no_user_site=1)
            with patch('assemble.sys.flags', flags), patch('assemble.sys.prefix', str(root)), patch('assemble.sys.executable', str(executable)), patch('assemble.site.ENABLE_USER_SITE', None):
                require_private_interpreter(root)
                for field in ('isolated', 'no_site', 'no_user_site'):
                    with patch.object(flags, field, 0), self.assertRaisesRegex(RuntimeError, '"' + field + '": false'):
                        require_private_interpreter(root)
                with patch('assemble.site.ENABLE_USER_SITE', True), self.assertRaisesRegex(RuntimeError, '"user_site_disabled": false'):
                    require_private_interpreter(root)
                with patch('assemble.sys.prefix', str(external)), self.assertRaisesRegex(RuntimeError, '"prefix_identity": false'):
                    require_private_interpreter(root)
                with patch('assemble.sys.executable', str(outside)), self.assertRaisesRegex(RuntimeError, '"executable_contained": false'):
                    require_private_interpreter(root)
                with self.assertRaisesRegex(RuntimeError, '"prefix_identity": false'):
                    require_private_interpreter(external)
                with patch('assemble.sys.executable', str(root / 'missing.exe')), self.assertRaisesRegex(RuntimeError, '"identity_readable": false'):
                    require_private_interpreter(root)
                with patch('assemble.sys.prefix', str(tmp / ('é' * 180))):
                    try: require_private_interpreter(root)
                    except RuntimeError as error:
                        diagnostic = str(error); self.assertLess(len(diagnostic), 4096)
                        self.assertIn('"truncated": true', diagnostic); self.assertNotIn('environ', diagnostic)
                    else: self.fail('Missing prefix was accepted')

    def test_resolved_containment_rejects_sibling_and_outside_link(self):
        with temporary_root(prefix='Assembly containment é 测试 ') as tmp:
            root = tmp / 'runtime'; root.mkdir(); inside = root / 'inside'; inside.mkdir()
            outside = tmp / 'runtime-other'; outside.mkdir(); (outside / 'python.exe').write_bytes(b'fixture')
            self.assertTrue(within_private_root(inside, root)); self.assertFalse(within_private_root(outside, root))
            self.assertTrue(within_private_root(root / 'new' / 'headers', root, allow_missing=True))
            with self.assertRaises(FileNotFoundError): within_private_root(root / 'missing.exe', root)
            with patch.object(Path, 'resolve', side_effect=PermissionError('denied')):
                with self.assertRaises(PermissionError): within_private_root(inside, root)
            link = root / 'outside-link'
            try: link.symlink_to(outside, target_is_directory=True)
            except OSError as error:
                if os.name == 'nt': self.skipTest('Native symlink creation unavailable: ' + type(error).__name__)
                raise
            self.assertFalse(within_private_root(link / 'python.exe', root))

    def test_windows_identity_fallback_rejects_remote_device_and_relative_namespaces(self):
        for path in (r'C:\private é 测试', r'\\?\C:\private é 测试'):
            self.assertTrue(local_windows_disk_path(path))
        for path in (r'\\server\share\runtime', r'\\?\UNC\server\share\runtime', r'\\.\C:\runtime', r'\\?\Volume{fixture}\runtime', r'C:runtime', 'relative'):
            self.assertFalse(local_windows_disk_path(path))
            with patch('assemble.sys.platform', 'win32'), patch.object(Path, 'resolve') as resolve:
                self.assertFalse(within_private_root(path, r'C:\private'))
                resolve.assert_not_called()

    @unittest.skipUnless(os.name == 'nt', 'Native Windows filesystem identity regression')
    def test_native_unicode_ordinary_and_verbatim_interpreter_identity(self):
        with temporary_root(prefix='Assembly native é 测试 ') as tmp:
            root = tmp / 'runtime'; root.mkdir(); executable = root / 'python.exe'; executable.write_bytes(b'fixture')
            ordinary = root.resolve(strict=True); verbatim = Path('\\\\?\\' + str(ordinary))
            # Establish the actual old-guard failure: resolve() retains the
            # caller's namespace spelling even for the exact same directory.
            self.assertNotEqual(ordinary.resolve(strict=True), verbatim.resolve(strict=True))
            self.assertTrue(ordinary.samefile(verbatim))
            flags = SimpleNamespace(isolated=1, no_site=1, no_user_site=1)
            for requested in (ordinary, verbatim):
                for prefix in (ordinary, verbatim):
                    for executable_root in (ordinary, verbatim):
                        with self.subTest(requested=str(requested), prefix=str(prefix), executable_root=str(executable_root)), \
                             patch('assemble.sys.flags', flags), patch('assemble.site.ENABLE_USER_SITE', None), \
                             patch('assemble.sys.prefix', str(prefix)), patch('assemble.sys.executable', str(executable_root / 'python.exe')):
                            require_private_interpreter(requested)
                            self.assertTrue(within_private_root(executable_root / 'python.exe', requested))
                            self.assertTrue(within_private_root(executable_root / 'missing' / 'headers', requested, allow_missing=True))
            sibling = tmp / 'runtime-other'; sibling.mkdir(); (sibling / 'python.exe').write_bytes(b'outside')
            self.assertFalse(within_private_root(sibling / 'python.exe', verbatim))
            different_drive = 'Z' if ordinary.drive.upper() != 'Z:' else 'Y'
            self.assertFalse(within_private_root(f'{different_drive}:\\not-the-private-root\\python.exe', verbatim, allow_missing=True))

    @unittest.skipUnless(os.name == 'nt', 'Native Windows pip-style path parsing regression')
    def test_native_pip_mixed_separator_script_path_requires_ordinary_namespace(self):
        with temporary_root(prefix='Pip path é 测试 ') as tmp:
            root = tmp / 'runtime'; (root / 'Lib' / 'site-packages').mkdir(parents=True); (root / 'Scripts').mkdir()
            ordinary = str(root.resolve(strict=True))
            if ordinary.startswith('\\\\?\\'): ordinary = ordinary[4:]
            verbatim = '\\\\?\\' + ordinary
            self.assertTrue(Path(ordinary).samefile(verbatim))
            mixed = r'\Lib\site-packages\../../Scripts/numba'
            self.assertLess(len((ordinary + mixed).encode('utf-16-le')) // 2, 260)
            # Pass the actual raw joined string to open(). pathlib construction
            # would normalize separators and would no longer reproduce pip.
            with self.assertRaises(OSError):
                with open(verbatim + mixed, 'wb') as stream: stream.write(b'not reached')
            self.assertFalse((root / 'Scripts' / 'numba').exists())
            with open(ordinary + mixed, 'wb') as stream: stream.write(b'private launcher fixture')
            self.assertEqual((root / 'Scripts' / 'numba').read_bytes(), b'private launcher fixture')
            self.assertTrue(within_private_root(root / 'Scripts' / 'numba', Path(verbatim)))

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
