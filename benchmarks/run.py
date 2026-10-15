#!/usr/bin/env python3
"""Isolated, alternating-order PMR/PM2 benchmark. Python 3.10+, Node, autocannon.

No packages are installed and no default manager home is used by this script.
Run --help for paths. Results include every sample, not just the fastest run.
"""
import argparse
import csv
import datetime
import hashlib
import json
import math
import os
import pathlib
import platform
import shutil
import signal
import socket
import statistics
import subprocess
import tempfile
import time
import urllib.request

HERE = pathlib.Path(__file__).resolve().parent


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--pmr', required=True, type=pathlib.Path)
    parser.add_argument('--pm2', action='append', required=True, type=pathlib.Path)
    parser.add_argument('--autocannon', required=True, type=pathlib.Path)
    parser.add_argument('--output', default=HERE / 'results' / 'local', type=pathlib.Path)
    parser.add_argument('--samples', type=int, default=30)
    parser.add_argument('--lifecycle-samples', type=int, default=15)
    parser.add_argument('--http-seconds', type=int, default=5)
    parser.add_argument('--http-samples', type=int, default=3)
    args = parser.parse_args()
    if min(args.samples, args.lifecycle_samples, args.http_seconds, args.http_samples) < 1:
        parser.error('sample counts and durations must be positive')
    node = shutil.which('node')
    if not node:
        parser.error('node must be on PATH')
    args.output.mkdir(parents=True, exist_ok=True)
    # Do not inherit the parent supervisor's flattened app configuration.
    base_env = {k: os.environ[k] for k in ('HOME', 'PATH', 'TMPDIR', 'LANG', 'LC_ALL', 'SYSTEMROOT')
                if k in os.environ}
    base_env.update(NO_COLOR='1', FORCE_COLOR='0', PM2_SILENT='true')
    for port in [17880 + i for i in range(1 + len(args.pm2))] + [17890]:
        with socket.socket() as probe:
            probe.bind(('127.0.0.1', port))
    root = pathlib.Path(tempfile.mkdtemp(prefix='pmr-bench-'))
    samples, http_samples, managers, direct = [], [], [], None

    def call(command, env, timeout=30):
        start = time.perf_counter_ns()
        p = subprocess.run(command, env=env, capture_output=True, timeout=timeout)
        elapsed = (time.perf_counter_ns() - start) / 1e6
        if p.returncode:
            raise RuntimeError(f'{command[0:2]} exited {p.returncode}: {p.stderr.decode()[-2000:]}')
        return elapsed, p.stdout

    def cmd(m, *tail):
        return call(m['command'] + list(tail), m['env'])

    def health(m, previous=None):
        deadline = time.monotonic() + 15
        while time.monotonic() < deadline:
            try:
                with urllib.request.urlopen(f"http://127.0.0.1:{m['port']}/health", timeout=.2) as r:
                    pid = json.load(r)['pid']
                    if pid != previous:
                        return pid
            except (OSError, ValueError):
                pass
            time.sleep(.005)
        raise RuntimeError(f"{m['name']}: application did not become ready")

    def start(m):
        return cmd(m, 'start', str(HERE / 'http.cjs'), '--name', 'bench-http',
                   '--interpreter', node)

    def loop(metric, count, warmups, action):
        print(f'{metric}: {count} samples per manager; {warmups} warmups', flush=True)
        for index in range(-warmups, count):
            order = managers if index % 2 == 0 else list(reversed(managers))
            for m in order:
                elapsed = action(m)
                if index >= 0:
                    samples.append(dict(metric=metric, manager=m['name'], sample=index + 1, milliseconds=elapsed))
        save()

    def read_command(command):
        try:
            return subprocess.check_output(command, env=base_env, text=True, stderr=subprocess.DEVNULL).strip()
        except (OSError, subprocess.CalledProcessError):
            return None

    metadata = {
        'started_at_utc': datetime.datetime.now(datetime.timezone.utc).isoformat(),
        'system': platform.system(), 'architecture': platform.machine(),
        'os': read_command(['sw_vers', '-productVersion']) or platform.release(),
        'os_build': read_command(['sw_vers', '-buildVersion']),
        'model': read_command(['sysctl', '-n', 'hw.model']),
        'cpu': read_command(['sysctl', '-n', 'machdep.cpu.brand_string']),
        'logical_cpus': os.cpu_count(),
        'memory_bytes': read_command(['sysctl', '-n', 'hw.memsize']),
        'load_average_start': os.getloadavg(),
        'node': read_command([node, '--version']),
        'rust': read_command(['rustc', '--version']),
        'source_commit': read_command(['git', '-C', str(HERE.parent), 'rev-parse', 'HEAD']),
        'source_diff_sha256': hashlib.sha256(subprocess.check_output(['git', '-C', str(HERE.parent), 'diff'], env=base_env)).hexdigest(),
        'pmr_binary_sha256': hashlib.sha256(args.pmr.read_bytes()).hexdigest(),
        'fixture_sha256': hashlib.sha256((HERE / 'http.cjs').read_bytes()).hexdigest(),
        'configuration': {'cli_samples': args.samples, 'lifecycle_samples': args.lifecycle_samples,
                          'http_samples': args.http_samples, 'http_seconds': args.http_seconds,
                          'http_connections': 20, 'http_pipelining': 1, 'cli_warmups': 5,
                          'lifecycle_warmups': 2, 'http_warmup_seconds': 2},
        'status': 'running',
    }

    def save():
        report = {'metadata': metadata, 'latency_samples': samples, 'http_samples': http_samples}
        (args.output / 'raw.json').write_text(json.dumps(report, indent=2) + '\n')
        if samples:
            with (args.output / 'latency.csv').open('w') as f:
                writer = csv.DictWriter(f, fieldnames=samples[0].keys())
                writer.writeheader()
                writer.writerows(samples)

    def stop_on_signal(signum, _frame):
        raise KeyboardInterrupt(f'signal {signum}')

    signal.signal(signal.SIGTERM, stop_on_signal)
    try:
        configurations = [('pmr', [str(args.pmr.resolve())])]
        for path in args.pm2:
            package = json.loads((path.resolve().parent.parent / 'package.json').read_text())
            configurations.append(('pm2-' + package['version'], [node, str(path.resolve())]))
        if len({name for name, _ in configurations}) != len(configurations):
            raise ValueError('Provide distinct PM2 versions')
        for i, (name, command) in enumerate(configurations):
            home = root / name
            env = dict(base_env, PMR_HOME=str(home), PM2_HOME=str(home), PMR_BENCH_PORT=str(17880 + i))
            managers.append(dict(name=name, command=command, env=env, port=17880 + i))
        metadata['versions'] = [m['name'] for m in managers]
        metadata['pmr_version'] = cmd(managers[0], '--version')[1].decode().strip()
        metadata['autocannon_version'] = read_command([node, str(args.autocannon), '--version'])
        loop('help_cli', args.samples, 5, lambda m: cmd(m, '--help')[0])

        def cold(m):
            cmd(m, 'kill')
            return cmd(m, 'ping')[0]

        loop('cold_daemon_ping', args.lifecycle_samples, 2, cold)
        loop('list_empty_cli', args.samples, 5, lambda m: cmd(m, 'jlist')[0])

        def start_ready(m):
            if json.loads(cmd(m, 'jlist')[1]):
                cmd(m, 'delete', 'all')
            begin = time.perf_counter_ns()
            start(m)
            health(m)
            return (time.perf_counter_ns() - begin) / 1e6

        loop('start_to_http_ready', args.lifecycle_samples, 2, start_ready)
        loop('list_one_cli', args.samples, 5, lambda m: cmd(m, 'jlist')[0])

        def restart_ready(m):
            previous = health(m)
            begin = time.perf_counter_ns()
            cmd(m, 'restart', 'bench-http')
            health(m, previous)
            return (time.perf_counter_ns() - begin) / 1e6

        loop('restart_to_http_ready', args.lifecycle_samples, 2, restart_ready)
        for m in managers:
            for i in range(9):
                cmd(m, 'start', str(HERE / 'idle.cjs'), '--name', f'idle-{i}', '--interpreter', node)
        loop('list_ten_cli', args.samples, 5, lambda m: cmd(m, 'jlist')[0])
        for m in managers:
            for i in range(9):
                cmd(m, 'delete', f'idle-{i}')

        baseline = dict(name='node-direct', port=17890)
        direct = subprocess.Popen([node, str(HERE / 'http.cjs')], env=dict(base_env, PMR_BENCH_PORT='17890'),
                                  stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        health(baseline)
        targets = [baseline] + managers
        for sample in range(-1, args.http_samples):
            for m in targets if sample % 2 == 0 else reversed(targets):
                seconds = 2 if sample < 0 else args.http_seconds
                print(f"http: {m['name']} sample {sample + 1}", flush=True)
                _, output = call([node, str(args.autocannon.resolve()), '-j', '-c', '20', '-p', '1',
                                  '-d', str(seconds), f"http://127.0.0.1:{m['port']}/"], base_env, seconds + 20)
                result = json.loads(output)
                # Remove absolute paths and hardware identifiers; keep all measurements.
                result.pop('title', None)
                if sample >= 0:
                    http_samples.append(dict(manager=m['name'], sample=sample + 1, result=result))
                    save()
                if result.get('errors', 0) or result.get('timeouts', 0) or result.get('non2xx', 0):
                    raise RuntimeError(f"HTTP errors for {m['name']}; raw measurements retained")
        metadata['status'] = 'complete'
    except BaseException as error:
        metadata['status'] = 'failed'
        metadata['error'] = str(error)
        for log in root.glob('*/logs/*'):
            if log.is_file():
                (args.output / (log.parent.parent.name + '-' + log.name)).write_bytes(log.read_bytes())
        raise
    finally:
        for m in managers:
            try:
                cmd(m, 'kill')
            except Exception as error:
                metadata.setdefault('cleanup_errors', []).append(str(error))
        if direct:
            direct.terminate()
            try:
                direct.wait(timeout=5)
            except subprocess.TimeoutExpired:
                direct.kill()
                direct.wait()
        metadata['load_average_end'] = os.getloadavg()
        metadata['finished_at_utc'] = datetime.datetime.now(datetime.timezone.utc).isoformat()
        save()
        if not metadata.get('cleanup_errors'):
            shutil.rmtree(root)
        else:
            print(f'Cleanup incomplete; isolated state retained at {root}', flush=True)
    summary = []
    for metric in dict.fromkeys(s['metric'] for s in samples):
        for m in managers:
            values = sorted(s['milliseconds'] for s in samples if s['metric'] == metric and s['manager'] == m['name'])
            summary.append(dict(metric=metric, manager=m['name'], n=len(values),
                                median_ms=statistics.median(values), p95_ms=values[math.ceil(.95 * len(values)) - 1],
                                min_ms=min(values), max_ms=max(values)))
    (args.output / 'summary.json').write_text(json.dumps(summary, indent=2) + '\n')
    print(json.dumps(summary, indent=2), flush=True)


if __name__ == '__main__':
    main()
