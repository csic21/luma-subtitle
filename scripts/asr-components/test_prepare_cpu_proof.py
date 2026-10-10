"""Local-only CI metadata preparation tests; synthetic pins never ship."""
import copy
import io
import json
from pathlib import Path
import shutil
import unittest
from unittest.mock import patch

import build
import prepare_cpu_proof as metadata
from fixture_paths import temporary_root
from test_own_cpu_recipe import SyntheticPublishedCpu, encoded


class CpuMetadataPreparationTests(unittest.TestCase):
    def setUp(self):
        self.directory = self.enterContext(temporary_root())
        self.fixture = SyntheticPublishedCpu(self.directory)
        self.enterContext(patch.object(metadata, 'ROOT', self.fixture.root))
        self.cache = self.directory / 'metadata-cache'

    def test_explicit_local_proof_is_validated_and_cached_without_downloading(self):
        with patch.object(metadata, 'fetch', side_effect=AssertionError('offline path must not download')):
            result = metadata.prepare(self.cache, self.fixture.path)
            self.assertEqual(result, self.cache / self.fixture.lock['cpu_component']['proof']['sha256'])
            self.assertEqual(result.read_bytes(), self.fixture.path.read_bytes())
            self.assertEqual(metadata.prepare(self.cache, self.fixture.path), result)
            self.assertEqual(list(self.cache.iterdir()), [result])

    def test_only_exact_lock_pinned_metadata_uses_existing_downloader(self):
        pin = self.fixture.lock['cpu_component']['proof']
        def fetch(item, cache):
            self.assertEqual(item, pin)
            target = cache / item['sha256']
            shutil.copyfile(self.fixture.path, target)
            return target
        with patch.object(metadata, 'fetch', side_effect=fetch) as download:
            result = metadata.prepare(self.cache)
        download.assert_called_once_with(pin, self.cache)
        self.assertEqual(result.read_bytes(), self.fixture.path.read_bytes())

    def test_existing_verified_hash_cache_is_fully_offline(self):
        with patch.object(build.urllib.request, 'urlopen', side_effect=AssertionError('cache hit must not download')):
            self.assertEqual(metadata.prepare(self.fixture.cache), self.fixture.path)

    def test_invalid_explicit_bytes_fail_before_cache_creation_without_fallback(self):
        self.fixture.path.write_bytes(b'not the pinned proof')
        with patch.object(metadata, 'fetch', side_effect=AssertionError('no fallback download')):
            with self.assertRaises(ValueError):
                metadata.prepare(self.cache, self.fixture.path)
        self.assertFalse(self.cache.exists())

    def test_existing_release_validator_must_accept_local_and_downloaded_proof(self):
        self.fixture.proof['checks']['native_inference'] = False
        self.fixture.save_proof()
        with self.assertRaisesRegex(ValueError, 'publication validator'):
            metadata.prepare(self.cache, self.fixture.path)
        self.assertFalse(self.cache.exists())
        def fetch(item, cache):
            target = cache / item['sha256']; shutil.copyfile(self.fixture.path, target); return target
        with patch.object(metadata, 'fetch', side_effect=fetch):
            with self.assertRaisesRegex(ValueError, 'publication validator'):
                metadata.prepare(self.cache)

    def test_bad_or_unbounded_shipping_pins_fail_before_fetch(self):
        path = self.fixture.root / 'locks' / (metadata.PACK_ID + '.json')
        for field, value in [('url', 'https://github.com/other/publication-proof.json'),
                             ('bytes', metadata.MAX_PROOF), ('bytes', True), ('sha256', 'latest')]:
            lock = copy.deepcopy(self.fixture.lock); lock['cpu_component']['proof'][field] = value
            path.write_bytes(encoded(lock))
            with self.subTest(field=field, value=value), patch.object(metadata, 'fetch', side_effect=AssertionError('unreviewed source')):
                with self.assertRaises(ValueError): metadata.prepare(self.cache)
            self.assertFalse(self.cache.exists())
        lock = copy.deepcopy(self.fixture.lock); lock['wheels'][0]['url'] += '?unreviewed=1'
        path.write_bytes(encoded(lock))
        with self.assertRaises(ValueError): metadata.prepare(self.cache)

    def test_legacy_lock_needs_no_proof_cache_or_download(self):
        lock = copy.deepcopy(self.fixture.lock); del lock['cpu_component']
        lock['wheels'][0]['url'] = 'https://files.pythonhosted.org/old-reference.whl'
        (self.fixture.root / 'locks' / (metadata.PACK_ID + '.json')).write_bytes(encoded(lock))
        with patch.object(metadata, 'fetch', side_effect=AssertionError('legacy path must not download')):
            self.assertIsNone(metadata.prepare(self.cache))
        self.assertFalse(self.cache.exists())

    def test_cache_links_and_partial_files_are_not_followed_or_overwritten(self):
        self.cache.mkdir()
        target = self.cache / self.fixture.path.name
        partial = target.with_suffix('.partial')
        partial.write_bytes(b'in-flight download')
        with self.assertRaises(FileExistsError): metadata.prepare(self.cache, self.fixture.path)
        self.assertEqual(partial.read_bytes(), b'in-flight download')
        partial.unlink()
        try: target.symlink_to(self.fixture.path)
        except OSError as error: self.skipTest('Symlink unavailable: ' + str(error))
        with patch.object(metadata, 'fetch', side_effect=AssertionError('cache link must not download')):
            with self.assertRaisesRegex(ValueError, 'link or reparse'):
                metadata.prepare(self.cache)

    def test_cli_prints_only_verified_cache_path_for_generators(self):
        argv = ['prepare', '--cache', str(self.cache), '--cpu-publication-proof', str(self.fixture.path)]
        with patch('sys.argv', argv), patch('sys.stdout', new_callable=io.StringIO) as output, \
             patch.object(metadata, 'fetch', side_effect=AssertionError('offline path must not download')):
            metadata.main()
        self.assertEqual(output.getvalue().strip(), str(self.cache / self.fixture.path.name))

    def test_component_workflow_seeds_shared_cache_before_both_generations(self):
        repo = Path(__file__).resolve().parents[2]
        source = (repo / '.github/workflows/asr-components.yml').read_text(encoding='utf-8')
        block = source.split('- name: Reproduce review-only recipes', 1)[1].split('- name:', 1)[0]
        self.assertIn('prepare_cpu_proof.py --cache "$RUNNER_TEMP/asr-downloads"', block)
        self.assertIn('proof_args=(--cpu-publication-proof "$CPU_PUBLICATION_PROOF")', block)
        commands = [line.strip() for line in block.splitlines() if 'generate_recipes.py' in line]
        self.assertEqual(len(commands), 2)
        self.assertTrue(all('"${proof_args[@]}"' in command for command in commands))
        self.assertIn('first.read_bytes() == second.read_bytes()', block)
        self.assertNotIn('build_cpu.py', source)


if __name__ == '__main__':
    unittest.main()
