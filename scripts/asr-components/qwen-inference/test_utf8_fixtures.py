"""Exercise the real catalog fixture imports with Windows' legacy text default."""
import importlib.util
import io
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

HERE = Path(__file__).resolve().parent
CATALOG_PATH = HERE.parents[2] / 'src-tauri/resources/asr/catalog.json'


def cp1252_default(encoding, *args):
    """Change only omitted/locale encodings; explicit UTF-8 must still be used."""
    return 'cp1252' if encoding in (None, 'locale') else encoding


class Utf8Fixtures(unittest.TestCase):
    def test_real_catalog_imports_preserve_unicode_and_pins_with_cp1252_default(self):
        original = CATALOG_PATH.read_bytes()
        expected = json.loads(original.decode('utf-8'))
        self.assertTrue(any(byte > 127 for byte in original))
        original_open = Path.open
        for newline in (b'\n', b'\r\n'):
            with self.subTest(newline=newline), tempfile.TemporaryDirectory(prefix='qwen-encoding-') as temporary:
                # Real UTF-8 catalog bytes, as either native checkout newline.
                fixture = Path(temporary).resolve() / 'catalog.json'
                contents = original.replace(b'\r\n', b'\n').replace(b'\n', newline)
                fixture.write_bytes(contents)
                def catalog_open(path, *args, **kwargs):
                    return original_open(fixture if path == CATALOG_PATH else path, *args, **kwargs)
                # Redirect only this fixture input to an owned real file; the
                # importer and the underlying cp1252 decoder remain genuine.
                with patch.object(Path, 'open', catalog_open), \
                     patch.object(io, 'text_encoding', side_effect=cp1252_default):
                    with self.assertRaises(UnicodeDecodeError):
                        CATALOG_PATH.read_text()
                    self.assertEqual(json.loads(CATALOG_PATH.read_text(encoding='utf-8')), expected)
                    for name in ('test_fixture', 'test_download_diagnostics'):
                        with self.subTest(module=name):
                            spec = importlib.util.spec_from_file_location('_qwen_cp1252_' + name, HERE / (name + '.py'))
                            module = importlib.util.module_from_spec(spec)
                            spec.loader.exec_module(module)
                            self.assertEqual(module.CATALOG, expected)
                            models = module.m.reviewed_models(module.CATALOG)
                            self.assertEqual({model['id']: model['version'] for model in models}, {
                                'qwen3-asr-0-6b': '5eb144179a02acc5e5ba31e748d22b0cf3e303b0',
                                'qwen3-forced-aligner-0-6b': 'c7cbfc2048c462b0d63a45797104fc9db3ad62b7',
                            })
                            for model in models:
                                readme = next(item for item in model['files'] if item['path'] == 'README.md')
                                self.assertEqual(readme['bytes'], 57456)
                                self.assertEqual(readme['sha256'], '5058416891bc47a2051557765997e8c42f8eb78a0e33c3e775bd17d4b0ba4d50')
                self.assertEqual(fixture.read_bytes(), contents)
        self.assertEqual(CATALOG_PATH.read_bytes(), original)


if __name__ == '__main__':
    unittest.main()
