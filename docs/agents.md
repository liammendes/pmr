# Process management for coding agents

Use pmr to give background work a name, retain its logs and inspect its result. The same CLI works for an interactive developer and an agent running commands.

## A command that runs once

From your project directory:

```sh
pmr start cargo --name check-unique-id --interpreter none --no-autorestart -- test
pmr logs check-unique-id --nostream --lines 100
pmr jlist
```

Find the entry by name. Read `pm2_env.status` and `pm2_env.exit_code`; a stopped job with exit code 0 completed successfully. If it is still online, it has not finished. The return code from `pmr start` confirms launch, not the outcome of `cargo test`.

After checking the result and retaining any needed diagnostics:

```sh
pmr delete check-unique-id
```

## A long-running service

```sh
pmr start npm --name frontend-unique-id --interpreter none -- run dev
pmr logs frontend-unique-id --nostream --lines 40
```

Check the application's health endpoint or actual output before declaring it ready. An `online` process can still be starting up or serving errors. Remove only the process you own when the task is finished.

## Isolation and output

Set a dedicated `PMR_HOME` for an independent session, and use it for every subsequent command. Avoid inherited `PM2_*` path overrides. Never point PMR at a live PM2 directory; do not call `delete all` or `kill` in a shared manager home.

`jlist` contains detailed configuration, including environment values. Parse it locally and return only the fields needed by the task. Use `--nostream` when an agent needs a bounded log read, and use `--no-autorestart` for one-off checks, builds and migrations.
