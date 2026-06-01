# Browser Rendering And Compositor Plan

This plan covers the paint, raster, compositor, animation, hit-testing, and
visual-regression work required before Brutal Browser can credibly claim
browser rendering progress. It is a gate plan, not proof of implementation.

## Rendering Standard

- Rendering correctness must be measured with deterministic fixtures,
  screenshot baselines, and pixel-diff gates before compatibility claims.
- Rendering performance must track first paint, full paint, raster time,
  compositor frame time, input latency, memory high-water marks, and GPU timeout
  behavior.
- Paint and compositing work must keep the search static-text fast lane
  independent from full visual rendering.
- GPU or accelerated paths must have CPU fallbacks, resource caps, crash
  containment, and security review before broad use.

## Display List And Paint Model

Required end state:

- Typed display-list commands for text, backgrounds, borders, images, SVG,
  canvas, video frames, clipping, transforms, opacity, filters, scroll layers,
  focus rings, selection, and debug overlays.
- Stable paint order for stacking contexts, z-index, positioned elements,
  pseudo-elements, iframes, composited layers, and top-layer UI.
- Damage tracking and invalidation for DOM changes, style changes, layout
  changes, animations, scrolling, image decode completion, video frames, and
  input focus.
- Hit-test data tied to display-list items, DOM nodes, scrolling boxes, pointer
  events, accessibility focus, and text selection.

Evidence gates:

- Display-list serialization fixtures for block, inline, positioned, overflow,
  transform, opacity, clip, form, and focus cases.
- Paint-order fixtures that compare expected display-list order and screenshot
  pixels.
- Invalidation fixtures that prove minimal repaint after text, style, image,
  scroll, and animation changes.
- Hit-test fixtures for transformed, clipped, overlapping, scrolling, and
  iframe-like surfaces.
- Layout-tree fixtures for block, inline, replaced, form-control, overflow,
  positioned, fragmented inline, and scrolling-box cases.
- Current local scaffold: `brutal-browser hit-test <page> --x <n> --y <n>
  --json` reports the topmost supported display-list command for a point in
  local fixture coordinates. It does not cover transforms, clipping, overlapping
  stacking contexts, scrolling, iframe routing, pointer-event semantics,
  accessibility focus, or text selection until those fixture gates exist. The
  `browser-shell-coordinate-click` feature gate may consume this hit-test result
  only to route a CLI shell coordinate into the supported generated
  `pointerdown`/`mousedown`/`pointerup`/`mouseup`/click/default-action navigation
  path; it is not full browser pointer routing, full PointerEvent semantics, or
  full MouseEvent semantics.
- Current local scaffold: `brutal-browser layout-tree <page> --json` reports a
  deterministic retained layout-tree/debug snapshot for paint-backed element
  boxes in the implemented local layout/display-list subset, including
  parent/child box links, bounds, and source command indices. It does not prove
  full CSS layout, anonymous boxes, inline fragmentation, scrolling boxes,
  browser-accurate geometry, or compositor-ready layout until those gates exist.

## Rasterization

Required end state:

- CPU raster path for deterministic screenshots and fallback rendering. Current
  baseline: `cpu-text-raster` converts text and styled-text display-list
  commands into a deterministic grayscale pixel buffer with stable hashes and
  PGM output, and `rgba-screenshot-artifact` exports that deterministic raster
  as RGBA8 PNG screenshot artifacts. Optional viewport-window CPU culling
  supports terminal/text and screenshot output. The viewport window uses
  clamped whole-document scroll offsets and reports visible retained layout-box
  state plus cell-space dirty regions for the supported shell path. The
  `browser-viewport-frame-surface` path now couples those clamped viewport
  metrics to deterministic RGBA pixels and dirty pixel rectangles so a native
  shell can consume one frame-surface contract. `BrowserApp` now consumes that
  contract for tab, navigation, scroll, input, and frame-presentation state,
  and `brutal-browser app` exposes the same contract as a scripted/stdin
  profile-aware history/bookmark, find/find-next, visible-viewport, and
  PNG-producing app surface, including a deterministic browser-window PNG that
  composites simple tab/location/status chrome above the page viewport.
  This is fixture raster evidence only, not proof of browser-accurate
  scrolling, compositor tiling, full browser screenshots, compositor damage, or
  platform presentation correctness.
- GPU/accelerated raster strategy with resource budgets, context loss handling,
  cache eviction, and disabled-GPU fallback.
- Text raster integration with font shaping, subpixel policy, emoji/color fonts,
  bidi text, selection highlights, and accessibility focus.
- Image/SVG/canvas/video raster integration with color management, scaling,
  interpolation, clipping, opacity, and decode timing.
- Backend decision covering CPU raster, GPU API choice, shader compilation,
  shader/disk cache keys, tile cache eviction, resource lifetime, and
  cross-process GPU memory accounting.
- Color management and high-DPI policy for sRGB, wide-gamut displays, image
  color profiles, device scale factors, zoom, and per-platform text AA.

Evidence gates:

- Current text-raster gate: `brutal-browser verify
  bench/browser-fixtures/manifest.json --json` checks fixture raster hashes, and
  `brutal-browser visual-verify bench/browser-fixtures/manifest.json --json
  --artifact-dir <dir>` reports expected/actual hashes and emits deterministic
  PGM raster artifacts. When passed `--baseline-dir <dir>`, the same command
  computes exact pixel-diff counts/ratios, writes `*-diff.pgm` artifacts, and
  enforces `--max-diff-pixels` / `--max-diff-ratio` thresholds.
  The current display-list baseline includes text, styled text color,
  horizontal-rule rectangle, block background, block border, block padding
  layout, block margin layout, block size constraints, max-width auto-margin
  document-column layout, image placeholder, tiny SVG decoded-image paint
  commands, and data URI PNG decoded-image paint commands with decoded pixels
  carried into rasterization.
  `brutal-browser raster <page>` and
  `brutal-browser raster-file <fixture> --json --output <pgm>` remain
  available for one-off raster inspection, including
  `--viewport-x` / `--viewport-y` (`--scroll-x` / `--scroll-y` aliases) and
  `--viewport-width` / `--viewport-height` for fixed terminal/text viewport
  windows.
- Future screenshot pixel gates for text, images, SVG, canvas, borders,
  backgrounds, transforms, filters, clipping, and form controls.
- Raster stress benchmarks for large pages, large images, many layers, text-heavy
  pages, animated transforms, and scroll-heavy pages.
- CPU-vs-GPU fallback parity fixtures for supported rendering features.
- GPU timeout, context-loss, memory-pressure, and resource-budget tests.

## Compositor

Required end state:

- Layer tree construction from display-list commands, scroll containers,
  transforms, opacity, filters, video/canvas surfaces, fixed/sticky elements,
  and browser UI overlays.
- Frame scheduler with explicit deadlines, vsync strategy, input priority,
  animation ticks, paint/raster/composite phases, and dropped-frame metrics.
- Scroll, transform, opacity, and video/canvas updates that can composite without
  full layout or paint when valid.
- Surface lifecycle management, occlusion/culling, tile cache, memory budgets,
  and crash-safe recovery.
- OS presentation paths for macOS, Linux, and Windows, including swapchain or
  layer ownership, resize behavior, vsync source, frame pacing, and fallback
  when the GPU process exits.
- Async scrolling with scroll snap, scroll anchoring, fixed/sticky positioning,
  overscroll policy, hit testing in scrolled layers, and main-thread fallback
  when layout-dependent scroll effects are active.
- Remote iframe/OOPIF surface composition with damage propagation, input routing,
  focus handoff, visibility throttling, and crash placeholders.

Evidence gates:

- Compositor frame-time benchmarks for static pages, scroll-heavy pages,
  transform animations, opacity animations, videos/canvas, and many-layer pages.
- Layer-tree snapshot fixtures with expected layer counts, bounds, scroll state
  where supported, and explicit reasons for grouping/promotion. Current local
  scaffold: `brutal-browser layer-tree <page> --json` reports a deterministic
  layer-tree/debug snapshot for the implemented local layout/display-list subset
  only. It does not prove compositor frame scheduling, GPU compositing, tile
  cache behavior, OS presentation, async scrolling, transform/clip correctness,
  iframe/OOPIF composition, or Chromium compositor parity.
- Input-latency tests for pointer, wheel, keyboard, touch, and focus updates.
- Memory-budget tests for layer churn, tile cache pressure, and many-tab
  rendering.

## Visual Regression Gates

- Curated static HTML/CSS page suite at desktop and mobile viewport sizes.
- Screenshot baselines for layout, paint, images, SVG, canvas, fonts, forms,
  focus, selection, scrolling, transforms, opacity, filters, and animations.
- Pixel-diff thresholds with explicit anti-aliasing policy and per-platform
  expected differences.
- Chrome comparison mode for fixture pages where standards-compatible behavior
  is expected.
- Artifact retention for screenshots, diffs, display lists, layer trees,
  performance traces, and environment metadata.
- Current grayscale PGM artifacts remain valid for deterministic fixture hashes;
  color screenshots, OS-window screenshots, remote surfaces, and compositor
  frames require separate baselines before browser-rendering claims.

## Performance Gates

- First paint, full paint, raster time, compositor frame time, input latency,
  memory high-water mark, local layer count, promoted-layer count,
  layer-metrics build time, root/child layer bounds, tile-cache hit rate, and
  dropped frames. Local layer metrics remain debug/scaffold evidence until real
  frame scheduling, compositing, scrolling, and presentation gates exist.
- Benchmarks for text-heavy pages, image-heavy pages, canvas-heavy pages,
  animation-heavy pages, scroll-heavy pages, form-heavy pages, and long
  documents.
- Regression thresholds for p50/p95/p99 timing and memory.
- Separate metrics for parser, style, layout, paint, raster, composite, and
  browser-shell overhead.

## Security And Reliability Gates

- Renderer/GPU process isolation plan before untrusted GPU work.
- Bounds checks and fuzzing for display-list decoding, raster command handling,
  image/SVG/canvas inputs, shader inputs, and layer-tree IPC.
- Crash containment for renderer, raster, GPU, and compositor failures.
- Safe fallback to static text extraction for search when visual rendering times
  out, crashes, or exceeds budget.

## Implementation Sequence

1. Define display-list command types, serialization, and fixture assertions.
2. Add deterministic CPU raster screenshots for the existing static text layout
   path. Baseline implemented for text and styled text color display-list
   commands.
3. Add screenshot baseline runner and pixel-diff report artifacts. Baseline
   runner implemented for text/styled-text raster hashes, PGM artifact
   retention, exact pixel-diff counts/ratios, diff artifacts, and threshold
   gates. Rectangle paint/raster commands are implemented for horizontal rules,
   block background rectangles are implemented for the current background-color
   CSS subset, CSS text color emits styled text commands, and block border
   rectangles are implemented for the current border CSS subset, image
   placeholders are implemented for static img replaced elements, a tiny
   local SVG rect subset decodes into image pixels, and a minimal non-interlaced
   data URI PNG subset decodes through the same cache/raster path; next step is
   adding broader PNG/JPEG decode, clipping, and broader replaced elements.
4. Implement paint order for backgrounds, borders, text, replaced elements,
   focus, selection, positioned content, transforms, opacity, and clipping.
5. Build retained layout-tree snapshots for the supported document layout path.
   Initial `brutal-browser layout-tree <page> --json` scaffold may count only as
   local paint-backed box/debug evidence until full CSS layout, scrolling boxes,
   anonymous boxes, inline fragmentation, and compositor-ready geometry exist.
6. Add invalidation and damage tracking for DOM/style/layout changes.
7. Add layer-tree construction and layer snapshot fixtures. Initial
   `brutal-browser layer-tree <page> --json` scaffold may count only as local
   snapshot/debug evidence until frame scheduling, compositing, scrolling, and
   presentation gates exist.
8. Add compositor frame scheduler and input-latency benchmarks.
9. Add GPU/accelerated raster strategy with resource caps and CPU fallback.
10. Add animation, scrolling, video/canvas, and many-layer stress tests.
11. Promote `REQ-BROWSER-PAINT-COMPOSITOR` only when the gates above have direct
    passing evidence.
