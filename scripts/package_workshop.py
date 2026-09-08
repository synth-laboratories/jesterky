#!/usr/bin/env python3
"""Build a native optional Workshop runtime and its pinned artifact receipt.

Does not publish. --register-dev installs only the local development pointer.
"""
import argparse, hashlib, json, os, pathlib, platform, shutil, subprocess, tomllib
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
receipt={'version':version,'target':target,'sha256':hashlib.sha256(binary.read_bytes()).hexdigest(),'size':binary.stat().st_size,'url':f'https://github.com/synth-laboratories/jesterky/releases/download/v{version}/{name}'}
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
