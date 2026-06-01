# Brutal Browser Engine Strategy

This repository is now browser-first. Brutal Browser is the product and engine
direction: an independent Rust-native browser engine and shell designed for
performance, safety, inspectability, and automation. Blackium Starium✴ remains
important, but it is a fast text/index/extraction mode inside that broader
browser project, not the project identity.

The current code is an early scaffold. It is useful because it already has
repeatable fixtures, local rendering, a narrow session model, storage/cookie
state, a tiny scripting subset, deterministic display-list/raster artifacts, and
benchmark hooks. It is not a modern browser, not Chromium-compatible, and not a
global Chrome competitor.

## Non-Negotiables

- Correctness before speed.
- No Chromium, WebKit, or Gecko page-engine code.
- Chromium may be used only as a comparison oracle or benchmark baseline.
- Benchmarks must count failed renders, blank renders, crashes, missing assets,
  and timed-out pages as failures.
- Every "better than Chromium" claim must name the page class, correctness
  gates, hardware, OS, Rust build profile, Chromium version, command, and
  artifacts.
- Claims are scoped. We do not ask whether Brutal Browser is better than Chrome
  globally. We ask whether it is better for a named page class under named gates.
- No random tiny feature accumulation. Each feature must move a subsystem toward
  a staged browser milestone.

## Current Execution Stance

For the next stretch, optimize for a functional Stage 1 browser pipeline before
spending much effort on Chromium benchmarking. The daily gates should be local:
document corpus pass/fail, visible text, screenshot artifacts, phase timing,
memory, crash/timeout accounting, and feature coverage. Chromium comparisons
return as end-of-stage oracle runs only after the local browser path can load,
render, scroll, navigate, and interact with the corpus without missing pages.

## Core Pipeline

The engine target is explicit:

```text
network/loading -> HTML parser -> DOM -> style cascade -> layout
-> paint/display list -> raster/compositor -> window shell
```

Cross-cutting subsystems are first-class:

- JavaScript engine strategy, Web APIs, Web IDL bindings, and event loop.
- Storage/profile: cookies, localStorage/sessionStorage, IndexedDB, Cache API,
  permissions, private mode, partitioning, and profile directories.
- Accessibility tree and platform accessibility bridge.
- Security and sandboxing: process model, origins, CSP, mixed content,
  permissions, site isolation, IPC, and fuzzing.
- Devtools/automation protocol.
- Benchmarks, compatibility dashboards, crash reporting, and release gates.

## Product Wedge

The first product wedge is a fast desktop document browser, not a whole-web
Chromium clone.

Stage 1 must handle real HTML documents: docs, blogs, news articles, search
results, package docs, specs, and mostly-static product pages. It should support
links, images, enough CSS for readable layout, scrolling, forms, history, tabs,
visual output, and deterministic automation. Limited JavaScript is acceptable,
but the JS strategy must be decided before Stage 2.

Initial narrow claim shape:

> Brutal Browser renders the document-page corpus with equivalent visible text,
> no failed renders, lower p95 render time, and lower peak memory than Chromium
> on the recorded hardware/software configuration.

That claim is not valid until the document corpus, visual/text correctness
gates, failure accounting, memory measurement, and reproducible Chromium
baseline all exist and pass.

## Staged Roadmap

### Stage 1: Fast Document Browser

Goal: a usable desktop document browser for static and lightly-scripted pages.

Required capabilities:

- Real URL loading and navigation lifecycle for documents and subresources.
- HTML parser and DOM tree robust enough for malformed real-world documents.
- CSS cascade with selectors, inheritance, box model, fonts, colors, images,
  scrolling, and print/readability-friendly layout.
- Paint/display-list and RGBA raster output with screenshot artifacts.
- Window shell with address bar, back/forward/reload, tabs, find in page,
  scroll/input routing, downloads handoff, and basic settings.
- Forms and session history good enough for search/docs/news workflows.
- Image support for common web formats through Rust crates or system libraries
  where appropriate.
- Accessibility snapshot good enough for document navigation audits.
- Search/text extraction mode preserved as a fast lane.

Exit gates:

- Document corpus renders with no blank pages, crashes, timeout skips, or missing
  critical assets.
- Visible text and screenshot expectations pass against checked-in local
  baselines first; Chromium-visible-text comparison is a late oracle gate for
  the same pinned corpus, not the daily development driver.
- Screenshot/pixel-diff thresholds for representative pages.
- p50/p95/p99 render, layout, paint, raster, memory, and startup reports.
- Failed renders counted as benchmark failures.
- Security review for local/HTTP document loading before wider browsing.

### Stage 2: JS-Capable App Browser

Goal: a browser that can run selected JavaScript-heavy applications with clear
compatibility boundaries.

Required capabilities:

- Decide JS strategy: embed an engine, build a constrained engine, or isolate a
  Rust-first runtime with Web IDL bindings. The choice must include performance,
  safety, license, maintenance, sandbox, and API-binding implications.
- Event loop, task/microtask queues, DOM events, fetch/XHR, URL, history,
  storage, forms, workers/modules policy, and error reporting.
- Incremental style/layout invalidation after DOM/CSS mutations.
- Broader WPT subset harness and expectation management.
- App corpus with correctness, performance, memory, and stability gates.

Exit gates:

- Named JS app corpus passes text/visual/interaction flows.
- WPT subset pass rate, flakes, expected failures, and timeouts are tracked.
- Runtime crashes, runaway scripts, memory growth, and network failures are
  bounded and reported.
- Security/sandbox strategy is implemented for script execution.

### Stage 3: Full Desktop Browser Product

Goal: a credible independent desktop browser product.

Required capabilities:

- Multiprocess or otherwise isolated browser/renderer/network/storage/GPU
  architecture.
- Site isolation, permissions, profile storage, private browsing, cache, cookies,
  downloads, updates, crash recovery, devtools, extensions policy, signed
  packages, and release channels.
- Full accessibility, IME/text editing, media, canvas/WebGL/WebGPU strategy,
  printing, platform integration, power management, and telemetry policy.
- Sustained compatibility, stability, security, and performance dashboards.

Exit gates:

- Release audit with no critical security, privacy, update, or crash-recovery
  gaps.
- Signed reproducible builds and rollback drills.
- Real-page canary suite, WPT subsets, visual regressions, memory/power tests,
  and Chromium comparison artifacts.
- Clear unsupported-surface disclosure.

## Benchmark And Evaluation Gates

A browser benchmark report is valid only if it records:

- Corpus identity, page class, URLs/fixtures, corpus hash, and asset policy.
- Hardware, OS, Rust version, build profile, browser version, and command.
- Success/failure counts, including blank renders, crashes, timeouts, missing
  resources, and correctness mismatches.
- Correctness gates: visible text, screenshot/pixel diffs, DOM/AX snapshots,
  interaction flow results, and known expectation files.
- Performance gates: cold/warm navigation, parse, style, layout, paint, raster,
  compositor/frame time, input latency, scroll latency, memory high-water mark,
  startup, and power where relevant.
- Regression policy: a speedup cannot pass if correctness regresses.

Initial page classes:

| Page class | Correctness gate | Performance gate |
| --- | --- | --- |
| Document pages | Visible text, key screenshots, link/form smoke | p95 render, memory, startup |
| Search/result pages | Visible text, forms, navigation, accessibility snapshot | p95 render plus input latency |
| Image-heavy articles | Screenshot diff, image decode success, scroll smoke | paint/raster p95 and memory |
| Light JS docs | DOM text after scripts, lifecycle/timer fixtures, no timeout | script plus layout p95 |

## Next Implementation Milestone

The next implementation milestone should be Stage 1 Document Browser M1:

1. Split the current monolithic browser runtime into explicit engine modules
   aligned to the pipeline: loading, parser/DOM, style, layout, paint/display
   list, raster, session/navigation, storage/profile, shell, and benchmarks.
2. Replace terminal-text-first rendering as the primary Stage 1 evidence with a
   visual document viewport path: RGBA frame-surface report, screenshot
   artifact, scrollable viewport, page dimensions, dirty pixel regions, and
   deterministic pixel baselines, consumed through a reusable `BrowserApp`
   state boundary plus the scripted/profile-aware and interactive/stdin
   `brutal-browser app` command with find/find-next state, visible viewport
   output, deterministic browser-window PNG output with simple chrome, narrow
   window-coordinate click routing, a small JSON history/bookmark profile, and
   a feature-gated native CPU-backed `brutal-browser window` shell with narrow
   location entry, text-control input routing, resize-aware viewport updates
   before product browser chrome work.
3. Define and check in a small document-page corpus manifest with local fixtures
   and pinned remote snapshots. Every page must have expected visible text,
   screenshot expectations or reviewed baselines, and failure accounting.
4. Add benchmark output for document pages that treats correctness failures as
   benchmark failures and records render phase timings plus memory high-water
   mark.
5. Only after this pipeline milestone should we add broader JS/Web API features,
   because visual/layout correctness and module boundaries are the current
   bottleneck for a usable browser.

This milestone is deliberately not "add one more DOM property." It is the
bridge from a terminal fixture renderer to a document browser engine.

## Anti-Overclaim Language

Use these phrases until evidence proves otherwise:

- "early independent Rust browser engine scaffold"
- "fixture-backed document rendering subset"
- "local CLI shell over a supported subset"
- "Chromium comparison oracle for named fixtures"
- "not modern-site compatible yet"
- "not a browser product yet"

Do not use these phrases without a completed audit:

- "Chromium-class"
- "Chrome competitor"
- "faster than Chrome"
- "full browser"
- "modern web compatible"
- "secure browser"

When a narrow speed claim is ready, write it like this:

```text
On <hardware/OS>, with <Rust build> and Chromium <version>, Brutal Browser
renders <page class/corpus hash> with <correctness gates> passing, <failure
count> failures, and <metric> better than Chromium by <amount>, using
<reproducible command/report path>.
```
