# Changelog

## 1.1.0 — 2026-10-16

- Add indexed SQLite command history, disk-backed one-shot queues, shared concurrency limits, opt-in retention and restart recovery.
- Add checksum-verified GitHub self-updates without restarting the running daemon.
- Require a Semantic Version bump for every PR. Centre the README header.

- Add `logs --live`: a terminal view that automatically follows the newest process, shows its latest output and exit status, and retains the last result after completion.
- Bound the live viewer's output and event queue, read file tails without loading whole logs, and add a lightweight latest-process lookup for updated daemons.
- Apply ID-based list limits inside updated daemons before refreshing metrics or cloning environments; tail ordinary logs with backwards file reads and cache only the selected process's logs in `monit`.
- Limit `ls` and its aliases to 25 processes and `logs` to 5 unique log files by default. Add `--limit`, with `0` for an unlimited view.
- Support `--sort asc|desc` for process ID order in `ls` and file modification time in `logs`, newest first by default.
- Apply sorting in `monit`, with a visible order indicator and an `o` shortcut to reverse it while keeping the selected process.

## 1.0.0 — 2026-10-15

Initial community packaging of the existing PMR implementation.

- Rust CLI and supervisor with PM2-style lifecycle commands, ecosystem files, restart policies, logs, persistence and terminal monitoring.
- Companion `pmr-runtime`, `pmr-docker` and `pmr-dev` commands.
- Installation, migration, agent and Raspberry Pi camera guides; contribution and security policies.
- Reproducible PM2 comparison benchmarks, with hardware details and raw samples.
- CI and release automation for macOS and Linux on x86-64 and ARM64.
- Formatting and Clippy fixes; explicit non-truncating open for the daemon lock file.
- AGPL-3.0-only licence, upstream attribution and locked dependencies.

This is an early release despite the inherited 1.0.0 version number. Platform builds and integration checks do not establish exhaustive PM2 parity.
