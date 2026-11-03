#!/usr/bin/env python3
import importlib.machinery
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest

loader = importlib.machinery.SourceFileLoader('package_frontend', str(Path(__file__).with_name('package-frontend')))
spec = importlib.util.spec_from_loader(loader.name, loader)
pack = importlib.util.module_from_spec(spec)
loader.exec_module(pack)


class PackageTests(unittest.TestCase):
    def test_versions_are_retained_and_existing_releases_are_not_overwritten(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            source = root / 'source'
            source.mkdir()
            (source / 'a.js').write_text('old script')
            (source / 'b.js').write_text('second part')
            manifest = {'version': 1, 'assets': [{'name': 'bundle.js', 'content_type': 'js', 'files': ['a.js', 'b.js'], 'separator': '\n'}]}
            (source / 'manifest.json').write_text(json.dumps(manifest))
            pack.package(source, root / 'first')
            self.assertEqual((root / 'first').stat().st_mode & 0o777, 0o755)
            self.assertEqual((root / 'first' / 'manifest.json').stat().st_mode & 0o777, 0o644)
            old = list(pack.entries(root / 'first'))
            self.assertEqual(old[0][3], b'old script\nsecond part')
            (source / 'a.js').write_text('new script')
            pack.package(source, root / 'second', root / 'first')
            versions = list(pack.entries(root / 'second'))
            self.assertEqual(len(versions), 2)
            self.assertEqual(versions[0][3], b'new script\nsecond part')
            self.assertEqual(versions[1][3], old[0][3])
            self.assertNotEqual(versions[0][4], versions[1][4])
            self.assertEqual(list(pack.entries(root / 'first')), old)
            with self.assertRaises(ValueError):
                pack.package(source, root / 'first')
            (root / 'second' / 'objects' / versions[0][4]).write_text('corrupted')
            with self.assertRaisesRegex(ValueError, 'digest mismatch'):
                list(pack.entries(root / 'second'))

    def test_bad_inputs_do_not_publish_a_package(self):
        for file in ['../outside.js', '/outside.js', 'missing.js', 'escape.js']:
            with self.subTest(file=file), tempfile.TemporaryDirectory() as temporary:
                root = Path(temporary)
                source = root / 'source'
                source.mkdir()
                (root / 'outside.js').write_text('outside')
                (source / 'escape.js').symlink_to(root / 'outside.js')
                (source / 'manifest.json').write_text(json.dumps({'version': 1, 'assets': [
                    {'name': 'test.js', 'content_type': 'js', 'files': [file]}]}))
                with self.assertRaises((ValueError, FileNotFoundError)):
                    pack.package(source, root / 'release')
                self.assertFalse((root / 'release').exists())

    def test_repository_catalog_packages_every_asset_without_changing_bytes(self):
        source = Path(__file__).resolve().parent.parent / 'static'
        with tempfile.TemporaryDirectory() as temporary:
            destination = Path(temporary) / 'release'
            pack.package(source, destination)
            self.assertEqual(list(pack.entries(source)), list(pack.entries(destination)))


if __name__ == '__main__':
    unittest.main()
