import hashlib
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location('publisher', Path(__file__).parents[1] / 'scripts/publish_workshop.py')
publisher = importlib.util.module_from_spec(spec)
spec.loader.exec_module(publisher)


class PublicationTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        for target in ('linux-aarch64', 'linux-x86_64'):
            binary = self.root / f'jesterky-0.1.3-{target}'
            binary.write_bytes(target.encode())
            data = {'target': target, 'version': '0.1.3', 'sourceRevision': 'abc', 'size': binary.stat().st_size, 'sha256': hashlib.sha256(binary.read_bytes()).hexdigest(), 'url': 'https://github.com/synth-laboratories/jesterky/releases/download/v0.1.3/' + binary.name}
            Path(str(binary) + '.json').write_text(json.dumps(data))

    def test_wrong_source_and_corrupt_binary_fail(self):
        with self.assertRaisesRegex(ValueError, 'source mismatch'):
            publisher.validate(self.root, 'v0.1.3', 'other')
        (self.root / 'jesterky-0.1.3-linux-aarch64').write_bytes(b'wrong')
        with self.assertRaisesRegex(ValueError, 'digest mismatch'):
            publisher.validate(self.root, 'v0.1.3', 'abc')

    def test_existing_identical_assets_are_not_uploaded(self):
        files = publisher.validate(self.root, 'v0.1.3', 'abc')
        def fake(*args):
            if args[0] == 'git': return 'abc\n'
            if args[2] == 'view': return json.dumps({'assets': [{'name': p.name} for p in files]})
            if args[2] == 'download':
                (Path(args[-1]) / args[5]).write_bytes((self.root / args[5]).read_bytes())
                return ''
            self.fail('unexpected mutation: ' + repr(args))
        with patch.object(publisher, 'run', side_effect=fake): publisher.publish(self.root, 'v0.1.3')

    def test_mismatch_prevents_all_uploads(self):
        def fake(*args):
            if args[0] == 'git': return 'abc\n'
            if args[2] == 'view': return json.dumps({'assets': [{'name': 'jesterky-0.1.3-linux-x86_64'}]})
            if args[2] == 'download':
                (Path(args[-1]) / args[5]).write_bytes(b'different')
                return ''
            self.fail('unexpected mutation: ' + repr(args))
        with patch.object(publisher, 'run', side_effect=fake), self.assertRaisesRegex(ValueError, 'published bytes differ'):
            publisher.publish(self.root, 'v0.1.3')

    def test_missing_assets_uploaded_without_clobber(self):
        calls = []
        def fake(*args):
            calls.append(args)
            if args[0] == 'git': return 'abc\n'
            if args[2] == 'view': return '{"assets": []}'
            if args[2] == 'download':
                (Path(args[-1]) / args[5]).write_bytes((self.root / args[5]).read_bytes())
                return ''
            self.assertEqual(args[2], 'upload')
            self.assertNotIn('--clobber', args)
            return ''
        with patch.object(publisher, 'run', side_effect=fake): publisher.publish(self.root, 'v0.1.3')
        self.assertEqual(sum(c[2] == 'upload' for c in calls if c[0] == 'gh'), 4)

if __name__ == '__main__': unittest.main()


class PackagerPublisherContractTests(unittest.TestCase):
    """The publisher must accept what the packager actually writes.

    Every existing case in this file hand-writes a receipt that already carries
    `sourceRevision`, so none of them notices that `package_workshop.py` never
    wrote one. The publisher refuses such a receipt with "release source
    mismatch", which meant no packaged artifact could reach a release at all.
    """

    packager = (Path(__file__).parents[1] / 'scripts/package_workshop.py').read_text()

    def test_the_packager_writes_every_field_the_publisher_reads(self):
        publisher_source = (Path(__file__).parents[1] / 'scripts/publish_workshop.py').read_text()
        required = {'version', 'target', 'sha256', 'size', 'url', 'sourceRevision'}
        for field in required:
            self.assertIn(
                f"'{field}'", publisher_source,
                f'{field} should be part of the publish contract')
            self.assertIn(
                f"'{field}'", self.packager,
                f'package_workshop.py must record {field}; the publisher refuses a receipt without it')

    def test_the_packager_refuses_to_name_a_revision_for_a_dirty_tree(self):
        self.assertIn('refusing to record a source revision for a dirty tree', self.packager)

    def test_a_packager_shaped_receipt_passes_validation(self):
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        root = Path(tmp.name)
        revision = '9cab88460243f31c18829422d89df6d81539f500'
        for target in ('linux-x86_64', 'linux-aarch64'):
            name = f'jesterky-0.1.3-{target}'
            binary = root / name
            binary.write_bytes(target.encode())
            # Exactly the keys, and the key order, package_workshop.py writes.
            receipt = {
                'version': '0.1.3',
                'target': target,
                'sha256': hashlib.sha256(binary.read_bytes()).hexdigest(),
                'size': binary.stat().st_size,
                'sourceRevision': revision,
                'url': f'https://github.com/synth-laboratories/jesterky/releases/download/v0.1.3/{name}',
            }
            Path(str(binary) + '.json').write_text(json.dumps(receipt, indent=2) + '\n')
        self.assertEqual(len(publisher.validate(root, 'v0.1.3', revision)), 4)

    def test_a_dry_run_validates_without_touching_the_release(self):
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        root = Path(tmp.name)
        revision = 'abc'
        for target in ('linux-x86_64', 'linux-aarch64'):
            name = f'jesterky-0.1.3-{target}'
            binary = root / name
            binary.write_bytes(target.encode())
            Path(str(binary) + '.json').write_text(json.dumps({
                'version': '0.1.3', 'target': target,
                'sha256': hashlib.sha256(binary.read_bytes()).hexdigest(),
                'size': binary.stat().st_size, 'sourceRevision': revision,
                'url': f'https://github.com/synth-laboratories/jesterky/releases/download/v0.1.3/{name}',
            }))

        def fake(*args):
            if args[0] == 'git':
                return revision + '\n'
            self.fail('a dry run must not contact GitHub: ' + repr(args))

        with patch.object(publisher, 'run', side_effect=fake):
            publisher.publish(root, 'v0.1.3', dry_run=True)
