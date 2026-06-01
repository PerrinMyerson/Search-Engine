# Platform Completeness Plan

This plan covers the non-negotiable browser platform work required before Brutal
Browser can credibly claim desktop browser readiness for any named page class.
It is a gate plan, not proof that the platform is implemented.

## Platform Standard

- Every browser subsystem has an owner, compatibility target, performance
  budget, security/privacy review, automated tests, and user-visible fallback.
- Platform claims are measured against reproducible fixtures, standards subsets,
  visual regressions, accessibility audits, and performance reports.
- Search extraction can use platform features only behind timeouts, resource
  caps, and fallback to the static fast lane.
- Packaged browser builds must be signed, updateable, rollback-safe, and tested
  on each supported operating system.

## Subsystem Matrix

| Subsystem | Required End State | Evidence Gate |
| --- | --- | --- |
| Fonts and text shaping | Font discovery, fallback, glyph metrics, ligatures, bidi, line breaking, emoji, variable fonts | Text layout fixtures, shaping corpus, bidi tests, font fallback screenshots |
| Images and SVG | Decode common formats, sizing, lazy loading, responsive images, SVG parsing/raster, color management. Current baseline has static img replaced-element placeholder sizing/paint, a tiny local SVG rect decode path, and a minimal non-interlaced 8-bit PNG decode path including data URI images; broader PNG and JPEG pixels remain required. | Image decode tests, SVG compatibility set, screenshot baselines |
| CSS visual effects | Cascade completeness, selectors, box model, positioning, transforms, filters, animations. Current baseline has display/color/background/border parsing, text color paint, plus block padding, margin, sizing, max-width, and horizontal auto-margin layout fixtures; full cascade and standards box model remain required. | CSS standards subset, visual regression, layout stress benchmarks |
| Canvas/WebGL/WebGPU | Canvas 2D, image data, text drawing, WebGL/WebGPU strategy, GPU/resource isolation, shader/cache policy | Canvas pixel tests, graphics conformance subset, GPU crash/timeout tests |
| Media | Audio/video decode, controls, captions, autoplay policy, streaming, device policy; WebRTC/media capture is either implemented with permission/privacy tests or explicitly deferred from the claim scope | Media fixture suite, codec matrix, playback timing, permission tests |
| Accessibility | Accessibility tree, ARIA, focus order, keyboard nav, platform accessibility bridge. Current baseline has a deterministic static role/name/state snapshot for the supported DOM/CSS/tiny-script subset through `brutal-browser accessibility-tree`; focus, keyboard navigation, live regions, and platform bridge remain required. | Accessibility audits, tree snapshots, keyboard navigation fixtures |
| Input and editing | Keyboard, pointer, touch, IME, selection, clipboard, drag/drop, forms, editing | Input event fixtures, IME tests, form validation, clipboard permission tests |
| Storage and profiles | Persistent profile directory, cookies, localStorage, IndexedDB, Cache API, quota, eviction, profile isolation, private browsing, keychain/encryption integration | Storage WPT subset, quota/eviction tests, clear-data and private-mode tests |
| Downloads and files | Safe downloads, file picker, MIME handling, blocked schemes, external-app handoff, completed/partial file lifecycle | Download policy tests, file access denial tests, dangerous file warnings, handoff workflow tests |
| Devtools | Console, network log, DOM inspector, source view, timeline, protocol bridge | Devtools protocol fixtures, console/network/DOM inspection tests |
| Extensions | Extension manifest policy, permission prompts, content scripts, update policy, isolation | Extension compatibility fixtures, permission and isolation tests |
| Packaging and updates | Signed macOS/Linux/Windows packages, updater, rollback, channel policy | Signature verification, update/rollback drills, install/uninstall tests |

## Profile, Storage, And Shell Gates

- Persistent profiles must define directory layout, schema migrations, history,
  cookies, permissions, bookmarks, downloads, passwords/autofill state, Cache
  API, IndexedDB, quota accounting, eviction order, corruption recovery, and
  backup/restore behavior.
- Private browsing must use separate in-memory cookies, storage, cache,
  permissions, downloads metadata, and service worker state, with tests proving
  no private state survives profile close.
- Profile secrets must use platform keychain or documented encrypted storage;
  password and autofill features need policy tests for save prompts, fill
  eligibility, origin binding, clearing data, and import/export.
- Browser shell evidence must cover address-bar navigation, tabs, history,
  reload/stop, downloads, permission prompts, settings, crash recovery, and
  profile switching workflows. The current reusable `BrowserApp` scaffold owns
  tabs, navigation, viewport scrolling, input actions, and presentable RGBA
  frames over the supported `BrowserSession` path, and `brutal-browser app`
  can drive that state boundary with scripted actions, an interactive/stdin
  command stream, JSON cookie/localStorage files, JSON app-level visit
  history/bookmarks, find/find-next match state, visible viewport text, and PNG
  frame output plus a deterministic browser-window PNG with simple chrome and
  narrow window-coordinate hit testing. The feature-gated `brutal-browser
  window` command presents that frame in a native CPU-backed window and routes
  mouse, wheel, basic keyboard input, narrow location entry, focused-control
  text input, resize-aware viewport updates through `BrowserApp`,
  while the CLI shell can reload the current entry, resolve typed `open` / `go` targets against the
  current page source, report current source/title/history/viewport metadata
  with clamped max-scroll, visible retained-layout-box state, and cell-space
  dirty-region accounting plus deterministic RGBA frame/dirty-pixel reporting
  for the supported text viewport, inspect and
  clear the current in-memory `BrowserSession` cookie jar, optionally load/save
  that jar through a local JSON `--cookie-jar` file, optionally load/save
  origin-scoped localStorage through a local JSON `--local-storage` file,
  inspect current origin-scoped localStorage entries, inspect and clear current
  in-memory origin-scoped sessionStorage entries, list current-page links, and
  activate a resolved link by index, exact text, or anchor selector
  through `BrowserSession` navigation, and normal `click
  <selector>` can follow an anchor `href` through the same history as a narrow
  default action after supported click-handler dispatch. The current loader also
  follows a bounded HTTP(S) redirect chain for document, form, and static
  resource loads, updates the session entry target to the final URL, and carries
  cookies set by redirect responses into the next hop. The
  `browser-shell-location-command` gate is not a real address bar, omnibox,
  browser chrome, or tab UI. The
  `browser-shell-relative-open` gate is current-page-relative shell navigation
  only, not omnibox search, URL autocomplete, tab UI, security UI, or full
  browser chrome. The
  `browser-shell-cookie-inspection` gate is read-only in-memory session
  inspection, not persistent profiles, cookie settings UI, permissions,
  partitioning, or browser chrome. The
  `browser-shell-clear-cookies` gate clears only in-memory session cookies, not
  persistent profiles, storage partitions, settings UI, permissions, or browser
  chrome. The
  `browser-shell-cookie-jar-file` gate is only local JSON cookie state
  persistence around the CLI shell, not encrypted profile storage, expiration
  persistence, partitioning, cookie settings UI, or browser chrome. The
  `browser-shell-local-storage-file` gate is only local JSON localStorage state
  persistence around the CLI shell, not IndexedDB, Cache API, quota management,
  private browsing, partitioning, settings UI, or browser chrome. The
  `browser-shell-local-storage-inspection` gate is only read-only CLI session
  entry inspection, not devtools storage panels, IndexedDB, Cache API, quota
  management, partitioning, settings UI, or browser chrome. The
  `browser-shell-clear-local-storage` gate clears only current localStorage
  session state, not full site data, IndexedDB, Cache API, quota management,
  private browsing, partitioning, settings UI, or browser chrome. The
  `browser-shell-session-storage-inspection` and
  `browser-shell-clear-session-storage` gates cover only in-memory
  BrowserSession-scoped sessionStorage inspection/clearing, not persistent
  profiles, full site data, devtools storage panels, IndexedDB, Cache API,
  quota management, private browsing, partitioning, settings UI, or browser
  chrome. The
  `http-redirect-navigation` gate is not full navigation lifecycle,
  mixed-content/referrer/CORS policy, HSTS, redirect UI, browser-grade error
  pages, or broad compatibility. The
  `browser-shell-fragment-navigation` gate covers only rendered `id` and legacy
  anchor `name` target discovery plus CLI text-viewport scrolling after
  supported open/link/click navigation. The `browser-shell-coordinate-click`
  gate covers only CLI shell coordinate routing through display-list hit testing
  into that supported generated
  `pointerdown`/`mousedown`/`pointerup`/`mouseup`/click/default-action path.
  The `browser-cli-click-at-viewport-offset` gate covers only one-shot CLI
  explicit viewport-offset translation before that same supported path.
  The `browser-shell-wheel-events` gate covers only a narrow document-level
  `wheel` dispatch before local shell scroll/left/right viewport movement,
  exposing `event.deltaX`/`event.deltaY` and honoring `preventDefault()` by
  canceling that local offset. It is not browser scroll-container behavior, CSS
  overflow, compositor scrolling, precise device deltas, full WheelEvent
  semantics, or platform input integration.
  The `browser-shell-form-fill-state` gate covers only filled remembered
  text-like field values on the current `BrowserSession` entry being merged into
  later GET form submission. The `browser-session-select-form-state` and
  `browser-shell-select-form-choice` gates cover only single-select option
  metadata, enabled-option validation, focused choice commands, and remembered
  selected values for later GET/URL-encoded POST submission. The `browser-session-checkable-form-state` and
  `browser-shell-checkable-form-toggle` gates cover only checkbox/radio checked
  state, explicit toggle commands, focused-space toggles, and narrow
  selector-click/label defaults for supported checkbox/radio controls. The
  `browser-session-required-form-validation` gate covers only value-missing
  checks for supported required text-like, select, checkbox, and radio controls
  in BrowserSession/CLI submit paths, honoring form `novalidate` and submitter `formnovalidate`.
  The `browser-session-type-value-validation` gate covers only non-empty
  email/URL value checks on the same submit paths. The
  `browser-session-submitter-action-method-overrides` gate covers only
  submitter `formaction` and GET/POST `formmethod` overrides on supported
  BrowserSession/CLI submit-control click and focused-submit paths; it is not
  external form ownership, target/enctype/dialog handling, or full submitter
  semantics. The
  `browser-session-focused-form-control` and `browser-shell-focused-text-input`
  gates cover only selector focus, associated label focus/click, and typed text
  append into editable text-like controls, while the focus traversal gates cycle
  forward/backward through rendered fillable, select, checkable, and
  submit/reset action controls and the focused text edit gates add
  backward-delete and clear commands. The focused form submit gates add a local
  `enter` path that submits focused fillable/select controls, activates focused
  submit controls with submitter state, or resets focused reset controls.
  `browser-shell-find-text` covers rendered text-line search and viewport
  scrolling only. Those remain local shell
  evidence rather than product browser chrome, full browser pointer routing,
  full PointerEvent semantics, full MouseEvent semantics, full browser event/default-action and event-cancellation
  semantics, full interactive form state, full label activation semantics,
  full constraint validation, validation UI, CSS scroll behavior, `:target`
  styling, history scroll restoration, keyboard events, caret positioning, DOM
  tab order, tabindex, selection, IME, autofill, or browser UI.
- The `browser-session-reload` and `browser-shell-reload` gates cover only
  replacing the current `BrowserSession` entry from its target. They are not
  full reload lifecycle, cache policy, POST replay UI, service worker handling,
  BFCache policy, browser UI, or broad navigation compatibility.
- The `browser-session-urlencoded-post-form-submit` coverage id covers only
  narrow `application/x-www-form-urlencoded` POST submission through
  `BrowserSession` and the local CLI shell. It is not multipart forms, file
  upload, fetch/XHR, validation, full navigation lifecycle, browser UI, or a
  broad browser compatibility claim.
- The `browser-session-form-submit-button-click-default` coverage id covers only
  supported submit/input/button click default actions routed through
  `BrowserSession` and the local CLI shell for GET or URL-encoded POST forms. It
  is not form event dispatch, validation, focus/input behavior, browser UI, or
  broad input/form compatibility.
- The `browser-session-form-reset-click-default` coverage id covers only
  supported reset-control click default actions routed through `BrowserSession`
  and the local CLI shell. It is not form event dispatch, validation, focus/input
  behavior, browser UI, or broad input/form compatibility.

## Compatibility Gates

- Web Platform Test gate by subsystem with tracked pass rate and allowed
  failures.
- Visual regression gate for curated real-world pages at desktop and mobile
  viewport sizes.
- Screenshot pixel gate for layout, images, SVG, canvas, fonts, and form
  controls.
- Accessibility gate with tree snapshots, keyboard navigation, and screen-reader
  bridge checks.
- Devtools gate with console, network, DOM, and source inspection fixtures.
- Extension gate with permission prompts, content script isolation, and update
  policy fixtures.

## Performance Gates

- Startup time, first text render, full layout, paint/raster, compositing frame
  time, input latency, memory high-water mark, and cache hit rate.
- Font shaping and bidi performance over representative multilingual corpora.
- Image decode/raster benchmarks for common formats and large images.
- Canvas/media/GPU timeout and resource-budget reports.
- Browser-shell responsiveness under tabs, history, downloads, and devtools.

## Security And Privacy Gates

- Platform features follow the security plan: origin checks, permissions,
  sandbox boundaries, and storage partitioning.
- Canvas, media, font, and graphics APIs have fingerprinting and resource-abuse
  reviews.
- Downloads, file access, clipboard, notifications, camera, microphone, and
  external-app handoff require explicit policy and permission tests.
- Extensions cannot bypass renderer isolation, storage partitioning, or
  permission grants.

## Packaging Gates

- macOS signed app bundle and CLI package with reproducible build metadata.
- Linux package targets with sandbox profile and desktop integration.
- Windows package target with installer, updater, sandbox policy, and code
  signing.
- Update channels: dev, beta, stable, emergency rollback, and disabled-update
  policy for managed environments.
- Install, update, rollback, uninstall, profile migration, and crash-report
  opt-in tests for each platform.

## Search Integration Gates

- JS-rendered extraction can request platform features only through isolated
  render workers with budgets and deterministic snapshots.
- Static fast lane remains available when platform render times out or fails.
- Rendered extraction records include required platform features, budgets used,
  final URL, content hash, and timeout/failure reason.
- Search ranking can distinguish static text, rendered text, media captions,
  image alt text, and accessibility names.

## Implementation Sequence

1. Define platform feature IDs in browser coverage for fonts, images, SVG,
   canvas, media, accessibility, input, storage, devtools, extensions,
   packaging, and updates.
2. Add fixtures and visual regression harness for images, box-model layout,
   fonts, forms, accessibility tree snapshots, canvas pixels, and screenshots.
3. Implement font discovery/fallback and text shaping strategy.
4. Extend the current img placeholder, cached tiny SVG decode fixtures, and
   minimal PNG decoder into common image decode/raster and broader SVG subset
   screenshot gates.
5. Implement accessibility tree and keyboard/focus model.
6. Implement storage/profile model with private browsing and clear-data tests.
7. Implement Canvas 2D and choose WebGL/WebGPU strategy.
8. Implement media pipeline strategy and permission/autoplay policy.
9. Implement devtools protocol and extension policy.
10. Implement signed packages, updater, rollback, and platform QA.
11. Promote `Platform Completeness` in `brutal-bench readiness` only when the
    gates above have direct passing evidence.
