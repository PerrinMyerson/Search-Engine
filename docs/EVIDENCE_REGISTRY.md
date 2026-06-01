# Evidence Registry

This registry is the map from public claim requirements to proof artifacts. It
is not proof by itself. It defines the command, fixture, report, or external
suite that must exist before a search, browser, milestone, or combined claim
can move from partial to implemented.

## Evidence Rules

- Every requirement in `REQUIREMENTS_TRACEABILITY.md` must have at least one
  evidence row before its current state can move to implemented.
- Every evidence row must be reproducible from a command, fixture corpus,
  external standards suite, signed release artifact, review record, or
  operations drill.
- Evidence that depends on a corpus, browser version, hardware, operating
  system, Rust version, or benchmark threshold must record those inputs in the
  saved report.
- Planned evidence can keep a requirement partial, but it cannot support a
  public completion claim.
- Unweighted `brutal-browser coverage` ratios are feature-fixture inventory
  progress only; they are not browser-completion or Chromium-parity percentages.
- The local WPT-subset runner is a compatibility scaffold over checked-in
  fixture rows. It is not full upstream WPT, WebDriver/testdriver coverage,
  reftest coverage, or Chromium parity evidence by itself.
- A `browser-compat` pass can satisfy only the local scaffold gate until an
  imported upstream WPT subset records expectations, skipped tests, flakes,
  shard metadata, WebDriver/testdriver and reftest coverage where relevant, and
  pinned Chromium comparison for the same test set.
- Deterministic grayscale PGM raster artifacts count as current fixture raster
  evidence only; full browser screenshot claims require separate color,
  viewport, compositor, platform-presentation, and real-page evidence.
- `brutal-browser raster` / `brutal-browser raster-file` viewport-window
  culling uses whole-document terminal/text scroll offsets (`--viewport-x` /
  `--viewport-y`, also `--scroll-x` / `--scroll-y`) plus `--viewport-width` /
  `--viewport-height`; it is not browser-accurate scrolling or compositor
  tiling evidence.
- `browser-shell-coordinate-click` is CLI shell scaffold evidence only: it
  routes a terminal-cell coordinate through local display-list hit testing into
  the supported `pointerdown`/`mousedown`/`pointerup`/`mouseup`/click/default-action
  path, and must not be described as full browser pointer routing, full
  PointerEvent semantics, or full MouseEvent semantics.
- `browser-cli-click-at-viewport-offset` is one-shot CLI scaffold evidence only:
  it adds explicit viewport offsets to `click-at`/`tap` coordinates before the
  same supported display-list hit-test/default-action path, and must not be
  described as browser-accurate scrolling or platform pointer routing.
- `browser-shell-wheel-events` is CLI shell scaffold evidence only: it dispatches
  a narrow document-level `wheel` event before local shell scroll/left/right
  viewport movement, exposes `event.deltaX`/`event.deltaY`, and lets
  `preventDefault()` cancel that local movement. It must not be described as
  browser scroll containers, CSS overflow, async/compositor scrolling, full
  WheelEvent semantics, or platform input integration.
- `browser-shell-relative-open` is CLI shell/session scaffold evidence only: it
  resolves `open` / `go` targets against the current page source before
  navigation. It is not omnibox search, URL autocomplete, tab UI, security UI,
  or full browser chrome evidence.
- `browser-shell-cookie-inspection` is CLI shell/session scaffold evidence only:
  it prints the current in-memory `BrowserSession` cookie jar without changing
  page state. It is not persistent profile storage, cookie settings UI,
  permissions, partitioning, or browser chrome evidence.
- `browser-shell-clear-cookies` is CLI shell/session scaffold evidence only: it
  clears the current in-memory `BrowserSession` cookie jar without navigating or
  changing page state. It is not persistent profile clearing, storage partition
  clearing, permissions UI, settings UI, or browser chrome evidence.
- `browser-shell-cookie-jar-file` is CLI shell/session scaffold evidence only:
  it loads and saves the current in-memory cookie jar through a local JSON
  `--cookie-jar` file around a `brutal-browser browse` run. It is not encrypted
  profile storage, cookie expiration persistence, partitioning, cookie settings
  UI, permissions UI, or browser chrome evidence.
- `browser-shell-local-storage-file` is CLI shell/session scaffold evidence
  only: it loads and saves origin-scoped localStorage through a local JSON
  `--local-storage` file around a `brutal-browser browse` run. It is not
  IndexedDB, Cache API, quota management, private browsing, partitioning,
  settings UI, permissions UI, or browser chrome evidence.
- `browser-shell-local-storage-inspection` is CLI shell/session scaffold
  evidence only: it prints current origin-scoped localStorage session entries
  without changing page state. It is not devtools storage panels, IndexedDB,
  Cache API, quota management, partitioning, settings UI, permissions UI, or
  browser chrome evidence.
- `browser-shell-clear-local-storage` is CLI shell/session scaffold evidence
  only: it clears current origin-scoped localStorage session state without
  navigating. It is not full site-data clearing, IndexedDB, Cache API, quota
  management, private browsing, partitioning, settings UI, permissions UI, or
  browser chrome evidence.
- `browser-shell-session-storage-inspection` is CLI shell/session scaffold
  evidence only: it prints current in-memory origin-scoped sessionStorage
  entries without changing page state. It is not persistent profile storage,
  devtools storage panels, IndexedDB, Cache API, quota management, partitioning,
  settings UI, permissions UI, or browser chrome evidence.
- `browser-shell-clear-session-storage` is CLI shell/session scaffold evidence
  only: it clears current in-memory origin-scoped sessionStorage state without
  navigating. It is not full site-data clearing, persistent profiles, IndexedDB,
  Cache API, quota management, private browsing, partitioning, settings UI,
  permissions UI, or browser chrome evidence.
- `http-redirect-navigation` is loader/session scaffold evidence only: it
  follows a bounded HTTP(S) redirect chain for document, form, and static
  resource loads, records final session entry URLs, carries redirect-set cookies
  into the next hop, and applies the supported 303/POST redirect method rewrite.
  It is not full navigation lifecycle, mixed-content/referrer/CORS policy, HSTS,
  redirect UI, browser-grade error pages, or broad compatibility evidence.
- `browser-shell-form-fill-state` is CLI shell/session scaffold evidence only:
  it remembers filled text-like field values on the current `BrowserSession`
  entry and merges them into later GET form submissions. It is not full
  interactive form state, validation, focus/input events, autofill, or
  browser UI.
- `browser-session-select-form-state` and
  `browser-shell-select-form-choice` are CLI shell/session scaffold evidence
  only: they expose single-select option metadata, validate enabled option
  values, and route focused `choose <value>` plus explicit
  `select <form> <control> <value>` commands into later GET or URL-encoded POST
  form submission. They are not native select UI, multi-select, optgroup
  inheritance, input/change event dispatch, validation, keyboard events, or
  browser UI evidence.
- `browser-session-checkable-form-state` and
  `browser-shell-checkable-form-toggle` are CLI shell/session scaffold evidence
  only: they remember checkbox/radio checked state, expose explicit
  `toggle <form> <control>` shell commands, and run narrow selector-click
  and label defaults for supported checkbox/radio controls. They are not full
  input/change event dispatch, full label activation semantics, indeterminate
  state, custom controls, validation, or browser UI evidence.
- `browser-session-required-form-validation` is CLI shell/session scaffold
  evidence only: supported BrowserSession/CLI submit paths block enabled
  required text-like, select, checkbox, and radio controls when they are empty or
  unchecked, and honor form `novalidate` and submitter `formnovalidate`. It is not full constraint validation,
  validation UI, custom validity, invalid events, or browser UI evidence.
- `browser-session-type-value-validation` is CLI shell/session scaffold
  evidence only: supported BrowserSession/CLI submit paths block non-empty email
  and URL controls with invalid values, honoring the same validation bypasses. It
  is not full HTML type validation, IDNA/email grammar, validation UI, invalid
  events, or browser UI evidence.
- `browser-session-focused-form-control` and
  `browser-shell-focused-text-input` are CLI shell/session scaffold evidence
  only: they track a selector-focused or associated-label-focused supported
  control and append typed text into existing form state only for editable text-like controls.
  `browser-session-focus-traversal` and `browser-shell-focus-traversal` cycle
  through named enabled fillable, select, checkable, and submit/reset action
  controls in rendered form order.
  `browser-session-focused-text-edit` and
  `browser-shell-focused-text-edit` add Unicode-safe backward deletion and clear
  operations for that focused control. `browser-session-keyboard-events` and
  `browser-session-beforeinput-events` add narrow `keydown`/`keyup` and
  `beforeinput` dispatch around supported text insertion and Backspace deletion,
  including `preventDefault()` gates before mutation. `browser-session-focused-form-submit` and
  `browser-shell-enter-submit` submit focused fillable/select controls, activate
  focused submit controls with submitter state, or reset focused reset controls.
  They are not full keyboard or `InputEvent` semantics, caret
  positioning, DOM tab order, tabindex, selection, IME, constraint validation,
  submit event ordering, autofill, platform text input, undo, or browser UI
  evidence.
- `browser-shell-find-text` is CLI shell scaffold evidence only: it searches
  rendered text viewport lines and scrolls to the matching document line. It is
  not full browser find UI, highlighting, selection, match-count reporting,
  locale-aware search, or Chromium parity.
- `browser-session-reload` and `browser-shell-reload` are CLI shell/session
  scaffold evidence only: they reload the current target and replace that
  history entry. They are not full reload lifecycle, cache policy, POST replay
  UI, service worker handling, BFCache policy, or Chromium parity.
- `browser-session-urlencoded-post-form-submit` is CLI shell/session scaffold
  evidence only: it submits `application/x-www-form-urlencoded` POST forms
  through `BrowserSession` and renders returned HTML. It is not multipart/file
  upload, fetch/XHR, validation, full navigation lifecycle, browser UI, broad
  POST support, or Chromium parity.
- `browser-session-form-submit-button-click-default` is CLI shell/session
  scaffold evidence only: it covers supported submit/input/button click default
  actions that route through `BrowserSession` and the CLI shell for GET or
  URL-encoded POST forms. It is not full form event dispatch, validation,
  focus/input behavior, browser UI, or Chromium parity.
- `browser-session-form-reset-click-default` is CLI shell/session scaffold
  evidence only: it covers supported reset-control click default actions that
  clear remembered text-like state for the owning form. It is not full form event
  dispatch, validation, focus/input behavior, browser UI, or Chromium parity.
- A release bundle must include the traceability report, readiness report,
  browser-engine strategy, benchmark reports, compatibility reports,
  security/privacy review, operations review, and any documented exceptions.

## Registry

| Evidence ID | Requirement IDs | Required for | Artifact or command | Produces | Completion standard |
| --- | --- | --- | --- | --- | --- |
| EV-TRACEABILITY | `REQ-BENCHMARKS-STANDARDS`, `REQ-GOVERNANCE-CLAIMS` | Plan coverage, governance, every claim scope, every milestone | `brutal-bench traceability --json` plus `--require-no-missing`, `--require-claim-complete <search/browser/combined>`, and `--require-milestone-complete <m0..m6>` | Requirement rows, readiness areas, milestones, claim scopes, partial/missing IDs | Passes for the requested claim or milestone with no unknown IDs, no unknown areas, no unknown milestones, no unknown claim scopes, no duplicate rows, and no partial or missing rows for the requested gate |
| EV-READINESS | `REQ-BENCHMARKS-STANDARDS`, `REQ-OPERATIONS-RELIABILITY`, `REQ-GOVERNANCE-CLAIMS` | Plan coverage, search claim, browser claim, combined claim | `brutal-bench readiness --claim <search/browser/combined> --json` plus `--require-complete` | Claim-filtered readiness areas, direct evidence markers, missing work | Passes for the requested claim with every required area implemented and no partial or missing areas |
| EV-SEARCH-SMOKE | `REQ-SEARCH-PRODUCT`, `REQ-SEARCH-QUERY-SERVING`, `REQ-BENCHMARKS-STANDARDS` | Search product and local runability | `brutal-bench smoke --json --save-report` | Local corpus build/search/render timings and saved bench status | Deterministic local corpus builds, searches, renders, and saves a report without failures |
| EV-SEARCH-PERF | `REQ-SEARCH-QUERY-SERVING`, `REQ-BENCHMARKS-STANDARDS` | Low-latency search serving and benchmark claims | `brutal-bench search --mode daemon --chromium-baseline --require-speedup 10 --json --save-report` | p50/p95/p99 latency, throughput, corpus hash, index hash, Rust version, Chromium version, OS, hardware | Warm p95 query-plus-render latency meets or exceeds the required speedup over the Chromium baseline on the same corpus and query set |
| EV-SEARCH-RELEVANCE | `REQ-SEARCH-RELEVANCE`, `REQ-BENCHMARKS-STANDARDS` | Google-style relevance and quality | `brutal-bench eval --require-mrr <n> --require-ndcg <n> --require-recall <n> --require-precision <n> --max-unresolved 0 --json` | MRR, NDCG, recall, precision, unresolved judgments, per-query diagnostics | Representative judged query suites meet thresholds with no unresolved required judgments |
| EV-CRAWL-FRESHNESS | `REQ-SEARCH-CORPUS`, `REQ-SEARCH-EXTRACTION`, `REQ-OPERATIONS-RELIABILITY` | Public-web corpus and recrawl freshness | `brutal-search crawl`, `brutal-search recrawl-plan`, `brutal-search recrawl-scheduler`, `/api/crawl-status` | Frontier state, crawl snapshots, host stats, changed/unchanged/missing counts, freshness lag | Large crawl and recrawl runs prove restart/resume, politeness, freshness SLOs, and bounded resource use |
| EV-INDEX-INTEGRITY | `REQ-SEARCH-INDEX-STORAGE`, `REQ-OPERATIONS-RELIABILITY` | Indexing and storage | Planned `brutal-bench index-integrity --index <path> --json`, plus corpus/index hashes from existing reports | Segment checksums, manifest compatibility, corruption checks, rollback/restore results | Versioned index validates after clean build, interrupted build, compaction, corruption injection, rollback, and restore |
| EV-SEARCH-UX | `REQ-SEARCH-PRODUCT`, `REQ-SEARCH-PRIVACY-ABUSE` | Search product UX and API safety | Planned browser/UI smoke suite plus existing `/api/search`, `/api/render`, `/api/suggest`, `/api/spell`, `/api/stats` endpoints | UI workflow results, accessibility checks, API response safety checks | Search UI, filters, suggestions, spelling, render views, status views, and accessibility gates pass on representative workflows |
| EV-BROWSER-FIXTURES | `REQ-BROWSER-ENGINE`, `REQ-BROWSER-PARSER-DOM`, `REQ-BROWSER-CSS-LAYOUT`, `REQ-BROWSER-JS-WEB-APIS`, `REQ-BENCHMARKS-STANDARDS` | Browser engine smoke coverage | `brutal-browser verify bench/browser-fixtures/manifest.json --json`; `brutal-browser verify bench/document-pages/manifest.json --json`; targeted local image-render smoke via `brutal-browser render-images <page> --display-list` or `--json` | Fixture pass/fail counts for static render, Stage 1 document-page visible-text and RGBA screenshot-hash gates, `text-color.html` styled text color (`css-color-property` / `text-color-paint`) and rectangle/image display-list raster hashes, tiny SVG decoded pixels, data URI PNG decoded pixels, render-to-raster cache coverage, image-resource fetch/decode rerender report for the supported SVG/PNG/data-URL subset, CSS display/color/background/border/padding/margin/sizing/max-width/auto-margin paint and layout, scripts, DOM creation/tree mutation/traversal/insertion, `DocumentFragment` insertion, selector element methods, `innerHTML` mutation/readback, form-control DOM properties, location readback properties, attributes, classList mutation/readback, DOM query collections, style property mutation/readback, localStorage, sessionStorage, timer queue mutation, document/window lifecycle events, external scripts, and events | Fixture suites pass with no failures and fixtures cover every implemented browser capability being claimed; `render-images` evidence is local scaffold evidence for the supported image subset only, not full image compatibility or Chromium parity |
| EV-BROWSER-COVERAGE | `REQ-BROWSER-ENGINE`, `REQ-BROWSER-PLATFORM`, `REQ-BROWSER-SHELL-DISTRIBUTION`, `REQ-BENCHMARKS-STANDARDS` | Browser platform feature tracking | `brutal-browser coverage --json --require <feature> --min-implemented-ratio <n> --max-missing <n>`; local shell smoke via `brutal-browser browse <target> --cmd <command>`; BrowserApp engine tests; scripted/stdin app smoke via `brutal-browser app <target> --cmd <command> --cookie-jar <json> --local-storage <json> --output <png> --json`; static accessibility smoke via `brutal-browser accessibility-tree <target> --json`; retained layout smoke via `brutal-browser layout-tree <target> --json`; viewport-state smoke via `brutal-browser viewport <target> --json`; viewport frame smoke via `brutal-browser viewport-frame <target> --output <png> --json` | Implemented/partial/missing feature-fixture inventory plus reusable BrowserApp state coverage for tabs, navigation, viewport scrolling, input actions, full/partial repaint decisions, presentable RGBA frame reports, scripted/stdin app-command JSON profile persistence, app history/bookmark persistence, app find/find-next state, browser-window frame output, window-coordinate click routing, feature-gated native window shell source/compile coverage, narrow native location entry, focused-control text routing, resize-aware viewport updates, visible viewport output, and PNG output; CLI shell smoke output for open/back/forward, local tab list/new/switch/close commands over multiple `BrowserSession` instances, current-page reload/refresh, current-page cookie inspection/clearing, JSON cookie-jar load/save, JSON localStorage load/save/clearing, sessionStorage inspection/clearing, current-page link listing, resolved-link activation by zero-based index, exact text, or anchor selector, selector click including narrow anchor href default navigation through `BrowserSession` history, coordinate-click routing through local display-list hit testing to the supported `pointerdown`/`mousedown`/`pointerup`/`mouseup`/click/default-action path, one-shot `click-at` viewport-offset routing, narrow wheel-event dispatch before shell viewport movement with default-prevention cancellation, rendered text find scrolling, remembered text-like form field values merged into later GET form submissions, single-select option metadata and selected-value state, checkbox/radio checked-state persistence plus explicit toggle, focused-space toggle, selector-click default, and label-default smoke, required-control value-missing checks and email/URL value checks on supported submit paths, selector/associated-label focus, focus traversal, and typed-text append/backward-delete/clear editing in the active editable text-like control, focused-control form submission/reset activation via `enter`, URL-encoded POST form submission through `BrowserSession`/CLI, supported submit-control click defaults through `BrowserSession`/CLI, supported reset-control click defaults through `BrowserSession`/CLI, fixed text-viewport scroll, current-page render over the supported `BrowserSession` subset, deterministic retained paint-backed layout box snapshots, clamped viewport state and dirty-region accounting, RGBA viewport frame-surface smoke, and deterministic static accessibility role/name/state snapshots over the supported DOM/CSS/tiny-script subset | Required features are implemented and missing-feature budgets meet the gate for the target milestone; unweighted ratios remain fixture progress indicators only. The `BrowserApp`, `app` command, `browse` shell, `layout-tree` snapshot, `viewport` report, `viewport-frame` report, and `accessibility-tree` snapshot are local usability/platform evidence only, and their tab commands, link activation, reload/refresh, cookie inspection/clearing/JSON jar persistence, JSON localStorage persistence/clearing, sessionStorage inspection/clearing, anchor-click default navigation, coordinate-click routing, one-shot click-at viewport-offset routing, narrow generated pointer/mouse/wheel events, find-text scrolling, form fill state, select form state/choice, checkable form state/toggle/focused-space/label defaults, required-control value-missing checks, email/URL value checks, focused text input/traversal/editing/submission/reset activation, URL-encoded POST marker, submit-control click default marker, reset-control click default marker, retained paint-backed layout boxes, viewport dirty regions, dirty pixel rectangles, JSON profile-file output, app history/bookmark output, app find-state output, browser-window frame output, window-coordinate click routing, visible viewport output, frame PNG output, and role/name/state snapshot are resolved-href/session or supported event/default-action/form-submission/layout/accessibility paths over tiny subsets, not product browser chrome, full OS-window lifecycle, full omnibox, autocomplete, search-provider integration, IME, clipboard, shared browser profile synchronization, tab process isolation, session restore, full JS/CSS/browser interaction, full CSS layout, anonymous box generation, inline fragmentation, browser-accurate geometry, full event/default-action semantics, browser-accurate scrolling, scroll containers, CSS overflow, compositor scrolling, compositor damage tracking, full browser pointer routing, full PointerEvent semantics, full MouseEvent semantics, full WheelEvent semantics, full interactive form state, full checkbox/radio/select/label/custom-control behavior, full constraint validation, validation UI, full HTML type validation, keyboard events, DOM tab order, tabindex, caret positioning, selection, IME, autofill, encrypted profiles, sessionStorage profile persistence, cookie partitioning, IndexedDB, Cache API, quota management, full site-data clearing, multipart/file uploads, fetch/XHR, full reload lifecycle, cache policy, service worker handling, BFCache policy, full navigation lifecycle, devtools, platform accessibility bridge, broad browser POST support, or Chromium parity |
| EV-BROWSER-CHROMIUM-PARITY | `REQ-BROWSER-ENGINE`, `REQ-BROWSER-PARSER-DOM`, `REQ-BROWSER-CSS-LAYOUT`, `REQ-BROWSER-JS-WEB-APIS`, `REQ-BENCHMARKS-STANDARDS` | Browser compatibility against Chromium behavior | `brutal-browser compare-chromium bench/browser-fixtures/manifest.json --json` | Normalized output parity between Brutal Browser and Chromium for curated fixtures | Required fixtures match Chromium-normalized expectations with no unreviewed failures; this supports curated fixture parity only, and cannot establish Chromium-class compatibility without the WPT-subset, visual, platform, security/privacy, operations, and release-review gates |
| EV-BROWSER-PERF | `REQ-BROWSER-ENGINE`, `REQ-BROWSER-NETWORKING`, `REQ-BROWSER-PARSER-DOM`, `REQ-BROWSER-CSS-LAYOUT`, `REQ-BROWSER-PAINT-COMPOSITOR`, `REQ-BROWSER-JS-WEB-APIS`, `REQ-BROWSER-PLATFORM`, `REQ-BENCHMARKS-STANDARDS` | Browser performance claims | `brutal-bench browser-perf --manifest bench/browser-fixtures/manifest.json --chromium-baseline --json --save-report`, optionally gated with `--min-chromium-p95-speedup`, `--max-chromium-text-mismatches`, `--max-layer-metrics-p95-us`, `--min-total-layers`, `--min-total-image-layers`, `--max-layer-count`, and `--max-image-layer-count`, with planned frame/memory/power/startup extensions and Speedometer/JetStream/MotionMark-style suites or equivalents | Fixture render p50/p95/p99, parser/script/style/collection/layout phase timings, throughput, suite hash, Rust/OS/hardware/Chromium metadata, headless Chromium iframe fixture p50/p95/p99, local fixture-script inlining metadata through text-match evidence, Rust-vs-Chromium p95 speedup, Chromium text match/mismatch counts and hashes, per-fixture DOM/layout/paint/layer counts and local scalar layer-tree shape metrics, multi-tab memory, power, startup, and real-page canary metrics | Current fixture suite reports reproducible render latency, phase cost breakdowns, local layer-shape metrics, and an optional headless Chromium fixture baseline for supported fixtures only; the baseline checks expected text so blank/missed fixture work is visible. Milestone claims additionally require pinned browser version policy, broader Chromium comparison, memory, frame, power, correctness, real-page canaries, and no unreviewed regressions |
| EV-WPT-SUBSETS | `REQ-BROWSER-NETWORKING`, `REQ-BROWSER-PARSER-DOM`, `REQ-BROWSER-CSS-LAYOUT`, `REQ-BROWSER-JS-WEB-APIS`, `REQ-BROWSER-PLATFORM`, `REQ-BROWSER-SECURITY`, `REQ-BENCHMARKS-STANDARDS` | Standards compatibility | Local WPT-subset/compatibility scaffold at `bench/wpt-subsets/manifest.json` and `bench/wpt-subsets/expectations.jsonl`; runnable with `brutal-browser wpt bench/wpt-subsets/manifest.json --expectations bench/wpt-subsets/expectations.jsonl --json` and gated with `brutal-bench browser-compat --manifest bench/wpt-subsets/manifest.json --expectations bench/wpt-subsets/expectations.jsonl --min-pass-rate 1 --max-unexpected-failures 0 --max-flakes 0 --json`; plus planned upstream WPT subset manifests and reports for HTML, DOM, CSS, JS/Web APIs, networking, storage, accessibility, and security subsets, with testdriver/WebDriver, reftests, expectations, flake policy, CI sharding, pass thresholds, and pinned Chromium comparison | Current scaffold records and checks local HTML/DOM/CSS/images/rendering fixture expectations only; planned standards evidence adds upstream WPT pass/fail reports, skipped-test manifests, expectation files, flake quarantine, shard metadata, WebDriver/testdriver/reftest metadata, and browser version comparison | Scaffold pass means only that the checked-in local subset matches its expectations with no unexpected failures or flakes under the requested gate; target upstream WPT subsets must pass at the required threshold with documented expectations and no unreviewed regressions before standards compatibility, Chromium-parity, or wider web-platform claims |
| EV-RENDERING-VISUAL | `REQ-BROWSER-CSS-LAYOUT`, `REQ-BROWSER-PAINT-COMPOSITOR`, `REQ-BROWSER-PLATFORM`, `REQ-BENCHMARKS-STANDARDS` | Paint, raster, compositor, and visual correctness | `brutal-browser visual-verify bench/browser-fixtures/manifest.json --json --artifact-dir <dir> --baseline-dir <dir> --max-diff-pixels <n>`, `brutal-browser raster <page> --viewport-width <w> --viewport-height <h> --json --output <pgm>`, `brutal-browser raster-file <fixture> --scroll-y <n> --viewport-width <w> --viewport-height <h> --json --output <pgm>`, `brutal-browser screenshot <page> --output <png> --json`, `brutal-browser screenshot-file <fixture> --output <png> --json`, `brutal-browser viewport-frame <page> --viewport-width <w> --viewport-height <h> --json --output <png>`, local image rerender checks with `brutal-browser render-images <page> --json` or `--display-list`, local hit-test checks with `brutal-browser hit-test <page> --x <n> --y <n> --json`, local viewport-state checks with `brutal-browser viewport <page> --json`, local layer snapshot checks with `brutal-browser layer-tree <page> --json`, and retained layout snapshot checks with `brutal-browser layout-tree <page> --json` for current CPU text/styled-text/rectangle/background/border/padding/margin/sizing/max-width/auto-margin/image-placeholder/SVG-subset/PNG-subset raster and display-list grouping, plus planned frame-time benchmarks | Pixel hashes, PGM raster artifacts, RGBA8 PNG screenshot artifacts, viewport-frame PNG artifacts, dirty pixel rectangles, pixel-diff counts/ratios, diff artifacts, display lists, viewport-window visible/culled command counts, viewport max-scroll/dirty-region reports, image fetch/decode/rerender reports, hit-test target reports for supported local display-list items, coordinate-click shell-routing evidence that consumes those local hit-test targets, layer-tree/debug snapshots, retained layout-box snapshots, compositor timing, input latency | Current text, styled text color (`css-color-property` / `text-color-paint`, covered by `text-color.html`), horizontal-rule rectangle, block background, block border, block padding layout, block margin layout, block size constraints, max-width auto-margin document-column layout, image placeholder, tiny SVG decoded-image raster, data URI PNG decoded-image raster, fixed viewport-window CPU raster culling, clamped viewport state and cell-space dirty-region accounting, RGBA viewport frame-surface reporting, RGBA8 PNG screenshot export over the deterministic CPU raster path, image-resource rerender path, local display-list hit-test path, coordinate-click shell routing into the supported generated pointer/mouse/click/default-action path, retained paint-backed layout-box snapshot path, and local layer-tree/debug snapshot path can produce deterministic grayscale fixture raster artifacts, RGBA screenshot artifacts, viewport-frame artifacts, hashes, baseline reports, exact pixel-diff gates, cached decoded-pixel reuse, visible/culled command counts, viewport dirty-region reports, dirty pixel-region reports, point-target reports, layout box parent/child and command ownership reports, and layer grouping/bounds reports; this is local scaffold evidence for supported fixtures only. Viewport-window, viewport-frame, hit-test, coordinate-click, screenshot export, layout-tree, viewport, and layer-tree scaffold evidence is limited to the implemented local layout/display-list subset and must not be described as full compositor/input routing, full browser pointer routing, full PointerEvent semantics, full MouseEvent semantics, browser scrolling, compositor damage tracking, GPU compositing, frame scheduling, OS presentation, or Chromium compositor parity, while milestone broader PNG/JPEG decode, full raster/compositor benchmarks, hit testing, and input latency gates must still pass for browser claims |
| EV-JS-WEB-API | `REQ-SEARCH-RENDERED-EXTRACTION`, `REQ-BROWSER-JS-WEB-APIS`, `REQ-BENCHMARKS-STANDARDS` | JavaScript and Web APIs | Planned JS benchmark suite, WebIDL generation reports, Web API fixtures, timeout/sandbox tests, rendered extraction parity reports | JS compatibility metrics, API fixture results, WebAssembly/WebCrypto/WebSockets/WebTransport results, SharedArrayBuffer isolation results, GC/JIT/bytecode-cache metrics, rendered-vs-static extraction reports | JS runtime, event loop, modules, fetch/XHR, workers, storage APIs, required Web APIs, and rendered extraction gates pass at the target milestone; WebRTC/media capture is implemented with tests or explicitly excluded from the claim |
| EV-SECURITY-PRIVACY | `REQ-SEARCH-PRIVACY-ABUSE`, `REQ-BROWSER-SECURITY`, `REQ-GOVERNANCE-CLAIMS` | Security, privacy, abuse, and compliance | Threat model review, planned sandbox denial tests, site-instance/frame-tree/OOPIF tests, origin/CSP/COOP/COEP/CORP/CORS/mixed-content fixtures, JIT/W^X tests, fuzzing reports, query-log retention tests, takedown drills | Security review record, fuzz findings, sandbox results for renderer/GPU/network/storage processes, crash-containment reports, privacy/abuse/takedown reports | Security/privacy review signs off and all required sandbox, origin, site isolation, privacy, abuse, fuzzing, and compliance gates pass |
| EV-PLATFORM-COMPLETE | `REQ-BROWSER-PLATFORM`, `REQ-BROWSER-SHELL-DISTRIBUTION`, `REQ-BENCHMARKS-STANDARDS` | Browser platform completeness | Planned media/canvas/GPU/accessibility/input/storage/profile/devtools/extensions/package/update and browser-shell workflow reports | Feature reports, accessibility audits, storage quota/eviction and private-browsing reports, password/autofill policy results, media/canvas benchmarks, signed package/update verification | Platform subsystem reports pass for the target browser claim with no unreviewed critical gaps |
| EV-OPERATIONS | `REQ-OPERATIONS-RELIABILITY`, `REQ-SEARCH-CORPUS`, `REQ-SEARCH-QUERY-SERVING`, `REQ-GOVERNANCE-CLAIMS` | Production operations and reliability | SLO dashboards, restore drills, failure-injection automation, load tests, release manifests, incident runbooks | SLO/error-budget report, backup/restore report, failure-drill report, load/capacity report, incident review | Operations review proves deploy/rollback, backup/restore, monitoring, alerts, capacity, cost controls, and incident handling |
| EV-RELEASE-REVIEW | `REQ-GOVERNANCE-CLAIMS`, `REQ-BENCHMARKS-STANDARDS`, `REQ-OPERATIONS-RELIABILITY` | Public release and claim governance | Release review record plus traceability, readiness, benchmark, security, privacy, compatibility, operations, and exception reports | Signed release bundle and documented exceptions | Release review signs off only when the exact claim gate passes and every exception is documented with owner, risk, and expiration |

## Release Bundle Layout

Release evidence should be stored under a timestamped directory such as
`reports/releases/<date>-<claim>/`:

- `traceability.json`
- `readiness.json`
- `browser-engine-strategy.md`
- `search-bench.json`
- `relevance-eval.json`
- `browser-coverage.json`
- `browser-parity.json`
- `wpt/`
- `visual-regression/`
- `security-privacy-review.md`
- `operations-review.md`
- `release-review.md`

The release bundle is valid only when the matching traceability and readiness
claim gates pass for the exact claim being made.
