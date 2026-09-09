#!/usr/bin/env python3
"""Build a native optional Workshop runtime and its pinned artifact receipt.

Does not publish. --register-dev installs only the local development pointer.
"""
import argparse, hashlib, json, os, pathlib, platform, shutil, subprocess, tomllib


def source_revision(root):
 """The commit these bytes were built from.

 The publisher requires this on every receipt and refuses a release without it,
 so a receipt that omits it can never be published. A dirty tree cannot honestly
 claim a revision, so it refuses rather than naming one it did not build.
 """
 revision=subprocess.check_output(['git','rev-parse','HEAD'],cwd=root,text=True).strip()
 if subprocess.check_output(['git','status','--porcelain'],cwd=root,text=True).strip():
  raise SystemExit('refusing to record a source revision for a dirty tree; commit or stash first')
 return revision
p=argparse.ArgumentParser()
p.add_argument('--output',type=pathlib.Path,required=True)
p.add_argument('--register-dev',action='store_true')
a=p.parse_args()
root=pathlib.Path(__file__).resolve().parents[1]
version=tomllib.loads((root/'Cargo.toml').read_text())['workspace']['package']['version']
subprocess.run(['cargo','build','--locked','--release','-p','jesterky-cli'],cwd=root,check=True)
system={'Darwin':'macos','Linux':'linux','Windows':'windows'}[platform.system()]
arch={'arm64':'aarch64','AMD64':'x86_64'}.get(platform.machine(),platform.machine())
target=f'{system}-{arch}'
a.output.mkdir(parents=True,exist_ok=True)
name=f'jesterky-{version}-{target}'
binary=a.output/name
shutil.copy2(root/'target/release/jesterky',binary)
subprocess.run([str(binary.resolve()),'--version'],check=True)
receipt={'version':version,'target':target,'sha256':hashlib.sha256(binary.read_bytes()).hexdigest(),'size':binary.stat().st_size,'sourceRevision':source_revision(root),'url':f'https://github.com/synth-laboratories/jesterky/releases/download/v{version}/{name}'}
(a.output/f'{name}.json').write_text(json.dumps(receipt,indent=2)+'\n')
if a.register_dev:
 parent=pathlib.Path.home()/f'.synth-desktop/dev-builds/jesterky/{version}'
 build=parent/receipt['sha256']
 build.mkdir(parents=True,exist_ok=True)
 shutil.copy2(binary,build/'jesterky')
 (build/'artifact.json').write_text(json.dumps(receipt,indent=2)+'\n')
 pointer=parent/f'.current-{os.getpid()}'
 pointer.symlink_to(build, target_is_directory=True)
 pointer.replace(parent/'current')
print(json.dumps(receipt,indent=2))
