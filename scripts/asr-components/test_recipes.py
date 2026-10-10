import hashlib
import json
from pathlib import Path
import unittest
from unittest.mock import patch
from fixture_paths import temporary_root

from generate_recipes import ROOT, generate, inventory_text, verified_term


class RecipeCandidateTests(unittest.TestCase):
    def test_original_cuda_containing_wheel_cannot_become_cpu_recipe(self):
        original = {'id': 'faster-whisper-cpu-windows-x64', 'archive': {'url': 'unused'}, 'recipe': {'unsafe': True}, 'plan_sha256': 'stale'}
        with temporary_root() as root:
            (root / 'locks').mkdir()
            (root / 'licenses.lock.json').write_text(json.dumps({'files': []}))
            (root / 'locks/faster-whisper-cpu-windows-x64.json').write_text(json.dumps({'wheels': []}))
            with patch('generate_recipes.ROOT', root):
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
        ids = [term['id'] for term in lock['terms']]
        self.assertEqual(len(ids), len(set(ids)))
        self.assertTrue({'mlx-engine-license', 'qwen-engine-license', 'faster-whisper-engine-license'}.issubset(ids))
        self.assertEqual(len([term for term in lock['terms'] if term['id'].startswith('luma-cpu-')]), 11)
        for source in lock['terms']:
            term = verified_term(source, ROOT)
            self.assertEqual(len(term['text'].encode('utf-8')), source['text_bytes'])
            self.assertEqual(term['sha256'], source['raw_sha256'])
            if 'source_commit' in source:
                self.assertIn('/' + source['source_commit'] + '/', source['url'])

    def test_activated_catalog_has_exact_recipes_and_experimental_labels(self):
        catalog = json.loads((ROOT.parent.parent / 'src-tauri/resources/asr/catalog.json').read_text(encoding='utf-8'))
        self.assertEqual(len(catalog['runtimes']), 4)
        self.assertEqual(len(catalog['models']), 7)
        # The existing seven model definitions are intentionally unchanged by runtime activation.
        model_bytes = json.dumps(catalog['models'], ensure_ascii=False, sort_keys=True, separators=(',', ':')).encode('utf-8')
        self.assertEqual(hashlib.sha256(model_bytes).hexdigest(), 'deb258db1804a2b931f3f255e7d85fc88c0091e9aa889994a257373c4d4a76c9')
        caps = json.loads((ROOT / 'recipe-caps.json').read_text(encoding='utf-8'))
        for runtime in catalog['runtimes']:
            self.assertNotIn('unavailable_reason', runtime)
            self.assertIn('experimental', runtime['label'])
            self.assertNotIn('archive', runtime)
            self.assertEqual(runtime['installed_bytes'], caps[runtime['id']]['installed_bytes'])
            self.assertEqual(runtime['max_files'], caps[runtime['id']]['max_files'])
            lock = json.loads((ROOT / 'locks' / (runtime['id'] + '.json')).read_text(encoding='utf-8'))
            self.assertEqual(len(runtime['recipe']['wheels']), len(lock['wheels']))
            for wheel, pin in zip(runtime['recipe']['wheels'], lock['wheels']):
                for key in ('name', 'version', 'filename', 'url', 'bytes', 'sha256'):
                    self.assertEqual(wheel[key], pin[key])
            terms = runtime['recipe']['terms']
            for term in terms:
                self.assertEqual(hashlib.sha256(term['text'].encode('utf-8')).hexdigest(), term['sha256'])
            if runtime['backend'] == 'faster-whisper':
                self.assertFalse(any(term['id'].startswith('intel-') for term in terms))
                self.assertEqual(len([term for term in terms if term['id'].startswith('luma-cpu-')]), 11)
                self.assertTrue(any(term['id'] == 'faster-whisper-engine-license' for term in terms))
                self.assertEqual(runtime['recipe']['windows_crt'], 'msvc-14.44.35211-x64')

    def test_cpu_notice_mapping_excludes_only_obsolete_vendor_intel_notices(self):
        supplemental = json.loads((ROOT / 'supplemental-notices.lock.json').read_text(encoding='utf-8'))
        self.assertFalse(any('intel-openmp' in item['path'] and 'faster-whisper-cpu-windows-x64' in item['packs'] for item in supplemental['files']))
        proprietary = json.loads((ROOT.parent.parent / 'src-tauri/resources/asr/terms/sources.json').read_text(encoding='utf-8'))
        self.assertTrue(any(item['id'].startswith('intel-') and 'qwen3-asr-cpu-windows-x64' in item['applies_to'] for item in proprietary['terms']))


if __name__ == '__main__': unittest.main()
