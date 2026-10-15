---
name: "PMR launch media"
description: "Deep blue identity and real process evidence on warm paper."
colors:
  paper: "#faf9f6"
  ink: "#000000"
  identity-top: "color(display-p3 0.0000 0.5000 1.0000)"
  identity-bottom: "color(display-p3 0.0000 0.3400 1.0000)"
  blue: "#0057ff"
  white: "#ffffff"
  track: "#e5e5e5"
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
    lineHeight: "82px"
    letterSpacing: "-.035em"
  camera-headline:
    fontFamily: "Manrope, sans-serif"
    fontSize: "80px"
    fontWeight: 750
    lineHeight: 1.1
    letterSpacing: "-.035em"
  poster-headline:
    fontFamily: "Manrope, sans-serif"
    fontSize: "56px"
    fontWeight: 750
    lineHeight: "80px"
    letterSpacing: "-.035em"
  body:
    fontFamily: "Manrope, sans-serif"
    fontSize: "28px"
    fontWeight: 400
    lineHeight: "41px"
  metric:
    fontFamily: "Manrope, sans-serif"
    fontSize: "34px"
    fontWeight: 650
  label:
    fontFamily: "Manrope, sans-serif"
    fontSize: "22px"
    fontWeight: 400
  command:
    fontFamily: "monospace"
    fontSize: "28px"
    fontWeight: 400
    lineHeight: "41px"
rounded:
  screenshot: "16px"
  bar: "12px"
spacing:
  content-inset: "100px"
  author-inset: "24px"
  bar-label-gap: "17px"
  bar-row-gap: "58px"
components:
  scene-heading:
    textColor: "{colors.ink}"
    typography: "{typography.display}"
  hero-heading:
    textColor: "{colors.ink}"
    typography: "{typography.hero-headline}"
  caption:
    textColor: "{colors.ink}"
    typography: "{typography.body}"
    width: "1720px"
  screenshot:
    rounded: "{rounded.screenshot}"
    width: "1720px"
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
  recovery-note:
    textColor: "{colors.ink}"
    typography: "{typography.label}"
  author-logo:
    width: "148px"
    height: "73.5432px"
---

# Design System: PMR launch media

## Overview

**Creative North Star: "Blue on porcelain"**

PMR is a native process manager with a clear, useful identity. A deep blue vector wordmark and a native material terminal icon sit on warm paper. Black Manrope text and generous space keep the product evidence easy to read; the writing stays direct, warm and professional.

This system governs Liam Mendes’s personal launch media in media/video: the 24-second film, reduced-motion film and static poster. The sequence moves from identity to the real dashboard, a measured JSON-list benchmark and self-healing camera services. It is a fixed media composition, with no responsive website or application-control system.

The implementation is [Launch.tsx](media/video/src/Launch.tsx), registered by [index.tsx](media/video/src/index.tsx). [PRODUCT.md](PRODUCT.md) records the durable brand commitments. The current blue identity supersedes the earlier Prism exploration.

**Key Characteristics:**

- A shared vertical blue gradient in the PMR lettering and native terminal icon.
- A white pearl icon backing, white terminal prompt and blue status jewel.
- Black supporting text, a shared film left edge, centred poster copy and the official Liam Mendes vector logo.
- Real terminal captures, stable measured values and visible qualifications.
- One shared identity transition, restrained entrances and accessible alternatives.

## Colors

One saturated blue identity sits on a warm neutral stage. The frontmatter records the composition colours and the identity’s Display P3 endpoints.

### Primary

- **Clear blue** (identity-top) and **deep blue** (identity-bottom) form the shared vertical gradient, from top centre to bottom centre. [palette.json](media/brand/palette.json) and the SVG retain exact hex fallbacks **#0080FF → #0057FF** alongside the Display P3 stops. These are paired colour-space definitions, not interchangeable numeric conversions.
- **Process blue** (blue) is the solid PMR benchmark fill, matching the bottom hex fallback.

### Neutral

- **Warm porcelain** (paper) fills every frame.
- **Black** (ink) is used for all supporting composition typography and the official author logo.
- **Pearl white** (white) supplies the native icon backing and terminal prompt.
- **Pale track** (track) carries the benchmark scale.
- **Comparator slate** (pm2-gray) identifies the PM2 bar, together with its written label.

Terminal captures retain their original colours. Icon material rendering adds highlights and shading; those pixels are not extra palette tokens. The SVG’s fine edge highlight is an asset detail. Sidecar tonal strips are synthesized preview aids, not shipped colours.

**The Shared Blue Rule.** Use the same vertical blue gradient in the wordmark and icon. Keep the warm paper ground and black supporting composition text; preserve the original colours inside real terminal captures.

## Typography

**Supporting family:** Manrope, with sans-serif fallback. The bundled Latin variable font spans weights 200–800; retain its [SIL Open Font License](media/video/public/fonts/manrope-LICENSE.txt). Literal command fragments use monospace. Native terminal lettering remains inside the unaltered JPEG captures.

**Identity lettering:** PMR is the supplied [vector wordmark](media/video/public/pmr-wordmark.svg), with rounded, substantial lowercase forms. The [official Liam Mendes logo](media/video/public/liam-mendes.svg) is also vector artwork. Neither is reconstructed with a font.

The frontmatter records the exact sizes used on the 1920 × 1080 canvas; there is no inferred modular scale. Display serves dashboard and benchmark headings; hero, camera and poster headings use their own optical sizes. Body serves film captions, method and limitations. The poster description uses 20px Manrope with a 41px line height. Metric uses tabular numerals. Label is the camera startup-setup note.

**The Text Carries Meaning Rule.** Keep headings, values and caveats as clear Manrope text. Preserve the vector PMR lettering and official author logo as assets, with their text alternatives.

## Layout

The film and poster use a fixed **1920 × 1080** canvas. Both films run at **60 fps** for **1440 frames / 24 seconds**. Playback scales the whole frame; there are no breakpoints or responsive reflow rules. The transcript carries detailed meaning when terminal text becomes too small.

In the film, PMR lettering, headings, descriptions and screenshots share **x=100**. The official author logo stays visible throughout at **148 × 73.5432 px**, anchored **24 px from the right and 24.019 px from the bottom**. The poster places the author logo at **(32, 32), 96 × 48 px**.

| Surface | Current arrangement |
| --- | --- |
| Intro | Wordmark (100, 235), 1000 × 374; icon (1137, 281), 256 × 256, rotated 346.14° about its top-left corner; headline (100, 735); caption y=842. |
| Evidence scenes | Compact wordmark (100, 64), scale 0.21; heading y=197; caption y=310; dashboard/camera capture (100, 425), 1720 px wide. |
| Benchmark | Comparison (100, 420), 1720 px wide; context lines y=816, 870 and 924. |
| Poster | Wordmark centred at x=960, top y=85, 670 × 250.58; headline centred at x=960, top y=382, 56px/80px; description centred at x=959.5, top y=462, 20px/41px; capture (100, 550), 1720 px wide; no icon. |

The poster centres its wordmark and copy, following Liam’s current Paper composition. Artwork has no repository URL or native-supervisor footer. The camera scene retains a factual startup-setup note at (100, 1003).

The sequence is intro (0–4 s), dashboard (4–10 s), benchmark (10–17 s) and camera recovery (17–24 s). Each scene carries one dominant message.

## Elevation & Depth

The white pearl native icon, its layered face/prompt/jewel and restrained diffuse shadows provide depth on a flat paper stage.

- **Screenshot lift:** 0 3px 5px #12214118, 0 26px 55px #12214128, 0 70px 90px #15358b12.
- **Poster screenshot lift:** the same contact and middle layers, without the broad third layer.
- **Icon lift:** drop-shadow(0 3px 4px #2d38551a) drop-shadow(0 32px 36px #3e538629).
- **Wordmark finish:** the current source combines drop-shadow(0 3px 0 #788bb133) with drop-shadow(0 16px 16px #34476c1f). The diffuse layer is reusable; the zero-blur offset is not a general shadow token.

Apple Icon Composer holds the icon’s native material settings. Its prompt and jewel groups use neutral shadow opacity 0.5; the blue face uses 0.3. Translucency is disabled for all three groups.

**The Material With Purpose Rule.** Use the native icon materials and diffuse shadows to lift the identity and genuine dashboard captures. Keep the paper stage flat and the product evidence unaltered.

## Shapes

The SVG owns the wordmark’s rounded silhouettes and fine edge highlight. The native icon owns its rounded square, pearl backing, prompt and status jewel. Screenshot corners use the screenshot radius and overflow clipping. Benchmark tracks and fills use the bar radius; fills grow from their left edge. No buttons, badges or input fields are introduced by this media system.

## Components

### Blue identity and author logo

Use [pmr-wordmark.svg](media/video/public/pmr-wordmark.svg), [pmr-icon.png](media/video/public/pmr-icon.png) and [liam-mendes.svg](media/video/public/liam-mendes.svg) at their original proportions. The editable icon is [PMR.icon](media/brand/PMR.icon), built and exported in Apple Icon Composer. Its three separate groups are **Terminal Prompt**, **Status Jewel** and **Blue Face**, over a white backing. Preserve that editable structure.

The wordmark travels from (100, 235), scale 1, to (100, 64), scale 0.21 during frames 210–246 (600 ms), using cubic-bezier(0.77, 0, 0.175, 1). The intro icon keeps a static 346.14° rotation about its top-left corner. Inside that rotated wrapper, it settles from rotateY(-8deg) and translateY(15px) to rest over frames 0–90 (1500 ms), with a 1300-pixel perspective.

### Headings and captions

Headings have zero margin, tight tracking and an open surrounding field. All supporting text uses black; method and qualifications remain at the main caption size. No secondary grey text treatment is used.

Scene entrances begin eight frames after the boundary and last 30 frames (500 ms), using cubic-bezier(0.23, 1, 0.32, 1). Opacity rises while the scene moves up from 22 px below. Dashboard and benchmark exits run for 28 frames, fading while moving 12 px up. Intro content fades during frames 204–228 (400 ms).

### Real dashboard captures

The Screenshot component uses [pmr-dashboard.jpg](media/video/public/pmr-dashboard.jpg) and [pmr-camera-recovery.jpg](media/video/public/pmr-camera-recovery.jpg). Preserve their original pixels, colours and proportions. The normal dashboard entrance combines scale 0.96→1 and rotateX(3deg→0), with a 2200-pixel perspective; the recovery capture uses scale 0.97→1. Film captions identify the capture context.

### Benchmark comparison

Each row pairs a left-aligned manager name with a right-aligned, tabular value. The 60-pixel bars share a 120 ms scale. Widths use unrounded medians **14.704125 ms** and **100.091417 ms**; labels stay at **14.7 ms** and **100.1 ms**. Both fills grow over frames 640–676 (600 ms), using the scene entrance easing. Hardware, samples, method and command-specific limits remain visible beside the comparison.

### Accessible alternatives and provenance

LaunchReducedMotion removes scene translation, the animated icon settle, screenshot transforms and bar growth. It preserves the icon’s static Paper rotation. It retains opacity transitions. The wordmark switches position and scale at frame 225, when its opacity is zero, with 21-frame (350 ms) fades on either side. This is a separate export.

Deliver the [static poster](docs/assets/pmr-launch.png), [reduced-motion film](docs/assets/pmr-launch-reduced-motion.mp4) and [written transcript plus poster alt text](docs/launch-draft.md). Both films are silent. JSX alternatives identify the logos, icon and real captures; exported PNG/MP4 files still need accompanying text alternatives.

[Asset provenance](media/asset-provenance.json) and [reproduction notes](media/README.md) describe the asset pipeline. The current identity is vector artwork and a native Icon Composer export. Real terminal captures show isolated local demo services. Recovery uses synthetic frames and one real process restart; it does not establish physical Raspberry Pi testing. Earlier generated explorations are not the current identity.

The sidecar previews the implemented headline, caption, benchmark and author-logo patterns. These static media examples have no invented hover, focus or active states. Text snippets embed the same licensed Manrope font.

## Do's and Don'ts

### Do:

- Do use the current blue SVG wordmark, Icon Composer export and official Liam Mendes SVG.
- Do keep the film wordmark, headline and description at x=100, with its author logo bottom-right; centre the poster wordmark and copy, with its author logo top-left.
- Do keep all supporting composition typography black and retain the original pixels in terminal captures.
- Do keep benchmark labels, values, sample count, hardware and command-specific limits visible.
- Do preserve the static poster, written transcript and reduced-motion film as accessible alternatives.
- Do retain the editable PMR.icon bundle, palette, asset provenance and Manrope licence.

### Don't:

- Don’t add a repository URL or native-supervisor footer to the artwork.
- Don’t restore the earlier multicolour identity or substitute typeset text for either supplied logo.
- Don’t recreate the dashboard, fabricate scrolling logs or present synthetic frames as physical-camera footage.
- Don’t animate measured numbers through values that were never recorded.
- Don’t treat exploratory artwork or preview tonal ramps as adopted design tokens.

Not canonized: the current wordmark’s zero-blur 3 px offset is a hard-shadow craft-floor defect, so it is excluded from reusable shadow tokens. This documentation pass leaves the artwork unchanged. Earlier multicolour explorations, removed footer patterns and synthesized preview tonal ramps are also excluded.
