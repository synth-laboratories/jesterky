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
