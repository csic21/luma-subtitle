import os
from pathlib import Path
import unittest

from fixture_paths import temporary_root, windows_short_path_alias


class TemporaryRootTests(unittest.TestCase):
    def test_owned_root_is_canonical_and_cleaned_up(self):
        with temporary_root(prefix='luma fixture ', suffix=' long name') as root:
            self.assertIsInstance(root, Path)
            self.assertEqual(root, root.resolve(strict=True))
            (root / 'child').write_text('owned fixture', encoding='utf-8')
        self.assertFalse(root.exists())

    def test_exception_still_cleans_only_the_owned_child(self):
        with temporary_root() as parent:
            marker = parent / 'keep'; marker.write_text('parent', encoding='utf-8')
            with self.assertRaisesRegex(RuntimeError, 'intentional'):
                with temporary_root(dir=parent) as child:
                    (child / 'file').write_bytes(b'owned')
                    raise RuntimeError('intentional')
            self.assertFalse(child.exists())
            self.assertEqual(marker.read_text(), 'parent')

    def test_aliased_parent_is_canonicalized_before_children_are_derived(self):
        with temporary_root() as parent:
            actual = parent / 'long temporary parent'; actual.mkdir()
            alias = parent / 'alias'
            try:
                alias.symlink_to(actual, target_is_directory=True)
            except OSError as error:
                self.skipTest('Directory aliases unavailable: ' + str(error))
            with temporary_root(dir=alias) as root:
                self.assertEqual(root.parent, actual)
                self.assertEqual(root, root.resolve(strict=True))
                deliberate = root / 'deliberate-link'
                deliberate.symlink_to(actual, target_is_directory=True)
                self.assertTrue(deliberate.is_symlink(), 'Child aliases must remain visible to production guards')
            self.assertTrue(alias.is_symlink())
            self.assertTrue(actual.exists())

    @unittest.skipUnless(os.name == 'nt', 'Requires native Windows GetShortPathNameW')
    def test_actual_windows_short_parent_yields_long_canonical_owned_root(self):
        with temporary_root(prefix='Luma native long fixture parent ') as parent:
            try:
                alias = windows_short_path_alias(parent)
            except OSError as error:
                self.skipTest('Native short-path probe unavailable: ' + str(error))
            if alias is None:
                self.skipTest('This filesystem does not expose a distinct 8.3 alias')
            self.assertNotEqual(os.path.normcase(str(alias)), os.path.normcase(str(parent)))
            self.assertEqual(alias.resolve(strict=True), parent)
            with temporary_root(dir=alias) as root:
                self.assertEqual(root.parent, parent)
                self.assertEqual(root, root.resolve(strict=True))
