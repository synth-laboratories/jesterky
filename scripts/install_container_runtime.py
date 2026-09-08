#!/usr/bin/env python3
"""Install a hash-pinned published or explicitly supplied local artifact into an image filesystem."""
import argparse, hashlib, json, os, pathlib, platform, tempfile, urllib.request
p=argparse.ArgumentParser()
p.add_argument('--receipt',type=pathlib.Path,required=True)
p.add_argument('--destination',type=pathlib.Path,required=True)
p.add_argument('--artifact',type=pathlib.Path,help='Use matching local release bytes without downloading; receipt verification still applies.')
a=p.parse_args();r=json.loads(a.receipt.read_text())
expected='linux-'+{'arm64':'aarch64','AMD64':'x86_64'}.get(platform.machine(),platform.machine())
if platform.system()!='Linux' or r['target']!=expected:raise SystemExit('receipt does not match container platform')
name=f"jesterky-{r['version']}-{r['target']}"
url=f"https://github.com/synth-laboratories/jesterky/releases/download/v{r['version']}/{name}"
if r['url']!=url or not 0<int(r['size'])<=256*1024*1024:raise SystemExit('invalid release receipt')
if a.artifact is not None:
 with a.artifact.open('rb') as source:body=source.read(int(r['size'])+1)
else:
 with urllib.request.urlopen(url,timeout=60) as response:body=response.read(int(r['size'])+1)
if len(body)!=r['size'] or hashlib.sha256(body).hexdigest()!=r['sha256']:raise SystemExit('artifact verification failed')
a.destination.parent.mkdir(parents=True,exist_ok=True)
fd,tmp=tempfile.mkstemp(dir=a.destination.parent,prefix='.jesterky-')
try:
 with os.fdopen(fd,'wb') as handle:handle.write(body);handle.flush();os.fsync(handle.fileno())
 os.chmod(tmp,0o755);os.replace(tmp,a.destination)
finally:
 if os.path.exists(tmp):os.unlink(tmp)
