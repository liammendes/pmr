# Showcase assets

The README dashboard is captured from the real `pmr monit` interface using local demo processes. `demo.cjs` serves HTTP requests and performs small checksum jobs. The `camera-sim` role produces synthetic frames and deliberately crashes once to demonstrate process recovery. It is not Raspberry Pi footage or a camera benchmark.

Use a fresh temporary directory and an isolated `PMR_HOME`. Copy `demo.cjs` into that directory, start roles `api`, `worker`, `camera-sim` and `docs` with the Node interpreter, and set their working directory to the temporary directory. The HTTP roles listen on loopback ports 18321 and 18322. Keep these ports unused by other applications.

The screenshot and launch composition contain only PMR's own interface and original artwork. Third-party inspiration screenshots are not distributed. Stop the isolated daemon after capture.

## Reproduce the launch assets

From `media/video`, run `npm ci`, `npm run studio`, `npm run still` and `npm run render`. Run `npm run render:reduced` for the reduced-motion version. Remotion, React and font versions are pinned in `package-lock.json`. The original dashboard captures live in `public/`; the TypeScript composition adds the layout and motion. Output is written to the ignored `out/` directory. Both MP4s are silent, 40 seconds, 1920 × 1080, 60 fps.

The two JPEG screenshots were captured through the Computer plugin from the real native terminal. The camera demo recovered from a deliberate process exit; no Raspberry Pi or physical camera was used. See [the draft and transcript](../docs/launch-draft.md).

## Identity and typography

The PMR wordmark and icon share Liam's vertical blue gradient: `#0080FF` to `#0057FF`, with Display P3 values retained in the SVG sources. Manrope provides the headings and supporting text in `#000000`. Liam’s current Paper layout places the official Liam Mendes logo at the film’s bottom-right (24 px right, 24.019 px bottom) and the poster’s top-left (32 px each). The intro keeps its 100 px left edge and pairs the wordmark with a 256 px icon rotated −13.86°. The poster centres the wordmark, 56 px heading and 20 px description, without an icon.

The final icon is built in Apple Icon Composer. Open [PMR.icon](brand/PMR.icon) to edit the separate blue face, terminal prompt and status jewel layers. Its exported PNG is used in Paper and Remotion. The wordmark remains an editable SVG. [Palette values](brand/palette.json) and [asset provenance](asset-provenance.json) record the sources. The font files include their licences. See [DESIGN.md](../DESIGN.md) for type sizes, spacing and motion timings.

The static poster presents the project identity; the written transcript covers all six scenes without motion. The reduced-motion video replaces animated tilts, movement and scaling with short opacity transitions, and uses a still frame in place of the scrolling agent logs. The normal video uses one shared wordmark transition, gentle screenshot entrances and proportionate benchmark bars; displayed measurements never count through invented values.

## AI agents and background workers

The final two scenes run from 24–32 seconds and 32–40 seconds. [use-cases.cjs](use-cases.cjs) supplies six isolated local demo roles. `agent-planner`, `agent-builder` and `agent-reviewer` emit clearly labelled scripted agent events; they do not invoke a model API. `report-worker`, `archive-worker` and `checksum-worker` perform real JSON, gzip and SHA-256 work on generated fixture records. Their output stays in `showcase-output/` beneath the chosen working directory.

For each scene, create a fresh temporary working directory and a separate `PMR_HOME`. Start the relevant roles with `pmr start /absolute/path/to/use-cases.cjs --name ROLE --interpreter node --cwd /temporary/workdir -- ROLE`. Capture `pmr logs` for the agent group and `pmr monit` for the worker group. Stop the isolated daemon with `pmr kill` after recording. These commands must use the same isolated `PMR_HOME`; never use the everyday supervisor for demo cleanup.

The log clip consists of timestamped Computer-plugin captures of the native Terminal window, sampled about once per second and encoded at 30 fps. Its first eight seconds retain that timing. Cropping removes window chrome and the empty cursor row; no terminal content is reconstructed. The worker still is also a native Terminal capture, cropped to the dashboard panels. Source captures show PMR 1.0.0. The reduced-motion version holds a frame from the log clip.
