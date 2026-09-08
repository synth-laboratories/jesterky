#!/usr/bin/env python3
"""Publish verified Linux assets without replacing already-published bytes."""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess
import tempfile


def run(*args):
    return subprocess.check_output(args, text=True)


def validate(root, tag, revision):
    files = []
    targets = set()
    for receipt in sorted(root.glob('jesterky-*-linux-*.json')):
        data = json.loads(receipt.read_text())
        target = data['target']
        if target not in {'linux-x86_64', 'linux-aarch64'} or target in targets:
            raise ValueError('unexpected or duplicate target')
        targets.add(target)
        expected = f'jesterky-{tag.removeprefix("v")}-{target}'
        if receipt.name != expected + '.json' or data['version'] != tag.removeprefix('v'):
            raise ValueError('release version mismatch')
        if data.get('sourceRevision') != revision:
            raise ValueError('release source mismatch')
        if data['url'] != f'https://github.com/synth-laboratories/jesterky/releases/download/{tag}/{expected}':
            raise ValueError('unexpected release URL')
        binary = receipt.with_suffix('')
        content = binary.read_bytes()
        if len(content) != data['size'] or hashlib.sha256(content).hexdigest() != data['sha256']:
            raise ValueError('artifact digest mismatch')
        files.extend([binary, receipt])
    if targets != {'linux-x86_64', 'linux-aarch64'}:
        raise ValueError('both Linux targets are required')
    return files


def publish(root, tag):
    revision = run('git', 'rev-parse', f'{tag}^{{commit}}').strip()
    files = validate(root, tag, revision)
    # Missing release is an explicit prerequisite failure, never an implicit create.
    release = json.loads(run('gh', 'release', 'view', tag, '--json', 'assets'))
    existing = {asset['name'] for asset in release['assets']}
    pending = []
    with tempfile.TemporaryDirectory() as directory:
        for path in files:
            if path.name in existing:
                run('gh', 'release', 'download', tag, '--pattern', path.name, '--dir', directory)
                if (Path(directory) / path.name).read_bytes() != path.read_bytes():
                    raise ValueError(f'published bytes differ: {path.name}')
            else:
                pending.append(path)
        # Validate every existing asset before any mutation. No --clobber.
        for path in pending:
            run('gh', 'release', 'upload', tag, str(path))
            verification = Path(directory) / 'uploaded'
            verification.mkdir(exist_ok=True)
            run('gh', 'release', 'download', tag, '--pattern', path.name, '--dir', str(verification))
            if (verification / path.name).read_bytes() != path.read_bytes():
                raise ValueError(f'uploaded bytes failed readback: {path.name}')
    print(json.dumps({'tag': tag, 'sourceRevision': revision, 'verifiedAssets': len(files), 'uploadedAssets': len(pending)}))


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('root', type=Path)
    parser.add_argument('--tag', required=True)
    args = parser.parse_args()
    publish(args.root, args.tag)
