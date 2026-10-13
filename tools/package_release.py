#!/usr/bin/env python3
"""Create portable release ZIPs, retaining bundle paths and executable permissions."""
import argparse
import json
import re
import shutil
import subprocess
import zipfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--platform', required=True, choices=['windows-x86_64','linux-x86_64','macos-universal'])
    parser.add_argument('--au', type=Path)
    args = parser.parse_args()
    version = re.search(r'^version = "([^"]+)"', (ROOT/'Cargo.toml').read_text(),re.M).group(1)
    name = f'reveler-rs-v{version}-{args.platform}'
    stage = ROOT/'target/release-package'/name
    if stage.exists():
        shutil.rmtree(stage)
    stage.mkdir(parents=True)
    for suffix in ['vst3','clap']:
        bundle = ROOT/'target/bundled'/f'Reveler-rs.{suffix}'
        if not bundle.exists():
            raise SystemExit(f'Missing built plugin: {bundle}')
        if bundle.is_dir():
            shutil.copytree(bundle, stage/bundle.name)
        else:
            shutil.copy2(bundle,stage/bundle.name)
    if args.platform == 'macos-universal':
        if not args.au or not args.au.is_dir():
            raise SystemExit('macOS release requires --au pointing at the built .component')
        shutil.copytree(args.au,stage/args.au.name)
    for file in ['README.md','LICENSE','THIRD_PARTY.md']:
        shutil.copy2(ROOT/file, stage/file)
    (stage/'docs').mkdir()
    shutil.copy2(ROOT/'docs/macos.md', stage/'docs/macos.md')
    metadata=json.loads(subprocess.check_output(['cargo','metadata','--locked','--format-version','1'],cwd=ROOT))
    notices=stage/'licenses';notices.mkdir()
    inventory=[]
    for package in metadata['packages']:
        if package['id'] in metadata['workspace_members']:
            continue
        pkg_root=Path(package['manifest_path']).parent
        inventory.append({key:package.get(key) for key in ['name','version','license','repository']})
        files=[p for p in pkg_root.iterdir() if p.is_file() and p.name.upper().startswith(('LICENSE','COPYING','NOTICE','UNLICENSE'))]
        if (pkg_root/'licenses').is_dir():
            files.extend(p for p in (pkg_root/'licenses').rglob('*') if p.is_file())
        destination=notices/f"{package['name']}-{package['version']}"
        destination.mkdir()
        for path in files:
            relative=path.relative_to(pkg_root)
            (destination/relative).parent.mkdir(parents=True,exist_ok=True)
            shutil.copy2(path,destination/relative)
    (notices/'rust-dependencies.json').write_text(json.dumps(inventory,indent=2)+'\n')
    if args.au:
        deps=ROOT/'target/au'
        for label, file in [
            ('clap-wrapper',deps/'_deps/clap_wrapper-src/LICENSE'),
            ('clap',deps/'_deps/clap_wrapper-build/cpm/clap/LICENSE'),
            ('AudioUnitSDK',deps/'cpm/AudioUnitSDK/LICENSE.txt'),
            ('fmt',deps/'_deps/clap_wrapper-src/libs/fmt/fmt/format.h'),
        ]:
            # Local CMake may use FETCHCONTENT_SOURCE_DIR_CLAP_WRAPPER.
            if not file.exists() and label in ['clap-wrapper','fmt']:
                file=ROOT/'target/deps/clap-wrapper'/('LICENSE' if label=='clap-wrapper' else 'libs/fmt/fmt/format.h')
            if not file.is_file():
                raise SystemExit(f'Missing AU dependency license: {file}')
            if label == 'fmt':
                (notices/(label+'-LICENSE.txt')).write_text(file.read_text().split('*/',1)[0]+'*/\n')
            else:
                shutil.copy2(file,notices/(label+'-LICENSE.txt'))
    dist=ROOT/'dist';dist.mkdir(exist_ok=True)
    archive=dist/(name+'.zip')
    temporary=archive.with_suffix('.zip.tmp')
    with zipfile.ZipFile(temporary,'w',compression=zipfile.ZIP_DEFLATED,compresslevel=6,strict_timestamps=False) as output:
        for path in sorted(stage.rglob('*')):
            output.write(path,path.relative_to(stage.parent))
    temporary.replace(archive)
    print(archive)

if __name__=='__main__':
    main()
