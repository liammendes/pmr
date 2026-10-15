# Contributing to pmr

Thanks for taking a look. A small, well-explained change is a good place to start.

## Set up

Use macOS or Linux, Rust via rustup, Node.js and curl. The toolchain is pinned in `rust-toolchain.toml`; Cargo uses the checked-in lockfile.

```sh
git clone https://github.com/liammendes/pmr.git
cd pmr
cargo build --locked
cargo fmt --all -- --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked -- --test-threads=1
```

The integration tests start real processes in temporary manager homes. They need Node.js and curl. Run them serially because some fixtures use fixed ports. Use `target/debug/pmr` when testing changes so you do not replace an installed supervisor.

## Make a change

Open an issue first for a new feature or a change to process-management behaviour. For a clear bug or documentation fix, a pull request is fine.

Explain the problem, show the resulting behaviour and say how you checked it. Add a regression test when changing lifecycle, signals, environment handling or persistence. Keep documentation examples runnable. If a command looks compatible with PM2 but behaves differently, document the difference instead of hiding it.

Use an isolated home for manual testing:

```sh
export PMR_HOME="$(mktemp -d /tmp/pmr-dev.XXXXXX)"
export PM2_HOME="$PMR_HOME"
target/debug/pmr start examples/server.cjs --name demo
target/debug/pmr logs demo --nostream
target/debug/pmr kill
```

Before testing, unset inherited `PM2_*` path overrides that could point at another manager. Stop the isolated daemon before removing its directory. Do not run broad cleanup commands against a shared or production home.

## Report a bug

Include your OS and architecture, pmr version, a minimal configuration, the command you ran, expected behaviour and the relevant error. Strip credentials, environment secrets and personal paths from logs and JSON output before sharing them.

Raspberry Pi and older Linux testing are particularly useful. Please include the device, OS version, architecture and whether you built from source. See [benchmarks](benchmarks/README.md) for performance reports; share raw samples and a baseline, including results where pmr is slower.

## Code and review

`src/bin/pmr.rs` defines the CLI. `client.rs`, `protocol.rs` and `daemon.rs` handle communication. `god.rs` manages process lifecycle. `config.rs` parses app definitions; `startup.rs` handles persistence and startup integration. `tui.rs` and `ux.rs` render terminal output.

AI-assisted contributions are welcome. The contributor remains responsible for understanding the code, its licence, and its behaviour. Include meaningful validation and do not invent test results.

By submitting a contribution, you agree to license it under this project's AGPL-3.0-only licence. Preserve existing attribution. There is no separate contributor licence agreement.
