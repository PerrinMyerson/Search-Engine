# Brutal Browser Roadmap

This project is now browser-first. The primary target is an independent
Rust-native browser engine and shell. The existing fast Rust static-text search
core remains important, but it is a search/extraction mode inside the browser
project rather than the project identity.

The work has two linked systems:

1. An independent browser engine: load, parse, build DOM, cascade style, layout,
   paint, raster/composite, run JS/Web APIs, sandbox, expose a shell, and pass
   compatibility gates.
2. A fast search/extraction mode: crawl, extract, index, rank, serve, measure,
   and continuously refresh the public web.

The plan below keeps the speed thesis but narrows claims. We do not ask "are we
better than Chrome?" globally. We ask whether Brutal Browser is better for a
named page class under named correctness, performance, memory, stability, and
security gates.

For the browser-first strategy, see `BROWSER_ENGINE_STRATEGY.md`. For the
execution-level breakdown, see `PROGRAM_PLAN.md`. For the requirement-to-gate
audit matrix, see `REQUIREMENTS_TRACEABILITY.md`. For target diagrams,
interfaces, and decisions, see `ARCHITECTURE.md`.

## Current State

Implemented:

- Rust CLI and resident daemon.
- Static HTML crawl/index/search/render path.
- Custom inverted index with compressed postings and mmap-backed text.
- Benchmark harness that can compare daemon hot-path search against a headless
  Chromium JavaScript baseline over the same local corpus.
- Durable frontier prototype integrated into `crawl`, including on-disk URL
  state and fetched-document snapshots for restart/resume.
- Basic seed-list ingestion: `crawl` accepts one seed URL, `--seed-file`, or
  both; default same-host crawling now treats every seed host as in-bounds.
- Domain seed ingestion: `crawl --domain` and `--domain-file` normalize domain
  roots into crawl seeds and participate in robots sitemap discovery.
- Manual sitemap ingestion: `crawl --sitemap` loads XML/XML.gz URL sets and
  recursively follows sitemap indexes under explicit caps.
- Robots sitemap discovery: `crawl --discover-sitemaps` reads seed-host
  `robots.txt` files, extracts `Sitemap:` entries, and feeds them through the
  capped sitemap loader.
- Recrawl manifest ingestion: `crawl --recrawl-manifest` accepts JSONL records
  with URL, domain, and sitemap inputs. It now honors `priority` and
  `recrawl_after` as a single-run due queue, requeues already-fetched frontier
  records, and keeps latest fetched text when rebuilding from crawl snapshots.
- Frontier-driven recrawl planning: `brutal-search recrawl-plan` emits due JSONL
  manifests from fetched frontier records using an age interval and limit.
- Single-machine recrawl scheduler: `brutal-search recrawl-scheduler` loops over
  due frontier batches, recrawls them, rebuilds the hot index from latest crawl
  snapshots, and reports changed/unchanged/missing counts per round.
- Fielded document schema for canonical URL, title, meta description, language,
  headings, body, anchor text, outbound links, content hash, extraction mode, and
  fetch time. Crawls and local corpus indexing now preserve this metadata.
- Duplicate result clustering: index metadata tracks canonical, exact-text, and
  shingled-simhash near-duplicate representatives/counts, and query serving
  collapses repeated results while preserving direct render by document id or
  URL.
- Basic indexing quality policy: static extraction captures meta robots
  `noindex`, index builds skip noindex pages by default, optional
  `--min-body-terms` filters thin pages, and stats report skipped counts.
- Field-aware postings and weighted scoring for title, headings, anchor text,
  metadata, URL, and body terms.
- First-pass link authority scoring: crawled outbound links are resolved against
  indexed URLs/canonicals, normalized PageRank-style authority is stored in doc
  metadata, and query scoring uses it as a small ranking feature.
- First-pass query operators on the hot path: quoted phrases, negative phrases,
  positive `site:`, `filetype:`, `lang:`, `after:`, `before:`, and negative
  term/site/filetype/language/freshness filters are parsed before scoring.
  Required `+term` filters and uppercase `OR` term groups are also supported as
  candidate filters.
- Local HTTP search API and minimal browser UI via `brutal-search serve`,
  including index stats, host-level crawl status, result filters,
  lexicon-backed prefix suggestions, and first-pass spelling corrections.
- Minimal browser-runtime skeleton: `brutal-browser` can load local or HTTP(S)
  static HTML, build an independent DOM tree, parse simple CSS
  `display`/color/background/border/padding/margin/size rules, run block text and
  small box-model layout, extract/resolve anchor links, discover static
  subresources, fetch/cache discovered static resources, apply fetched
  stylesheets to the current text layout path, extract static forms, construct
  GET submission URLs, submit GET forms through a small back/forward session
  history, carry in-memory HTTP cookies between session navigations, follow
  bounded HTTP(S) redirects while carrying redirect-set cookies into the next
  hop, inspect/clear the current in-memory cookie jar from the CLI shell, load
  and save that jar through an optional local JSON `--cookie-jar` file, load
  and save origin-scoped localStorage through an optional local JSON
  `--local-storage` file, inspect current origin-scoped localStorage from the
  CLI shell, inspect/clear current in-memory origin-scoped sessionStorage from
  the CLI shell, execute a tiny inline
  JavaScript subset for document title/text mutations plus
  `createElement`/`createTextNode`/`appendChild`, tree mutation and DOM
  traversal/insertion methods, `DocumentFragment` insertion, selector element methods, `innerHTML` mutation/readback, form-control DOM properties, location readback properties,
  `setAttribute`/`getAttribute`,
  origin-scoped localStorage and BrowserSession-scoped sessionStorage
  `setItem`/`getItem`/`removeItem`/`clear`/`length`,
  deterministic `setTimeout`/`clearTimeout` task-queue draining,
  fetch external scripts into that same tiny DOM mutation subset, dispatch
  inline `onclick` and `addEventListener("click", ...)` handlers for a first
  click path, emit deterministic text/styled-text/rectangle/image display-list
  commands, and render terminal text plus deterministic grayscale fixture
  raster artifacts without Chromium/WebKit/Gecko. The raster path can cull to a
  fixed terminal/text viewport window from whole-document scroll offsets; this
  is fixture raster evidence, not browser-accurate scrolling, compositor
  tiling, or full browser screenshots. An early
  `brutal-browser browse` CLI shell makes this subset playable with
  open/back/forward, current-page-relative open/go target resolution,
  current-page reload/refresh, current-page location reporting, current
  in-memory cookie inspection/clearing plus optional local JSON `--cookie-jar`
  and `--local-storage` load/save, localStorage inspection/clearing,
  sessionStorage inspection/clearing, current-page link listing,
  resolved-link activation by zero-based index, exact text, or anchor selector,
  selector click with narrow anchor href default navigation through session
  history, rendered fragment-target scrolling for the CLI text viewport,
  coordinate-click routing through display-list hit testing into supported
  event/default-action navigation, remembered text-like form field values and
  single-select choices on the current `BrowserSession` entry for later GET form
  submission, selector/associated-label focus across
  fillable/select/checkable/submit-reset controls and typed-text append for editable text-like controls,
  fixed text-viewport scroll, current-page render, and optional repeated `--cmd` scripted runs. Link
  activation is scoped to the extracted `href` list and session navigation;
  shell click default navigation is limited to anchors, supported submit
  controls, and supported reset controls after supported click-handler dispatch.
  Relative open is scoped to current-source URL/path resolution only, not omnibox
  search, URL autocomplete, tab UI, security UI, or full browser chrome.
  Coordinate clicks are terminal-cell/display-list shell routing only. Location
  reporting is scoped to current source/title/history/viewport metadata, not a
  real address bar or browser chrome. Cookie inspection, clearing, JSON
  cookie-jar load/save, JSON localStorage load/save, localStorage inspection,
  localStorage clearing, sessionStorage inspection, and sessionStorage clearing
  are scoped to the in-memory `BrowserSession` jar, origin-scoped localStorage
  map, and current sessionStorage map, not encrypted profile storage,
  expiration persistence, devtools storage panels, IndexedDB, Cache API, quota
  management, cookie settings UI, full site-data clearing, storage partition
  clearing, permissions, partitioning, or browser chrome.
  Redirect handling is scoped to
  bounded HTTP(S) document/form/resource load following with final session entry
  URLs and redirect-set cookies only, not full navigation lifecycle,
  mixed-content/referrer/CORS policy, HSTS, redirect UI, browser-grade error
  pages, or broad compatibility. Fragment
  navigation is scoped only to rendered `id`/anchor-name target discovery and
  text-viewport scrolling. Form fill state is the narrow remembered-value path
  feeding GET or URL-encoded POST submission. Select state is scoped only to
  single-select option metadata,
  enabled-option validation, and explicit/focused CLI choice commands.
  Checkable state is scoped only to supported checkbox/radio
  checked-state persistence, explicit toggle/focused-space commands, and narrow
  selector-click and label defaults. Required form validation is scoped only to
  value-missing checks for supported required text-like, select, checkbox, and
  radio controls in BrowserSession/CLI submit paths, honoring form
  `novalidate` and submitter `formnovalidate`; type value validation is scoped
  only to non-empty email/URL checks on those same submit paths. Submitter
  action/method overrides are scoped only to `formaction` and GET/POST
  `formmethod` on supported BrowserSession/CLI submit-control click and
  focused-submit paths. The `browser-session-form-submit-button-click-default`
  marker is scoped only to BrowserSession/CLI submit/input/button click default
  action, and `browser-session-form-reset-click-default` is scoped only to
  reset-control click default action. Focused enter is scoped only to submitting
  focused fillable/select controls, activating focused submit controls with
  submitter state, and resetting focused reset controls. Focused text input is scoped only to
  selector/associated-label focus plus type commands for editable text-like
  controls over the local shell, and reload is scoped only to replacing the
  current session entry target. It is not GUI browser chrome, full
  JS/CSS/browser interaction, full event/default-action semantics, full
  event-cancellation semantics, full browser pointer routing, full PointerEvent
  semantics, full MouseEvent semantics,
  full interactive form state, full label activation semantics, full constraint validation, validation UI,
  CSS scroll behavior, `:target` styling, history scroll restoration, external
  form ownership, target/enctype/dialog handling, keyboard events, selection,
  IME, autofill, POST, full reload lifecycle, cache policy, service worker
  handling, tab/process isolation, devtools/accessibility, or Chromium parity.
  `brutal-browser coverage` now emits a machine-readable
  feature-fixture coverage report and optional gates for required feature IDs
  and implemented-ratio thresholds; the unweighted ratios are not
  browser-completion percentages. `brutal-browser compare-chromium` compares
  static and tiny-script fixture title/text output against headless Chromium.

Not implemented yet:

- Production-grade web-scale crawl frontier.
- Distributed crawling, indexing, merge, replication, and serving.
- Advanced query understanding and relevance signals beyond lexical
  field-weighted BM25-style scoring, first-pass operators, first-pass link
  authority, first-pass duplicate clustering, and basic noindex/thin filtering.
- General JavaScript execution, event loop, DOM/Web API bindings, full CSS
  cascade/layout, paint/raster, compositing, sandboxing, storage, accessibility,
  media/canvas, GUI browser chrome, tab/process isolation,
  devtools/accessibility, or browser-platform parity.
- Web standards conformance testing.
- Production operations, telemetry, abuse controls, and distributed freshness
  pipelines.

## Product Targets

Search target:

- Given a query, return relevant web documents with title, URL, snippet, and
  optional full text.
- Maintain a continuously refreshed index from public web crawls.
- Support low-latency serving from a resident hot index, then scale to sharded
  distributed serving.
- Optimize for text retrieval speed first, then add relevance, freshness, and
  quality signals without breaking the hot path.

Browser/runtime target:

- Load and render modern web pages without depending on Chromium/WebKit/Gecko as
  the page engine.
- Be honest about scope: the current browser path is a static fixture-backed
  scaffold, so every claim must name the exact supported subsystem, page set,
  test suite, and baseline browser version.
- Prioritize the browser-product work around JS runtime/Web APIs,
  loading/navigation, compositor-backed RGBA raster output, sandbox/site
  isolation, profiles/storage, fonts/text, canvas/media, and file-size/module
  boundaries. Each area needs direct compatibility, performance, and security
  evidence before it can support a browser claim.
- Execute JavaScript, implement DOM/Web APIs, CSS cascade/layout, painting,
  compositing, input, navigation, storage, networking, and security sandboxing.
- Define a site-instance/frame-tree architecture with OOPIFs and separate
  browser, renderer, GPU, network, and storage processes before broad untrusted
  browsing.
- Cover the surrounding browser platform: fonts, text shaping, images, media,
  canvas, accessibility, devtools, profiles, downloads, packaging, and updates.
- Treat [`PLATFORM_COMPLETENESS_PLAN.md`](PLATFORM_COMPLETENESS_PLAN.md) as the
  initial feature, compatibility, performance, accessibility, devtools,
  extension, packaging, update, and search-integration gate list.
- Run standards tests and browser benchmarks as release gates.
- Keep a "fast text mode" for search extraction, but do not confuse it with
  browser-accurate rendering.

## Architecture Tracks

### 1. Web Crawler And Frontier

- Replace the in-memory crawl queue with a durable frontier store.
- Track URL state: discovered, queued, fetching, fetched, failed, deferred,
  recrawl-at, canonical target, content hash, and host politeness state.
- Implement host-level politeness: robots.txt cache, crawl-delay support where
  available, per-host concurrency, backoff, retry budgets, and failure windows.
- Add redirect canonicalization, duplicate URL detection, fragment/query
  normalization, and content-type filtering.
- Add DNS caching, connection pooling, HTTP/2 and HTTP/3 readiness, compression,
  byte caps, timeouts, and fetch telemetry.
- Add recrawl scheduling based on content change rate, page importance, and
  failure history; the current scheduler uses a fixed age interval.

### 2. Extraction And Document Understanding

- Keep the existing static HTML extractor as the fastest lane.
- Add structured extraction for title, headings, anchor text, language, charset,
  canonical link, meta robots, meta description, publication time, and outbound
  links.
- Add boilerplate reduction, production-tuned near-duplicate clustering,
  spam/malware flags, and low-quality content classifiers. Canonical,
  exact extracted-text, fixed-threshold shingled-simhash clustering, meta robots
  noindex, and configurable thin-page filtering are already implemented as early
  quality layers.
- Add optional JavaScript rendering for pages where static extraction is empty
  or obviously incomplete.
- Store raw response metadata, normalized plaintext, extracted fields, link
  graph edges, and content fingerprints.

### 3. Indexing And Storage

- Evolve the single-segment index into immutable shard segments with background
  compaction.
- Add shard manifests, checksums, schema versions, and forward-compatible
  migration tooling.
- Store lexicon, postings, positions, fields, document metadata, URL maps,
  anchor text, freshness signals, and page-quality signals separately so hot
  query paths mmap only what they need.
- Add block-max or WAND-style top-k retrieval, skip data, term-at-a-time and
  document-at-a-time query execution, and query-result caching.
- Move phrase/proximity support from the current query-layer phrase filter into
  positional postings and skip data after the core lexical top-k path is fast.
- Add offline index builders and online incremental segment writers.

### 4. Ranking And Relevance

- Start with BM25-style lexical ranking as the baseline.
- Add field weighting for title, headings, URL, body, and anchor text.
- Add richer freshness, language, location, site quality, deduplication, and
  safe-search signals; language and fetched-time metadata now exist as first
  query filters, not production ranking features.
- Add link graph scoring inspired by PageRank-style authority, but keep it as a
  feature input rather than the only ranker.
- Add learned ranking after sufficient click/judgment data exists.
- Build evaluation sets with queries, judged documents, and expected failure
  cases; track NDCG, MRR, recall, freshness, spam rate, and latency together.
  The first `brutal-bench eval` path now reports MRR, NDCG@K, recall@K,
  precision@K, unresolved judgments, and per-query diagnostics from JSONL
  judgments, with threshold gates for MRR, NDCG, recall, precision, and
  unresolved judgments.

### 5. Query Serving

- Keep `brutal-searchd` as the single-node hot-path prototype.
- Add a query server API with structured JSON/HTTP and a compact binary protocol
  for internal clients.
- Add distributed shard fanout, timeout budgets, partial results, result merging,
  ranking feature hydration, snippet generation, and cache layers.
- Extend query parsing beyond the current quoted phrases, `site:`, `filetype:`,
  `lang:`, `after:`, `before:`, `+term`, uppercase `OR`, and negative filters
  with ranking-time freshness boosts, richer boolean grouping, location filters,
  and safe defaults.
- Add query understanding beyond the current lexicon-backed autocomplete and
  bounded edit-distance spelling correction: synonyms, entity hints, freshness
  intent, and navigational-query handling.
- Add observability: p50/p95/p99 latency, recall canaries, shard health, cache
  hit rate, slow-query traces, and index freshness.
- Treat [`OPERATIONS_RELIABILITY_PLAN.md`](OPERATIONS_RELIABILITY_PLAN.md) as
  the initial SLO, topology, observability, backup/restore, rollout,
  failure-injection, cost-control, and incident-response gate list.

### 6. Browser Runtime / Chromium-Class Track

- Current seed implementation: `brutal-browser` is a static-page engine skeleton
  covering URL/file load, HTML token parsing, DOM tree construction, simple CSS
  `display` selector matching, a tiny inline/external-script DOM creation/text
  and tree mutation/traversal/insertion/`DocumentFragment`/selector-method/`innerHTML`/form-property subset, classList mutations over supported class selectors, DOM query
  collection bindings, style
  property mutations over the supported inline CSS subset, origin-scoped
  localStorage, BrowserSession-scoped sessionStorage,
  document/window lifecycle listeners, deterministic timer queue, inline `onclick` and
  `addEventListener("click", ...)` dispatch, terminal
  block-text layout, and display-list text/styled-text color commands. It is
  useful as a testable foothold, not as a standards-compatible browser.
- Split the browser effort into crates: networking, HTML parser, DOM, CSS parser,
  style cascade, layout, paint, compositing, JavaScript runtime bindings, storage,
  event loop, accessibility, and browser shell. Keep file-size and module
  boundaries explicit so parser/style/layout/raster/JS/storage work can be
  tested, benchmarked, fuzzed, and sandbox-reviewed independently.
- Choose or build a JavaScript engine strategy. A serious browser eventually needs a
  high-performance JIT or an embedded engine during early development; a pure
  from-scratch JS engine is a separate multi-year project.
- Implement the core web pipeline: URL load, fetch, parse, DOM construction,
  CSSOM, style calculation, layout, display list, RGBA raster/paint, compositor,
  input events, navigation, history, cookies, cache, service workers, profiles,
  and storage.
- Treat [`BROWSER_RENDERING_COMPOSITOR_PLAN.md`](BROWSER_RENDERING_COMPOSITOR_PLAN.md)
  as the paint/raster/compositor gate list for display-list correctness,
  rasterization, layer trees, frame scheduling, visual regressions, performance,
  GPU/resource safety, and hit testing.
- Add fonts/text shaping, images, SVG, canvas, media, accessibility tree,
  developer tools, downloads, printing/PDF strategy, profile management,
  extensions policy, and update packaging.
- Use [`PLATFORM_COMPLETENESS_PLAN.md`](PLATFORM_COMPLETENESS_PLAN.md) to track
  subsystem-specific gates and promote platform readiness only after direct
  evidence exists.
- Add process isolation, sandboxing, permission prompts, certificate validation,
  origin policy, COOP/COEP/CORP/CORS, mixed-content handling, site isolation,
  broker/zygote startup, JIT/W^X policy, OS sandbox targets, and crash
  containment.
- Treat [`SECURITY_PRIVACY_PLAN.md`](SECURITY_PRIVACY_PLAN.md) as the initial
  threat model and gate list for browser, search, render-worker, crawl, update,
  telemetry, abuse, and compliance work.
- Add compatibility gates with Web Platform Tests, testdriver/WebDriver,
  reftests, expectation files, flake policy, CI sharding, pinned Chromium
  versions, and browser performance gates such as page-load timing, JavaScript
  benchmarks, layout stress tests, and rendering correctness screenshots.
- Keep search extraction independent from browser rendering so the crawler can
  stay much faster than full page rendering whenever static HTML is enough.

Serious browser-product execution gates:

| Area | Required next step | Claim gate |
| --- | --- | --- |
| Subsystem boundaries | Define stable crate/API contracts for loading, parser, DOM, CSSOM/style, layout, paint/raster, compositor, JS/Web APIs, storage/profile, accessibility, shell, and devtools | Boundary map, owners, fuzz targets, benchmarks, and typed IPC schemas exist before cross-process claims |
| Networking/loading | Implement browser-grade navigation lifecycle, redirects, cache, cookies, TLS/certificate behavior, prioritization, service-worker handoff, downloads, and history/BFCache policy | Network/navigation WPT subsets, local fixture servers, failure-recovery tests, and per-phase timing reports |
| JS/Web APIs | Select the JS engine path, wire event loop/microtasks/timers/modules/WebIDL/fetch/storage/workers, and set resource caps | JS benchmarks, Web API WPT subsets, timeout/sandbox tests, rendered-extraction parity, and compatibility matrix |
| Layout/CSSOM | Build parser, selector engine, cascade, computed style, invalidation, block/inline/flex/grid/table/form/positioned/scroll/text layout | CSS/DOM WPT subsets, visual reftests, layout stress tests, text shaping/bidi corpus, and memory gates |
| Compositor/GPU | Move from fixture grayscale raster to RGBA screenshots, layer tree, frame scheduler, GPU path with CPU fallback, tiling, async scroll, and context-loss recovery | Pixel-diff baselines, frame-time/input-latency reports, GPU timeout tests, memory-pressure tests, and pinned Chromium comparisons |
| Security/sandbox | Split browser, renderer, GPU, network, and storage processes with site instances/OOPIFs, broker/zygote, OS sandbox profiles, origin policy, permissions, CSP, mixed content, and JIT/W^X policy | Threat model, sandbox denial tests, IPC fuzzing, origin-policy fixtures, crash-containment reports, and signed-update review |
| Storage/profile | Implement persistent profiles, cookies, localStorage, IndexedDB, Cache API, quotas, eviction, private browsing, clear-data, encryption/keychain hooks, and partitioning | Storage WPT subsets, quota/eviction tests, profile isolation tests, private-mode tests, and data deletion/export checks |
| Accessibility | Generate an accessibility tree, keyboard navigation, focus/editing/IME behavior, name/role/value mapping, and platform adapter strategy | Accessibility audits, keyboard-only smoke tests, platform-tree snapshots, and regression fixtures |
| Devtools | Add protocol endpoint, inspector, console, network panel, storage viewer, performance traces, logs, crash/debug bundles, and compatibility with automation hooks | Devtools smoke suite, protocol tests, trace artifact review, and debugging workflow acceptance |
| Release engineering | Produce signed reproducible builds, updater/rollback, crash reporting, versioned feature flags, release manifests, CI shards, WPT expectation files, flake quarantine, and release-blocking dashboards | `brutal-bench audit --claim browser --require-complete`, release checklist, security/compat/perf signoff, and documented exceptions |

### 7. User Product

- Add a minimal local search UI first: query box, results, snippets, render text,
  crawl status, and index stats.
- Add the `brutal-browser browse` CLI shell as a local playable wrapper around
  the supported static `BrowserSession` subset, including current-page link
  listing, resolved-link activation by index, text, or anchor selector,
  coordinate-click routing through display-list hit testing, and narrow anchor
  href default navigation from shell clicks.
- Add GUI browser chrome later: address bar, tabs, back/forward/reload,
  find-in-page, downloads, permissions, history, bookmarks, and crash recovery.
- Add privacy controls, telemetry opt-in, safe browsing style warnings, content
  blocking hooks, and clear crawl/data controls.
- Add packaging for macOS first, then Linux and Windows.

### 8. Benchmarks And Gates

- Search latency gate: resident daemon p95 query-plus-render latency must beat
  the headless Chromium JS baseline over the same local corpus. The current
  `brutal-bench search --chromium-baseline --require-speedup 10` command emits a
  reproducible report, can persist `bench-status.json` for the served status UI,
  and exits non-zero when the p95 speedup gate is unmet.
- Crawl throughput gate: pages fetched, extracted, deduped, and indexed per
  second under controlled local replay and live-web runs.
- Index quality gate: recall and ranking metrics on judged query sets.
- Freshness gate: time from fetch to searchable document in the served index.
- Browser compatibility gate: Web Platform Test pass rate by subsystem.
- Browser performance gate: startup, navigation, text extraction, layout, paint,
  memory, JavaScript, and input latency.
- Browser benchmark gate: Speedometer/JetStream/MotionMark-style suites or
  equivalents, multi-tab memory, power, startup, and real-page canaries with
  hardware, OS, Rust, build profile, and pinned Chromium version recorded.
- Reliability gate: crash-free daemon uptime, corrupt-index recovery, crawl
  restart correctness, and deterministic benchmark reports.
- Claim readiness gate: `brutal-bench readiness --require-complete` must pass
  before any search or browser completion claim. The report
  keeps plan coverage, search, browser, JavaScript/Web APIs, security, platform,
  and operations gaps explicit. Security/privacy is partial only while the
  threat model exists but sandbox, policy, privacy, fuzzing, and compliance
  gates are still unfinished. Operations/reliability is partial only while the
  SLO/runbook plan exists but dashboards, restore drills, failure injection,
  load tests, release manifests, and incident automation are still unfinished.
  Platform completeness is partial only while the subsystem plan exists but
  fonts/text, images/SVG, canvas/GPU, media, accessibility, input/editing,
  storage, devtools, extensions, packaging, updates, and platform QA gates are
  still unfinished.

## Milestones

### Milestone 0: Harden The Current Core

- Keep the current CLI/daemon/index working.
- Add clearer crawl-first UX, benchmark docs, and reproducible local fixtures.
  The current `brutal-bench smoke` command builds a deterministic fixture
  corpus, searches, renders, and benchmarks from one command.
- Add more extractor and index tests before changing storage formats.

Exit gate: one-command local crawl, daemon search, render, and benchmark all
work from the README.

### Milestone 1: Serious Single-Machine Web Search

- Durable frontier.
- Better URL canonicalization and robots handling.
- Incremental segment writing.
- Static extraction fields and duplicate detection.
- Local HTTP search API and basic UI.

Exit gate: crawl and search at least one million static pages on one machine
with recoverable restarts and p95 search latency targets.

### Milestone 2: Distributed Search Engine

- Sharded crawl frontier.
- Fetch workers, parse workers, index builders, segment merger, and query
  serving nodes.
- Replicated shard storage and deployable service configs.

Exit gate: distributed crawl/index/search pipeline with shard fanout, health
checks, and reproducible performance reports.

### Milestone 3: Relevance Engine

- Fielded ranking, anchor text, freshness, link graph features, dedupe clusters,
  spam filters, and an expanding evaluation harness.
- Query logs and judgments with privacy controls.

Exit gate: relevance metrics improve over lexical baseline without breaking
latency budgets.

### Milestone 4: JavaScript-Aware Search

- Optional render workers for pages that need JavaScript.
- Render cache and timeout policy.
- Extraction parity tests between static and rendered pages.

Exit gate: JS-heavy pages become searchable while the static fast lane remains
the default.

### Milestone 5: Browser Engine MVP

- Browser shell plus minimal engine that can load, lay out, and paint simple
  HTML/CSS pages.
- Basic DOM, events, navigation, cookies, cache, and storage.
- JavaScript integration strategy selected and wired.

Exit gate: passes a defined subset of standards tests and renders a curated page
set without Chromium/WebKit/Gecko.

### Milestone 6: Full Desktop Browser Product

- Expand standards coverage across HTML, CSS, DOM, Fetch, Storage, Workers,
  Canvas, Media, Accessibility, and security boundaries.
- Keep browser coverage gates current so missing platform surfaces are explicit
  before claiming broad browser progress.
- Add multi-process sandbox architecture.
- Add performance optimization across parser, style, layout, raster, compositing,
  JavaScript, memory, and startup.

Exit gate: competitive benchmark results, measurable standards coverage, and a
browser that can handle common modern sites. "Common modern sites" means a
versioned canary set with login-free news, commerce, docs, media, maps-like,
dashboard, and app-shell pages; gates must record load success, visual diff,
interaction success, console/network errors, crash-free minutes, memory, power,
and pinned Chromium comparison for the same URLs.

## Immediate Next Engineering Steps

1. Evolve the recrawl scheduler from fixed-interval single-machine batches to
   per-site freshness policy, persisted due queues, incremental indexing, and
   richer changed-page reports.
2. Add production hardening for the durable frontier: compaction, host-state
   reports, robots cache persistence, and restart/resume integration tests.
3. Add reproducible crawl fixtures and benchmark corpora.
4. Tune near-duplicate and thin-content thresholds on real crawl fixtures, add
   canonical quality policy, and expand quality filters.
5. Expand benchmark corpora, relevance diagnostics, and result exploration now
   that the search UI has filters, host-level crawl details, and benchmark
   status.
6. Expand `brutal-browser` beyond the current fixture verifier with a
   standards-test-backed parser/style/layout subset, curated display-list
   baselines, WPT expectation files, and screenshot baselines.
7. Expand the tiny script subset into a JS-render-worker design and API:
   parser/VM strategy, event loop model, DOM bindings, timeout policy, render
   cache, and extraction fallback rules.
8. Define the browser subsystem boundary map and owner/gate matrix so
   networking/loading, JS/Web APIs, layout/CSSOM, compositor/GPU, storage,
   accessibility, devtools, and sandbox work can land independently.
9. Add release-engineering gates for browser artifacts: signed builds,
   updater/rollback drills, WPT/compat dashboards, crash reporting, and
   documented exception policy.
10. Keep improving the current hot search path while larger systems are added.

## Completion Definition

This goal is not complete when the current static crawler works. It is complete
only when the project has:

- A continuously refreshable web index.
- Distributed crawl/index/query serving architecture.
- Relevance evaluation and ranking beyond simple lexical search.
- A user-facing search experience.
- A browser/runtime architecture that can independently render modern web pages.
- Compatibility, performance, security, and reliability gates proving progress
  against named browser-product expectations.
- A completion matrix tying every search, browser, platform, security, and
  operations claim to reproducible evidence before any public claim ships.
- A passing `brutal-bench readiness --require-complete` report.
