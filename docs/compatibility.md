# PM2 compatibility

PMR borrows PM2's CLI and process model. It is a separate implementation with its own daemon and transport; a familiar command is not a guarantee of identical behaviour.

| Area | Current evidence |
| --- | --- |
| Start, list, describe, stop, restart, delete and logs | Covered by integration tests |
| JSON/JavaScript ecosystem configuration, environment selection and saved state | Selected paths covered by integration tests |
| Automatic restarts, crash limits and stop exit codes | Covered by integration tests |
| Node cluster mode, readiness and reload | Selected lifecycle checks; validate traffic and graceful shutdown with your application |
| Modules, deploy hooks and alternate init systems | Implemented paths; not comprehensively validated in this release |
| PM2 Plus/Keymetrics, PM2 JavaScript API and wire-protocol interoperability | Not a supported compatibility guarantee |
| Windows | Unsupported; implementation uses Unix APIs |

## Moving a service

PMR limits `list` (including `ls`, `l`, `ps` and `status`) to 25 processes and
`logs` to 5 unique files by default, newest first. Use `--limit 0` for an unlimited
view. `--sort asc|desc` orders processes by ID and logs by file modification time
before applying the limit. `jlist` and `prettylist` remain unlimited. These display
defaults differ from PM2; they do not stop processes or remove log files.

`monit` starts in descending process ID order, with `--sort` to choose the initial
order and `o` to reverse it interactively. It retains the full process list.

Start with a non-production copy of your ecosystem file. Set a new `PMR_HOME`, remove inherited PM2 path overrides and choose unused application ports. Run lifecycle, crash recovery, readiness, log rotation and reload checks before moving traffic.

The default state directory is `~/.pmr`. If `PMR_HOME` is absent, `PM2_HOME` is honoured. Saved process lists use the filename `dump.pm2`. `resurrect` can fall back to `~/.pm2/dump.pm2` when neither home variable is set and no PMR dump is available. Set an explicit home to avoid accidentally restoring another manager's saved processes.

PMR and PM2 must not operate on the same home directory. Keep backups before importing saved state. JSON configuration avoids the Node dependency required to evaluate JavaScript configuration files.

PM2 documentation is a useful reference for the intended workflow, but PMR's `--help`, source and tests describe this implementation. Report a difference with a minimal reproduction, both versions and the expected behaviour.
