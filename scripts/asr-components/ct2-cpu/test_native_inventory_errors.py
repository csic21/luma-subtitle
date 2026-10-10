"""Rejected PE input diagnostics remain bounded and do not change acceptance."""
import hashlib
from pathlib import Path
import sys
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from fixture_paths import temporary_root
import native_inventory as native
from test_cpu_build import pe


class NativeInventoryErrorTests(unittest.TestCase):
    def test_official_torchaudio_pyd_dependency_requires_private_target(self):
        # Upstream v2.9.1, commit a224ab24a7f4797f6707051257265e223e12576f:
        # cmake/TorchAudioHelper.cmake uses .pyd for MSVC shared libraries;
        # src/libtorchaudio/CMakeLists.txt links _torchaudio to libtorchaudio.
        with temporary_root() as root:
            folder = root / 'Lib/site-packages/torchaudio/lib'; folder.mkdir(parents=True)
            (folder / '_torchaudio.pyd').write_bytes(pe(normal='libtorchaudio.pyd', delayed='kernel32.dll'))
            target = folder / 'libtorchaudio.pyd'
            target.write_bytes(pe(normal='kernel32.dll', delayed='kernel32.dll'))
            report = native.closure(native.inventory(root))
            self.assertTrue(report['passed'])
            dependency, = [x for x in report['dependencies'] if x['name'] == 'libtorchaudio.pyd']
            self.assertEqual(dependency['resolution'], 'private')
            self.assertEqual(dependency['private_candidates'], ['Lib/site-packages/torchaudio/lib/libtorchaudio.pyd'])
            target.unlink()
            missing = native.closure(native.inventory(root))
            self.assertFalse(missing['passed'])
            self.assertEqual({x['resolution'] for x in missing['blocked_dependencies']}, {'unresolved'})

    def test_delayed_pyd_is_parsed_but_paths_and_controls_stay_rejected(self):
        with temporary_root() as root:
            path = root / 'test.pyd'; path.write_bytes(pe(delayed='private_module.pyd'))
            self.assertEqual(native.pe_imports(path)['delay'], ['private_module.pyd'])
        for name in ('../evil.pyd', 'directory\\evil.pyd', 'drive:evil.pyd', 'line\nfeed.pyd'):
            with self.subTest(name=name):
                self.assertIn('Unsafe PE import name', self.rejection(pe(normal=name)))

    def test_api_set_prefix_does_not_exempt_missing_private_pyd(self):
        for prefix in ('api-ms-win-', 'ext-ms-win-'):
            name = prefix + 'private-module.pyd'
            with self.subTest(name=name):
                importer = dict(path='package/importer.pyd', normal=[name], delay=[])
                missing = native.closure([importer])
                self.assertFalse(missing['passed'])
                self.assertEqual(missing['blocked_dependencies'][0]['resolution'], 'unresolved')
                private = native.closure([importer, dict(path='package/' + name, normal=[], delay=[])])
                self.assertTrue(private['passed'])
                self.assertEqual(private['dependencies'][0]['resolution'], 'private')
                self.assertEqual(native.classify(prefix + 'contract.dll', {}), 'windows_api_set')
                self.assertEqual(native.classify(prefix + 'contract.drv', {}), 'unresolved')

    def rejection(self, data):
        with temporary_root() as root:
            path = root / 'Lib/problem.pyd'; path.parent.mkdir()
            path.write_bytes(data)
            with self.assertRaises(ValueError) as caught:
                native.inventory(root)
            message = str(caught.exception)
            self.assertNotIn(str(root), message)
            self.assertIn("'Lib/problem.pyd'", message)
            self.assertIn('sha256=' + hashlib.sha256(data).hexdigest(), message)
            self.assertLess(len(message), 900)
            self.assertTrue(message.isascii())
            return message

    def test_rejected_path_retains_relative_file_raw_name_kind_and_rva(self):
        message = self.rejection(pe(normal='../evil.dll'))
        self.assertIn('normal import: Unsafe PE import name at RVA 0x1100', message)
        self.assertIn("b'../evil.dll'", message)

    def test_delay_extensionless_name_is_still_rejected(self):
        message = self.rejection(pe(delayed='Extensionless'))
        self.assertIn('delay import: Unsafe PE import name at RVA 0x1180', message)
        self.assertIn("b'Extensionless'", message)

    def test_raw_non_ascii_and_control_bytes_are_escaped(self):
        data = pe(); data[0x500:0x507] = b'\xffx.dll\0'
        self.assertIn("b'\\xffx.dll'", self.rejection(data))
        message = self.rejection(pe(normal='line\nfeed.dll'))
        self.assertIn(r'line\nfeed.dll', message)
        self.assertNotIn('\n', message)

    def test_long_rejected_name_is_truncated_without_accepting_it(self):
        message = self.rejection(pe(normal='A' * 110 + '/bad.dll'))
        self.assertIn('(truncated)', message)
        self.assertNotIn('A' * 97, message)


if __name__ == '__main__':
    unittest.main()
