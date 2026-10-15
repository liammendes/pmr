# Showcase assets

The README dashboard is captured from the real `pmr monit` interface using local demo processes. `demo.cjs` serves HTTP requests and performs small checksum jobs. The `camera-sim` role produces synthetic frames and deliberately crashes once to demonstrate process recovery. It is not Raspberry Pi footage or a camera benchmark.

Use a fresh temporary directory and an isolated `PMR_HOME`. Copy `demo.cjs` into that directory, start roles `api`, `worker`, `camera-sim` and `docs` with the Node interpreter, and set their working directory to the temporary directory. The HTTP roles listen on loopback ports 18321 and 18322. Keep these ports unused by other applications.

The screenshot and launch composition contain only PMR's own interface and original artwork. Third-party inspiration screenshots are not distributed. Stop the isolated daemon after capture.

## Reproduce the launch assets

From `media/video`, run `npm ci`, `npm run studio`, `npm run still` and `npm run render`. Run `npm run render:reduced` for the reduced-motion version. Remotion, React and font versions are pinned in `package-lock.json`. The original dashboard captures live in `public/`; the TypeScript composition adds the layout and motion. Output is written to the ignored `out/` directory. Both MP4s are silent, 24 seconds, 1920 × 1080, 60 fps.

The two JPEG screenshots were captured through the Computer plugin from the real native terminal. The camera demo recovered from a deliberate process exit; no Raspberry Pi or physical camera was used. See [the draft and transcript](../docs/launch-draft.md).

## Identity and typography

The Prism identity pairs original, sculpted polychrome lettering with a glass-and-porcelain terminal icon. Cyan and blue, violet and pink, and coral and amber give each letter its own colour. Manrope provides the headings and supporting text. The art is generated, not a photograph of a physical object.

Generation prompts and capture origins are preserved in [asset provenance](asset-provenance.json) and embedded in the PNG metadata. The font files include their licences. See [DESIGN.md](../DESIGN.md) for colours, type sizes, spacing and motion timings.

The static poster and written transcript convey the same information without motion. The reduced-motion video replaces camera tilts, movement and scaling with short opacity transitions. The normal video uses one shared wordmark transition, gentle screenshot entrances and proportionate benchmark bars; displayed measurements never count through invented values.
