# Benchmarks: pmr and PM2

Measured on **15 October 2026**, using the same Node.js fixtures and isolated manager homes. The main result is lower management-command latency on this machine. It is not a claim that every application or deployment becomes faster.

## Hardware and software

| Item | Configuration |
| --- | --- |
| Computer | MacBook Pro, MacBookPro18,2 |
| CPU | Apple M1 Max, 10 cores: 8 performance + 2 efficiency |
| Memory | 32 GiB |
| Architecture | ARM64 |
| OS | macOS 27.2, build 26B5112a |
| PMR | 1.0.0, optimized Cargo release build |
| Rust | 1.98.1, commit 48a229cea |
| Node.js | v26.4.0 for every Node application and PM2 CLI |
| PM2 | 6.0.14 (installed comparator), 7.0.4 (npm latest when checked) |
| Load generator | autocannon 7.15.0 |
| PMR source | `7edd0a1c550e6a12dc526dbfa34d5716ec7afd8d` |

The run began at 20:28:20 UTC and finished at 20:31:03 UTC. This was a working desktop, not a dedicated idle benchmark machine. Load averages were 8.01 / 7.87 / 15.09 at the start and 7.49 / 7.67 / 13.76 at the end. Background applications stayed open; CPU affinity, thermal state and power mode were not controlled. Compilation and video rendering were kept outside the measurement window. These conditions limit extrapolation, especially for HTTP throughput.

The raw file records the binary and fixture SHA-256 hashes, software versions, source commit, source diff hash, configuration, every sample and cleanup status. Later packaging, documentation and media changes do not change the measured Rust implementation.

## Management latency

Lower is better. Each cell is **median / p95 in milliseconds**. p95 uses the nearest-rank method; with 15 samples it equals the observed maximum, not a confidence bound.

| Operation | Samples per manager | pmr | PM2 6.0.14 | PM2 7.0.4 |
| --- | ---: | ---: | ---: | ---: |
| CLI help | 30 | 11.06 / 13.25 | 100.11 / 104.55 | 91.63 / 98.70 |
| Daemon startup + ping | 15 | 65.48 / 67.28 | 308.79 / 392.90 | 294.15 / 306.26 |
| JSON list: 0 processes | 30 | 14.81 / 18.21 | 102.99 / 140.41 | 96.05 / 120.25 |
| Start → HTTP ready | 15 | 141.35 / 162.52 | 204.43 / 220.26 | 173.80 / 223.60 |
| JSON list: 1 process | 30 | 13.97 / 17.28 | 104.62 / 107.31 | 96.93 / 100.21 |
| Restart → new PID + HTTP ready | 15 | 133.87 / 158.79 | 328.81 / 350.08 | 300.61 / 307.50 |
| JSON list: 10 processes | 30 | 14.70 / 16.75 | 108.06 / 110.99 | 100.09 / 105.18 |

For `jlist` with 10 processes, PM2 7.0.4 took **6.81×** pmr's median time (100.09 / 14.70 ms); PM2 6.0.14 took **7.35×**. The smaller application-start difference is also shown: pmr took 141.35 ms versus 173.80 ms for PM2 7.0.4, a 1.23× ratio. We do not generalise the largest ratio to the whole product.

`--help` and `jlist` measure wall time from launching the CLI until it exits successfully, with stdout/stderr captured. `ping` measures daemon startup after shutdown, with no application; it does not mean cold filesystem caches or a freshly booted machine. Start and restart measure command launch through successful HTTP readiness. Restart also requires a different application PID. Readiness is checked after CLI completion, with a 5 ms polling interval, so those numbers include command and observation overhead.

## HTTP throughput

One fork-mode Node process per manager serves the same small JSON response on loopback. The direct-Node baseline runs the identical fixture without a manager. Higher requests per second is better.

| Manager | Median requests/s | Minimum–maximum across runs |
| --- | ---: | ---: |
| node-direct | 56624 | 56227–56842 |
| pmr | 56323 | 47779–57590 |
| pm2-6.0.14 | 42224 | 41539–42986 |
| pm2-7.0.4 | 43299 | 38723–43523 |

Each row has three five-second runs after a two-second warmup, using 20 connections and pipelining 1. All 12 recorded runs had **zero reported errors, timeouts and non-2xx responses**. The lower PM2 throughput in this sample is reported rather than omitted, but three short desktop runs are not enough to establish a general application-throughput advantage. PMR's third run also fell substantially below its first two. Longer runs on a controlled host are needed before making production-capacity claims.

No cluster throughput, reload availability, camera hardware, disk-intensive workload or memory-footprint claim is made from this test.

## Method

The harness alternates forward and reverse manager order on successive samples. It uses five warmups and 30 measured samples for each CLI query; lifecycle tests use two warmups and 15 measured samples. All managers use default restart behaviour and the same fixtures; the ten-process list contains one HTTP application and nine idle Node processes.

Every manager gets a fresh temporary home. A minimal environment prevents the supervising shell or another process manager from injecting app configuration. Setup, process deletion and final cleanup are outside the timed sections. No samples are trimmed or selected for being fast. A failed command or readiness check aborts the run and retains diagnostic output. Preliminary harness rehearsals exposed empty-list cleanup and inherited-environment issues; those were fixed before this complete run and their timings are not mixed into the reported results.

## Reproduce

Use Python 3.10+, Node.js and npm. From the repository root:

```sh
cargo build --release --locked
npm install --prefix .bench-pm2-6 --no-audit --no-fund pm2@6.0.14
npm install --prefix .bench-pm2-7 --no-audit --no-fund pm2@7.0.4
npm install --prefix .bench-load --no-audit --no-fund autocannon@7.15.0
python3 benchmarks/run.py \
  --pmr target/release/pmr \
  --pm2 .bench-pm2-6/node_modules/pm2/bin/pm2 \
  --pm2 .bench-pm2-7/node_modules/pm2/bin/pm2 \
  --autocannon .bench-load/node_modules/autocannon/autocannon.js \
  --output benchmarks/results/my-machine
```

Ports 17880–17882 and 17890 must be free. The harness checks availability before starting. It never uses your default PMR/PM2 home and cleans up its own daemons. Keep unrelated workload low when making comparisons; report any changed settings. npm top-level versions are pinned above, but these commands resolve transitive dependencies at install time. Keep your generated npm lockfiles with any new result if you need exact dependency reproduction.

Read [run.py](run.py), [the HTTP fixture](http.cjs) and [the idle fixture](idle.cjs) before running. Share hardware, software, configuration and raw output alongside any conclusion.

## Data

- [Raw measurements and environment](results/2026-10-15-m1-max/raw.json)
- [Latency samples as CSV](results/2026-10-15-m1-max/latency.csv)
- [Summary statistics](results/2026-10-15-m1-max/summary.json)

PM2 is an independently maintained project; see its [source](https://github.com/Unitech/pm2) and [documentation](https://pm2.keymetrics.io/docs/usage/quick-start/). This comparison does not imply affiliation or endorsement.
