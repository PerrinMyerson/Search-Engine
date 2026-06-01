# Rowser

Rowser is an early independent Rust browser engine project. The target
is a Rust-native browser engine and shell designed for performance, safety,
inspectability, and automation. It must not use Chromium, WebKit, or Gecko as
the page engine. Chromium is allowed only as a benchmark/comparison oracle for
named corpora and fixtures.

The current code is still an early scaffold. `rowser-browser` can load local or
HTTP(S) HTML, build a DOM-like tree, apply a small CSS subset, render terminal
text, emit deterministic display-list/raster artifacts, run a tiny JavaScript
DOM mutation subset, and drive a narrow local CLI shell with links, forms,
history, storage, cookies, focus, and basic click/default-action behavior. That
is useful evidence, but it is not a modern browser, not Chromium-compatible, and
not a full desktop product.

Blackium Starium✴ is now positioned as the fast text/index/extraction mode of the
browser engine: fetch or ingest static HTML, extract plaintext, build a custom
local inverted index, and query/render search results quickly. The search path
intentionally avoids full page rendering unless a rendered extraction lane is
explicitly selected.

For the browser-first strategy, see
[`docs/BROWSER_ENGINE_STRATEGY.md`](docs/BROWSER_ENGINE_STRATEGY.md). For the
broader search/browser execution plans, see
[`docs/COMPETITOR_ROADMAP.md`](docs/COMPETITOR_ROADMAP.md),
[`docs/PROGRAM_PLAN.md`](docs/PROGRAM_PLAN.md), and
[`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md). The requirement-to-gate audit
matrix lives in
[`docs/REQUIREMENTS_TRACEABILITY.md`](docs/REQUIREMENTS_TRACEABILITY.md).
The proof-artifact map lives in
[`docs/EVIDENCE_REGISTRY.md`](docs/EVIDENCE_REGISTRY.md).
Security and privacy gates are
tracked in [`docs/SECURITY_PRIVACY_PLAN.md`](docs/SECURITY_PRIVACY_PLAN.md);
operations and reliability gates are tracked in
[`docs/OPERATIONS_RELIABILITY_PLAN.md`](docs/OPERATIONS_RELIABILITY_PLAN.md);
browser platform completeness gates are tracked in
[`docs/PLATFORM_COMPLETENESS_PLAN.md`](docs/PLATFORM_COMPLETENESS_PLAN.md);
paint/raster/compositor gates are tracked in
[`docs/BROWSER_RENDERING_COMPOSITOR_PLAN.md`](docs/BROWSER_RENDERING_COMPOSITOR_PLAN.md);
performance-first browser architecture and benchmark sequencing are tracked in
[`docs/PERFORMANT_RUST_BROWSER_PLAN.md`](docs/PERFORMANT_RUST_BROWSER_PLAN.md).
The current code is an early browser engine scaffold plus a fast static-text
search mode, not the complete browser product.

Current implementation focus after the browser-first reframing: make Stage 1
Document Browser M1 credible before chasing broad Chromium comparisons. That
means tightening the loading -> DOM -> style -> layout -> paint/display-list ->
raster -> app/shell pipeline, adding document-page fixtures with visible-text
and screenshot gates, and treating search as the fast extraction/indexing lane.
Chromium remains useful as a late comparison oracle for named corpora, not the
day-to-day benchmark target while the browser is still becoming functional.

## Play With The Rust Browser Now

`rowser-browser` is a Rust browser scaffold for local fixtures and small HTML
experiments. It is not Chromium-compatible yet: there is no general JavaScript
VM, full Web API surface, full CSS/layout engine, browser-grade raster/
compositor, sandbox, native GUI chrome, devtools, platform accessibility tree,
or modern site compatibility, profile-backed persistent storage, IndexedDB,
Cache API, or quota/private mode. The engine now has a reusable `BrowserApp`
state model for tabs, navigation, viewport scrolling, input actions, and
presentable RGBA frames. `rowser-browser app` exposes that state model as a
scriptable and interactive/stdin app surface with JSON cookie/localStorage
profile files, a JSON app profile for visit history/bookmarks, app-level
find/find-next state, visible viewport text, page PNG frame output, and a
deterministic browser-window PNG with tab/location/status chrome. A
feature-gated `rowser-browser window` command can now present that same frame in
a native CPU-backed window and route mouse, wheel, and basic keyboard input
through `BrowserApp`. It also has a narrow location-entry mode with live
location-bar text, Enter-to-open, Escape cancel, Backspace edit, resize-aware
viewport updates, printable-key routing into focused text controls. The product browser chrome still
needs to be built.

```sh
cargo build --release --bins

# Browse a checked-in fixture in the terminal shell.
./target/release/rowser-browser browse bench/browser-fixtures/static-text.html \
  --cmd "links" \
  --cmd "render"

# Drive the browser-app state boundary and write the final viewport frame.
./target/release/rowser-browser app bench/browser-fixtures/static-text.html \
  --cmd "new-tab max-width-layout.html" \
  --cookie-jar ./profile/cookies.json \
  --local-storage ./profile/local-storage.json \
  --profile ./profile/app-profile.json \
  --output ./app-frame.png \
  --window-output ./app-window.png \
  --json

# Keep the browser-app state alive while feeding commands from stdin.
printf 'bookmark\nfind static\nbookmarks\nprofile-history\nquit\n' | \
  ./target/release/rowser-browser app ./page.html --stdin --output ./app-frame.png

# Route a browser-window pixel click through simple chrome/page hit testing.
./target/release/rowser-browser app ./page.html \
  --cmd "window-click 5 50" \
  --window-output ./app-window.png

# Open the feature-gated native Rust window shell.
cargo run --release --features native-window --bin rowser-browser -- \
  window bench/browser-fixtures/static-text.html

# Try the narrow form helpers against a local page containing a simple form.
./target/release/rowser-browser form-url ./path/to/form.html --field q=rust
./target/release/rowser-browser submit ./path/to/form.html --field q=rust --json

# Dispatch a supported click handler, then render the session view.
./target/release/rowser-browser browse bench/browser-fixtures/click-event.html \
  --cmd "click #go" \
  --cmd "render"

# Verify fixtures, run the local WPT-subset scaffold, and measure fixture perf.
./target/release/rowser-browser verify bench/browser-fixtures/manifest.json --json
./target/release/rowser-browser wpt bench/wpt-subsets/manifest.json \
  --expectations bench/wpt-subsets/expectations.jsonl \
  --json
./target/release/rowser-bench browser-perf \
  --manifest bench/browser-fixtures/manifest.json \
  --iterations 50 \
  --chromium-baseline \
  --json
```

Those gates prove only the checked-in fixture subset and local WPT scaffold.
They are useful regression checks, not evidence of upstream WPT coverage,
modern-site compatibility, or Chromium parity.

## Search Mode Commands

The daemon needs an index before it can answer local queries. The normal path is
still crawl or ingest pages first, then query the hot local index. For full-web
discovery during development, `serve` can also use a third-party search provider
as a fallback and background-crawl those returned URLs into the local index.
The `index` command is only for a directory of already-saved `.html`, `.htm`,
`.xhtml`, or `.txt` files.

```sh
cargo build --release --bins

./target/release/rowser-search crawl https://example.com \
  --index .rowser-index \
  --max-pages 1000 \
  --max-depth 4 \
  --concurrency 64 \
  --max-fetching-per-host 4

./target/release/rowser-searchd --index .rowser-index --preload aggressive
```

In another terminal:

```sh
./target/release/rowser-search search "your query" --index .rowser-index --limit 20
```

The query parser supports a small first pass of Google-style operators on the
same fast index path:

```sh
./target/release/rowser-search search "rust site:example.com -deprecated" --index .rowser-index
./target/release/rowser-search search "guide site:example.com/docs filetype:html" --index .rowser-index
./target/release/rowser-search search '"exact phrase" site:example.com -"old phrase"' --index .rowser-index
./target/release/rowser-search search "release notes lang:en after:2025-01-01 before:2025-12-31" --index .rowser-index
./target/release/rowser-search search "+fast rust OR browser -slow" --index .rowser-index
./target/release/rowser-search suggest "bru" --index .rowser-index --limit 10
./target/release/rowser-search spell "exampel" --index .rowser-index --limit 5
```

Useful sanity checks after a crawl:

```sh
./target/release/rowser-search stats --index .rowser-index
./target/release/rowser-search render 0 --index .rowser-index
```

Or start the local search UI/API:

```sh
./target/release/rowser-search serve --index .rowser-index --addr 127.0.0.1:8765
```

To enable full-web fallback with Brave Search, use the raw Search plan/API key
instead of the Answers product:

```sh
BRAVE_SEARCH_API_KEY=... \
./target/release/rowser-search serve --index .rowser-index --addr 127.0.0.1:8765
```

Local results are returned first. If the local index has fewer than the
requested result count, the Brave provider returns web results, stores them in
`.rowser-index/web-cache.jsonl`, queues the top URLs for a polite background
crawl, rebuilds the index from `.rowser-index/crawl-docs.jsonl`, and hot-reloads
the local provider without restarting the server. Useful controls:

- `rowser_WEB_FALLBACK=0` disables third-party search.
- `rowser_WEB_CACHE_PATH=/path/to/web-cache.jsonl` changes the provider cache.
- `rowser_WEB_FALLBACK_COUNT=20` caps third-party results per query.
- `rowser_BACKGROUND_CRAWL=0` disables background crawl/index.
- `rowser_BACKGROUND_CRAWL_TOP_N=5` controls how many web result URLs are queued.
- `rowser_BACKGROUND_CRAWL_MAX_PAGES=5` controls the per-batch crawl budget.
- `rowser_BACKGROUND_CRAWL_MAX_DEPTH=0` keeps the worker to returned result URLs.

Then open `http://127.0.0.1:8765`. The UI includes compact filters for
`site:`, `filetype:`, `lang:`, `after:`, and `before:`; they compile to the
same hot query path as the CLI operators. API endpoints:

- `/crawl` opens a human-readable crawl status page with host-level frontier stats
- `/bench` opens the latest saved benchmark report for the served index
- `/render?id=0` opens a human-readable full-text render page
- `/api/search?q=terms&limit=20`
- `/api/suggest?q=pre&limit=10`
- `/api/spell?q=typo&limit=5`
- `/api/render?id=0`
- `/api/stats`
- `/api/crawl-status`
- `/api/bench-status`

The current browser engine scaffold does not use Chromium/WebKit/Gecko. It can
load local or HTTP(S) HTML, build a DOM, apply simple `display`, `text-align`,
`white-space: pre`, `max-width`, and horizontal auto-margin CSS, run block text,
preformatted text, list-marker, nested-list indentation, and constrained
document-column layout, apply small blockquote/definition-list indentation
defaults, flow simple table cells across rows with basic column padding, render
common input/select/textarea controls as inline widgets, write
supported form edits into the live DOM with narrow `input`/`change` listener
dispatch, expose a retained paint-backed layout-tree snapshot for supported
element bounds and parent/child links, dispatch narrow `focus`/`blur` plus
bubbling `focusin`/`focusout`
listeners for supported form focus transitions, expose `document.activeElement`
for that focus path, dispatch narrow `keydown`/`keyup` events with
`event.type`/`event.key`/`event.target` for focused text edits and honor
`keydown` `preventDefault()` before mutating text, dispatch narrow
`beforeinput` events with `event.inputType`/`event.data` before supported text
insertion and Backspace deletion while honoring `preventDefault()`, dispatch
narrow `pointerdown`/`mousedown`/`pointerup`/`mouseup` events around coordinate
clicks with coordinate and pointer readback before the generated
click/default-action path, dispatch
narrow form `submit`/`reset` events before supported default navigation/reset
while honoring `preventDefault()`, expose `event.currentTarget` and honor
`stopPropagation()` and `stopImmediatePropagation()` for supported bubbling
events, parse narrow capture and `once` listener options, remove supported
listeners by named callable handler plus capture, dispatch supported events in
capture/target/bubble order with `event.eventPhase`, route narrow delegated
`document` listeners through the DOM document node, keep supported `window`
listeners on a distinct top-level event target with limited
`event.currentTarget`/`this` identity readback, keep a live per-entry
DOM/runtime for repeated supported clicks and event listeners, emit a
deterministic display list,
resolve anchor links, discover static subresources, extract static forms,
construct GET submission URLs, submit GET forms through a small session history,
carry session cookies across HTTP navigations, follow bounded HTTP redirects
while carrying redirect-set cookies into the next hop, fetch/cache discovered
static resources, apply fetched stylesheets to text layout, execute a tiny
inline JavaScript subset for document title/text mutations plus
`createElement`/`createTextNode`/`appendChild`, tree mutation methods,
DOM traversal properties, insertion convenience methods, `DocumentFragment`
insertion, `Element.matches`/`Element.closest`, `innerHTML`, form-control DOM properties,
location readback properties, `setAttribute`/`getAttribute`
plus tiny `element.classList` and `element.style` mutations over the supported
CSS selector/property subset, DOM query collections, plus origin-scoped `localStorage` and in-memory `sessionStorage`, dispatch
document/window lifecycle listeners, drain a deterministic `setTimeout` task queue, fetch external scripts into that same tiny runtime, dispatch inline `onclick` and
`addEventListener("click", ...)` handlers for a first click path, let shell
`click <selector>` follow anchor hrefs as a narrow default action, route shell
coordinate clicks through display-list hit testing into that same supported
event/default-action navigation path, remember text-like form field values on
the current `BrowserSession` entry for later GET form submission, remember
validated select option choices for later submission, track a
focused form control for `browse` shell `focus`/`type`/`backspace`/
`clear-input`/`enter`/`space` commands, cycle focus with `tab`/`shift-tab`, search
rendered text with `find`/`find-next`, reload the current session entry without
pushing new history, manage multiple local shell tabs, report the current
browse viewport as RGBA frame metadata, write the final visible browse viewport
as a PNG, and render terminal text plus deterministic raster artifacts:

```sh
./target/release/rowser-browser render ./page.html --width 100
./target/release/rowser-browser render ./page.html --display-list
./target/release/rowser-browser render-styled ./page.html --display-list
./target/release/rowser-browser render-scripted ./page.html --display-list
./target/release/rowser-browser render-images ./page.html --display-list
./target/release/rowser-browser raster ./page.html --viewport-width 80 --viewport-height 24 --viewport-y 20 --output viewport.pgm
./target/release/rowser-browser raster-file ./page.html --scroll-y 20 --viewport-width 80 --viewport-height 24 --json
./target/release/rowser-browser screenshot-file ./page.html --output ./page.png --json
./target/release/rowser-browser hit-test ./page.html --x 12 --y 4 --json
./target/release/rowser-browser layer-tree ./page.html --json
./target/release/rowser-browser layout-tree ./page.html --json
./target/release/rowser-browser viewport ./page.html --viewport-width 80 --viewport-height 24 --viewport-y 20 --previous-x 0 --previous-y 0 --previous-width 80 --previous-height 24 --json
./target/release/rowser-browser viewport-frame ./page.html --viewport-width 80 --viewport-height 24 --viewport-y 20 --previous-x 0 --previous-y 0 --previous-width 80 --previous-height 24 --output ./viewport.png --json
./target/release/rowser-browser accessibility-tree ./page.html --json
./target/release/rowser-browser click ./page.html "#go" --display-list
./target/release/rowser-browser click-at ./page.html 0 0 --display-list
./target/release/rowser-browser click-at ./page.html 0 0 --viewport-y 12 --json
./target/release/rowser-browser browse ./page.html
./target/release/rowser-browser browse ./page.html --cmd "links" --cmd "link 0" --cmd "render"
./target/release/rowser-browser browse ./page.html --cmd "click #go" --cmd "scroll 12" --cmd "render"
./target/release/rowser-browser browse ./page.html --cmd "focus input[name=q]" --cmd "type rust browser" --cmd "submit 0"
./target/release/rowser-browser browse ./page.html --cmd "reload" --cmd "render"
./target/release/rowser-browser browse ./page.html --cmd "new-tab ./other.html" --cmd "tabs"
./target/release/rowser-browser browse ./page.html --cmd "down 20" --screenshot-output ./viewport.png --json
./target/release/rowser-browser app ./page.html --cmd "click #go" --cmd "down 20" --output ./app-frame.png --json
./target/release/rowser-browser render https://example.com --json
./target/release/rowser-browser resources ./page.html
./target/release/rowser-browser fetch-resources ./page.html
./target/release/rowser-browser session ./first.html ./second.html --back 1
./target/release/rowser-browser form-url ./path/to/form.html --field q=rust
./target/release/rowser-browser submit ./path/to/form.html --field q=rust --json
./target/release/rowser-browser verify bench/browser-fixtures/manifest.json
./target/release/rowser-browser verify bench/document-pages/manifest.json --json
./target/release/rowser-browser compare-chromium bench/browser-fixtures/manifest.json --json
./target/release/rowser-browser wpt bench/wpt-subsets/manifest.json --expectations bench/wpt-subsets/expectations.jsonl --json
./target/release/rowser-browser capabilities
./target/release/rowser-browser coverage --json --require static-html-parse --require display-list --require display-list-hit-testing --require layer-tree-snapshot --require retained-layout-tree --require viewport-raster-culling --require browser-viewport-layout-state --require browser-viewport-invalidation --require browser-viewport-frame-surface --require browser-app-state-surface --require browser-app-cli-surface --require browser-app-interactive-shell --require browser-app-visible-viewport --require browser-app-profile-history-bookmarks --require browser-app-find-text --require browser-app-window-frame --require browser-app-window-hit-testing --require browser-native-window-shell --require browser-native-window-location-input --require rgba-screenshot-artifact --require stage1-document-page-corpus --require static-accessibility-tree --require browser-shell-cli --require browser-shell-visual-frame --require browser-shell-tabs --require browser-shell-relative-open --require browser-shell-location-command --require browser-shell-cookie-inspection --require browser-shell-clear-cookies --require browser-shell-cookie-jar-file --require browser-shell-local-storage-file --require browser-shell-local-storage-inspection --require browser-shell-session-storage-inspection --require browser-shell-clear-local-storage --require browser-shell-clear-session-storage --require http-redirect-navigation --require browser-shell-link-activation --require browser-session-reload --require browser-shell-reload --require browser-shell-anchor-click-default --require browser-shell-fragment-navigation --require browser-shell-coordinate-click --require browser-cli-click-at-viewport-offset --require css-max-width-auto-margin-layout --require browser-shell-wheel-events --require browser-shell-find-text --require browser-shell-form-fill-state --require browser-session-select-form-state --require browser-shell-select-form-choice --require browser-session-checkable-form-state --require browser-shell-checkable-form-toggle --require browser-session-focused-form-control --require browser-session-focus-traversal --require browser-shell-focused-text-input --require browser-shell-focus-traversal --require browser-session-focused-text-edit --require browser-shell-focused-text-edit --require browser-session-focused-form-submit --require browser-shell-enter-submit --require browser-session-urlencoded-post-form-submit --require browser-session-form-submit-button-click-default --require browser-session-submitter-action-method-overrides --require browser-session-form-reset-click-default --require browser-session-pointer-events --require browser-session-mouse-events --require external-script-render --require dom-tree-mutation --require dom-node-traversal --require dom-insertion-methods --require document-fragment --require dom-selector-methods --require dom-inner-html-mutation --require dom-form-control-properties --require dom-location-readback --require dom-set-attribute --require dom-get-attribute --require dom-style-property-mutation --require dom-class-list-mutation --require dom-query-collections --require local-storage-api --require session-storage-api --require timer-task-queue --require document-lifecycle-events --require inline-onclick-event --require event-listener-click --require complex-query-selector --require complex-click-selector --require compound-css-selectors --require attribute-css-selectors --require hidden-attribute --require responsive-image-selection --require network-image-render --min-implemented-ratio 0.40
./target/release/rowser-bench browser-perf --manifest bench/browser-fixtures/manifest.json --iterations 50 --chromium-baseline --json
./target/release/rowser-bench browser-compat --manifest bench/wpt-subsets/manifest.json --expectations bench/wpt-subsets/expectations.jsonl --min-pass-rate 1 --max-unexpected-failures 0 --max-flakes 0 --json
./target/release/rowser-bench audit --claim combined --json
./target/release/rowser-bench traceability --json
./target/release/rowser-bench evidence --json
./target/release/rowser-bench readiness
```

Fixture manifests are JSON files whose paths are resolved relative to the
manifest file. Each fixture can assert the page title, terminal text, and
deterministic display-list commands. `compare-chromium` runs the same fixtures
through headless Chromium and compares normalized title/text output:

```json
{
  "fixtures": [{
    "name": "static wrap smoke",
    "path": "page.html",
    "width": 40,
    "expected_title": "Page",
    "expected_text": "visible text",
    "expected_display_list": [
      {"command": "text", "x": 0, "y": 0, "text": "visible text"}
    ],
    "expected_screenshot_hash": "optional RGBA screenshot hash"
  }]
}
```

`render-images` is the image-aware local render path. It navigates to a local
or HTTP(S) page, discovers image resources, fetches them with the same
`--resource-max-bytes` cap used by resource fetches, decodes the currently
supported SVG/PNG/data-URL image subset, and rerenders with decoded image
metadata and pixels available to the display-list/raster path:

```sh
./target/release/rowser-browser render-images ./page.html \
  --width 100 \
  --resource-max-bytes 1048576 \
  --display-list

./target/release/rowser-browser render-images ./page.html --json
```

That command is local image-rendering scaffold evidence for the current
supported subset. It does not imply full image-format coverage, browser
compositor correctness, upstream WPT coverage, or Chromium parity.

`raster` and `raster-file` can rasterize a fixed terminal/text viewport window
from whole-document scroll offsets. Use `--viewport-x` / `--viewport-y` (also
accepted as `--scroll-x` / `--scroll-y`) for those offsets and
`--viewport-width` / `--viewport-height` for the output window. This culls
commands outside the window for deterministic CPU raster output; it is not
browser-accurate scrolling, async scroll handling, or compositor tiling.
The text viewport report also clamps requested shell offsets to the current
document extents and reports max scroll offsets plus the retained layout boxes
visible in that window. That is useful browser-shell state for the supported
document path; it is not CSS overflow, scroll anchoring, async compositor
scrolling, or full viewport semantics.
`viewport` exposes the same document-viewport state directly, including
requested and clamped viewport coordinates, scroll delta, visible boxes, and
dirty regions between a previous and current viewport. The dirty regions are
cell-space repaint accounting for the current whole-document path, not a
compositor damage tracker or GPU tile invalidation model.
`viewport-frame` combines that clamped viewport state with the deterministic
RGBA raster path and maps dirty cell regions to dirty pixel rectangles that a
future native shell can present. It is still CPU presentation scaffold evidence,
not an OS window, GPU swapchain, or compositor.
The reusable Rust `BrowserApp` API builds on the same frame-surface contract
and owns browser-level state for tabs, navigation, scroll offsets, click/focus/
typing actions, and full-versus-partial repaint decisions. It is the intended
state boundary for the upcoming native window shell. The `rowser-browser app`
command now drives that boundary directly, can run `--cmd` scripts, can keep an
interactive/stdin command stream alive across navigations and tabs, can
load/save JSON cookie and localStorage files, can persist app-level visit
history/bookmarks through `--profile`, can track find/find-next match state,
can refresh a viewport PNG while printing the visible text viewport, and can
compose a deterministic browser-window PNG with simple tab/location/status
chrome plus narrow window-coordinate hit testing for future native shell
backends. The feature-gated `rowser-browser window` command presents the same
RGBA window frame through a native CPU framebuffer and routes mouse, wheel, and
basic keyboard input through `BrowserApp`; it now includes narrow location entry
and focused-control text routing, JSON app-profile visit-history/bookmark
persistence, and small tab/bookmark shortcuts. It is still early shell evidence,
not product-grade browser chrome, a full omnibox, autocomplete, search-provider
integration, IME, clipboard, encrypted profile storage, sync, private browsing,
downloads, settings, menus, or process isolation.

`hit-test` runs the local display-list hit-test scaffold against the current
supported layout/display-list subset:

```sh
./target/release/rowser-browser hit-test ./page.html --x 12 --y 4 --json
```

It reports the topmost supported display-list command at the given terminal-cell
coordinate. This is fixture/debug evidence, not proof of full input routing,
scrolling, transforms, iframe routing, pointer-event semantics, text selection,
or browser-grade hit-testing correctness. The `browser-shell-coordinate-click`
feature gate is narrower still: CLI shell coordinate clicks may consume this
hit-test result only to enter the supported generated
`pointerdown`/`mousedown`/`pointerup`/`mouseup`/click-handler/default-action
navigation path, not to prove full browser pointer routing, full PointerEvent
semantics, or full MouseEvent semantics.
The one-shot `click-at`/`tap` command also accepts `--viewport-x`/`--viewport-y`
aliases (`--scroll-x`/`--scroll-y`) and translates the provided visible point to
document coordinates before the same supported hit-test/default-action path.
Shell `scroll`/`down`/`up`/`left`/`right` commands now dispatch a narrow
document-level `wheel` event first, expose `event.deltaX`/`event.deltaY` to the
tiny listener subset, rerender supported DOM mutations, and cancel the local
viewport movement when a handler calls `preventDefault()`. This is CLI shell
input evidence only; it is not browser-accurate scroll containers, CSS overflow,
full WheelEvent semantics, compositor scrolling, or platform device input.

`layer-tree` emits a local layer-tree/debug snapshot derived from the current
supported display-list/layout subset:

```sh
./target/release/rowser-browser layer-tree ./page.html --json
```

This is scaffold evidence for layer snapshot shape, bounds, paint-source
grouping, and benchmarked layer-count/topology metrics in local fixtures. It is
not proof of a compositor scheduler, GPU compositing, OS presentation, async
scrolling, tile caching,
transforms/clips correctness, iframe/OOPIF surfaces, or Chromium compositor
parity.

`layout-tree` emits a retained layout-tree/debug snapshot for the current
paint-backed element boxes:

```sh
./target/release/rowser-browser layout-tree ./page.html --json
```

This is scaffold evidence for supported element bounds, parent/child layout-box
links, and command-to-box ownership. It is not full CSS layout, anonymous box
generation, inline fragmentation, scrolling boxes, browser-accurate geometry,
or a replacement for future layout/compositor architecture.

`browse` is the current early local CLI shell that makes the current static
engine playable from a terminal. It wraps the supported `BrowserSession` subset:
`open <url-or-path>`, `location` / `url` / `where`, `cookies`,
`local-storage` / `storage` / `localstorage`, `session-storage` /
`sessionstorage`, `clear-cookies`, `clear-local-storage`,
`clear-session-storage`, `tabs`, `new-tab <url-or-path>`,
`switch-tab <index>`, `close-tab [index]`, `back`, `forward`, `links`, `link <index>`,
`follow text <label>`, `activate selector <selector>`, `reload` / `refresh`,
`click <selector>`, `click-at <x> <y>` / `tap <x> <y>`,
`submit <form-index> name=value`,
`submit-get <form-index> name=value`, `submit-post <form-index> name=value`,
`focus <selector>`, `tab`, `shift-tab`, `type <text>`, `backspace [count]`,
`clear-input`, `enter`, `space`, `toggle <form-index> <control-index>`,
`choose <value>`, `select <form-index> <control-index> <value>`, `styles`,
`scripts`, `images`, `scroll`, `left`/`right`, `top`, `bottom`, `history`, and
`render` for
the current page in a fixed text viewport. The optional `--cookie-jar <path>`
and `--local-storage <path>` flags load JSON state before the shell run and
save the current cookie jar and origin-scoped localStorage after it finishes;
sessionStorage stays in memory for the current `BrowserSession`. Shell tabs are
local `BrowserSession` instances with independent history and viewport state;
new tabs copy cookies and localStorage from the active tab at creation time, but
this is not browser-grade shared profile synchronization, GUI tab chrome,
session restore, or process isolation.
`open` / `go` resolve relative
targets against the current page source before navigating, which makes sibling
paths, query strings, and fragments usable in scripted and interactive shells.
`links` lists the current page's
extracted anchor text, href, and resolved target; `link` / `follow` /
`activate` navigate to a resolved target by zero-based index, exact link text,
or a selector that resolves to an anchor. `click <selector>` dispatches the
supported click-handler subset and, when the selected node resolves to an anchor
with an `href`, navigates the resolved target through the same `BrowserSession`
history as a narrow default action. Coordinate-click shell evidence routes a
terminal-cell coordinate through display-list hit testing to that same supported
generated `pointerdown`/`mousedown`/`pointerup`/`mouseup`/click/default-action
path. The
`browser-shell-relative-open` gate covers only
current-page-relative shell `open` / `go` resolution; it is not omnibox search,
URL autocomplete, tab UI, security UI, or full browser chrome. The
`browser-shell-location-command` gate reports
the current source, title, history position, and text viewport without changing
page state; it is not a real address bar, omnibox, browser chrome, or tab UI.
The `browser-shell-cookie-inspection` gate prints the current in-memory
`BrowserSession` cookie jar without changing page state; it is not persistent
profile storage, cookie settings UI, permissions, partitioning, or browser
chrome. The `browser-shell-clear-cookies` gate clears that in-memory cookie jar
without navigating or changing page state; it is not persistent profile
clearing, storage partition clearing, settings UI, or browser chrome. The
`browser-shell-cookie-jar-file` gate loads and saves that in-memory cookie state
through a local JSON `--cookie-jar` file for `rowser-browser browse`; it is not
encrypted profile storage, expiration persistence, partitioning, cookie settings
UI, or browser chrome. The `browser-shell-local-storage-file` gate loads and
saves origin-scoped localStorage through a local JSON `--local-storage` file; it
is not IndexedDB, Cache API, quota management, private browsing, partitioning,
or browser chrome. The `browser-shell-local-storage-inspection` gate prints
current origin-scoped localStorage session entries without changing page state;
it is not a devtools storage panel, IndexedDB, Cache API, quota management,
partitioning, or browser chrome. The `browser-shell-clear-local-storage` gate
clears current origin-scoped localStorage session state; it is not full
site-data clearing, IndexedDB, Cache API, quota management, private browsing,
partitioning, or browser chrome. The
`browser-shell-session-storage-inspection` gate prints current in-memory
origin-scoped sessionStorage entries without changing page state; it is not
persistent profile storage, a devtools storage panel, IndexedDB, Cache API,
quota management, partitioning, or browser chrome. The
`browser-shell-clear-session-storage` gate clears current in-memory
origin-scoped sessionStorage state without navigating; it is not full site-data
clearing, persistent profiles, IndexedDB, Cache API, quota management, private
browsing, partitioning, or browser chrome. The
`http-redirect-navigation` gate covers bounded HTTP(S) redirects
for document, form, and static resource loads, including redirect-set cookies
and final session entry URLs; it is not full navigation lifecycle,
mixed-content/referrer/CORS policy, HSTS, redirect UI, or browser-grade error
pages. The `browser-shell-fragment-navigation` gate records rendered `id` and
legacy anchor `name` targets and scrolls the CLI text viewport to a matching
fragment after supported open/link/click navigation. It is not full CSS scroll
behavior, `:target` styling, history scroll restoration, or browser UI. The
`browser-shell-form-fill-state` gate covers only
filled remembered values for text-like controls on the current `BrowserSession`
entry being merged into a later GET form submission. It is not full interactive
form state, validation, focus/input events, autofill, POST, or browser UI.
The select form gates cover only single-select option metadata, enabled-option
validation, focused `choose <value>`, and explicit
`select <form> <control> <value>` commands feeding later GET or URL-encoded POST
submission. They are not native select UI, multi-select, optgroup inheritance,
input/change events, validation, keyboard events, or browser UI.
The checkable form gates cover only checkbox/radio checked state remembered on a
`BrowserSession` entry, explicit `toggle <form> <control>` and focused `space`
commands, and narrow selector-click and label defaults for supported
checkbox/radio controls. They are not full input/change event dispatch, full
keyboard event dispatch, full label activation semantics, indeterminate
checkboxes, custom controls, validation, or browser UI.
The `browser-session-required-form-validation` gate blocks supported
BrowserSession/CLI GET and URL-encoded POST submissions when enabled required
text-like, select, checkbox, or radio controls are empty or unchecked, and honors
form `novalidate` and submitter `formnovalidate`. It is not full constraint validation, validation UI, custom
validity, invalid events, or broad browser form compatibility.
The `browser-session-type-value-validation` gate blocks supported submit paths
for non-empty email and URL controls with invalid values. It is not full HTML
type validation, IDNA/email grammar, validation UI, or broad browser form
compatibility.
The `browser-session-submitter-action-method-overrides` gate honors submitter
`formaction` and GET/POST `formmethod` overrides for supported
BrowserSession/CLI submit-control click and focused-submit paths. It is not
external form ownership, `target`/`enctype`/`dialog` handling, full event
ordering, or broad browser form compatibility.
The `browser-session-focused-form-control` and
`browser-shell-focused-text-input` gates cover only selector focus, associated
label focus/click, and typed text append into editable text-like controls in the local
shell/session path. Focus traversal gates cycle forward/backward through named, enabled, fillable,
select, checkable, and submit/reset action controls in rendered form order. The
`browser-session-focus-events` gate adds narrow target `focus`/`blur`,
bubbling `focusin`/`focusout`, and `document.activeElement` for those supported
focus transitions. The focused text edit gates add Unicode-safe backward
deletion and clearing for the active editable text-like control. The
`browser-session-keyboard-events` gate adds narrow bubbling `keydown`/`keyup`
dispatch for typed text and Backspace, `event.type`/`event.key`/`event.target`
readback, and keydown `preventDefault()` blocking of the default text mutation.
The `browser-session-beforeinput-events` gate adds narrow bubbling
`beforeinput` dispatch for supported text insertion and Backspace deletion before
live DOM/form mutation, exposes `event.inputType`/`event.data`, and honors
`preventDefault()` by skipping the edit and following `input` event.
The `browser-session-pointer-events` gate adds narrow bubbling
`pointerdown`/`pointerup` dispatch around supported coordinate clicks before the
existing click/default-action path, exposes coordinate and pointer readback, and
forwards coordinate readback to the generated click event.
The `browser-session-mouse-events` gate adds narrow bubbling `mousedown` and
`mouseup` compatibility events around supported coordinate clicks, with
coordinate and button readback, before the generated click event.
The `browser-shell-wheel-events` gate adds narrow document-level `wheel`
dispatch before local shell viewport scroll commands, with `event.deltaX` and
`event.deltaY` readback plus `preventDefault()` cancellation of that shell
viewport movement. It is not browser scroll-container behavior, compositor
scrolling, or full WheelEvent/platform input semantics.
The `browser-session-event-target-propagation` gate adds
`event.target`/`event.currentTarget` readback across supported bubbling events
and narrow `stopPropagation()` behavior that blocks ancestor listeners without
suppressing later listeners on the same target.
The `browser-session-stop-immediate-propagation` gate adds narrow
`stopImmediatePropagation()` behavior that suppresses later same-target
listeners and stops remaining propagation for supported session events.
The `browser-session-document-event-listeners` gate lets `document`
`addEventListener` handlers receive supported events through the DOM document
node for delegated click, keyboard, form, input/change, and focusin/out paths.
The `browser-session-capture-event-listeners` gate parses boolean and object
`capture` listener options for supported session events, dispatches
capture/target/bubble order, and exposes `event.eventPhase`.
The `browser-session-once-event-listeners` gate parses object-form `once`
listener options for supported session events and removes those listeners after
their first invocation across repeated interactions.
The `browser-session-remove-event-listener` gate supports narrow
`removeEventListener` matching by resolved callable handler and capture option
for supported session/lifecycle event lists.
The `browser-session-window-event-target` gate keeps supported `window`
listeners on a distinct top-level event target around the document path and adds
limited `event.currentTarget === window` / `this === window` readback for that
listener path.
The `browser-session-submit-reset-events` gate adds narrow bubbling
`submit`/`reset` handlers on supported forms before the default navigation/reset
path, including `preventDefault()` cancellation and submit-handler mutations to
supported live form values before submission.
The focused form submit
gates add the local `enter` path that submits focused fillable/select controls,
activates focused submit controls, or resets focused reset controls using current
remembered field state. They are not full keyboard event semantics, full DOM
tab order, tabindex, `relatedTarget`,
full `InputEvent` semantics, composition events, `SubmitEvent.submitter`, invalid events,
requestSubmit semantics, broad function declaration parsing, exact browser
listener identity, dispatch-time listener removal ordering, `passive`/`signal`
listener options, composed paths, shadow DOM retargeting, the full Window API,
global event handler attributes, custom elements, caret positioning, text
selection, shortcuts, repeat state, IME, validation UI, autofill, platform text
input, undo, or browser UI.
The `browser-shell-find-text` gate covers only rendered text-line search and
viewport scrolling in the local shell; it is not full browser find UI,
highlighting, selection, match counting, or locale-aware search.
The `browser-session-reload` and `browser-shell-reload` gates cover only
reloading the current session entry target and replacing that entry without
pushing new history. They are not full reload lifecycle, cache policy, POST
replay UI, service worker handling, or BFCache policy.
The `browser-session-urlencoded-post-form-submit` gate covers only a narrow
`application/x-www-form-urlencoded` POST path through `BrowserSession` and the
local CLI shell; it must not be used to claim multipart/file uploads,
fetch/XHR, validation, full navigation lifecycle, browser UI, or broad browser
compatibility.
The `browser-session-form-submit-button-click-default` gate covers only supported
submit/input/button click default actions that route through `BrowserSession` and
the CLI shell for GET or URL-encoded POST forms. It is not full form event
dispatch, validation, focus/input behavior, browser UI, or Chromium parity.
The `browser-session-form-reset-click-default` gate covers only supported reset
control click default actions that clear remembered text-like state for the owning
form in `BrowserSession` and the CLI shell. It is not constraint validation, full
form event dispatch, focus/input behavior, browser UI, or Chromium parity.
Interactive use reads commands from stdin; scripted smoke runs can pass repeated
`--cmd` arguments and then exit:

```sh
./target/release/rowser-browser browse ./page.html
./target/release/rowser-browser browse \
  ./page.html \
  --cmd "links" \
  --cmd "link 0" \
  --cmd "render"
./target/release/rowser-browser browse \
  ./page.html \
  --cmd "click #go" \
  --cmd "click-at 0 0" \
  --cmd "scroll 12" \
  --cmd "render"
./target/release/rowser-browser browse \
  ./page.html \
  --cmd "reload" \
  --cmd "render"
./target/release/rowser-browser browse \
  ./page.html \
  --cmd "focus input[name=q]" \
  --cmd "type browser" \
  --cmd "tab" \
  --cmd "type notes" \
  --cmd "shift-tab" \
  --cmd "backspace 3" \
  --cmd "type ser" \
  --cmd "enter"
./target/release/rowser-browser browse ./page.html --cmd "focus input[type=checkbox]" --cmd "space"
```

This is a local CLI shell over the existing static renderer, not GUI browser
chrome. Link activation and anchor-click default navigation are resolved-href
session navigation, form fill state is remembered-value GET-submit scaffold
only, select state is single-select option scaffold only, checkable state is checkbox/radio toggle, focused-space, and label scaffold only,
fragment navigation is rendered-id/name text-viewport scrolling only,
focused text traversal/input/editing/submission is tab/type/backspace/clear/
enter-command scaffold only, and find text is rendered-line viewport scrolling
only, not full browser event/default-action semantics,
full event-cancellation semantics, full pointer routing, full PointerEvent
semantics, full MouseEvent semantics,
full interactive form state, checkable form behavior beyond checkbox/radio
state toggles, validation, keyboard events, text selection, IME, autofill,
transformed/scrolling hit testing, SPA navigation, tab/process isolation,
devtools/accessibility, browser UI, or Chromium parity.
The `browser-session-urlencoded-post-form-submit` marker is explicitly scoped to
URL-encoded POST submission in `BrowserSession`/CLI only, not broad form
compatibility.
The `browser-session-form-submit-button-click-default` marker is explicitly
scoped to BrowserSession/CLI submit-control click default action, not full form
events, validation, focus/input state, browser UI, or broad input compatibility.
The `browser-session-form-reset-click-default` marker is explicitly scoped to
BrowserSession/CLI reset-control click default action, not full form events,
validation, focus/input state, browser UI, or broad input compatibility.
The `browser-session-required-form-validation` marker is explicitly scoped to
value-missing checks for supported required controls in BrowserSession/CLI submit
paths, with form `novalidate` and submitter `formnovalidate`, not full
constraint validation or validation UI.
The `browser-session-type-value-validation` marker is explicitly scoped to
non-empty email/URL value checks in BrowserSession/CLI submit paths, not full
type validation or browser validation UI.
The `browser-session-submitter-action-method-overrides` marker is explicitly
scoped to submitter `formaction` and GET/POST `formmethod` overrides on
supported BrowserSession/CLI submit-control paths, not full submitter
semantics, external form ownership, target/enctype/dialog handling, or browser
compatibility.

A minimal WPT-subset/compatibility scaffold lives in
`bench/wpt-subsets/manifest.json` with expectation notes in
`bench/wpt-subsets/expectations.jsonl`. It reuses tiny local
`bench/browser-fixtures` pages across HTML, DOM, CSS, images, and rendering
subsystems; it is not a full upstream Web Platform Tests import, a
WebDriver/testdriver runner, a reftest harness, or proof of Chromium parity.

Run the local scaffold directly with:

```sh
./target/release/rowser-browser wpt bench/wpt-subsets/manifest.json \
  --expectations bench/wpt-subsets/expectations.jsonl \
  --json
```

To use the same scaffold as a strict local fixture-compatibility gate:

```sh
./target/release/rowser-bench browser-compat \
  --manifest bench/wpt-subsets/manifest.json \
  --expectations bench/wpt-subsets/expectations.jsonl \
  --min-pass-rate 1 \
  --max-unexpected-failures 0 \
  --max-flakes 0 \
  --json
```

To surface that compatibility report in the local `/bench` status UI, save it
beside the served index:

```sh
./target/release/rowser-bench browser-compat \
  --manifest bench/wpt-subsets/manifest.json \
  --expectations bench/wpt-subsets/expectations.jsonl \
  --min-pass-rate 1 \
  --max-unexpected-failures 0 \
  --max-flakes 0 \
  --report-output .rowser-index/bench-status.json \
  --json

./target/release/rowser-search serve --index .rowser-index
```

Those commands currently assert only the local subset entries in the manifest
and their expectation rows. Passing them means the local compatibility scaffold
matches its checked-in expectations; it does not mean the browser passes
upstream WPT, matches Chromium behavior, or supports the wider web platform.
The next standards gates are imported upstream WPT slices with explicit
expectations and skip manifests, WebDriver/testdriver and reftest coverage where
the subset needs them, flake quarantine, and a pinned Chromium comparison for
the same tests.

This is an engine skeleton, not a modern browser yet: no general JavaScript VM,
full event loop, Web API surface, full CSS cascade, browser-accurate
painting/raster, compositor, sandbox, persistent storage, media, canvas,
accessibility tree, GUI browser chrome, tab/process isolation, devtools, or
Chromium parity.

By default `crawl` stays on the seed URL's exact host. To follow links across
the wider web, use `--boundary any-domain` with a strict `--max-pages` cap:

```sh
./target/release/rowser-search crawl https://example.com \
  --boundary any-domain \
  --max-pages 10000 \
  --index .rowser-index
```

For a larger crawl, put one seed URL per line in a file. Blank lines and lines
starting with `#` are ignored:

```sh
./target/release/rowser-search crawl \
  --seed-file seeds.txt \
  --index .rowser-index \
  --max-pages 100000 \
  --max-depth 6
```

You can combine a positional seed URL with `--seed-file`. With the default
`--boundary same-host`, links are followed when they stay on any seed host.
Use `--boundary any-domain` only with a strict page cap.

You can also start from domains instead of full URLs. Bare domains default to
`https://` and are normalized to the root URL:

```sh
./target/release/rowser-search crawl \
  --domain example.com \
  --domain-file domains.txt \
  --index .rowser-index \
  --max-pages 100000
```

Domain files use the same one-entry-per-line format as seed files.

You can also import URLs from XML or XML.gz sitemaps. Sitemap indexes are
followed recursively up to `--max-sitemaps`, and page URLs are capped by
`--max-sitemap-urls`:

```sh
./target/release/rowser-search crawl \
  --sitemap https://example.com/sitemap.xml \
  --index .rowser-index \
  --max-pages 100000 \
  --max-sitemap-urls 100000
```

`--sitemap` can be repeated and can point at an HTTP(S) URL or a local sitemap
file. To discover sitemap URLs advertised in seed-host `robots.txt` files, add
`--discover-sitemaps`:

```sh
./target/release/rowser-search crawl https://example.com \
  --discover-sitemaps \
  --index .rowser-index \
  --max-pages 100000
```

Robots-discovered sitemap URLs share the same `--max-sitemaps` and
`--max-sitemap-urls` caps as manual `--sitemap` input.

For refresh workflows, `--recrawl-manifest` reads JSON Lines records with
`url`, `domain`, or `sitemap` fields. `priority` sorts due work, and
`recrawl_after` gates future work until the timestamp is due. `recrawl_after`
accepts RFC3339 timestamps or Unix seconds:

```json
{"url":"https://example.com/page","priority":10}
{"domain":"example.org","recrawl_after":"2026-06-01T00:00:00Z"}
{"sitemap":"https://example.net/sitemap.xml"}
```

```sh
./target/release/rowser-search crawl \
  --recrawl-manifest recrawl.jsonl \
  --discover-sitemaps \
  --index .rowser-index
```

Future-dated recrawl records are skipped by default. Use
`--include-future-recrawls` to replay every manifest entry regardless of
`recrawl_after`. Due manifest entries requeue matching fetched URLs in the
frontier, and repeated snapshot rows are resolved with the latest fetched
document when the index is rebuilt.

To generate a recrawl manifest from persisted frontier state, use
`recrawl-plan`. It selects fetched URLs whose last fetch is at least
`--interval-secs` old, sorts older URLs first, and writes JSON Lines compatible
with `--recrawl-manifest`:

```sh
./target/release/rowser-search recrawl-plan \
  --index .rowser-index \
  --interval-secs 604800 \
  --limit 10000 \
  --output recrawl.jsonl
```

To keep a single-machine index fresh, run the scheduler loop. Each round selects
due fetched URLs from the frontier, recrawls the batch, rebuilds from the latest
crawl snapshot, and prints a JSON status row with changed/unchanged counts:

```sh
./target/release/rowser-search recrawl-scheduler \
  --index .rowser-index \
  --interval-secs 604800 \
  --batch-size 1000 \
  --poll-secs 300
```

For a bounded development run, add `--max-rounds 1`. The current scheduler is a
single-machine freshness loop; distributed scheduling, per-site change-rate
policy, and incremental segment writes remain roadmap items.

During a crawl, `.rowser-index/frontier.bin` stores durable URL state and
`.rowser-index/crawl-docs.jsonl` stores fetched documents before the final index
is written. If a crawl is interrupted, run the same `crawl` command again and it
will resume from those files.

The built index also writes `.rowser-index/field_docs.bin`, which preserves
fielded metadata such as canonical URL, meta description, language, headings,
anchor text, outbound links, content hash, extraction mode, and fetch time.
Document metadata records canonical, exact-text, and shingled-simhash
near-duplicate clusters, and search returns one result per cluster while keeping
every document renderable by id or URL. Postings keep per-field term counts, so
title, heading, anchor, meta, URL, and body matches can be weighted differently
without duplicating the rendered text.

Meta robots `noindex` pages are skipped by default during index builds. Add
`--ignore-noindex` to keep them. To filter very small pages in a crawl or local
corpus, set `--min-body-terms N`; skipped noindex/thin counts are shown in
`stats` and build output.

You can also index a saved local corpus:

```sh
cargo run --release --bin rowser-search -- index ./corpus --index .rowser-index
cargo run --release --bin rowser-searchd -- --index .rowser-index --preload aggressive
cargo run --release --bin rowser-search -- search "query text" --index .rowser-index --limit 20
cargo run --release --bin rowser-search -- render 0 --index .rowser-index
cargo run --release --bin rowser-search -- serve --index .rowser-index
cargo run --release --bin rowser-bench -- smoke --json --save-report
cargo run --release --bin rowser-bench -- eval --index .rowser-index --judgments bench/judgments.jsonl --require-ndcg 0.9 --require-recall 0.9 --max-unresolved 0 --json --save-report
cargo run --release --bin rowser-bench -- search --index .rowser-index --queries bench/queries.txt --save-report
cargo run --release --bin rowser-bench -- search --index .rowser-index --queries bench/queries.txt --chromium-baseline --require-speedup 10 --json --save-report
cargo run --release --bin rowser-bench -- browser-perf --manifest bench/browser-fixtures/manifest.json --iterations 50 --chromium-baseline --json --save-report
cargo run --release --bin rowser-bench -- browser-compat --manifest bench/wpt-subsets/manifest.json --expectations bench/wpt-subsets/expectations.jsonl --min-pass-rate 1 --max-unexpected-failures 0 --max-flakes 0 --json
cargo run --release --bin rowser-bench -- gate --require-ndcg 0.9 --require-recall 0.9 --max-unresolved 0 --require-browser-feature static-html-parse --require-browser-feature display-list --require-browser-feature retained-layout-tree --require-browser-feature viewport-raster-culling --require-browser-feature browser-viewport-layout-state --require-browser-feature browser-viewport-invalidation --require-browser-feature browser-viewport-frame-surface --require-browser-feature browser-app-state-surface --require-browser-feature browser-app-cli-surface --require-browser-feature browser-app-interactive-shell --require-browser-feature browser-app-visible-viewport --require-browser-feature browser-app-profile-history-bookmarks --require-browser-feature browser-app-find-text --require-browser-feature browser-app-window-frame --require-browser-feature browser-app-window-hit-testing --require-browser-feature browser-native-window-shell --require-browser-feature browser-native-window-location-input --require-browser-feature static-accessibility-tree --require-browser-feature browser-shell-cli --require-browser-feature browser-shell-relative-open --require-browser-feature browser-shell-location-command --require-browser-feature browser-shell-cookie-inspection --require-browser-feature browser-shell-clear-cookies --require-browser-feature browser-shell-cookie-jar-file --require-browser-feature browser-shell-local-storage-file --require-browser-feature browser-shell-local-storage-inspection --require-browser-feature browser-shell-session-storage-inspection --require-browser-feature browser-shell-clear-local-storage --require-browser-feature browser-shell-clear-session-storage --require-browser-feature http-redirect-navigation --require-browser-feature browser-shell-link-activation --require-browser-feature browser-session-reload --require-browser-feature browser-shell-reload --require-browser-feature browser-shell-anchor-click-default --require-browser-feature browser-shell-fragment-navigation --require-browser-feature browser-shell-coordinate-click --require-browser-feature browser-cli-click-at-viewport-offset --require-browser-feature css-max-width-auto-margin-layout --require-browser-feature browser-shell-wheel-events --require-browser-feature browser-shell-find-text --require-browser-feature browser-shell-form-fill-state --require-browser-feature browser-session-select-form-state --require-browser-feature browser-shell-select-form-choice --require-browser-feature browser-session-checkable-form-state --require-browser-feature browser-shell-checkable-form-toggle --require-browser-feature browser-session-focused-form-control --require-browser-feature browser-session-focus-traversal --require-browser-feature browser-shell-focused-text-input --require-browser-feature browser-shell-focus-traversal --require-browser-feature browser-session-focused-text-edit --require-browser-feature browser-shell-focused-text-edit --require-browser-feature browser-session-focused-form-submit --require-browser-feature browser-shell-enter-submit --require-browser-feature browser-session-urlencoded-post-form-submit --require-browser-feature browser-session-form-submit-button-click-default --require-browser-feature browser-session-submitter-action-method-overrides --require-browser-feature browser-session-form-reset-click-default --require-browser-feature browser-session-pointer-events --require-browser-feature browser-session-mouse-events --require-browser-feature inline-script-dom-text --require-browser-feature inline-script-dom-create --require-browser-feature dom-tree-mutation --require-browser-feature dom-node-traversal --require-browser-feature dom-insertion-methods --require-browser-feature document-fragment --require-browser-feature dom-selector-methods --require-browser-feature dom-inner-html-mutation --require-browser-feature dom-form-control-properties --require-browser-feature dom-location-readback --require-browser-feature dom-set-attribute --require-browser-feature dom-get-attribute --require-browser-feature local-storage-api --require-browser-feature session-storage-api --require-browser-feature timer-task-queue --require-browser-feature document-lifecycle-events --require-browser-feature external-script-render --require-browser-feature inline-onclick-event --require-browser-feature event-listener-click --min-browser-implemented-ratio 0.4 --max-browser-missing 20 --browser-compat --browser-compat-min-pass-rate 1 --browser-compat-max-unexpected-failures 0 --browser-compat-max-flakes 0 --json --save-report
cargo run --release --bin rowser-bench -- readiness --json
```

The browser performance harness reports fixture render/raster latency plus
local DOM, layout, paint, raster, and scalar layer-tree shape metrics for the
supported fixture subset only. With `--chromium-baseline`, it also runs a
headless Chromium iframe fixture baseline, inlines local fixture scripts for
deterministic `--dump-dom` timing, steps fixtures through Chromium virtual-time
callbacks so timer and click tasks can settle, reports Chromium p50/p95/p99,
records Rust-vs-Chromium p95 speedup, and hashes/checks Chromium text output
against manifest expectations. It can gate those metrics with
`--min-chromium-p95-speedup`, `--max-chromium-text-mismatches`,
`--max-layer-metrics-p95-us`, `--min-total-layers`,
`--min-total-image-layers`, `--max-layer-count`, and `--max-image-layer-count`.

The headline path is the daemon: it keeps the lexicon, document metadata,
selected postings, and text mmap hot so repeated searches avoid process startup
and index-open costs.

`rowser-bench smoke` builds the deterministic fixture corpus in
`bench/fixtures/corpus`, searches the first query in `bench/queries.txt`, renders
the top result, and emits an in-process benchmark report. It is the quickest
local proof that indexing, search, render, and benchmarking are wired together.

Search benchmark reports include p50/p95/p99 latency, throughput, Rust/Chrome/OS/
hardware metadata, corpus hash, and index hash. `--require-speedup 10` turns the
Chromium comparison into an acceptance gate and exits non-zero if the Rust path
does not beat the headless Chromium JavaScript baseline by at least 10x at p95.
Add `--save-report` to persist `.rowser-index/bench-status.json`; the local
server exposes that report at `/bench` and `/api/bench-status`.

`rowser-bench gate` is the one-command local development acceptance check. It
builds the fixture corpus, searches and renders, runs relevance judgments,
enforces browser feature-fixture coverage, and can optionally launch Chrome for
the p95 search-speed gate and curated static browser fixture parity:

```sh
cargo run --release --bin rowser-bench -- gate --chromium-search-baseline --require-speedup 10 --browser-chromium-parity --require-ndcg 0.9 --require-recall 0.9 --max-unresolved 0 --require-browser-feature static-html-parse --require-browser-feature display-list --require-browser-feature retained-layout-tree --require-browser-feature viewport-raster-culling --require-browser-feature browser-viewport-layout-state --require-browser-feature browser-viewport-invalidation --require-browser-feature browser-viewport-frame-surface --require-browser-feature browser-app-state-surface --require-browser-feature browser-app-cli-surface --require-browser-feature browser-app-interactive-shell --require-browser-feature browser-app-visible-viewport --require-browser-feature browser-app-profile-history-bookmarks --require-browser-feature browser-app-find-text --require-browser-feature browser-app-window-frame --require-browser-feature browser-app-window-hit-testing --require-browser-feature browser-native-window-shell --require-browser-feature browser-native-window-location-input --require-browser-feature static-accessibility-tree --require-browser-feature browser-shell-cli --require-browser-feature browser-shell-relative-open --require-browser-feature browser-shell-location-command --require-browser-feature browser-shell-cookie-inspection --require-browser-feature browser-shell-clear-cookies --require-browser-feature browser-shell-cookie-jar-file --require-browser-feature browser-shell-local-storage-file --require-browser-feature browser-shell-local-storage-inspection --require-browser-feature browser-shell-session-storage-inspection --require-browser-feature browser-shell-clear-local-storage --require-browser-feature browser-shell-clear-session-storage --require-browser-feature http-redirect-navigation --require-browser-feature browser-shell-link-activation --require-browser-feature browser-session-reload --require-browser-feature browser-shell-reload --require-browser-feature browser-shell-anchor-click-default --require-browser-feature browser-shell-fragment-navigation --require-browser-feature browser-shell-coordinate-click --require-browser-feature browser-cli-click-at-viewport-offset --require-browser-feature css-max-width-auto-margin-layout --require-browser-feature browser-shell-wheel-events --require-browser-feature browser-shell-find-text --require-browser-feature browser-shell-form-fill-state --require-browser-feature browser-session-select-form-state --require-browser-feature browser-shell-select-form-choice --require-browser-feature browser-session-checkable-form-state --require-browser-feature browser-shell-checkable-form-toggle --require-browser-feature browser-session-focused-form-control --require-browser-feature browser-session-focus-traversal --require-browser-feature browser-shell-focused-text-input --require-browser-feature browser-shell-focus-traversal --require-browser-feature browser-session-focused-text-edit --require-browser-feature browser-shell-focused-text-edit --require-browser-feature browser-session-focused-form-submit --require-browser-feature browser-shell-enter-submit --require-browser-feature browser-session-urlencoded-post-form-submit --require-browser-feature browser-session-form-submit-button-click-default --require-browser-feature browser-session-submitter-action-method-overrides --require-browser-feature browser-session-form-reset-click-default --require-browser-feature browser-session-pointer-events --require-browser-feature browser-session-mouse-events --require-browser-feature inline-script-dom-text --require-browser-feature inline-script-dom-create --require-browser-feature dom-tree-mutation --require-browser-feature dom-node-traversal --require-browser-feature dom-insertion-methods --require-browser-feature document-fragment --require-browser-feature dom-selector-methods --require-browser-feature dom-inner-html-mutation --require-browser-feature dom-form-control-properties --require-browser-feature dom-location-readback --require-browser-feature dom-set-attribute --require-browser-feature dom-get-attribute --require-browser-feature local-storage-api --require-browser-feature session-storage-api --require-browser-feature timer-task-queue --require-browser-feature document-lifecycle-events --require-browser-feature external-script-render --require-browser-feature inline-onclick-event --require-browser-feature event-listener-click --min-browser-implemented-ratio 0.4 --max-browser-missing 20 --json --save-report
```

That local gate is useful regression evidence, not a browser-product claim.
Browser or combined release claims still need the traceability,
readiness, evidence-registry, WPT-subset, visual, platform, security/privacy,
operations, and release-review gates for the exact claim being made.

`rowser-bench audit` is the top-level claim gate. It composes
traceability, evidence-registry coverage, and readiness for `--claim search`,
`--claim browser`, or `--claim combined`, and exits non-zero with
`--require-complete` until that exact claim has no partial or missing
requirements, no uncovered evidence, and no partial or missing readiness areas.

`rowser-bench readiness` lists the required search-mode and browser-product
areas, marks current status as implemented/partial/missing, verifies required
plan-document and implementation evidence markers, and exits non-zero with
`--require-complete` until every area has direct evidence. Use `--claim search`,
`--claim browser`, or `--claim combined` to inspect only the search readiness
areas, only the browser readiness areas, or the full combined surface.
Security/privacy is currently partial: the threat model exists,
but sandbox, origin-policy, privacy, abuse, fuzzing, and compliance gates remain
unfinished. Operations/reliability is also partial: the SLO and runbook plan
exists, but dashboards, restore drills, failure injection, load tests, release
manifests, and incident automation remain unfinished. Platform completeness is
partial too: the subsystem plan exists, but fonts/text shaping, images/SVG,
canvas/GPU, media, accessibility, input/editing, storage, devtools, extensions,
packaging, updates, and platform QA remain unfinished.

`rowser-bench traceability` validates
[`docs/REQUIREMENTS_TRACEABILITY.md`](docs/REQUIREMENTS_TRACEABILITY.md)
directly. It fails if required `REQ-*` rows are missing, duplicated, unknown,
malformed, assigned to unknown readiness areas or roadmap milestones, assigned
to unknown or mismatched release claim scopes, or contain placeholders, and it
can also enforce `--require-complete` or `--require-no-missing` gates over the
matrix state. Use `--require-claim-complete search`, `browser`, or `combined`
to gate only the search claim, the browser claim, or the full combined claim.
Use `--require-milestone-complete m0`
through `m6` to require all traceability rows assigned up through that roadmap
milestone to be implemented.

`rowser-bench evidence` validates
[`docs/EVIDENCE_REGISTRY.md`](docs/EVIDENCE_REGISTRY.md). It fails if evidence
rows are malformed, duplicated, contain unknown requirement IDs, or leave any
required `REQ-*` row without at least one proof artifact mapped to it.

`rowser-bench eval` reads JSON Lines relevance judgments and reports MRR,
NDCG@K, recall@K, precision@K, unresolved judgments, corpus hash, index hash,
and per-query diagnostics. Add `--require-mrr`, `--require-ndcg`,
`--require-recall`, `--require-precision`, or `--max-unresolved` to turn it
into a relevance gate that exits non-zero on regression. Judgment rows look like
this:

```json
{"query":"rowser search","relevant":[{"url":"https://fixtures.local/rowser-search","grade":3}]}
```
