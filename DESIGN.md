---
name: "PMR launch media"
description: "Prism lettering and real process evidence on a warm porcelain stage."
colors:
  paper: "#faf9f6"
  ink: "#18213b"
  secondary: "#51596e"
  blue: "#254bea"
  track: "#dde2ee"
  pm2-gray: "#767d90"
typography:
  display:
    fontFamily: "Manrope, sans-serif"
    fontSize: "86px"
    fontWeight: 750
    lineHeight: 1.1
    letterSpacing: "-.035em"
  hero-headline:
    fontFamily: "Manrope, sans-serif"
    fontSize: "74px"
    fontWeight: 750
    lineHeight: 1.1
    letterSpacing: "-.035em"
  camera-headline:
    fontFamily: "Manrope, sans-serif"
    fontSize: "80px"
    fontWeight: 750
    lineHeight: 1.1
    letterSpacing: "-.035em"
  poster-headline:
    fontFamily: "Manrope, sans-serif"
    fontSize: "72px"
    fontWeight: 750
    lineHeight: 1.1
    letterSpacing: "-.035em"
  body:
    fontFamily: "Manrope, sans-serif"
    fontSize: "28px"
    fontWeight: 400
    lineHeight: 1.45
  poster-body:
    fontFamily: "Manrope, sans-serif"
    fontSize: "30px"
    fontWeight: 400
  metric:
    fontFamily: "Manrope, sans-serif"
    fontSize: "34px"
    fontWeight: 650
  label:
    fontFamily: "Manrope, sans-serif"
    fontSize: "22px"
    fontWeight: 400
  credit:
    fontFamily: "Manrope, sans-serif"
    fontSize: "23px"
    fontWeight: 400
  command:
    fontFamily: "monospace"
    fontSize: "28px"
    fontWeight: 400
    lineHeight: 1.45
rounded:
  screenshot: "16px"
  bar: "12px"
spacing:
  content-inset: "100px"
  screenshot-inset: "90px"
  bar-label-gap: "17px"
  bar-row-gap: "58px"
components:
  scene-heading:
    textColor: "{colors.ink}"
    typography: "{typography.display}"
  caption:
    textColor: "{colors.secondary}"
    typography: "{typography.body}"
    width: "1720px"
  screenshot:
    rounded: "{rounded.screenshot}"
    width: "1740px"
  screenshot-poster:
    rounded: "{rounded.screenshot}"
    width: "1496.4px"
  benchmark-track:
    backgroundColor: "{colors.track}"
    rounded: "{rounded.bar}"
    height: "60px"
    width: "1720px"
  benchmark-pmr:
    backgroundColor: "{colors.blue}"
    rounded: "{rounded.bar}"
    height: "60px"
  benchmark-comparator:
    backgroundColor: "{colors.pm2-gray}"
    rounded: "{rounded.bar}"
    height: "60px"
  footer:
    textColor: "{colors.secondary}"
    typography: "{typography.label}"
    width: "1720px"
  hero-heading:
    textColor: "{colors.ink}"
    typography: "{typography.hero-headline}"
---

# Design System: PMR launch media

## Overview

**Creative North Star: "Prism on porcelain"**

Warm porcelain gives the sculpted Prism wordmark and terminal icon room to carry the colour. Dark ink, clear Manrope headings and generous open space keep the product evidence easy to read. The material finish is playful and tactile; the writing stays direct, warm and professional.

Scope: Liam Mendes’s personal PMR launch media in media/video, comprising the 24-second film, reduced-motion film and static poster. This is the shipped React/Remotion system recorded from media/video/src/Launch.tsx; the native Rust CLI has its own interface. The original generated artwork provides the dimensional identity, while real terminal captures supply the product evidence.

**Key Characteristics:**

- Original polychrome acrylic-and-glass artwork, with soft contact shadows.
- Warm, quiet canvas and large dark headings.
- Real dashboard evidence and stable, explicitly labelled measurements.
- One shared identity transition, restrained entrances and accessible alternatives.

The implementation is in [Launch.tsx](media/video/src/Launch.tsx), registered by [index.tsx](media/video/src/index.tsx). [PRODUCT.md](PRODUCT.md) records the durable brand commitments. This is a fixed media composition, with no application controls, hover states or responsive website system.

## Colors

Polychrome artwork sits on a warm neutral stage; a single blue identifies PMR in the measured comparison. The frontmatter is the source of truth for the CSS colours.

### Primary

- **Process blue** (blue): the PMR benchmark bar. It is distinct from the richer raster gradients in the identity.

### Neutral

- **Warm porcelain** (paper): the full-frame ground in both video compositions and the poster.
- **Deep ink** (ink): headings, benchmark labels and values, command text and emphasized evidence.
- **Slate text** (secondary): supporting copy, author credit, context and footer.
- **Pale track** (track): the full benchmark scale behind both bars.
- **Comparator slate** (pm2-gray): the PM2 comparison bar; the written label also identifies the series.

The wordmark’s cyan/blue, violet/pink and coral/gold are baked into the transparent PNG. They are not approximated as CSS gradient tokens. The source declares violet and coral palette entries but does not use them in the composition; they are omitted from the normative token layer. Sidecar tonal strips are synthesized preview aids, not additional shipped colours.

At full opacity, ink on paper measures **15.12:1** and secondary text on paper **6.64:1**. Process blue against the track measures **4.95:1**; comparator slate against the track measures **3.17:1**. These checks cover the composition colours, not every pixel inside the dashboard captures or artwork. Series labels and numeric values preserve meaning without relying on colour alone.

**The Warm Stage Rule.** Keep the warm paper ground consistent across the film and poster. The generated identity carries the broad colour range; supporting copy stays in ink or secondary text.

## Typography

**Supporting family:** Manrope, with sans-serif fallback. The bundled Latin variable font is @fontsource-variable/manrope **5.3.0**, spanning weights 200–800, with its [SIL Open Font License](media/video/public/fonts/manrope-LICENSE.txt). The command fragment uses the generic monospace stack. Native terminal lettering remains inside the JPEG captures.

**Identity lettering:** Prism is an original generated raster wordmark, not an installed typeface. Its rounded, substantial letterforms and polished highlights establish the expressive voice. Manrope provides the clear supporting voice. Rounded and Geometric were comparison directions, not shipping fonts.

The frontmatter records the exact optical sizes used on the 1920 × 1080 canvas. There is no inferred modular scale.

| Role | Use | Treatment |
| --- | --- | --- |
| display | Dashboard and benchmark headings | Largest Manrope heading |
| hero-headline | Intro promise | Slightly smaller than scene headings |
| camera-headline | Longer recovery heading | Sized to remain on one line |
| poster-headline | Centered poster promise | Cleared from the wordmark descender |
| body | Captions, method and limitations | Relaxed line height; regular weight |
| poster-body | Centered poster description | Regular weight |
| metric | Benchmark names and times | Semibold, with tabular numerals |
| label / credit | Footer and film author credit | Quiet regular text |
| command | pmr monit fragment | Monospace inside the caption |

**The Text Carries Meaning Rule.** Keep headings, values and caveats as clear Manrope text. Preserve the Prism asset as artwork, retain its pmr text alternative, and use plain text when that raster is too small to read.

## Layout

The film and poster use a fixed **1920 × 1080** canvas. Both video exports run at **60 fps** for **1440 frames / 24 seconds**. There are no implemented breakpoints or responsive reflow rules. A smaller playback surface scales the complete frame; the transcript carries detailed meaning when the terminal text becomes too small.

The recurring scene layout uses the content inset token on both sides. Headings sit at y=197; the main caption starts at y=310. Dashboard captures begin at x=90, y=425. The benchmark block starts at x=100, y=420 with a 1720-pixel scale. A small wordmark stays at the upper left and the author credit at the upper right. The footer sits 28 pixels above the bottom edge.

The intro gives the identity the largest share of the frame: the wordmark occupies a 1000 × 500 element at (100, 180), and the terminal icon a 500 × 500 element at (1175, 225). The headline begins at (120, 735), with supporting copy at y=842.

The poster uses a centered arrangement. The icon element is 235 × 235 at (350, 92); the wordmark element is 790 × 395 at (650, 20). The headline begins at y=389 and the supporting line at y=482. The screenshot starts at y=556, centered at an effective x=211.8, using its poster width token. The footer sits 38 pixels above the bottom edge. These are current composition measurements, not a grid for unrelated surfaces.

The sequence is intro (0–4 s), dashboard (4–10 s), benchmark (10–17 s) and camera recovery (17–24 s). Space and a persistent identity connect those scenes; each has one dominant message.

## Elevation & Depth

Depth comes from two sources: baked acrylic/glass material in the generated identity and one diffuse CSS shadow around the real dashboard captures. The stage itself remains flat. No blur panel, background texture or extra logo shadow is applied by the composition.

**Screenshot lift:** 0 3px 5px #12214118, 0 26px 55px #12214128, 0 70px 90px #15358b12. The three layers provide contact, middle-distance softness and a broad low-opacity blue cast. The sidecar records the exact shadow value.

**The Material With Purpose Rule.** Use the shared soft shadow to lift genuine dashboard captures. Preserve the artwork’s own bevels, highlights and contact shadows without adding another CSS shadow to the logo or icon.

## Shapes

The PNG artwork owns its sculpted rounded silhouette, bevels and internal refraction. Real screenshots use the screenshot radius token and overflow clipping. Both the track and fill of each benchmark bar use the bar radius token; the fills grow from the left edge. Rounded shapes serve these specific materials and data marks. The film introduces no UI buttons, badges or input fields.

## Components

### Prism identity

Use [the wordmark](media/video/public/pmr-wordmark.png) and [terminal icon](media/video/public/pmr-icon.png) with their transparent margins intact. The wordmark uses object-fit: contain. Its shared transition changes position from (100, 180) to (94, 22) and scale from 1 to 0.21 during frames 210–246 (600 ms), with cubic-bezier(0.77, 0, 0.175, 1). The intro icon settles from rotateY(-8deg) and translateY(15px) to rest over frames 0–90 (1500 ms), with a 1300-pixel perspective.

### Headings and captions

Headings have zero margin, tight spacing and an open surrounding field. Captions share the same left/right alignment and secondary colour. Method text and qualifications use the caption size, so they remain part of the message rather than tiny footnotes. The footer aligns a repository address and a concise qualification at opposite ends.

Scene entrances begin eight frames after the scene boundary and last 30 frames (500 ms), using cubic-bezier(0.23, 1, 0.32, 1). Opacity rises to one while the scene moves up from 22 pixels below. Dashboard and benchmark exits run for 28 frames, fading out while moving a further 12 pixels up. The intro content fades during frames 204–228 (400 ms).

### Real dashboard capture

The reusable Screenshot component displays [pmr-dashboard.jpg](media/video/public/pmr-dashboard.jpg) or [pmr-camera-recovery.jpg](media/video/public/pmr-camera-recovery.jpg). Preserve their real JPEG content and proportions. Normal dashboard entrance combines scale 0.96→1 with rotateX(3deg→0) and a 2200-pixel perspective; the camera capture uses scale 0.97→1. Both follow the scene entrance. Capture context remains visible beneath each image.

### Benchmark comparison

Each row pairs a left-aligned manager name with a right-aligned, tabular numeric value. The 60-pixel bars share a 120 ms scale; their widths use the unrounded medians 14.704125 ms and 100.091417 ms. Text shows 14.7 ms and 100.1 ms throughout. Both fills use scaleX(0→1) together over frames 640–676 (600 ms), with the scene entrance easing and a left-center transform origin. Hardware, samples, method and command-specific limits remain stationary beside the comparison.

### Accessible alternatives

LaunchReducedMotion removes scene translation, icon tilt, screenshot transforms and bar growth. It retains opacity transitions. The wordmark switches to its compact position at frame 225, where its opacity is zero; each side of that fade lasts 21 frames (350 ms). This is a separate export, not a playback preference detected inside the MP4.

Deliver the [static poster](docs/assets/pmr-launch.png), [reduced-motion film](docs/assets/pmr-launch-reduced-motion.mp4) and [written transcript plus poster alt text](docs/launch-draft.md). The poster provides the introduction and dashboard; the transcript supplies the full four-scene content. Both films are silent, with all meaning carried on screen. JSX image alternatives identify the wordmark, sculpted icon and real captures; exported video and PNG delivery still need the accompanying text alternative.

### Provenance

[Asset provenance](media/asset-provenance.json) preserves the generation prompts and capture origins. The wordmark and icon are original generated art, not photographs of physical objects. Dashboard JPEGs came from the real native terminal with isolated local demo services. The recovery capture shows synthetic frames and one real process restart; it does not depict physical Raspberry Pi testing. The poster is an original Remotion composition. Third-party inspiration screenshots are not distributed. [Reproduction notes](media/README.md) describe the asset pipeline.

The sidecar previews the implemented headline, caption, benchmark and footer patterns. They are static media examples, so they have no invented hover, focus or active states. Each snippet embeds the same licensed Manrope font to render independently.

## Do's and Don'ts

### Do:

- Do use the selected Prism assets and Manrope 5.3.0 for this launch-media family.
- Do preserve the warm ground, generous margins and clear separation between the poster wordmark descender and headline.
- Do show real JPEG dashboard captures and keep their capture context with them.
- Do keep benchmark labels, values, sample count, hardware and limits visible beside the comparison.
- Do preserve the static poster, written transcript and reduced-motion export as a set of accessible alternatives.
- Do retain generation prompts, capture origins, PNG provenance metadata and the Manrope licence when redistributing assets.

### Don't:

- Don’t recreate the dashboard, fabricate scrolling logs or present synthetic frames as physical-camera footage.
- Don’t animate measured numbers through values that were never recorded.
- Don’t replace the original identity with third-party artwork or redistribute the inspiration screenshots.
- Don’t add looping background colour that competes with the evidence.
- Don’t treat the font exploration alternatives, unused palette declarations or preview tonal ramps as adopted design tokens.

Not canonized: unused violet/coral declarations and discarded type explorations are excluded from the token system. No remaining visual defect was identified in the final scoped review; the warm-ground and poster-clearance fixes are reflected here.
