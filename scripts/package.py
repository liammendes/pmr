#!/usr/bin/env python3
"""Package native binaries with upstream and locked dependency notices."""
import argparse
import hashlib
import json
import pathlib
import shutil
import subprocess
import tarfile
import tempfile

root = pathlib.Path(__file__).resolve().parent.parent
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--target', required=True)
parser.add_argument('--tag', required=True)
args = parser.parse_args()
version = json.loads(subprocess.check_output(['cargo', 'metadata', '--locked', '--no-deps', '--format-version', '1'], cwd=root))['packages'][0]['version']
if args.tag != 'v' + version:
    parser.error('tag must match the Cargo package version')
metadata = json.loads(subprocess.check_output(['cargo', 'metadata', '--locked', '--format-version', '1'], cwd=root))
dist = root / 'dist'
dist.mkdir(exist_ok=True)
with tempfile.TemporaryDirectory(prefix='pmr-package-') as tmp:
    stage = pathlib.Path(tmp)
    for name in ['pmr', 'pmr-runtime', 'pmr-docker', 'pmr-dev']:
        shutil.copy2(root / 'target' / args.target / 'release' / name, stage / name)
    for name in ['LICENSE', 'NOTICE', 'README.md', 'CHANGELOG.md']:
        shutil.copy2(root / name, stage / name)
    notices = stage / 'THIRD-PARTY-LICENSES'
    notices.mkdir()
    inventory = []
    for package in sorted(metadata['packages'], key=lambda p: (p['name'], p['version'])):
        if package['source'] is None:
            continue
        directory = pathlib.Path(package['manifest_path']).parent
        candidates = set()
        for pattern in ['LICENSE*', 'LICENCE*', 'COPYING*', 'NOTICE*', 'license*', 'licence*', 'licenses/*']:
            candidates.update(p for p in directory.glob(pattern) if p.is_file())
        if package.get('license_file'):
            candidates.add(directory / package['license_file'])
        if not candidates:
            raise RuntimeError(f"Missing licence text for {package['name']} {package['version']}")
        destination = notices / (package['name'] + '-' + package['version'])
        destination.mkdir()
        files = []
        for path in sorted(candidates):
            relative = path.relative_to(directory)
            target = destination / relative
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(path, target)
            files.append(str(relative))
        inventory.append({'name': package['name'], 'version': package['version'], 'license': package['license'],
                          'repository': package.get('repository'), 'files': files})
    (notices / 'index.json').write_text(json.dumps(inventory, indent=2) + '\n')
    archive = dist / f'pmr-{args.tag}-{args.target}.tar.gz'
    with tarfile.open(archive, 'w:gz') as tar:
        for path in sorted(stage.iterdir()):
            tar.add(path, arcname=path.name)
    digest = hashlib.sha256(archive.read_bytes()).hexdigest()
    archive.with_suffix(archive.suffix + '.sha256').write_text(f'{digest}  {archive.name}\n')
    print(f'{archive.name}: {len(inventory)} dependency notices')
