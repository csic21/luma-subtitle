import hashlib
import json
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import crt_proof as crt


class PrivateCrtTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.work = Path(temporary.name)
        self.runtime = self.work / 'private'; self.runtime.mkdir()
        self.reports = self.work / 'reports'; self.reports.mkdir()
        self.installation = self.work / 'Visual Studio'
        self.redist = self.installation / 'VC/Redist/MSVC/14.44.35112'
        folder = self.redist / 'x64/Microsoft.VC143.CRT'; folder.mkdir(parents=True)
        candidates = []
        for mapping in (crt.EXISTING, crt.MISSING):
            changed = {}
            for name, (_, _, thumbprint) in mapping.items():
                data = ('unit fixture ' + name).encode()
                changed[name] = (len(data), hashlib.sha256(data).hexdigest(), thumbprint)
                source = folder / name; source.write_bytes(data)
                if name in crt.EXISTING:
                    (self.runtime / name).write_bytes(data)
                candidates.append({'filename': name, 'source_path': str(source), 'bytes': len(data),
                                   'sha256': changed[name][1], 'file_version': crt.VERSION,
                                   'product_version': crt.VERSION, 'signature_status': 'Valid',
                                   'signer_subject': crt.SIGNER, 'signer_thumbprint': thumbprint})
            self.enterContext(patch.dict(mapping, changed, clear=True))
        self.enterContext(patch.dict(os.environ, {'GITHUB_ACTIONS': 'true'}))
        self.write('toolchain.json', {'github_actions': True, 'runner_environment': 'github-hosted',
                                     'product_id': 'Microsoft.VisualStudio.Product.Enterprise',
                                     'visual_studio_version': '17.14.37710.0', 'vc_tools_version': '14.44.35207',
                                     'installation_path': str(self.installation), 'redist_path': str(self.redist)})
        self.inventory = {'binary_publication_authorized': False, 'official_redist_directory': str(self.redist),
                          'candidates': candidates, 'official_sources': ['https://visualstudio.microsoft.com/license-terms/vs2022-cruntime/']}
        self.write('crt-candidates.json', self.inventory)
        pointer = ('Distributable Code for Microsoft Visual Studio 2022 (Includes Utilities & BuildServer Files)\r\n\r\n'
                   'For the latest version of this Redist file, please visit https://aka.ms/vs/17/redist.txt.\r\n')
        self.write('installed-license-evidence.json', {'documents': [{'sha256': crt.POINTER_SHA256,
                   'relative_path': 'Licenses/1033/Redist.txt', 'link_only': True,
                   'terms_recovered': False, 'pointer_text': pointer}]})

    def write(self, name, data):
        (self.reports / name).write_text(json.dumps(data), encoding='utf-8')

    def test_only_two_original_files_copied_and_existing_notices_preserved(self):
        notices = self.runtime / 'licenses'; notices.mkdir()
        (notices / 'PBS.txt').write_text('unchanged')
        before = {name: (self.runtime / name).read_bytes() for name in crt.EXISTING}
        result = crt.copy_proof_crt(self.runtime, self.reports)
        self.assertEqual(result['copied_names'], ['msvcp140.dll', 'msvcp140_1.dll'])
        self.assertFalse(result['public_redistribution_authorized'])
        self.assertFalse(result['redistribution_grant_verified'])
        for name, original in before.items():
            self.assertEqual((self.runtime / name).read_bytes(), original)
        self.assertEqual((notices / 'PBS.txt').read_text(), 'unchanged')
        self.assertEqual(hashlib.sha256((notices / 'MICROSOFT-CRT-REDIST-POINTER.txt').read_bytes()).hexdigest(), crt.POINTER_SHA256)

    def test_invalid_signature_blocks_all_copies(self):
        self.inventory['candidates'][-1]['signature_status'] = 'NotSigned'
        self.write('crt-candidates.json', self.inventory)
        with self.assertRaises(ValueError):
            crt.copy_proof_crt(self.runtime, self.reports)
        self.assertFalse((self.runtime / 'msvcp140.dll').exists())

    def test_original_source_byte_change_is_rejected(self):
        Path(self.inventory['candidates'][-1]['source_path']).write_bytes(b'changed')
        with self.assertRaises(ValueError):
            crt.copy_proof_crt(self.runtime, self.reports)

    def test_existing_pbs_crt_must_match_original(self):
        (self.runtime / 'vcruntime140.dll').write_bytes(b'other version')
        with self.assertRaises(ValueError):
            crt.copy_proof_crt(self.runtime, self.reports)

    def test_preexisting_msvcp_destination_is_never_replaced(self):
        target = self.runtime / 'msvcp140.dll'; target.write_bytes(b'existing')
        with self.assertRaises(ValueError):
            crt.copy_proof_crt(self.runtime, self.reports)
        self.assertEqual(target.read_bytes(), b'existing')

    def test_wrong_scope_or_original_path_is_rejected(self):
        with patch.dict(os.environ, {'GITHUB_ACTIONS': 'false'}), self.assertRaises(ValueError):
            crt.copy_proof_crt(self.runtime, self.reports)
        self.inventory['candidates'][-1]['source_path'] = str(self.work / 'substitute.dll')
        self.write('crt-candidates.json', self.inventory)
        with self.assertRaises(ValueError):
            crt.copy_proof_crt(self.runtime, self.reports)
