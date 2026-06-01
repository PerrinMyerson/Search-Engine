# Performant Rust Browser Plan

This plan defines the browser-specific execution path for turning the current
Rust static-page runtime into a fast independent web browser. It complements the
compatibility, rendering, platform, security, and operations plans by making
performance a first-class requirement for every subsystem instead of a late
benchmark pass.

## Performance Objective

- Build an independent Rust browser engine that does not use Chromium, WebKit,
  or Gecko as its page engine.
- Keep a fast static-text lane for search and lightweight pages while growing a
  standards-compatible full-rendering lane.
- Treat the current implementation as a scaffold: fixture-backed static loading,
  tiny script, tree mutation, DOM traversal, insertion convenience methods,
  `DocumentFragment` insertion, selector element methods, `innerHTML` mutation/readback, form-control DOM properties, location readback properties, class mutation, query collection, lifecycle listener, and style-mutation support, CLI shell workflows,
  and grayscale raster evidence are not market-browser parity.
- Optimize for cold navigation, warm navigation, first contentful text, first
  paint, full layout, input latency, scroll latency, memory high-water mark, and
  power-sensitive background behavior.
- Require direct measurements before any claim that the browser is faster than a
  market browser, and record hardware, OS, Rust version, corpus/page-set hash,
  build profile, and browser baseline version.

## Engine Architecture Steps

1. Split the browser into Rust crates for URL/networking, HTML tokenizer/tree
   builder, DOM, CSS parser, style cascade, layout, display list, raster,
   compositor, JavaScript bindings, storage/profile, accessibility, and shell.
   Keep large files split on subsystem boundaries and require ownership maps so
   JS/Web API, loading/navigation, raster/compositor, sandbox, and storage
   changes have focused tests and review.
2. Define zero-copy or arena-backed ownership between parser, DOM, style, and
   layout so hot paths avoid repeated allocation and string copying.
3. Add deterministic task scheduling for parser, scripts, timers, style,
   layout, paint, raster, compositor, and input priority.
4. Add explicit resource budgets for document bytes, DOM nodes, style rules,
   script time, layout boxes, images, GPU memory, storage, and background tabs.
5. Preserve process boundaries for untrusted page work, privileged browser UI,
   network service, storage service, and GPU/raster work.
6. Promote subsystems independently only when their boundary contract, fixture
   tests, WPT/compat subset, performance gate, fuzz/security gate, and release
   owner are recorded.

## Process And Site Architecture

- Target a site-instance/frame-tree model: each top-level page owns a frame tree,
  each document is assigned to a site instance, and cross-site iframes become
  out-of-process iframes with remote compositor surfaces.
- Keep privileged browser UI, renderer, GPU, network, and storage processes
  separate; renderers request network/storage/download work through typed IPC
  and never receive direct profile-file or arbitrary socket access.
- Define a broker/zygote strategy before untrusted browsing: spawn renderers
  from a constrained template, broker file/device handles, and document OS
  sandbox targets for macOS, Linux, and Windows.
- Enforce JIT and executable-memory policy explicitly: W^X by default,
  per-platform JIT entitlements only after review, bytecode caches partitioned
  by site/profile, and crash containment for renderer/GPU/network/storage
  process exits.

## Networking And Loading

- Implement connection pooling, HTTP/2 strategy, HTTP/3 strategy, DNS cache,
  redirect policy, content sniffing, MIME checks, cache policy, TLS/certificate
  reporting, and request priority.
- Define navigation lifecycle stages for URL parsing, IDNA, scheme handling,
  referrer policy, redirect/CORS checks, service worker interception, response
  commit, history entry creation, BFCache eligibility, and session restore.
- Stream HTML into the tokenizer as bytes arrive, start speculative preload
  scanning, and prioritize preload, preconnect, critical CSS, scripts, fonts,
  images, and iframes.
- Add persistent HTTP cache, cookie jar, partition keys, Cache API handoff,
  download/file handoff policy, blocked-scheme handling, and clear-data hooks.
- Treat loading and navigation as a first-class browser-product axis:
  every milestone should preserve evidence for redirects, history/session
  restore, process assignment, storage/profile partitioning, and failure
  recovery.
- Track per-phase timing for DNS, connect, TLS, first byte, response body,
  parser blocked time, resource queue wait, and cache hits.
- Gate with local fixture servers, network WPT subsets, cache tests, redirect
  tests, cookie/storage tests, and load-time benchmarks.

## Parser, DOM, And CSS

- Use arena allocation and compact node handles for DOM and style data.
- Implement incremental HTML tree construction, parser pause/resume for
  blocking scripts, mutation-safe DOM APIs, query APIs, and event target data.
- Build a selector engine with indexed id/class/tag/attribute lookup, cascade
  layers, specificity, inheritance, computed style caching, and invalidation.
- Gate with DOM/HTML/CSS WPT subsets, mutation fixtures, selector benchmarks,
  style recalculation benchmarks, and memory-growth tests.

## JavaScript And Web APIs

- Decide whether v1 embeds a proven JS engine behind Rust bindings or grows a
  dedicated Rust VM in stages; either path needs measured startup, parse,
  execution, memory, and sandbox behavior.
- Prioritize standards-compatible JavaScript runtime and Web API behavior over
  broad UI chrome: modules, event-loop integration, Web IDL bindings,
  fetch/storage/workers, and resource limits are release gates, not polish.
- Implement event loop phases: tasks, timers, microtasks, rendering update,
  input events, networking callbacks, idle work, and cancellation.
- Add Web IDL bindings for DOM, events, fetch/XHR, storage, workers, modules,
  history, URL, streams, and error reporting.
- Cover WebAssembly, WebCrypto, WebSockets, WebTransport, SharedArrayBuffer only
  behind cross-origin isolation, bytecode/cache partitioning, GC pause budgets,
  JIT warmup budgets, and memory ceilings. WebRTC and media capture may be
  explicitly deferred, but must have permission and privacy tests before any
  browser-completeness claim.
- Gate with JS benchmarks, Web API fixtures, timeout/resource-limit tests, WPT
  subsets, and rendered extraction parity reports.

## Layout, Paint, Raster, And Compositor

- Implement block, inline, flex, grid, table, positioned, overflow, scroll,
  transforms, writing modes, fragmentation, and viewport/mobile layout stages.
- Generate typed display lists and separate paint invalidation from layout
  invalidation.
- Add deterministic CPU raster first. Current baseline rasterizes text, styled
  text color, horizontal-rule rectangles, block backgrounds, and block border
  display-list commands with block padding, margin, size, max-width, and
  horizontal auto-margin layout plus image placeholder replaced elements, a tiny
  SVG rect decode subset, and a minimal non-interlaced 8-bit PNG decode subset
  including `data:` image URLs to grayscale pixels with stable hashes, PGM
  output, cached decoded pixels between layout and raster, optional fixed
  viewport-window culling for terminal/text raster output from whole-document
  scroll offsets, deterministic RGBA viewport frame surfaces with dirty pixel
  rectangles, a reusable `BrowserApp` state boundary for tab/navigation/input
  actions over those frames, a scripted/stdin `brutal-browser app` command for
  JSON cookie/localStorage files, JSON visit history/bookmarks, find/find-next
  match state, visible viewport text, page PNG frame output, and deterministic
  browser-window PNG output with simple chrome and window-coordinate click
  routing through that state boundary, plus a feature-gated native CPU-backed
  `brutal-browser window` shell with narrow location entry, focused text-control
  input routing, resize-aware viewport updates over the same state boundary,
  and
  `visual-verify` baseline and exact pixel-diff reports plus local retained
  layout-tree and layer-tree/debug snapshots for supported display-list
  grouping; next stages broaden PNG support, add JPEG decode and full paint
  commands, then GPU raster/compositor paths with CPU fallback, tile cache,
  layer promotion, occlusion culling, and context-loss recovery.
- Move the raster target from fixture grayscale evidence toward deterministic
  RGBA pixel buffers, screenshot baselines, and compositor surfaces that can be
  compared against pinned-browser references.
- Decide the rendering backend, shader/cache strategy, OS presentation path,
  vsync source, color-management and high-DPI policy, async scrolling model,
  scroll snap/anchoring behavior, and remote iframe surface integration before
  broad compositor claims.
- Gate with screenshot baselines, pixel diffs, layer-tree snapshots, frame-time
  traces, scroll/input latency tests, and memory-pressure tests.

## Text, Images, Media, And Platform

- Integrate font discovery, fallback, shaping, bidi, emoji/color fonts,
  selection, and text input/IME.
- Add image decoders, SVG, color management, lazy loading, responsive images,
  canvas 2D, WebGL/WebGPU strategy, video/audio decode, and media controls.
- Keep fonts, canvas, and media on the browser-critical path because they affect
  layout metrics, visual diffs, fingerprinting/privacy surface, and modern-site
  compatibility.
- Build accessibility tree generation, keyboard navigation, focus rings,
  clipboard, downloads, profile storage, private browsing, devtools, and update
  packaging.
- Gate with platform smoke suites, accessibility audits, media/canvas
  benchmarks, packaging verification, and signed-update drills.

## Performance Instrumentation

- Every release benchmark must break down navigation into loading, parsing,
  scripting, style, layout, paint, raster, composite, input, and shell overhead.
- Browser performance reports must include p50/p95/p99, throughput where
  relevant, memory high-water mark, allocation counts, CPU time, dropped frames,
  long tasks, and timeout counts.
- Benchmark suites must include static text pages, JS-heavy pages, CSS-heavy
  pages, image-heavy pages, long documents, form-heavy pages, scroll-heavy
  pages, animation-heavy pages, media/canvas pages, and multi-tab sessions.
- Standards and performance gates must include a pinned Chromium baseline,
  WPT/testdriver/WebDriver runner metadata, reftest expectations, CI sharding,
  flake policy, Speedometer/JetStream/MotionMark-style suites or equivalents,
  multi-tab memory, power, startup, and real-page canary reports.
- The current `brutal-bench browser-perf --chromium-baseline` path records a
  headless Chromium iframe fixture baseline and Rust-vs-Chromium p95 speedup for
  the checked-in fixture suite only. It inlines local fixture scripts for the
  `--dump-dom` baseline, steps fixtures through Chromium virtual-time callbacks
  so timer and click tasks can settle, and records expected-text match/mismatch
  counts so missed fixture work is visible; broader browser claims still require
  pinned Chromium policy, canary pages, visual correctness, memory/power/frame
  metrics, and standards coverage.
- Regression gates must fail on unreviewed latency, memory, correctness, crash,
  or compatibility regressions.

## Security And Reliability

- Performance work cannot bypass origin checks, process isolation, sandboxing,
  CSP, mixed-content policy, permissions, storage partitioning, or download
  safety.
- Add fuzzing for parsers, selectors, layout inputs, display-list decoding,
  image/SVG/canvas inputs, JS bindings, IPC, and storage.
- Add crash recovery, watchdogs, task cancellation, hung-renderer handling,
  resource exhaustion tests, and safe fallback to static-text extraction for
  search.

## Release Engineering

- Keep browser releases gated by signed reproducible builds, versioned manifests,
  updater and rollback drills, crash-report collection, release notes that name
  unsupported surfaces, and documented exception expiry dates.
- CI must shard WPT/compat suites, visual regression, browser performance,
  fuzzing, accessibility, storage/profile, networking/loading, and sandbox tests
  with pinned baselines and flake quarantine.
- Browser release dashboards must show standards coverage, canary-page success,
  visual diff status, JS/Web API compatibility, layout/CSSOM regressions,
  compositor/GPU health, memory/power/startup trends, security review state, and
  known blockers.
- No public Chromium-class, faster-than-Chromium, or broad modern-site claim can
  ship from fixture coverage alone; the exact claim must pass traceability,
  WPT/compat, performance, visual, sandbox, accessibility, devtools, profile,
  and release-engineering gates.

## Browser Milestone Gate Checklist

These milestones turn the current CLI scaffold into a usable browser without
turning fixture progress into compatibility claims.

| Milestone | Scope | Acceptance evidence |
| --- | --- | --- |
| B0 CLI scaffold baseline | Keep `brutal-browser browse`, fixture render, forms, click, raster, hit-test, layout-tree, layer-tree, coverage, and browser perf stable | `brutal-browser verify`, `brutal-browser coverage`, local WPT scaffold, and `brutal-bench browser-perf` pass for checked-in fixtures |
| B1 Module split | Extract loading/resources, forms, images, CSS/style, layout, display-list/paint, raster/compositor, runtime/Web APIs, input/events, session/navigation, storage/profile, shell, and gates into owned modules or crates | Boundary map, owner list, file-size budgets, focused unit tests, fuzz targets, and per-module perf counters |
| B2 Input/events/session | Add keyboard, pointer, focus, editing basics, default actions, cancellation, form state, and session navigation semantics beyond the current CLI subset | Input/event fixtures, form/session tests, hit-test-to-event traces, and event-order compatibility expectations |
| B3 Loading lifecycle | Implement navigation stages for URL parsing, redirects, TLS/cert reporting, referrer/CORS checks, cache, cookies, downloads, history, BFCache policy, and restore | Local fixture server suite, network/loading WPT subset, redirect/cache/cookie tests, failure-recovery tests, and per-phase timing |
| B4 JS runtime integration | Choose/embed the JS engine path; wire event loop, microtasks, modules, Web IDL bindings, DOM, timers, fetch/storage/workers, and resource caps | JS benchmarks, Web API WPT subset, timeout/resource-limit tests, rendered-extraction parity, and crash/oom containment |
| B5 CSS/layout growth | Grow CSSOM, cascade, computed style, invalidation, block/inline/flex/grid/table/forms/positioned/scroll/text layout | CSS/DOM WPT subset, reftests, visual baselines, layout stress tests, text shaping/bidi corpus, and memory gates |
| B6 Raster/compositor/windowing | Move from grayscale fixture raster to RGBA screenshots, platform window output, layer tree, frame scheduler, GPU path with CPU fallback, scrolling, and context-loss recovery | Pixel-diff baselines, screenshot artifacts, frame-time/input-latency reports, layer metrics, GPU timeout tests, and context-loss tests |
| B7 Storage/profile | Implement persistent profiles, cookies, localStorage, IndexedDB, Cache API, permissions, quota/eviction, private mode, clear-data, and partitioning | Storage WPT subset, quota/eviction tests, profile isolation tests, private-mode tests, and delete/export checks |
| B8 Security sandbox | Split browser, renderer, GPU, network, and storage processes with site instances/OOPIF plan, broker/zygote, OS sandbox profiles, IPC schemas, origin policy, permissions, CSP, mixed content, and JIT/W^X policy | Threat model update, sandbox denial tests, IPC fuzzing, origin-policy fixtures, crash-containment reports, and signed-update review |
| B9 WPT expansion | Replace the local scaffold-only WPT lane with imported upstream slices, expectations, skip manifests, reftests, testdriver/WebDriver where needed, CI sharding, and flake quarantine | WPT dashboard with pass/fail/skip/flake counts by subsystem, pinned expectations, and no unreviewed regressions |
| B10 Perf/Chromium comparison | Compare startup, navigation, JS, layout, paint, raster, compositor, memory, power, input latency, and canary-page behavior against pinned Chromium | `brutal-bench browser-perf` extensions, pinned Chromium metadata, Speedometer/JetStream/MotionMark-style reports, real-page canary reports, and regression budgets |
| B11 Release-ready browser | Package a signed browser with updater/rollback, crash reporting, release manifests, feature flags, known-limitations notes, support policy, and release dashboards | Signed artifacts, install/update/rollback drills, crash report smoke tests, release checklist, security/compat/perf signoff, and `brutal-bench audit --claim browser --require-complete` |

## Implementation Sequence

1. Keep expanding the current static runtime with measured, fixture-backed
   browser primitives: DOM APIs, selectors, events, timers, storage, forms,
   select/checkable form state, focused text traversal/input/editing, resource loading,
   and session navigation.
2. Split browser modules into crates once APIs stabilize enough to keep hot data
   paths explicit.
3. Add phase timing and allocation counters to the current `brutal-browser`
   commands and fixture verifier.
4. Extend the current `brutal-bench browser-perf --chromium-baseline`
   fixture-suite report into deterministic local page suites, pinned Chromium
   comparison reports, and startup/frame/memory/power metrics.
5. Implement CPU fixture raster output first, then full screenshot output and
   pixel-diff visual regression gates.
6. Expand style/layout correctness before broad paint/compositor work.
7. Add real event-loop/Web API coverage and decide the JS engine strategy.
8. Add process isolation, sandbox boundaries, origin policy, and storage
   partitioning before broad untrusted browsing.
9. Add GPU/compositor acceleration only after CPU correctness and security gates
   exist.
10. Add accessibility tree, devtools protocol/panels, persistent profiles, and
    release-engineering dashboards before packaged browser builds.
11. Promote any browser-performance claim only when readiness, traceability,
    visual, WPT, security, accessibility, devtools, storage/profile, release,
    and performance gates pass for that exact claim.

## Acceptance Gates

- `brutal-browser verify` passes all claimed capability fixtures.
- `brutal-browser coverage` requires every claimed feature and reports bounded
  missing feature count for the milestone. Its unweighted implemented ratios are
  feature-fixture progress indicators, not browser-completion or market-parity
  percentages.
- `brutal-bench browser-perf` reports current fixture render timings,
  parser/script/style/collection/layout phase timings, throughput, suite hash,
  DOM/layout/paint/layer counts, and local scalar layer-tree shape metrics;
  milestone extensions must add memory, real frame/compositor metrics, and
  Chromium comparison on deterministic page suites.
- Current deterministic grayscale PGM raster output, RGBA viewport-frame
  artifacts, `BrowserApp` frame presentation, window-frame, and find-state
  reports, and `brutal-browser app` scripted profile/frame outputs, including fixed
  viewport-window variants, count only as fixture/app-state evidence; full
  browser screenshot claims require color, fonts, images, browser-accurate
  viewport and scrolling behavior, compositor, and platform presentation gates.
  Planned screenshot and WPT gates pass at the milestone threshold.
- `browser-shell-coordinate-click` counts only as CLI shell coordinate routing
  through local display-list hit testing into supported generated
  `pointerdown`/`mousedown`/`pointerup`/`mouseup`/click/default-action
  navigation; it is not input-latency, platform pointer input, pointer capture,
  touch/pen, full MouseEvent semantics, or full browser pointer routing evidence.
- `browser-cli-click-at-viewport-offset` counts only as explicit offset
  translation for one-shot CLI `click-at`/`tap`; it is not browser-accurate
  scrolling, transformed hit testing, or platform pointer input.
- `browser-shell-wheel-events` counts only as document-level wheel dispatch
  before local CLI shell scroll/left/right viewport offset changes, with
  `event.deltaX`/`event.deltaY` readback and `preventDefault()` cancellation;
  it is not browser scroll-container semantics, CSS overflow, async/compositor
  scrolling, precise platform deltas, or full WheelEvent evidence.
- `browser-shell-relative-open` counts only as current-page-relative `open` /
  `go` resolution in the local CLI shell; it is not omnibox search, URL
  autocomplete, tab UI, security UI, or full browser chrome evidence.
- `browser-shell-cookie-inspection` counts only as a read-only CLI dump of the
  current in-memory `BrowserSession` cookie jar; it is not persistent profile
  storage, cookie settings UI, permissions, partitioning, or browser chrome
  evidence.
- `browser-shell-clear-cookies` counts only as clearing the current in-memory
  `BrowserSession` cookie jar; it is not persistent profile clearing, storage
  partition clearing, permissions UI, settings UI, or browser chrome evidence.
- `browser-shell-cookie-jar-file` counts only as loading and saving the current
  in-memory cookie jar through a local JSON `--cookie-jar` file around a
  `brutal-browser browse` run; it is not encrypted profile storage, cookie
  expiration persistence, partitioning, cookie settings UI, or browser chrome
  evidence.
- `browser-shell-local-storage-file` counts only as loading and saving
  origin-scoped localStorage through a local JSON `--local-storage` file around
  a `brutal-browser browse` run; it is not IndexedDB, Cache API, quota
  management, private browsing, partitioning, settings UI, or browser chrome
  evidence.
- `browser-shell-local-storage-inspection` counts only as read-only CLI
  inspection of current origin-scoped localStorage entries; it is not devtools
  storage panels, IndexedDB, Cache API, quota management, partitioning, settings
  UI, or browser chrome evidence.
- `browser-shell-clear-local-storage` counts only as clearing current
  origin-scoped localStorage session state from the local CLI shell; it is not
  full site-data clearing, IndexedDB, Cache API, quota management, private
  browsing, partitioning, settings UI, or browser chrome evidence.
- `browser-shell-session-storage-inspection` counts only as read-only CLI
  inspection of current in-memory origin-scoped sessionStorage entries; it is
  not persistent profile storage, devtools storage panels, IndexedDB, Cache API,
  quota management, partitioning, settings UI, or browser chrome evidence.
- `browser-shell-clear-session-storage` counts only as clearing current
  in-memory origin-scoped sessionStorage state from the local CLI shell; it is
  not full site-data clearing, persistent profiles, IndexedDB, Cache API, quota
  management, private browsing, partitioning, settings UI, or browser chrome
  evidence.
- `http-redirect-navigation` counts only as bounded HTTP(S) redirect following
  for document, form, and static resource loads with final session entry URLs,
  redirect-set cookies, and supported 303/POST method rewrite; it is not full
  navigation lifecycle, mixed-content/referrer/CORS policy, HSTS, redirect UI,
  browser-grade error pages, or broad compatibility evidence.
- `browser-shell-form-fill-state` counts only as CLI shell/session filled
  remembered values for text-like controls on the current `BrowserSession` entry
  feeding a later GET form submission; it is not full interactive form state,
  validation, focus/input events, autofill, POST, or browser UI.
- `browser-session-select-form-state` and
  `browser-shell-select-form-choice` count only as CLI shell/session
  single-select option metadata, enabled-option validation, focused choice, and
  explicit select commands feeding later GET/URL-encoded POST submission; they
  are not native select UI, multi-select, optgroup inheritance, input/change
  event dispatch, validation, keyboard events, or browser UI.
- `browser-session-checkable-form-state` and
  `browser-shell-checkable-form-toggle` count only as CLI shell/session
  checkbox/radio checked-state persistence, explicit toggle/focused-space
  commands, and narrow selector-click and label defaults for supported
  checkbox/radio controls; they are not full input/change event dispatch, full
  keyboard event dispatch, full label activation semantics, indeterminate state,
  custom controls, validation, or browser UI.
- `browser-session-required-form-validation` counts only as CLI shell/session
  value-missing checks for supported required text-like, select, checkbox, and
  radio controls in BrowserSession/CLI submit paths, honoring form
  `novalidate` and submitter `formnovalidate`; it is not full constraint validation, validation UI, custom
  validity, invalid events, or browser UI.
- `browser-session-type-value-validation` counts only as CLI shell/session
  non-empty email/URL value checks on supported submit paths; it is not full
  HTML type validation, IDNA/email grammar, validation UI, invalid events, or
  browser UI.
- `browser-session-reload` and `browser-shell-reload` count only as current
  `BrowserSession` entry replacement from the same target; they are not full
  reload lifecycle, cache policy, POST replay UI, service worker, BFCache, or
  broad navigation compatibility evidence.
- `browser-session-form-submit-button-click-default` is implemented only for
  supported BrowserSession/CLI submit/input/button click default action on GET or
  URL-encoded POST forms; it is not form event dispatch, validation, focus/input
  behavior, browser UI, or broad input/form compatibility.
- `browser-session-form-reset-click-default` is implemented only for supported
  BrowserSession/CLI reset-control click default action; it is not form event
  dispatch, validation, focus/input behavior, browser UI, or broad input/form
  compatibility.
- `brutal-bench audit --claim browser --require-complete` passes before any
  public broad-browser or faster-than-Chromium browser claim.
