# pmr

**Keep your processes running. Keep your terminal useful.**

`pmr` is a process manager written in Rust for developers and coding agents. Start a service, follow its logs, inspect it as JSON and restart it when it fails, using a familiar PM2-style CLI.

The supervisor runs as a native binary. Your applications keep their own runtimes: Node.js, Python, shell scripts or compiled executables.

![pmr monitoring real demo services in a terminal](docs/assets/pmr-dashboard.png)

[Install](#install) · [Quick start](#quick-start) · [Benchmarks](benchmarks/README.md) · [Agent guide](docs/agents.md) · [Contribute](CONTRIBUTING.md)

## Why pmr?

- **One place for your services.** Name processes, group them into namespaces and check CPU, memory, status and logs from the terminal.
- **Useful to people and automation.** Use `pmr monit` interactively or `pmr jlist` for structured process state. Run a command once with `--no-autorestart` and check its exit code afterwards.
- **Recovery you can configure.** Automatic restarts, restart delays, exponential backoff and memory limits help you recover from process failures without an endless tight restart loop.
- **A familiar starting point.** PM2-style commands and ecosystem files make the workflow easy to recognise. See the [compatibility boundaries](docs/compatibility.md) before migrating a service.

## Install

macOS and Linux are the supported operating systems. Download the archive for your machine from [Releases](https://github.com/liammendes/pmr/releases). Every archive includes `pmr`, `pmr-runtime`, `pmr-docker`, `pmr-dev` and licence notices, with a SHA-256 checksum file alongside it.

| Machine | Archive target |
| --- | --- |
| Apple silicon Mac | `aarch64-apple-darwin` |
| Intel Mac | `x86_64-apple-darwin` |
| Linux x86-64 | `x86_64-unknown-linux-gnu` |
| Linux ARM64, including 64-bit Raspberry Pi OS | `aarch64-unknown-linux-gnu` |

Linux binaries are built on Ubuntu 24.04 and require glibc 2.39 or newer. On an older distribution, build from source. Raspberry Pi hardware has not yet been tested; use 64-bit Linux and validate your camera stack on the device. Windows is not supported in this release.

After downloading and extracting the archive:

```sh
mkdir -p "$HOME/.local/bin"
install -m 755 pmr pmr-runtime pmr-docker pmr-dev "$HOME/.local/bin/"
export PATH="$HOME/.local/bin:$PATH"
pmr --version
```

Add that PATH entry to your shell configuration if it is not there already. Keep the four binaries together so the companion commands can find the supervisor. These initial macOS binaries are unsigned and not notarised; use a local source build if your security policy requires it.

To build from source, install [Rust](https://www.rust-lang.org/tools/install), then:

```sh
git clone https://github.com/liammendes/pmr.git
cd pmr
cargo install --path . --locked
```

This repository pins its Rust toolchain and commits `Cargo.lock`. No crates.io or npm publication is required. Node.js is needed for Node applications and JavaScript ecosystem files; JSON configuration works without it.

## Quick start

From a checkout, start the small HTTP example:

```sh
pmr start examples/server.cjs --name api --interpreter node
curl http://127.0.0.1:3000
pmr list
pmr logs api --nostream --lines 20
pmr monit
```

Quit the monitor with `q`. The service continues running in the background.

```sh
pmr restart api
pmr jlist
pmr stop api
pmr delete api
```

For a Python worker or a compiled executable:

```sh
pmr start worker.py --name worker --interpreter python3
pmr start ./my-server --name server --interpreter none -- --port 8080
```

An ecosystem file keeps configuration together:

```sh
pmr start examples/ecosystem.config.json
pmr save
```

`save` writes the process list; `pmr resurrect` restores it. For reboot recovery, configure `pmr startup` for your operating system, review the generated setup command and then save the list. See [operations](docs/operations.md).

## Where it fits

**Local development that survives a terminal session.** Keep your API, frontend and queue worker running together, with names and logs you can find again.

**Coding agents with accountable background work.** Give each job a name, inspect its state and exit code, collect the relevant logs, then clean it up. A successful `start` means the command was launched; it does not mean a build or test passed. The [agent guide](docs/agents.md) shows the complete lifecycle.

**Self-healing camera services on Raspberry Pi.** Keep capture, streaming and upload workers supervised at the edge. If a process crashes, pmr can restart it; with startup configured, it can bring saved services back after a reboot. Spend less time SSHing into a camera to bring it back online. The [camera guide](docs/raspberry-pi-cameras.md) covers backoff, crash loops and application health checks.

**Small servers with a straightforward deployment model.** Run services under a dedicated user, retain useful logs and manage their lifecycle without adding a cluster control plane.

## Performance

The [benchmark report](benchmarks/README.md) compares pmr with PM2 6.0.14 and 7.0.4 on an Apple M1 Max MacBook Pro with 10 CPU cores and 32 GB RAM. It includes CLI latency, application readiness, a direct-Node HTTP baseline, exact software versions and every recorded sample.

Command latency and application throughput answer different questions. A faster process-list command does not make your application faster. Results describe this machine and workload, and can be reproduced with the included harness.

## Status and compatibility

This is an early community release. Tests cover process lifecycle, logs, ecosystem configuration, saved state, crash handling, selected Node cluster behaviour and the local web/static-server commands. That is a useful baseline, not exhaustive PM2 parity or production certification.

PMR uses `~/.pmr` by default. `PMR_HOME` takes precedence over `PM2_HOME`; several PM2 path overrides are also recognised. Keep PMR and PM2 in separate homes. Read the [compatibility guide](docs/compatibility.md) for migration details and unverified features.

## Contributing and licence

Small fixes, clear bug reports, reproducible benchmarks and platform testing are welcome. Start with [CONTRIBUTING.md](CONTRIBUTING.md). Please follow the [Code of Conduct](CODE_OF_CONDUCT.md); report security issues using [SECURITY.md](SECURITY.md).

Created by [Liam Mendes](https://github.com/liammendes). PMR is a Rust port inspired by [PM2](https://github.com/Unitech/pm2), and credits its authors and contributors in [NOTICE](NOTICE). PMR is independently maintained and is not affiliated with or endorsed by PM2 or Keymetrics.

Licensed under [GNU AGPL version 3](LICENSE). Dependencies retain their own licences.
