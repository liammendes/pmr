# pmr 1.1.0

This release adds durable one-shot command history, bounded scheduling and quieter default terminal views.

- `pmr run` queues commands on disk and retains their metadata and output in indexed SQLite history. Use `history` to search, page, inspect, read output and cancel commands.
- Running children share a default limit of 64, with up to 10,000 pending commands. Both limits are configurable.
- History expiry is opt-in. Each command retains a rolling 1 MiB output tail by default; byte, age and completed-record policies are configurable.
- `ls` defaults to 25 processes and `logs` to five log files. Both accept `--limit` and `--sort asc|desc`. `monit` supports the `o` sorting shortcut.
- `logs --live` follows the newest service or queued command's result, with optional `--lines`.
- `self-update` installs newer GitHub releases after checksum validation and leaves the running daemon untouched. Every PR must now advance the Semantic Version.
- The README heading, wordmark and tagline are centred.

Existing service logs remain untouched. History starts recording with the new daemon; old logs are not imported as reconstructed command executions. Interrupted commands are not replayed automatically. Read docs/scaling.md for recovery behaviour, output limits and reproducible checks.

Download the archive for your platform and verify its checksum. macOS binaries remain unsigned and not notarised. Linux release binaries require glibc 2.39 or newer; build from source on older systems. Raspberry Pi hardware remains untested. This release does not establish exhaustive PM2 parity or certify an unbounded concurrency level.
