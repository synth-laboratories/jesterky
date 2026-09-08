#!/usr/bin/env python3
"""Verify the packaged bytes and execute the native CLI without provider access."""
import hashlib,json,pathlib,subprocess,sys
root=pathlib.Path(sys.argv[1]).resolve()
receipts=list(root.glob('jesterky-*.json'))
assert len(receipts)==1, 'one native receipt required'
receipt=json.loads(receipts[0].read_text());binary=receipts[0].with_suffix('')
assert binary.stat().st_size==receipt['size']
assert hashlib.sha256(binary.read_bytes()).hexdigest()==receipt['sha256']
version=subprocess.check_output([str(binary),'--version'],text=True).strip()
assert receipt['version'] in version
subprocess.run([str(binary),'validate','examples/trace_v5_acceptance.json'],check=True)
subprocess.run([str(binary),'schema','workflow'],check=True,stdout=subprocess.DEVNULL)
print(json.dumps({'status':'passed','target':receipt['target'],'sha256':receipt['sha256'],'version':version,'providerCalls':0}))
