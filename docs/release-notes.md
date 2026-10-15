# pmr 1.0.0

Initial community packaging of Liam Mendes's Rust process manager, with a PM2-style CLI for developers and coding agents.

The release includes four companion executables, installation and operations guides, examples, an agent workflow, a Raspberry Pi camera supervision guide, and reproducible benchmarks against PM2 6.0.14 and 7.0.4.

Download the archive matching your OS and architecture. Verify its SHA-256 checksum before installing. macOS binaries are unsigned and not notarised. Linux binaries are built on Ubuntu 24.04 and require glibc 2.39 or newer; build from source on older distributions. Raspberry Pi hardware remains untested.

This is an early release. Read the compatibility guide before migrating a PM2 workload. The inherited version number does not imply exhaustive PM2 parity or production certification.

Binary archives include AGPL and dependency notices. The corresponding-source archive includes vendored Cargo dependencies for an offline build with the pinned Rust toolchain. See CONTRIBUTING.md and SECURITY.md for reporting issues.
