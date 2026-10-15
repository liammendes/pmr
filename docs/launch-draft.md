# X launch thread

Published on 16 October 2026 from [@liammendess](https://x.com/liammendess), using X’s **Post all** thread composer. All four posts were verified live and linked in order. The repository became public on 16 October 2026 at Liam’s request; access was verified without authentication.

[Read the published thread](https://x.com/liammendess/status/2110876284813214052). The 40-second launch video is attached to the first post.

## Thread

### [Post 1](https://x.com/liammendess/status/2110876284813214052)

Introducing pmr, a familiar process manager with PM2-style commands

Dev servers, background workers, coding agents... keep them running, see the logs, recover when they crash.

Try now: https://github.com/liammendes/pmr

### [Post 2](https://x.com/liammendess/status/2110876287682355629)

Give your coding agents observability into what's going on.

They can check whether that build actually finished, read the logs, see what's still running & clean up after themselves.

Give your Agents the process management experience they deserve!!!!

### [Post 3](https://x.com/liammendess/status/2110876290308092178)

Listing 10 processes as JSON on my M1 Max... unbelievable difference.

pmr 1.0.0: 14.7ms
PM2 7.0.4: 100.1ms

30 runs each, medians. CLI latency.
MacBook Pro 16", 10 cores, 32GiB RAM.

Raw results: https://github.com/liammendes/pmr/tree/main/benchmarks

### [Post 4](https://x.com/liammendess/status/2110876294355530089)

Try it out today! Use pmr for your home-made cameras, long running processes and jobs where you need to see what ran and how it ended.

https://github.com/liammendes/pmr/

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
