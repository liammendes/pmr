# Running services

PMR manages processes under its own user account. Use a dedicated account for server workloads and keep its state directory on persistent storage.

## Restarts and readiness

Automatic restarts are enabled by default. Configure `restart_delay` or `exp_backoff_restart_delay`, `min_uptime` and `max_restarts` to suit your workload. A process that repeatedly dies before the minimum uptime can reach the restart limit and remain errored. Investigate the logs rather than restarting it forever.

For a Node application using `wait_ready`, send `process.send('ready')` only after dependencies and listening sockets are ready. Handle termination signals and close your listeners. Test reloads under traffic; a running PID alone does not establish availability.

## Restore after reboot

Start the desired services, run `pmr save`, then run `pmr startup` for the detected init system. Review and execute the generated setup command with the privileges it requests. `pmr startup systemd` is the Linux/systemd path; macOS uses launchd.

Keep the service user, executable path and `PMR_HOME` consistent between interactive commands and startup. Save again after changing the process list. After a controlled reboot, verify both `pmr list` and the application's health endpoint. This repository's local tests exercise save/resurrect, not a physical reboot.

## Logs and secrets

Use `pmr logs NAME --nostream --lines 100` for a bounded read. Plan log retention for unattended machines; disk exhaustion can stop an otherwise healthy service. Dump files and JSON diagnostics can contain inherited environment values, so restrict access and redact them before sharing.

## Companion commands

- `pmr-runtime`: foreground supervision for environments that manage the main process.
- `pmr-docker`: companion entry point for the runtime workflow.
- `pmr-dev`: development-oriented process execution.

Keep these executables alongside `pmr`. See each command's `--help` for its options. Container image publication is not part of this release.
