# Brutal Browser Program Plan

This is the execution plan for turning the current Rust scaffold into an
independent browser engine and desktop browser product. Blackium Starium✴ remains a
fast text/index/extraction mode inside that browser-first project. It is not the
main identity anymore.

The plan is intentionally concrete: each workstream has deliverables,
dependencies, and acceptance gates. The current implementation is an early
fixture-backed engine scaffold and search mode, not a modern browser and not a
desktop browser product.

## Mission And Non-Negotiables

- Build an independent Rust browser engine and shell that does not use
  Chromium, WebKit, or Gecko as its page engine.
- Correctness comes before speed.
- Treat static HTML extraction/search as the fast text lane, not as a substitute
  for browser rendering.
- Do not add random tiny browser features unless they move a staged browser
  subsystem toward a named milestone.
- Benchmarks must count failed renders, blank pages, crashes, missing critical
  assets, timeouts, and correctness mismatches as failures.
- Every "better than Chromium" claim must name the page class, correctness
  gates, hardware, OS, Rust build profile, Chromium version, reproducible
  command, and report artifact.
- Do not claim broad browser readiness until standards, security, rendering,
  JavaScript, platform, and user-product gates prove the exact scope.
- Do not claim Google-style search until crawl freshness, relevance, indexing,
  serving, abuse controls, and user experience gates prove it.

## Requirement Traceability

The compact table below summarizes the major requirements. The full
requirement-to-gate audit matrix lives in
[`REQUIREMENTS_TRACEABILITY.md`](REQUIREMENTS_TRACEABILITY.md) and is part of
the readiness evidence surface. The command, fixture, report, external-suite,
and release-bundle proof map lives in
[`EVIDENCE_REGISTRY.md`](EVIDENCE_REGISTRY.md). The performance-first browser
architecture and benchmark sequence lives in
[`PERFORMANT_RUST_BROWSER_PLAN.md`](PERFORMANT_RUST_BROWSER_PLAN.md).
The browser-first product strategy lives in
[`BROWSER_ENGINE_STRATEGY.md`](BROWSER_ENGINE_STRATEGY.md).

| Requirement | Required workstreams | Evidence required |
| --- | --- | --- |
| Independent Rust browser | Loading, DOM, style, layout, paint, raster/compositor, shell | Document corpus gates, visual/text correctness, browser perf/memory/stability reports |
| Fast document browser | HTML/CSS/images/forms/history/tabs/scroller/shell | Stage 1 document-page corpus with no failed renders and reproducible Chromium comparison |
| Search the public web | Frontier, crawler, extraction, index, query serving | Recoverable crawl of large seed set, searchable documents, freshness report |
| Google-style relevance | Ranking, link graph, quality, evaluation | Judged query set, NDCG/MRR reports, spam/duplicate metrics |
| Low-latency search | Index format, daemon, sharding, caches | p50/p95/p99 benchmark reports over reproducible corpora |
| Continuously refresh index | Scheduler, recrawl policy, incremental indexing | Recrawl simulations, changed-page freshness reports |
| Render/search JS-heavy pages | Render workers, browser runtime bridge | Static-vs-rendered extraction parity reports |
| Full browser path | Engine, JS, layout, paint, sandbox, shell | Web Platform Test subset, screenshot tests, perf reports |
| Modern browser platform | Media, canvas, fonts, accessibility, devtools, extensions | Feature coverage matrix and subsystem benchmarks |
| User-facing product | Search UI, browser shell, packaging | Usable app builds, smoke tests, UX acceptance checklist |
| Safe production operation | Security, privacy, compliance, observability | Threat model, audit logs, metrics dashboards, failure drills |

## Workstream A: Search Corpus Acquisition

Deliverables:

- Seed list importer for URLs, domains, sitemaps, and local corpora. The current
  CLI covers URL seed files, domain roots, domain files, manually supplied
  XML/XML.gz sitemaps, explicit robots.txt sitemap discovery, and JSONL recrawl
  manifests.
- Durable frontier database with URL states and host politeness metadata.
- Robots.txt cache with per-agent policy, crawl-delay handling where present,
  sitemap discovery, and expiration.
- Host scheduler with per-host concurrency, global concurrency, backoff, retries,
  failure windows, and recrawl timestamps.
- Fetch telemetry: status, bytes, latency, content type, redirects, canonical URL,
  compression, and TLS errors.

Dependencies:

- Existing `crawler` module remains as the prototype fetcher.
- Requires a persistent storage choice before production use.

Acceptance gates:

- Stop and resume a crawl without losing frontier state.
- Crawl at least 1 million URLs from a seed list on one machine with bounded RAM.
- Import and dedupe domain files, seed files, manually supplied sitemaps, and
  robots-discovered sitemaps deterministically before fetch work begins.
- Import JSONL recrawl manifests and honor `priority` plus `recrawl_after` for
  single-run due filtering.
- Requeue already-fetched frontier records when due recrawl inputs target them,
  and rebuild from the latest snapshot row for repeated document URLs.
- Generate JSONL recrawl manifests from fetched frontier records using a
  deterministic age-based due policy.
- Run a bounded or continuous recrawl scheduler loop that selects due frontier
  batches, refreshes those URLs, rebuilds from latest crawl snapshots, and emits
  changed/unchanged/missing counts.
- Respect host politeness and robots rules by default.
- Produce deterministic crawl state reports.

## Workstream B: Document Extraction And Normalization

Deliverables:

- Fielded document model: URL, canonical URL, title, meta description, language,
  charset, headings, body text, anchor text, outbound links, content hash,
  response metadata, and extraction mode.
- Static HTML extractor with fixture coverage for malformed HTML, encodings,
  boilerplate, hidden/noisy sections, canonical tags, and meta robots.
- Canonical, exact extracted-text, and fixed-threshold shingled-simhash
  duplicate clustering in index/query serving; production tuning against crawl
  fixtures remains required.
- Optional rendered extraction lane for pages whose static text is empty,
  blocked, or low-confidence.
- Content quality flags: meta robots noindex and configurable thin-page
  filtering are implemented; spam, malware/suspicious links, adult content where
  configured, and richer indexing exclusions remain required.

Dependencies:

- Search crawler supplies raw responses and metadata.
- Browser/runtime or external render worker supplies rendered DOM text later.

Acceptance gates:

- Extraction quality measured against fixture pages and real-page samples.
- Duplicate clusters reduce repeated results without deleting canonical copies.
- Canonical, exact-text, and near-duplicate fixtures prove repeated results
  collapse while direct render by document id or URL still works.
- Noindex and thin-content fixtures prove excluded pages are counted and omitted
  from searchable documents unless explicitly allowed/configured.
- Static extraction remains the default fast path.

## Workstream C: Indexing And Storage

Deliverables:

- Versioned index schema with manifests, checksums, migrations, and compatibility
  policy.
- Immutable segment writer and reader with background compaction.
- Field-aware postings: body, title, headings, URL, anchor text, and metadata.
- Skip lists or block-max data for WAND-style top-k retrieval.
- Document store for metadata, snippets, text blobs, link graph features, and
  quality/freshness signals. The current single-segment index already stores a
  first normalized link-authority score in document metadata.
- Incremental indexing pipeline from fetched documents to searchable segments.
- Sharding plan: by document id or URL hash for corpus ownership; replicated
  shard manifests for serving.

Dependencies:

- Current `index` module is the seed implementation.
- Requires fielded document model from Workstream B.

Acceptance gates:

- Build and search a 1 million document single-machine index.
- Merge incremental segments without corrupting the index.
- Recover from interrupted indexing using checksums and manifests.
- Query latency stays within explicit p95 budgets after adding fields.

## Workstream D: Query Processing And Serving

Deliverables:

- Query parser with terms, quoted phrases, required/excluded terms, site filters,
  filetype filters, language filters, and safe defaults. The current hot path
  implements quoted phrase filters, negative phrases, `site:`, `filetype:`, and
  negative term/site/filetype filters as the first operator layer; it also now
  persists language/fetched-time metadata for `lang:`, `after:`, and `before:`
  filters, plus `+term` required terms and uppercase `OR` term groups.
- Query understanding pipeline beyond the current lexicon-backed prefix
  autocomplete and bounded edit-distance spelling correction: synonyms,
  stemming, entity hints, freshness intent, navigational queries, and safe query
  rewriting.
- Resident query service with HTTP/JSON API and internal compact protocol.
- Shard fanout coordinator with timeout budgets, partial-result behavior, result
  merging, and ranking feature hydration.
- Snippet generator using field offsets and highlighted matches.
- Query cache, posting cache, document metadata cache, and slow-query tracing.
- Public search UI backed by the query API.

Dependencies:

- Current `brutal-searchd` remains the single-node prototype.
- Needs sharded index metadata from Workstream C.

Acceptance gates:

- Serve concurrent queries under load without daemon crashes.
- p95 latency reported separately for parse, retrieval, ranking, snippet, and
  network overhead.
- Degraded shard behavior is defined and tested.

## Workstream E: Ranking, Quality, And Relevance

Deliverables:

- Lexical baseline: BM25 or variant with field weights.
- Anchor text and link graph feature extraction. The current implementation has
  the first version of this: outbound links from crawled/static documents feed a
  normalized PageRank-style authority feature used by lexical ranking.
- Page quality, freshness, language, location, dedupe, and safety features.
  Language and fetched-time query filters are implemented as the first metadata
  filters; ranking-time freshness and production language quality still need
  evaluation.
- Evaluation harness with query sets, judgments, metrics, and regression gates.
- Offline ranking experiments and online-safe configuration system.
- Query logs and click feedback only with explicit privacy controls.

Dependencies:

- Needs fielded documents, link graph data, and query serving.

Acceptance gates:

- Relevance improves over lexical baseline on judged sets.
- Spam and duplicate rates are measured and bounded.
- Ranking changes cannot ship without latency and quality reports.

## Workstream F: Browser Engine Foundations

Deliverables:

- Seed implementation now exists as `brutal-browser`: local/HTTP(S) static HTML
  loading, independent DOM construction, simple CSS `display` and `color`
  matching, block text layout, deterministic text/styled-text display-list
  output, terminal render output,
  anchor-link extraction/resolution, static subresource discovery for scripts,
  stylesheets, images, media, frames, icons, preloads, manifests, and embeds,
  static subresource fetch reports with in-memory duplicate-resource cache hits,
  external stylesheet fetch/application for the existing display/color-rule text
  layout path, static form extraction, GET form submission URL construction,
  query-safe local loading, GET form submission navigation, session-history
  navigation, CLI current-page-relative open/go target resolution, bounded
  HTTP(S) redirect following with redirect-set cookie propagation, in-memory
  session cookies, CLI cookie inspection/clearing, optional JSON cookie-jar and
  localStorage load/save plus localStorage and sessionStorage inspection/clearing
  for the browse shell, JSON inspection, capability
  reporting, machine-readable feature-fixture
  coverage/gate reporting, fixture-manifest
  verification for title/text/display-list drift, a tiny inline JavaScript
  subset for document title/text mutations plus
  `createElement`/`createTextNode`/`appendChild`, tree mutation methods,
  DOM traversal properties, insertion convenience methods,
  `DocumentFragment` insertion, selector element methods, `innerHTML` mutation/readback, form-control DOM properties, location readback properties, `setAttribute`/`getAttribute`,
  classList mutation/readback over supported class selectors, DOM query
  collection bindings, style property
  mutation/readback over the supported inline CSS subset,
  origin-scoped localStorage and BrowserSession-scoped sessionStorage
  `setItem`/`getItem`/`removeItem`/`clear`/`length`,
  document/window lifecycle listener dispatch,
  deterministic `setTimeout`/`clearTimeout` task-queue draining,
  external script fetch/application for that same tiny DOM mutation subset,
  inline `onclick` plus `addEventListener("click", ...)` dispatch for first
  interaction fixtures, local image-aware rerendering through
  `brutal-browser render-images` for fetched image resources in the currently
  supported SVG/PNG/data-URL subset, local display-list hit-test reporting
  through `brutal-browser hit-test`, an early `brutal-browser browse`
  local CLI shell over the supported `BrowserSession` subset for
  open/back/forward, current-page reload/refresh, current-page link listing,
  resolved-link activation by zero-based index, exact text, or anchor selector,
  selector click with narrow anchor href default navigation through
  `BrowserSession` history, coordinate click routing through display-list hit
  testing into the supported generated
  `pointerdown`/`mousedown`/`pointerup`/`mouseup`/click/default-action navigation
  path, narrow document-level wheel dispatch before local shell viewport scroll
  movement with `event.deltaX`/`event.deltaY` readback and `preventDefault()`
  cancellation, remembered
  text-like form field values on the current `BrowserSession` entry for later
  GET form submission, single-select choices for later submission,
  checkbox/radio checked-state toggles plus label defaults,
  selector/associated-label focus across fillable/select/checkable/submit-reset
  controls and typed-text append for editable text-like controls,
  fixed text-viewport scroll, current-page render, optional scripted `--cmd` runs,
  and headless Chromium title/text parity comparison for static, tiny-script,
  tiny-attribute, and tiny-click fixtures. The CLI shell link activation is
  session navigation to an extracted resolved href, and the shell click default
  is limited to anchors with resolved hrefs after supported click handlers
  dispatch; coordinate clicks are local terminal-cell/display-list routing only
  with narrow generated `pointerdown`/`mousedown`/`pointerup`/`mouseup` events
  before click/default action.
  The `browser-session-form-submit-button-click-default` marker is scoped only to
  BrowserSession/CLI submit/input/button click default action on GET or
  URL-encoded POST forms, and `browser-session-form-reset-click-default` is
  scoped only to reset-control click default action.
  The focused enter markers are scoped only to submitting focused
  fillable/select controls, activating focused submit controls with submitter
  state, and resetting focused reset controls.
  The `browser-session-select-form-state` and
  `browser-shell-select-form-choice` markers are scoped only to single-select
  option metadata, enabled-option validation, and explicit/focused CLI choice
  commands. The `browser-session-checkable-form-state` and
  `browser-shell-checkable-form-toggle` markers are scoped only to supported
  checkbox/radio checked-state persistence, explicit toggle/focused-space
  commands, and narrow selector-click/label defaults.
  The `browser-session-required-form-validation` marker is scoped only to
  value-missing checks for supported required text-like, select, checkbox, and
  radio controls in BrowserSession/CLI submit paths, honoring form
  `novalidate` and submitter `formnovalidate`.
  The `browser-session-type-value-validation` marker is scoped only to non-empty
  email/URL value checks on those same submit paths.
  The `browser-shell-reload` marker is scoped only to reloading the current
  `BrowserSession` entry target without pushing a new history entry.
  The CLI shell is not GUI browser chrome, full JS/CSS/browser
  interaction, full event/default-action and event-cancellation semantics,
  full pointer routing, full PointerEvent semantics, full MouseEvent semantics,
  full WheelEvent semantics, full scroll-container behavior, full interactive form state, full label
  activation semantics, full constraint validation, validation UI, keyboard events, selection, IME, autofill,
  POST, full reload lifecycle, cache policy, service worker handling,
  tab/process isolation, devtools/accessibility, or Chromium parity.
- Browser workspace split into crates: URL/networking, HTML parser, DOM, CSS
  parser, style cascade, layout, paint, compositor, JS bindings, storage,
  permissions, accessibility, and shell.
- Event loop and navigation model covering URL/IDNA/scheme handling, redirects,
  HTTP cache, cookies, service worker interception, preload/preconnect,
  BFCache/session restore, downloads, and file/external-app handoff.
- Site-instance/frame-tree process model with out-of-process iframes and
  separate browser, renderer, GPU, network, and storage processes.
- DOM tree construction and mutation APIs.
- CSS parser, selector matching, cascade, inheritance, and computed style.
- Font discovery, fallback, shaping, bidi text, emoji, internationalization, and
  text selection.
- Layout engines in order: block, inline text, replaced elements, flexbox, grid,
  tables, forms, transforms, and scrolling.
- Paint/display list, rasterization, compositing, invalidation, hit testing, and
  local layer-tree/debug snapshots. The current gate plan is
  [`BROWSER_RENDERING_COMPOSITOR_PLAN.md`](BROWSER_RENDERING_COMPOSITOR_PLAN.md),
  which records display-list, paint-order, rasterization, compositor, visual
  regression, performance, security, reliability, and implementation gates.
- Images, SVG, canvas, WebGL/WebGPU roadmap, media elements, audio/video
  pipeline, fullscreen, pointer/keyboard/touch input, clipboard, printing, PDF
  viewing/export, and accessibility tree. The current baseline is
  [`PLATFORM_COMPLETENESS_PLAN.md`](PLATFORM_COMPLETENESS_PLAN.md), which
  records subsystem gates for fonts/text shaping, images/SVG, CSS visual
  effects, canvas/GPU, media, accessibility, input/editing, storage/profiles,
  downloads/files, devtools, extensions, packaging, updates, and search
  integration.
- Developer tools plan: inspector, console, network panel, performance tracing,
  storage viewer, protocol endpoint, and crash/debug reports.
- Extension/plugin policy and API compatibility strategy.
- Web Platform Test runner integration and curated compatibility dashboard. The
  runner must support testdriver/WebDriver automation, reftests, expectation
  files, flake quarantine, CI sharding, pass thresholds, and pinned Chromium
  comparison versions. The current local WPT-subset and image-render commands
  are scaffold evidence over checked-in fixtures and supported local subsets;
  they are not full upstream WPT coverage or Chromium parity evidence.

Dependencies:

- Can start with the tiny `brutal-browser browse` CLI shell independent of the
  search index; GUI chrome remains a later product layer.
- Needs JS strategy from Workstream G for modern sites.

Acceptance gates:

- Current seed gate: deterministic unit tests for DOM parse, script/style
  exclusion, CSS display hiding and color styling, wrapping, text/styled-text
  display-list coordinates, plus link extraction,
  subresource discovery/fetch/cache reports, external stylesheet application,
  form extraction/submission URL construction, local query-string navigation,
  HTTP cookie round trips, session back/forward behavior, CLI smoke
  rendering/display-list output, `browse --cmd` local shell smoke runs for link
  listing/activation, coordinate-click routing, submit-control and reset-control
  click default action, viewport rendering, coverage tracking, and fixture
  verification of a local page.
- Render curated static HTML/CSS pages with screenshot baselines.
- Pass defined Web Platform Test subsets by subsystem.
- Pass browser feature smoke suites for text shaping, images, forms, canvas,
  media, accessibility, and input.
- Track rendering performance and memory over test pages.

## Workstream G: JavaScript And Web APIs

Deliverables:

- JavaScript strategy decision: embed an engine first, build one later, or a
  hybrid. A real modern app browser eventually needs JIT-class performance.
- DOM bindings, Web IDL strategy, promises/microtasks, timers, events, fetch,
  storage, workers, modules, and error reporting.
- WebAssembly, WebCrypto, WebSockets, WebTransport, bytecode cache policy, GC
  and JIT budgets, SharedArrayBuffer gated by cross-origin isolation, and
  explicit WebRTC/media-capture implementation or deferral policy.
- Security boundaries for origins, cross-origin policy, mixed content, cookies,
  storage partitioning, and permissions.
- Render-worker API for the search crawler to ask for rendered text when needed.

Dependencies:

- Browser DOM/event loop foundations.
- Security architecture from Workstream H.

Acceptance gates:

- JS-enabled curated site set loads and becomes searchable/renderable.
- API conformance tracked with Web Platform Tests.
- Timeouts and resource caps prevent render-worker abuse.

## Workstream H: Security, Privacy, And Compliance

Deliverables:

- Threat model for crawler, index, query service, browser shell, render workers,
  and update/distribution pipeline. The current baseline is
  [`SECURITY_PRIVACY_PLAN.md`](SECURITY_PRIVACY_PLAN.md), which records assets,
  threat actors, trust boundaries, browser/search requirements, render-worker
  requirements, and security gates.
- Sandboxing/process isolation plan for site instances, frame trees, OOPIFs,
  browser tabs, render workers, renderer/GPU/network/storage processes, broker
  or zygote startup, OS sandbox profiles, JIT/W^X policy, and privileged UI.
- TLS validation, certificate error behavior, safe downloads, permission prompts,
  content security policy support, COOP/COEP/CORP/CORS, and site isolation
  roadmap.
- Privacy model: telemetry opt-in, query log minimization, crawl data retention,
  user data export/delete, and storage partitioning.
- Compliance checklist for robots.txt, takedown handling, copyright-sensitive
  cached content policy, user data protection, and abuse response.

Dependencies:

- Must be designed before public deployment or browser packaging.

Acceptance gates:

- Security review checklist completed for each network-exposed component.
- Browser/runtime does not ship without sandbox boundaries.
- Search logs and telemetry have explicit retention and opt-in policy.

## Workstream I: Infrastructure And Operations

Deliverables:

- Local dev environment, reproducible corpora, fixture generator, and benchmark
  scripts. The current baseline is
  [`OPERATIONS_RELIABILITY_PLAN.md`](OPERATIONS_RELIABILITY_PLAN.md), which
  records service topology, SLOs, observability, backup/restore,
  deployment/rollback, failure injection, capacity, incident response, and
  operations gates.
- Service configs for crawler workers, parse workers, index builders, shard
  servers, query frontends, benchmark runners, and browser test runners.
- Observability: metrics, structured logs, traces, health checks, alert rules,
  dashboards, and crash reports.
- Backup/restore for frontier, raw crawl metadata, index manifests, and ranking
  data.
- Deployment strategy from single-machine to multi-node cluster.

Dependencies:

- Needed before distributed crawl and serving.

Acceptance gates:

- Full local pipeline can be started from documented commands.
- Production-like pipeline can be deployed and health-checked.
- Failure drills prove restart and recovery behavior.

## Workstream J: Product Experience

Deliverables:

- Search UI: query box, results, snippets, filters, full-text render, crawl
  status, index stats, and benchmark status.
- Browser shell: address bar, tabs, history, downloads, permissions, bookmarks,
  find-in-page, settings, crash recovery, dev/debug panels, persistent profile
  management, private browsing isolation, password/autofill policy, update flow,
  and import/export.
- Search product features: current prefix autocomplete and first-pass spelling
  suggestions plus future related queries, cached text policy, result filters,
  freshness labels, site links where available, and clear crawl/index
  transparency controls.
- Packaging: macOS first, then Linux and Windows.
- User documentation for crawling, indexing, searching, benchmarking, and browser
  testing.

Dependencies:

- Search UI depends on query API.
- GUI browser shell depends on browser engine foundations; the CLI `browse`
  shell can exercise the current static session subset, including link listing
  resolved-link activation by index, text, or anchor selector, coordinate-click
  routing through display-list hit testing, narrow anchor href default navigation
  from shell clicks, and supported submit-control/reset-control click default
  action.

Acceptance gates:

- A user can install/run, crawl, search, benchmark, and inspect results without
  reading source code.
- Browser MVP can navigate, render, and interact with the curated page set.

## Cross-Workstream Sequence

1. Stabilize current CLI/daemon/index and docs.
2. Add durable frontier and richer extracted document schema.
3. Move from one-shot index builds to incremental segment writing.
4. Add HTTP query API and minimal search UI.
5. Add benchmark corpora, quality evaluation, and regression gates.
6. Scale to million-page single-machine crawl and search.
7. Add sharding, distributed serving, and operations.
8. Add ranking features, link graph, dedupe, and quality systems.
9. Add JS-rendered extraction for search.
10. Expand search-product query understanding beyond the current prefix
    autocomplete.
11. Start browser engine MVP with HTML/CSS/static rendering.
12. Add JavaScript/Web APIs, sandboxing, and standards gates.
13. Add media, canvas, accessibility, devtools, extension policy, and platform
    completeness work.
14. Build browser shell and packaging.
15. Iterate until search and browser gates prove real competition.

## Immediate Implementation Backlog

P0:

- Keep expanding the fixture corpus and smoke pipeline. Current
  `brutal-bench smoke` builds `bench/fixtures/corpus`, searches, renders the top
  hit, and emits an in-process benchmark report from one command.
- Keep hardening benchmark correctness and failure accounting. Current
  `brutal-bench search
  --chromium-baseline --require-speedup 10` records p50/p95/p99, throughput,
  Rust/Chrome/OS/hardware metadata, corpus hash, index hash, optionally persists
  `bench-status.json`, and fails when the p95 speedup gate is unmet.
  Chromium baselines should be reserved for explicit claim or release gates
  while Stage 1 browser development focuses on local document rendering,
  interaction, memory, and correctness gates.
- Add relevance evaluation coverage. Current `brutal-bench eval` reads JSONL
  judgments and reports MRR, NDCG@K, recall@K, precision@K, unresolved
  judgments, and per-query diagnostics. It also supports threshold gates for
  MRR, NDCG, recall, precision, and unresolved judgments.
- Add browser coverage gating. Current `brutal-browser coverage` reports
  implemented, partial, and missing browser/runtime features and can fail when
  required feature IDs or implemented-ratio thresholds are unmet. Its unweighted
  ratios measure feature-fixture progress only; they are not browser-completion
  percentages and cannot justify Chromium-class claims by themselves.
- Add top-level claim audit gating. Current `brutal-bench audit` composes
  traceability, evidence-registry coverage, and readiness for `--claim search`,
  `--claim browser`, or `--claim combined`, and fails with
  `--require-complete` until that exact claim has no partial requirements,
  uncovered evidence, or partial readiness areas. Current
  `brutal-bench readiness` emits the machine-readable search plus browser
  readiness area audit across plan coverage,
  crawling/freshness, indexing/storage, relevance/quality, serving/product UX,
  browser engine, JavaScript/Web APIs, security/privacy, platform completeness,
  and operations/reliability. Use `--require-complete` only when every area has
  direct evidence. The
  security/privacy area is currently partial because the threat model exists but
  sandbox, policy, fuzzing, privacy, and compliance gates do not. The
  operations/reliability area is currently partial because the SLO/runbook plan
  exists but dashboards, restore drills, failure injection, load tests, release
  manifests, and incident automation do not.
- Add platform completeness gating. Current
  [`PLATFORM_COMPLETENESS_PLAN.md`](PLATFORM_COMPLETENESS_PLAN.md) makes the
  browser platform area partial instead of missing, but implementation,
  standards subsets, visual regression, accessibility, media/canvas, devtools,
  extension, packaging, update, and platform QA gates are still required.
- Add production hardening for frontier persistence, host reports, robots cache
  persistence, and recrawl scheduler restart tests.
- Continue expanding `serve`; it now has a human-readable rendered-text page for
  results and a human-readable crawl status page with host-level frontier
  details, first-pass filters for site, file type, language, and freshness, and
  a benchmark status page backed by saved benchmark reports.

P1:

- Add segment manifests, checksums, and incremental segment writer.
- Tune field weights with a relevance fixture set.
- Tune near-duplicate/thin-content detection and add canonical quality policy.
- Add crawler restart/resume integration tests.
- Add local search UI.
- Improve spelling correction with language-aware dictionaries and query-level
  rewrites.

P2:

- Add sharding abstractions and shard fanout query serving.
- Add link graph extraction and offline graph scoring.
- Add relevance evaluation harness.
- Add JS-render worker design and prototype.
- Add browser subsystem specs for fonts/text, media, canvas, accessibility,
  devtools, and sandbox boundaries.

P3:

- Split the current `brutal-browser` module into browser workspace crates and
  shell.
- Expand DOM/CSS/layout and add paint/display-list MVP for static pages.
- Integrate a JavaScript engine strategy.
- Add Web Platform Test runner and screenshot baseline tests.
- Add browser platform feature matrix and performance dashboard.

## Completion Audit Checklist

The plan should be treated as complete only when every item below has direct
evidence:

| Area | Required end state | Evidence gate |
| --- | --- | --- |
| Search crawling | Continuous polite crawl, sitemap discovery, robots, recrawl, change detection, failure recovery | Crawl dashboards, host stats, refresh reports, replayable crawl fixtures |
| Search indexing | Incremental/sharded index builds, durable manifests, corruption checks, rollback | Index integrity verifier, shard health, corpus/index hashes |
| Search relevance | Link analysis, field ranking, dedupe, spam/quality, freshness, query understanding | Judged query suites, NDCG/MRR/recall, spam and duplicate metrics |
| Search serving | Low-latency daemon, shard fanout, cache, snippets, render, suggestions, spell correction | p50/p95/p99 benchmarks, load tests, API/UI integration tests |
| Search product | Google-style result workflows, filters, crawl/search status, saved benchmark visibility | Usability fixtures, browser tests, production-like smoke runs |
| Browser engine | Independent network, parser, DOM, CSS cascade/layout, paint/raster, compositor | Standards subsets, screenshot/visual regression, layout and paint benchmarks |
| JavaScript/Web APIs | JS engine strategy, WebIDL generation, event loop, DOM bindings, timers, fetch, storage, workers, modules, WebAssembly, WebCrypto, WebSockets/WebTransport | Web Platform Test subsets, JS benchmarks, API compatibility matrix |
| Security/privacy | Sandboxing, site isolation, origin model, CSP, COOP/COEP/CORP/CORS, mixed content, permissions, privacy controls | Threat model, fuzzing, sandbox tests, policy conformance tests |
| Platform completeness | Fonts, images, media, canvas, accessibility, storage/profiles, shell workflows, devtools, extensions, packaging | Feature coverage gates, platform smoke suites, accessibility audits |
| Operations | Observability, crash recovery, deploy/rollback, backups, incident playbooks | SLO dashboards, failure-injection tests, restore drills |

- Search corpus can be crawled, refreshed, indexed, and served continuously.
- Search quality is evaluated with judged queries and relevance metrics.
- Search performance is evaluated with reproducible p50/p95/p99 reports.
- System can recover from crawler, indexer, shard, and query-server failures.
- Search UI exists and covers normal user workflows.
- Browser runtime has independent networking, parsing, DOM, CSS, layout, paint,
  JS/Web API, storage, security, and shell components.
- Browser platform covers fonts/text shaping, images, media, canvas, input,
  accessibility, developer tools, and packaging decisions.
- Browser compatibility and performance are tracked with standards and visual
  regression tests, pinned Chromium baselines, Speedometer/JetStream/MotionMark
  style gates, multi-tab memory/power/startup reports, and real-page canaries.
- Security, privacy, compliance, observability, and packaging have shipped
  gates, not just notes.
- `brutal-bench audit --claim combined --require-complete` passes; until then
  the project is explicitly a partial search/browser engine and must not claim
  full product completion.
