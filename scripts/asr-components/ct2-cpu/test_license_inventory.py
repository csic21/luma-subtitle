from pathlib import Path
import sys
import unittest
import zipfile

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from fixture_paths import temporary_root

from license_inventory import discover, text_from_document

TERMS = 'MICROSOFT SOFTWARE LICENSE TERMS\nMicrosoft Visual Studio. Distributable Code conditions require review.'


class InstalledLicenseTests(unittest.TestCase):
    def test_only_actual_recognizable_vendor_terms_are_copied(self):
        with temporary_root() as work:
            root = Path(work); redist = root / 'VC/Redist'; redist.mkdir(parents=True)
            (redist / 'EULA.txt').write_text(TERMS)
            (redist / 'LICENSE-LINK.txt').write_text('https://example.invalid/terms')
            (redist / 'license-metadata.json').write_text('{"secret":"do not read"}')
            (redist / 'activation-license.txt').write_text(TERMS + '\nproduct key: should never enter report')
            report = discover(root, redist, 'Microsoft.VisualStudio.Product.Enterprise', 'pinned')
            docs = {Path(x['path']).name: x for x in report['documents']}
            self.assertEqual(set(docs), {'EULA.txt', 'LICENSE-LINK.txt'})
            self.assertEqual(docs['EULA.txt']['text'].splitlines(), TERMS.splitlines())
            self.assertTrue(docs['EULA.txt']['terms_recovered'])
            self.assertFalse(docs['LICENSE-LINK.txt']['terms_recovered'])
            self.assertTrue(docs['LICENSE-LINK.txt']['link_only'])
            self.assertFalse(report['redistribution_grant_verified'])
            self.assertFalse(report['downloads'])

    def test_rejects_a_redist_root_outside_installation(self):
        with temporary_root() as work:
            root = Path(work); (root / 'vs').mkdir(); (root / 'other').mkdir()
            with self.assertRaises(ValueError):
                discover(root / 'vs', root / 'other', 'product', 'version')

    def test_file_and_text_bounds(self):
        with temporary_root() as work:
            root = Path(work); redist = root / 'Redist'; redist.mkdir()
            for i in range(4):
                (redist / f'license-{i}.txt').write_text(TERMS)
            report = discover(root, redist, 'product', 'version', max_files=2, max_text_total=1)
            self.assertTrue(report['truncated'])
            self.assertTrue(all('text' not in x for x in report['documents']))

    def test_observed_visual_studio_redist_stub_is_pointer_only(self):
        stub = (b'Distributable Code for Microsoft Visual Studio 2022 (Includes Utilities & BuildServer Files)\r\n\r\n'
                b'For the latest version of this Redist file, please visit https://aka.ms/vs/17/redist.txt.\r\n')
        self.assertEqual(len(stub), 187)
        with temporary_root() as work:
            root = Path(work); redist = root / 'Redist'; redist.mkdir()
            path = root / 'Redist.txt'; path.write_bytes(stub)
            report = discover(root, redist, 'Microsoft.VisualStudio.Product.Enterprise', 'pinned')
            doc, = report['documents']
            self.assertTrue(doc['link_only'])
            self.assertFalse(doc['terms_recovered'])
            self.assertEqual(doc['pointer_text'], stub.decode())
            self.assertEqual(doc['sha256'], 'da53b097e02b08e0fc69706102a60bc384fe756426ae4dc4a855e96f95cb2b9c')
            self.assertNotIn('text', doc)
            self.assertFalse(report['redistribution_grant_verified'])

    def test_credential_labels_prevent_text_copying(self):
        with temporary_root() as work:
            root = Path(work); redist = root / 'Redist'; redist.mkdir()
            (redist / 'LICENSE.txt').write_text(TERMS + '\nProduct key: XXXXX-XXXXX-XXXXX-XXXXX-XXXXX')
            report = discover(root, redist, 'product', 'version')
            self.assertNotIn('text', report['documents'][0])

    def test_existing_docx_is_read_without_external_entities(self):
        with temporary_root() as work:
            path = Path(work) / 'EULA.docx'
            with zipfile.ZipFile(path, 'w') as archive:
                archive.writestr('word/document.xml', '<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:p><w:r><w:t>' + TERMS + '</w:t></w:r></w:p></w:document>')
            text, reason = text_from_document(path)
            self.assertIn('Distributable Code', text)
            self.assertIsNone(reason)
            with zipfile.ZipFile(path, 'w') as archive:
                archive.writestr('word/document.xml', '<!DOCTYPE x [<!ENTITY external SYSTEM "https://invalid.example">]><x/>')
            text, reason = text_from_document(path)
            self.assertIsNone(text)
            self.assertIn('not allowed', reason)


if __name__ == '__main__':
    unittest.main()
