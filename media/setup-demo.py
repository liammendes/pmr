#!/usr/bin/env python3
"""Start real, isolated screenshot fixtures. Never uses the default pmr home."""
import argparse
import json
import os
import pathlib
import shutil
import socket
import subprocess
import tempfile
import time
import urllib.request

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--pmr', required=True)
parser.add_argument('--home', required=True, type=pathlib.Path)
args = parser.parse_args()
if args.home.exists():
    parser.error('choose a new home directory')
for port in [18321, 18322]:
    with socket.socket() as probe:
        probe.bind(('127.0.0.1', port))
args.home.mkdir(parents=True)
work = args.home / 'apps'
work.mkdir()
env = {k: os.environ[k] for k in ['HOME', 'PATH', 'TMPDIR', 'LANG'] if k in os.environ}
env.update(PMR_HOME=str(args.home), PM2_HOME=str(args.home))
for role in ['api', 'worker', 'camera-sim', 'docs']:
    script = work / (role + '.cjs')
    shutil.copyfile(pathlib.Path(__file__).with_name('demo.cjs'), script)
    subprocess.run([args.pmr, '-s', 'start', str(script), '--name', role, '--interpreter', 'node',
                    '--cwd', str(work), '--', role], env=env, check=True)
for port in [18321, 18322]:
    for attempt in range(100):
        try:
            with urllib.request.urlopen(f'http://127.0.0.1:{port}/health', timeout=.5) as response:
                assert response.status == 200
            break
        except OSError:
            time.sleep(.1)
    else:
        raise RuntimeError('demo did not become ready')
    for _ in range(12):
        urllib.request.urlopen(f'http://127.0.0.1:{port}/health', timeout=1).close()
print('Demo ready. Use the same PMR_HOME for monit and kill: ' + str(args.home))
