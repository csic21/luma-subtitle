import hashlib
import json
from pathlib import Path
import unittest
from fixture_paths import temporary_root

from generate_recipes import ROOT, generate, inventory_text, verified_term


class RecipeCandidateTests(unittest.TestCase):
    def test_original_cuda_containing_wheel_cannot_become_cpu_recipe(self):
        original = {'id': 'faster-whisper-cpu-windows-x64', 'archive': {'url': 'unused'}, 'recipe': {'unsafe': True}, 'plan_sha256': 'stale'}
        result = generate({'runtimes': [original]}, {}, {}, {'terms': []}, ROOT)
        candidate = result['runtimes'][0]
        self.assertNotIn('recipe', candidate); self.assertNotIn('archive', candidate)
        self.assertNotIn('plan_sha256', candidate)
        self.assertIn('CPU-only', candidate['unavailable_reason'])
        self.assertEqual(result['status'], 'review-only-not-active')

    def test_embedded_display_bytes_are_exact_and_paths_are_confined(self):
        with temporary_root() as tmp:
            directory = tmp; data = 'Exact upstream text.\n'.encode()
            (directory / 'license.txt').write_bytes(data)
            source = {'id': 'license', 'version': '1', 'url': 'https://example.org/license',
                      'path': 'license.txt', 'sha256': hashlib.sha256(data).hexdigest()}
            term = verified_term(source, directory)
            self.assertEqual(term['text'].encode(), data); self.assertNotIn('path', term)
            with self.assertRaisesRegex(ValueError, 'hash mismatch'):
                verified_term(dict(source, sha256='0' * 64), directory)
            with self.assertRaisesRegex(ValueError, 'escaped'):
                verified_term(dict(source, path='../license.txt'), directory)

    def test_inventory_states_provenance_boundary_without_new_legal_terms(self):
        wheel = {'name': 'engine', 'version': '1', 'filename': 'engine.whl', 'url': 'https://example.org/engine.whl', 'sha256': 'a' * 64, 'license': None}
        text = inventory_text({'label': 'Engine'}, {'wheels': [wheel]})
        self.assertIn('not declared', text); self.assertIn('does not replace upstream license texts', text)
        self.assertIn(wheel['sha256'], text)

    def test_engine_terms_preserve_pinned_display_and_raw_hashes(self):
        lock = json.loads((ROOT / 'engine-terms.lock.json').read_text(encoding='utf-8'))
        self.assertEqual(len(lock['terms']), 2)
        for source in lock['terms']:
            term = verified_term(source, ROOT)
            self.assertEqual(len(term['text'].encode('utf-8')), source['text_bytes'])
            self.assertEqual(term['sha256'], source['raw_sha256'])
            self.assertIn('/' + source['source_commit'] + '/', source['url'])


if __name__ == '__main__': unittest.main()
