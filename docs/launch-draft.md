# X launch draft

Draft only. Nothing has been posted. The repository remains private. These posts are written for a future public launch; publish them only after Liam separately decides to make the repository public. Do not use a public launch link while access remains private.

## Recommended main post

Attach the static poster for an image-led announcement, or the 40-second video as an alternative. Use one, not both.

I built pmr: a Rust process manager for developers and coding agents.

Start services, inspect them as JSON and restart them when they crash. Familiar PM2-style commands, with a native supervisor.

Source, downloads and benchmarks: https://github.com/liammendes/pmr

## Optional replies, in order

### Reply 1

On my M1 Max (10 CPU cores, 32 GB RAM), listing 10 processes took 14.7 ms in pmr vs 100.1 ms in PM2 7.0.4: a 6.8× median-time ratio.

30 samples each, on a working desktop. That is CLI latency, not a promise that your app runs faster. Raw results are in the repo.

### Reply 2

One use case I love: self-healing camera services on Raspberry Pi.

Supervise capture and upload workers. Restart crashes. Restore saved services after reboot setup.

The demo uses synthetic frames on a Mac; physical Pi testing is still needed.

### Reply 3

For coding agents, a successful start only means the command launched.

Name the job, check its exit code, read the logs and clean up afterwards. pmr gives humans and agents the same process state.

Early release. Bug reports and reproducible tests welcome.

## Image alt text

Rounded lowercase pmr lettering in a deep blue gradient is centred above the heading and description. The official Liam Mendes logo appears at the top left. The heading reads “Keep your processes running.” A real terminal dashboard shows four local demo services online: api, worker, camera-sim and docs, with process state, logs and resource use. A Rust process manager for developers and coding agents, built by Liam Mendes.

## Video transcript and description

The video is silent, 40 seconds long, 1920 × 1080 at 60 fps. All meaning is in the on-screen text. A reduced-motion version and static poster are available.

- 0–4 seconds: Deep blue pmr lettering and a smaller, tilted terminal icon, with the official Liam Mendes logo at the bottom right. “Keep your processes running. A Rust process manager for developers and coding agents.”
- 4–10 seconds: “Every process. One clear view.” Names, logs and resource use, ready for people and agents. A real `pmr monit` dashboard shows four isolated local demo services.
- 10–17 seconds: “A faster process list.” Median JSON-list latency for 10 processes: pmr 1.0.0, 14.7 ms; PM2 7.0.4, 100.1 ms. PM2/pmr median command-time ratio: 6.8×. Thirty samples each on an M1 Max with 10 CPU cores, 32 GiB RAM, macOS 27.2 and Node 26.4.0. Measured on a working desktop on 15 October 2026. Application speed varies; full method, p95 and samples are in the benchmark report.
- 17–24 seconds: “Self-healing camera services.” Supervise Raspberry Pi capture and upload workers; recover from process crashes. A real dashboard shows camera-sim online after one restart. The demo uses synthetic frames on macOS; Raspberry Pi hardware is untested. Reboot recovery requires startup setup.

- 24–32 seconds: “Supervise your AI agents.” Run named agent workers and follow their inputs, steps and results. A real `pmr logs` capture shows three local processes: agent-planner, agent-builder and agent-reviewer. Agent events are scripted demo fixtures; no model API calls or actual AI decisions are shown.
- 32–40 seconds: “Keep background jobs running.” A real PMR dashboard shows report, archive and checksum workers online. The processes write JSON reports, gzip archives and SHA-256 checks locally; the dashboard displays completed-job and processed-record metrics.

## Assets

- [Static poster](assets/pmr-launch.png)
- [40-second MP4](assets/pmr-launch.mp4)
- [Reduced-motion MP4](assets/pmr-launch-reduced-motion.mp4)
- [Deep blue wordmark](assets/pmr-wordmark.svg)
- [Terminal icon](assets/pmr-icon.png)
- [Original dashboard screenshot](assets/pmr-dashboard.jpg)
- [Camera recovery screenshot](assets/pmr-camera-recovery.jpg)
- [Editable composition](../media/video/src/Launch.tsx)
- [Full benchmark report](../benchmarks/README.md)

## Editorial choices

Lead with the job the tool does, show its real interface, then explain one measured advantage. Keep the caveat next to the number. Invite specific testing rather than implying exhaustive PM2 compatibility. The poster and video contain original composition and PMR screenshots only; no third-party post images are reused. Research notes and the personal bookmark archive stay outside this repository.
