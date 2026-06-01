# Browser And Search Requirements Traceability

This matrix is the audit map for the full ambition: an independent Rust browser
engine and desktop browser product, with fast search/index/extraction as a mode
of that engine. It is not proof that the product exists. It defines what must
be true, where the plan owns it, and what evidence must exist before any public
browser, search, or comparison claim is allowed.

For the browser-first product strategy, see `BROWSER_ENGINE_STRATEGY.md`.

## Objective Boundary

In scope:

- Public-web text search with continuous crawl, extraction, indexing, ranking,
  serving, product UI, freshness, abuse controls, and measurable relevance.
- A browser/runtime that independently implements networking, parsing, DOM,
  CSS, layout, paint/raster, compositing, JavaScript/Web APIs, storage,
  accessibility, media/canvas, security boundaries, and a desktop shell.
- Reproducible performance, compatibility, security, privacy, reliability, and
  operations gates.

Out of scope for the first public claim:

- Paid ads, paid inclusion, or monetization ranking. Those require separate
  policy, disclosure, abuse, relevance, and legal gates before shipping.
- A Google account ecosystem, cloud sync product, office suite, email service,
  mobile operating system, or app store.
- Mobile browser parity. Desktop packaging is the first platform target; mobile
  requires its own platform plan before any mobile claim.

## Evidence States

| State | Meaning |
| --- | --- |
| Implemented | Direct code, docs, fixtures, commands, and gates prove the required end state. |
| Partial | Some direct evidence exists, but production or standards-scale gates remain unfinished. |
| Missing | No direct evidence proves the requirement is materially covered. |

`brutal-bench readiness --require-complete` may pass only when every requirement
that supports a public claim has implemented evidence, not just a plan.

## Traceability Matrix

Milestones follow `COMPETITOR_ROADMAP.md`: `M0` current-core hardening, `M1`
single-machine web search, `M2` distributed search/operations, `M3` relevance,
`M4` JavaScript-aware search, `M5` browser engine MVP, and `M6` desktop browser
product readiness.

Claim scopes are machine-validated: `Search` rows gate search claims, `Browser`
rows gate browser claims, and `Shared` rows gate both.

| Requirement ID | Required capability | Necessary steps | Primary gates | Plan owners | Readiness area | Milestone | Claim scope | Current state |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| REQ-SEARCH-CORPUS | Continuously refreshed public-web corpus | Seed/domain/sitemap import, robots and crawl-delay policy, durable frontier, host scheduler, fetch telemetry, recrawl policy, crawl trap handling, raw metadata retention, replayable corpora | Large crawl replay, freshness lag report, restart/resume proof, host politeness report, crawler failure-injection report | `PROGRAM_PLAN.md` Workstream A, `COMPETITOR_ROADMAP.md` tracks 1 and 8, `OPERATIONS_RELIABILITY_PLAN.md` | Search Crawling And Freshness | M2 | Search | Partial |
| REQ-SEARCH-EXTRACTION | High-quality document extraction and normalization | Static extraction, fielded metadata, canonical URL, language, charset, headings, anchor text, boilerplate reduction, noindex/thin/spam/malware policy, duplicate clustering | Extraction fixture suite, real-page extraction quality report, duplicate metrics, noindex/thin exclusion tests | `PROGRAM_PLAN.md` Workstream B, `COMPETITOR_ROADMAP.md` track 2 | Search Indexing And Storage | M1 | Search | Partial |
| REQ-SEARCH-RENDERED-EXTRACTION | JavaScript-heavy rendered extraction for search | Render-worker queue, timeout/resource caps, render cache, static-vs-rendered confidence policy, final URL and content hash records, parity checks, fallback to static fast lane | Render-worker parity fixtures, timeout and budget tests, rendered extraction quality report, rendered freshness report | `PROGRAM_PLAN.md` Workstreams B/G, `COMPETITOR_ROADMAP.md` Milestone 4, `SECURITY_PRIVACY_PLAN.md` Render Worker Requirements | JavaScript And Web APIs | M4 | Search | Partial |
| REQ-SEARCH-INDEX-STORAGE | Durable incremental index storage | Versioned manifests, checksums, immutable segments, background compaction, incremental writer, postings skip/block-max data, doc store, link graph store, shard manifests, rollback and replication | Index integrity verifier, interrupted-build recovery, segment merge tests, shard health report, corpus/index hash report, rollback drill | `PROGRAM_PLAN.md` Workstream C, `ARCHITECTURE.md` data stores | Search Indexing And Storage | M2 | Search | Partial |
| REQ-SEARCH-QUERY-SERVING | Low-latency search serving | Resident hot services, structured API, compact protocol, shard fanout, timeouts, partial result semantics, snippet generation, caches, slow-query tracing, load shedding | p50/p95/p99 query-plus-render benchmark, load test, degraded-shard test, API/UI integration test, saved benchmark status | `PROGRAM_PLAN.md` Workstream D, `COMPETITOR_ROADMAP.md` track 5 | Search Serving And Product UX | M2 | Search | Partial |
| REQ-SEARCH-RELEVANCE | Google-style relevance and quality | BM25 baseline, field weights, link graph, anchor text, freshness, language/location, dedupe, spam/quality, safe search where configured, query understanding, judged query workflows, ranking experiments | Judged query suites, NDCG/MRR/recall/precision gates, spam and duplicate metrics, freshness metric, latency-quality regression report | `PROGRAM_PLAN.md` Workstream E, `COMPETITOR_ROADMAP.md` track 4 | Search Relevance And Quality | M3 | Search | Partial |
| REQ-SEARCH-PRODUCT | User-facing search product | Search UI, result snippets, filters/operators, suggestions/spelling, render/cached-text policy, freshness labels, crawl/index transparency, accessibility, settings, user docs | UX smoke suite, browser/UI tests, accessibility checks, public API safety checks, install/run/crawl/search benchmark walkthrough | `PROGRAM_PLAN.md` Workstream J, `COMPETITOR_ROADMAP.md` track 7 | Search Serving And Product UX | M1 | Search | Partial |
| REQ-SEARCH-PRIVACY-ABUSE | Search privacy, abuse, and compliance | Query-log minimization, retention/deletion, rate limits, hostile-query controls, robots/takedown workflow, cached-content policy, spam/malware host policy, audit logs | Privacy retention tests, abuse fixtures, takedown runbook drill, public API rate-limit tests, crawler legality review | `SECURITY_PRIVACY_PLAN.md`, `PROGRAM_PLAN.md` Workstream H | Security And Privacy | M2 | Search | Partial |
| REQ-BROWSER-ENGINE | Independent browser engine architecture | Split page engine from shell, define networking/parser/DOM/CSS/layout/paint/compositor/JS/storage/accessibility boundaries, avoid Chromium/WebKit/Gecko as page engine, connect subsystem gates to release claims | Architecture review, crate/interface map, subsystem coverage report, curated page compatibility suite, performance and memory reports | `PROGRAM_PLAN.md` Workstream F, `ARCHITECTURE.md` browser runtime pipeline | Browser Engine | M5 | Browser | Partial |
| REQ-BROWSER-NETWORKING | Browser-grade network and navigation stack | URL loading, DNS/cache policy, HTTP/2 and HTTP/3 strategy, TLS/certificate UI, redirects, history, cookies, cache, downloads, service workers, navigation lifecycle. Current local scope includes bounded HTTP(S) redirect following for document, form, and static resource loads with final session entry URLs, redirect-set cookies, and supported 303/POST method rewrite through `http-redirect-navigation`. | Network WPT subset, TLS/certificate tests, cookie/cache/storage tests, navigation/session fixtures, download policy tests; current local redirect fixtures cover bounded redirects, relative `Location`, redirect cookies, final URLs, and 303 POST-to-GET behavior only | `PROGRAM_PLAN.md` Workstream F, `ARCHITECTURE.md` browser runtime pipeline | Browser Engine | M5 | Browser | Partial |
| REQ-BROWSER-PARSER-DOM | Standards-capable HTML parser and DOM | HTML tree construction, DOM mutation APIs, events, forms, custom elements strategy, shadow DOM strategy, selector/query APIs, serialization, parser error handling | HTML/DOM WPT subset, DOM mutation fixtures, form fixtures, event fixtures, parser fuzzing | `PROGRAM_PLAN.md` Workstream F, `COMPETITOR_ROADMAP.md` track 6 | Browser Engine | M5 | Browser | Partial |
| REQ-BROWSER-CSS-LAYOUT | Modern CSS cascade and layout | CSS parser, selector matching, cascade/inheritance, computed styles, block/inline, flexbox, grid, tables, forms, positioning, transforms, scrolling, text shaping, bidi, font fallback | CSS WPT subset, layout stress benchmarks, text shaping corpus, bidi tests, visual regression screenshots | `PROGRAM_PLAN.md` Workstream F, `PLATFORM_COMPLETENESS_PLAN.md` | Browser Engine | M5 | Browser | Partial |
| REQ-BROWSER-PAINT-COMPOSITOR | Paint, raster, and compositor pipeline | Display lists, invalidation, clipping, stacking, z-index, filters, animations, rasterization, GPU strategy, frame scheduling, hit testing, screenshots | Screenshot pixel gates, paint/raster benchmarks, compositor frame-time tests, input latency tests, GPU timeout tests; current local scaffold includes `brutal-browser render-images <page> --json` / `--display-list` for supported image-resource decode/rerender evidence, `brutal-browser raster <page>` / `brutal-browser raster-file <fixture>` viewport-window CPU raster checks with whole-document scroll offsets via `--viewport-x` / `--viewport-y` (`--scroll-x` / `--scroll-y`) and `--viewport-width` / `--viewport-height`, `brutal-browser screenshot <page> --output <png>` / `brutal-browser screenshot-file <fixture> --output <png>` for RGBA8 PNG screenshot artifacts over the same deterministic CPU raster path, `brutal-browser hit-test <page> --x <n> --y <n> --json` for supported local display-list hit-test debugging, `browser-shell-coordinate-click` coverage for CLI shell coordinate routing through that local hit-test path into supported generated `pointerdown`/`mousedown`/`pointerup`/`mouseup`/click/default-action navigation, one-shot `brutal-browser click-at` viewport-offset routing through the same supported path, `brutal-browser layout-tree <page> --json` for retained paint-backed layout-box snapshots, `brutal-browser viewport <page> --json` for clamped text-viewport max-scroll, visible-layout-box state, and cell-space dirty-region accounting for the supported shell path, `brutal-browser viewport-frame <page> --output <png> --json` for that clamped viewport plus deterministic RGBA pixels and dirty pixel rectangles, and `brutal-browser layer-tree <page> --json` for supported local layer-tree/debug snapshots | `PROGRAM_PLAN.md` Workstream F, `BROWSER_RENDERING_COMPOSITOR_PLAN.md`, `PLATFORM_COMPLETENESS_PLAN.md` | Browser Engine | M5 | Browser | Partial |
| REQ-BROWSER-JS-WEB-APIS | JavaScript and Web APIs | JS engine strategy, event loop, microtasks, timers, modules, Web IDL bindings, DOM APIs, fetch/XHR, workers, storage APIs, error reporting, resource caps | JS benchmarks, Web API WPT subsets, timeout/sandbox tests, rendered extraction parity, compatibility matrix | `PROGRAM_PLAN.md` Workstream G, `COMPETITOR_ROADMAP.md` track 6 | JavaScript And Web APIs | M6 | Browser | Partial |
| REQ-BROWSER-PLATFORM | Browser platform completeness | Images, SVG, canvas, WebGL/WebGPU strategy, media, accessibility tree, input/IME/editing, clipboard, storage/profiles, private browsing, devtools, extensions | Feature coverage gates, image/SVG/canvas/media fixtures, accessibility audits, devtools tests, extension isolation tests; current `render-images` reports are local supported-subset scaffold evidence and do not satisfy full platform image compatibility | `PLATFORM_COMPLETENESS_PLAN.md`, `PROGRAM_PLAN.md` Workstream F | Platform Completeness | M6 | Browser | Partial |
| REQ-BROWSER-SHELL-DISTRIBUTION | Browser shell, packaging, and updates | Current early scope: reusable Rust `BrowserApp` state plus scripted and interactive/stdin `brutal-browser app` command over the supported `BrowserSession` subset for tabs, navigation, viewport scrolling, input actions, JSON cookie/localStorage files, JSON app history/bookmarks, app find/find-next state, visible viewport text, deterministic browser-window PNG output, window-coordinate click routing, feature-gated native CPU-backed window presentation, narrow native location entry, focused-control text routing, resize-aware viewport updates, and presentable RGBA frames, plus `brutal-browser browse` local CLI shell with open/back/forward, current-page reload/refresh, current-page-relative open/go navigation, current-page location reporting, current in-memory cookie inspection and clearing, optional local JSON `--cookie-jar` and `--local-storage` load/save around shell runs, current localStorage inspection and clearing, current in-memory sessionStorage inspection and clearing, current-page link listing, resolved-link activation by zero-based index, exact text, or anchor selector, selector click, coordinate click routed through display-list hit testing, narrow anchor href default navigation from shell clicks through session history, bounded redirect-followed final URLs, rendered fragment-target scrolling, narrow wheel dispatch before shell viewport movement, rendered text find scrolling, remembered text-like form field values and single-select choices merged into later GET or URL-encoded POST form submissions, checkbox/radio checked-state persistence plus explicit toggle/default-action/focused-space/label coverage, selector/associated-label focus plus forward/backward focus traversal across fillable/select/checkable/submit-reset controls, typed-text append/backward-delete/clear editing for editable text-like controls, and focused-form `enter` submission/reset activation, supported submit-control and reset-control click default actions through `BrowserSession`/CLI, fixed text-viewport scroll, current-page render, and optional scripted `--cmd` runs. This is not full interactive form state, full label activation semantics, validation, keyboard events, DOM tab order, tabindex, caret positioning, selection, IME, autofill, broad POST, multipart/file upload, fetch/XHR, omnibox search, URL autocomplete, browser-accurate scrolling, scroll containers, CSS overflow, full WheelEvent semantics, full platform input integration, full omnibox, autocomplete, search-provider integration, IME, clipboard, full reload lifecycle, cache policy, service worker handling, persistent profiles, encrypted cookie storage, IndexedDB, Cache API, quota management, full site-data clearing, devtools storage panels, sessionStorage profile persistence, cookie settings UI, storage partition clearing, native product browser UI, full OS-window lifecycle, or product browser chrome. Full product scope: address bar, tabs, reload/back/forward, bookmarks, history, find-in-page, downloads, settings, profiles, crash recovery, dev/debug panels, signed packages, updater, rollback | BrowserApp unit tests for frame presentation, scroll damage, tab state, viewport-origin clamping, link activation, profile-state load, and storage clearing; `brutal-browser app` CLI smoke for scripted and stdin BrowserApp actions, JSON profile persistence, app history/bookmark persistence, app find/find-next state, browser-window frame output, window-coordinate click routing, visible viewport output, and PNG frame output; feature-gated `brutal-browser window` compile coverage plus local buffer-conversion, location-mode, viewport-size tests for native window presentation; CLI shell smoke tests for the supported static subset including anchor-click default navigation, `browser-shell-reload`, `browser-shell-relative-open`, `browser-shell-location-command`, `browser-shell-cookie-inspection`, `browser-shell-clear-cookies`, `browser-shell-cookie-jar-file`, `browser-shell-local-storage-file`, `browser-shell-local-storage-inspection`, `browser-shell-session-storage-inspection`, `browser-shell-clear-local-storage`, `browser-shell-clear-session-storage`, `http-redirect-navigation`, `browser-shell-fragment-navigation`, `browser-shell-coordinate-click`, `browser-shell-wheel-events`, `browser-shell-find-text`, `browser-shell-form-fill-state`, `browser-session-select-form-state`, `browser-shell-select-form-choice`, `browser-session-checkable-form-state`, `browser-shell-checkable-form-toggle`, `browser-session-focused-form-control`, `browser-session-focus-traversal`, `browser-shell-focused-text-input`, `browser-shell-focus-traversal`, `browser-session-focused-text-edit`, `browser-shell-focused-text-edit`, `browser-session-focused-form-submit`, `browser-shell-enter-submit`, scoped `browser-session-urlencoded-post-form-submit`, scoped `browser-session-form-submit-button-click-default`, and scoped `browser-session-form-reset-click-default`; shell workflow tests, package install/update/rollback tests, signed artifact verification, crash recovery tests | `PROGRAM_PLAN.md` Workstream J, `PLATFORM_COMPLETENESS_PLAN.md` | Platform Completeness | M6 | Browser | Partial |
| REQ-BROWSER-SECURITY | Browser security model | Process split, renderer sandbox, site isolation, origin checks, CSP, mixed content, permissions, storage partitioning, safe downloads, update signing, IPC fuzzing | Sandbox denial tests, origin/CSP/mixed-content fixtures, permission tests, parser/IPC fuzz gates, signed update drills | `SECURITY_PRIVACY_PLAN.md`, `PROGRAM_PLAN.md` Workstream H | Security And Privacy | M6 | Browser | Partial |
| REQ-BENCHMARKS-STANDARDS | Truthful benchmark and standards gates | Reproducible corpora, Chrome baseline, WPT subsets, visual baselines, browser performance tests, search relevance gates, readiness gates, hardware/version metadata | `brutal-bench gate`, `brutal-bench audit`, `brutal-bench readiness`, browser fixture verifier, Chromium parity reports, saved corpus/index hashes | `COMPETITOR_ROADMAP.md` track 8, `PROGRAM_PLAN.md` Immediate Backlog | Plan Coverage | M0 | Shared | Partial |
| REQ-OPERATIONS-RELIABILITY | Production operations and reliability | Service topology, health checks, metrics/logs/traces, dashboards, alerts, backups, restore, failure injection, deployments, rollback, capacity/cost controls, incidents | SLO dashboards, restore drills, failure-injection automation, load tests, release manifests, incident runbooks | `OPERATIONS_RELIABILITY_PLAN.md`, `PROGRAM_PLAN.md` Workstream I | Operations And Reliability | M2 | Shared | Partial |
| REQ-GOVERNANCE-CLAIMS | Release governance and claim discipline | Requirement ownership, claim rules, readiness status, release checklist, security review, privacy review, benchmark review, compatibility review, documented exceptions | Completion audit, release review record, `brutal-bench audit --require-complete`, docs linked from README and architecture | `PROGRAM_PLAN.md` Completion Audit Checklist, this file | Plan Coverage | M0 | Shared | Partial |

## Release Claim Rules

- A search claim requires implemented evidence for every
  `Search` and `Shared` claim-scope row. The machine gate is
  `brutal-bench audit --claim search --require-complete`, which composes
  traceability, evidence-registry coverage, and readiness. The underlying
  checks remain `brutal-bench traceability --require-claim-complete search`,
  `brutal-bench evidence --require-complete`, and
  `brutal-bench readiness --claim search --require-complete`.
- A browser claim requires implemented evidence for every
  `Browser` and `Shared` claim-scope row. The machine gate is
  `brutal-bench audit --claim browser --require-complete`, with the same
  traceability, evidence, and readiness sub-gates applied to browser scope.
- A combined claim requires both sets and a passing
  `brutal-bench audit --claim combined --require-complete`.
- Milestone exit claims require
  `brutal-bench traceability --require-milestone-complete <m0..m6>`, which
  checks every row assigned to that milestone or any earlier milestone.
- Partial fixture success, local demos, or microbenchmarks can be announced only
  as partial progress and must name the missing gates.
- The current browser form-validation evidence is limited to
  `browser-session-required-form-validation`: value-missing checks for supported
  required text-like, select, checkbox, and radio controls in BrowserSession/CLI
  submit paths, with form `novalidate` and submitter `formnovalidate`; and
  `browser-session-type-value-validation`: non-empty email/URL value checks in
  the same submit paths. This is not full constraint validation, full HTML type
  validation, or validation UI.
- Local WPT-subset, `browser-compat`, `compare-chromium`, `render-images`,
  `hit-test`, `layout-tree`, `layer-tree`, `accessibility-tree`, browser-perf layer metrics,
  and `browser-perf --chromium-baseline` are scoped evidence for their
  checked-in fixtures or supported local subsets. They must not be described as
  upstream WPT coverage, broad web compatibility, full image support, full input
  routing, full CSS layout geometry, platform accessibility coverage, GPU
  compositor coverage, Chromium compositor parity, or Chromium parity without
  the remaining standards, visual, platform, security/privacy, operations, and
  release-review gates.

## Maintenance Rules

1. Add or update a requirement row before adding a new public-facing claim.
2. Keep `Current state` conservative; uncertain evidence is not implemented.
3. Every plan owner must have a command, fixture, report, or document gate before
   a readiness area can move from missing to partial.
4. Every readiness area must point to direct artifacts before moving from
   partial to implemented.
5. Removing a requirement requires updating the objective boundary and release
   claim rules in the same change.
