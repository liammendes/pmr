# pmr — Production Process Manager in Rust

`pmr` is a full-featured Rust port of [PM2](https://pm2.keymetrics.io/), providing an asynchronous background daemon, Unix domain socket RPC & Pub/Sub event bus, process supervision, Node.js & multi-language cluster/fork modes, zero-downtime reloads, terminal dashboards, static HTTP/SPA serving, and system startup script generation.

## Binaries

Building `pmr` produces four binaries:
- **`pmr`** — Main CLI and background daemon supervisor (drop-in replacement for `pm2`).
- **`pmr-runtime`** — Foreground container/production supervisor (drop-in replacement for `pm2-runtime`).
- **`pmr-docker`** — Alias for `pmr-runtime` (drop-in replacement for `pm2-docker`).
- **`pmr-dev`** — Development foreground runner with file watching enabled by default (drop-in replacement for `pm2-dev`).

## Key Features

- **Process Supervision (`God` Daemon)**
  - Automatic crash recovery (`autorestart`), `min_uptime`, `max_restarts`, exponential backoff (`exp_backoff_restart_delay`), and `stop_exit_codes`.
  - Memory ceiling monitoring (`max_memory_restart`, e.g. `150M`, `1G`) and cron-scheduled restarts (`cron_restart`).
  - File watching (`--watch`, `ignore_watch`, `watch_delay`) powered by `notify`.
  - Recursive process tree termination (`treekill`) with configurable `kill_timeout`, `wait_ready`, and `listen_timeout`.
- **Cluster & Fork Modes**
  - Multi-instance scaling (`-i max`, `-i <N>`, `pmr scale <app> <N>`).
  - Cross-platform TCP round-robin cluster load-balancer and Node.js IPC preload shim supporting `process.send('ready')`, graceful `shutdown` messages, custom AXM metrics (`axm:monitor`), custom AXM actions (`axm:action`), and seamless port sharing on Linux and macOS.
  - Zero-downtime sequential `pmr reload <target>`.
- **Ecosystem Configuration**
  - Supports `.json`, `.json5`, `.yaml`/`.yml`, `.toml`, and `.config.js`/`.cjs`/`.mjs` files with `--env <profile>` (`env_<profile>`) merging.
- **Logs & Real-Time Terminal UI**
  - `pmr ls` / `pmr status` (rich Unicode box tables + `--minimal`, `--sort`), `pmr jlist`, `pmr prettylist`, `pmr describe <id>`.
  - `pmr logs` with real-time Pub/Sub streaming, `--lines`, `--err`, `--out`, `--json`, `--timestamp`, `--nostream`, `pmr flush`, and `pmr reloadLogs`.
  - `pmr monit` / `pmr dashboard` (4-pane `ratatui` TUI) and `pmr imonit`.
- **Built-in HTTP Servers, Deployment & OS Integration**
  - `pmr serve [path] [port]` with `--spa`, Basic Auth, and CORS.
  - `pmr web` JSON system & process monitoring endpoint (`:9615`).
  - `pmr save` (`dump.pmr`), `pmr resurrect`, `pmr cleardump`.
  - `pmr startup` / `pmr unstartup` (`systemd`, `launchd`, `upstart`, `openrc`) and `pmr logrotate`.
  - `pmr deploy` SSH Git deployment workflow & `pmr install` / `pmr uninstall` module system.

## Build & Test

```bash
cargo build --release
cargo test
```
