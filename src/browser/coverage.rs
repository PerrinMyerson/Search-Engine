use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BrowserFeatureState {
    Implemented,
    Partial,
    Missing,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BrowserFeatureCoverage {
    pub id: String,
    pub category: String,
    pub status: BrowserFeatureState,
    pub evidence: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrowserCoverageReport {
    pub feature_count: usize,
    pub implemented_count: usize,
    pub partial_count: usize,
    pub missing_count: usize,
    pub implemented_ratio: f64,
    pub required_features: Vec<String>,
    pub missing_required_features: Vec<String>,
    pub min_implemented_ratio: Option<f64>,
    pub max_missing_features: Option<usize>,
    pub passed: Option<bool>,
    pub features: Vec<BrowserFeatureCoverage>,
}

#[derive(Debug, Clone, Default)]
pub struct BrowserCoverageGate {
    pub required_features: Vec<String>,
    pub min_implemented_ratio: Option<f64>,
    pub max_missing_features: Option<usize>,
}

impl BrowserCoverageGate {
    pub fn is_empty(&self) -> bool {
        self.required_features.is_empty()
            && self.min_implemented_ratio.is_none()
            && self.max_missing_features.is_none()
    }
}

impl BrowserCoverageReport {
    pub fn apply_gate(&mut self, gate: BrowserCoverageGate) -> bool {
        let missing_required_features = gate
            .required_features
            .iter()
            .filter(|required| !self.is_implemented(required))
            .cloned()
            .collect::<Vec<_>>();
        let passed = missing_required_features.is_empty()
            && gate
                .min_implemented_ratio
                .is_none_or(|min| self.implemented_ratio >= min)
            && gate
                .max_missing_features
                .is_none_or(|max| self.missing_count <= max);
        self.required_features = gate.required_features;
        self.missing_required_features = missing_required_features;
        self.min_implemented_ratio = gate.min_implemented_ratio;
        self.max_missing_features = gate.max_missing_features;
        self.passed = Some(passed);
        passed
    }

    fn is_implemented(&self, required: &str) -> bool {
        self.features.iter().any(|feature| {
            feature.id == required && feature.status == BrowserFeatureState::Implemented
        })
    }
}

pub fn unsupported_feature_summary() -> &'static [&'static str] {
    &[
        "javascript",
        "web-apis",
        "css-cascade-complete",
        "form-validation",
        "full-form-event-default-action-ordering",
        "post-form-submission",
        "font-shaping",
        "paint-raster",
        "compositor",
        "navigation-complete",
        "sandbox",
        "accessibility-platform-bridge",
        "persistent-cookies",
        "storage",
        "media",
        "canvas",
    ]
}

pub fn browser_coverage_report() -> BrowserCoverageReport {
    let features = browser_feature_catalog();
    let implemented_count = features
        .iter()
        .filter(|feature| feature.status == BrowserFeatureState::Implemented)
        .count();
    let partial_count = features
        .iter()
        .filter(|feature| feature.status == BrowserFeatureState::Partial)
        .count();
    let missing_count = features
        .iter()
        .filter(|feature| feature.status == BrowserFeatureState::Missing)
        .count();
    let feature_count = features.len();
    let implemented_ratio = if feature_count == 0 {
        0.0
    } else {
        implemented_count as f64 / feature_count as f64
    };

    BrowserCoverageReport {
        feature_count,
        implemented_count,
        partial_count,
        missing_count,
        implemented_ratio,
        required_features: Vec::new(),
        missing_required_features: Vec::new(),
        min_implemented_ratio: None,
        max_missing_features: None,
        passed: None,
        features,
    }
}

fn browser_feature_catalog() -> Vec<BrowserFeatureCoverage> {
    use BrowserFeatureState::{Implemented, Missing, Partial};

    vec![
        feature(
            "static-html-parse",
            "html",
            Implemented,
            "Tokenizes static HTML and builds a DOM-like tree.",
        ),
        feature(
            "dom-tree",
            "dom",
            Implemented,
            "Stores element/text/comment nodes with attributes and child links.",
        ),
        feature(
            "script-skip",
            "html",
            Implemented,
            "Excludes script/template/style text from terminal render text.",
        ),
        feature(
            "inline-script-dom-text",
            "runtime",
            Implemented,
            "Executes a tiny inline JavaScript subset for document title and DOM text mutations.",
        ),
        feature(
            "inline-script-dom-create",
            "runtime",
            Implemented,
            "Executes a tiny inline JavaScript subset for createElement, createTextNode, and appendChild.",
        ),
        feature(
            "dom-tree-mutation",
            "runtime",
            Implemented,
            "Applies a tiny DOM tree mutation subset for appendChild, insertBefore, replaceChild, removeChild, element.remove, and parentNode lookups.",
        ),
        feature(
            "dom-node-traversal",
            "runtime",
            Implemented,
            "Resolves a tiny DOM traversal subset for children, childNodes, first/last child, first/last element child, sibling navigation, childElementCount, and node metadata reads.",
        ),
        feature(
            "dom-insertion-methods",
            "runtime",
            Implemented,
            "Applies a tiny DOM insertion convenience subset for append, prepend, before, after, replaceWith, replaceChildren, and string-to-text-node insertion.",
        ),
        feature(
            "document-fragment",
            "runtime",
            Implemented,
            "Creates tiny DocumentFragment nodes and splices their children into appendChild, append, insertBefore, replaceChild, replaceChildren, and sibling insertion targets.",
        ),
        feature(
            "dom-selector-methods",
            "runtime",
            Implemented,
            "Evaluates a tiny Element.matches and Element.closest subset through the shared compound/attribute selector matcher.",
        ),
        feature(
            "dom-inner-html-mutation",
            "runtime",
            Implemented,
            "Parses tiny innerHTML assignment and append strings into DOM children, supports document.head lookup, and serializes simple child markup readback.",
        ),
        feature(
            "dom-form-control-properties",
            "runtime",
            Implemented,
            "Reflects tiny form-control DOM properties for value, name, type, action, method, checked, disabled, hidden, and selected into rendered DOM and extracted form state.",
        ),
        feature(
            "dom-location-readback",
            "runtime",
            Implemented,
            "Reflects a tiny location readback API for location/window.location/document.location, document URL fields, and common URL components into scripted DOM text.",
        ),
        feature(
            "dom-set-attribute",
            "runtime",
            Implemented,
            "Applies tiny element.setAttribute mutations to DOM attributes used by layout, links, resources, and forms.",
        ),
        feature(
            "dom-get-attribute",
            "runtime",
            Implemented,
            "Evaluates tiny element.getAttribute reads and string bindings for DOM text and attribute mutations.",
        ),
        feature(
            "dom-style-property-mutation",
            "runtime",
            Implemented,
            "Applies a tiny CSSStyleDeclaration subset for element.style property assignment, setProperty/getPropertyValue, and removeProperty over supported inline CSS declarations.",
        ),
        feature(
            "dom-class-list-mutation",
            "runtime",
            Implemented,
            "Applies a tiny DOMTokenList subset for element.classList add/remove/toggle/contains/length over supported class selectors.",
        ),
        feature(
            "dom-query-collections",
            "runtime",
            Implemented,
            "Resolves a tiny querySelectorAll/getElementsByClassName/getElementsByTagName collection subset with length, item(index), and indexed node access.",
        ),
        feature(
            "local-storage-api",
            "storage",
            Implemented,
            "Executes a tiny origin-scoped localStorage setItem/getItem/removeItem/clear/length subset.",
        ),
        feature(
            "session-storage-api",
            "storage",
            Implemented,
            "Executes a tiny BrowserSession-scoped sessionStorage setItem/getItem/removeItem/clear/length subset, partitioned by origin and kept in memory for the current session only.",
        ),
        feature(
            "timer-task-queue",
            "runtime",
            Implemented,
            "Queues and drains a deterministic setTimeout/clearTimeout callback subset for render and click tasks.",
        ),
        feature(
            "document-lifecycle-events",
            "events",
            Implemented,
            "Dispatches tiny document DOMContentLoaded and window load listener callbacks after initial scripts, before draining timer tasks.",
        ),
        feature(
            "link-extraction",
            "navigation",
            Implemented,
            "Extracts and resolves anchor href targets.",
        ),
        feature(
            "form-extraction",
            "forms",
            Implemented,
            "Extracts static controls, values, checked state, and disabled state.",
        ),
        feature(
            "get-form-url",
            "forms",
            Implemented,
            "Constructs GET submission URLs with field overrides.",
        ),
        feature(
            "get-form-submit-navigation",
            "forms",
            Implemented,
            "Submits GET forms through browser session navigation.",
        ),
        feature(
            "browser-session-focused-form-control",
            "forms",
            Implemented,
            "Tracks a focused supported form control by CSS selector, control click, or associated-label focus/click within BrowserSession; typed text/editing remains limited to editable text-like controls.",
        ),
        feature(
            "browser-session-focus-traversal",
            "forms",
            Implemented,
            "Cycles BrowserSession focus forward and backward through named enabled fillable, select, checkable, and submit/reset action controls in rendered form order; this is not full DOM tab order, tabindex, inert/hidden focus policy, keyboard events, or platform focus handling.",
        ),
        feature(
            "browser-session-focus-events",
            "forms",
            Implemented,
            "Dispatches narrow addEventListener focus/blur listeners on the focused control, bubbling focusin/focusout listeners through ancestors, and exposes document.activeElement for BrowserSession-supported form focus transitions; this is not relatedTarget, tabindex, inert/hidden focus policy, keyboard focus navigation, or platform focus integration.",
        ),
        feature(
            "browser-shell-focused-text-input",
            "platform",
            Implemented,
            "Adds local CLI browse-shell focus <selector> and type <text> commands over BrowserSession focused form state, including associated-label focus; this is not full keyboard events, IME, selection, validation, or OS browser UI.",
        ),
        feature(
            "browser-shell-focus-traversal",
            "platform",
            Implemented,
            "Adds local CLI browse-shell tab/focus-next and shift-tab/focus-prev commands over BrowserSession focus traversal for supported fillable, select, checkable, and submit/reset action controls; this is not full browser keyboard focus navigation, tabindex, accessibility focus, or browser chrome input handling.",
        ),
        feature(
            "browser-session-focused-text-edit",
            "forms",
            Implemented,
            "Edits the currently focused fillable form control through BrowserSession append, Unicode-safe backward deletion, and clear operations over remembered form state; this is not caret positioning, selection ranges, keyboard events, IME, undo, or validation.",
        ),
        feature(
            "browser-shell-focused-text-edit",
            "platform",
            Implemented,
            "Adds local CLI browse-shell backspace [count] and clear-input commands over BrowserSession focused text editing; this is not full keyboard event dispatch, selection, IME, or browser chrome input handling.",
        ),
        feature(
            "browser-session-focused-form-submit",
            "forms",
            Implemented,
            "Runs the narrow BrowserSession enter path for the currently focused form control: submits focused fillable/select controls, activates focused submit controls with submitter name/value, and resets focused reset controls; this is not full Enter-key event dispatch, constraint validation, submit/reset event ordering, or browser UI.",
        ),
        feature(
            "browser-shell-enter-submit",
            "platform",
            Implemented,
            "Adds local CLI browse-shell enter/submit-focused commands over BrowserSession focused-form submission and focused submit/reset activation; this is not full keyboard handling, IME, form validation, or browser chrome input routing.",
        ),
        feature(
            "subresource-discovery",
            "loading",
            Implemented,
            "Discovers static scripts, images, media, frames, embeds, objects, and link resources.",
        ),
        feature(
            "subresource-fetch-cache",
            "loading",
            Implemented,
            "Fetches discovered static resources and reports cache hits.",
        ),
        feature(
            "image-replaced-element",
            "layout",
            Implemented,
            "Lays out static img elements as deterministic replaced-element boxes from width/height attributes.",
        ),
        feature(
            "responsive-image-selection",
            "images",
            Implemented,
            "Selects deterministic static img srcset and picture/source candidates before image decode and raster.",
        ),
        feature(
            "network-image-render",
            "images",
            Implemented,
            "Fetches session image resources, decodes cached SVG/PNG bytes, and rerenders image display commands with cached pixels.",
        ),
        feature(
            "svg-image-decode",
            "images",
            Implemented,
            "Decodes a tiny local SVG rect subset into deterministic grayscale image pixels.",
        ),
        feature(
            "png-image-decode",
            "images",
            Implemented,
            "Decodes a minimal non-interlaced 8-bit PNG subset with PNG row filters into deterministic grayscale image pixels.",
        ),
        feature(
            "data-url-image-decode",
            "images",
            Implemented,
            "Decodes data: URL image payloads for the supported SVG and PNG image subsets.",
        ),
        feature(
            "image-decode-cache",
            "images",
            Implemented,
            "Carries decoded local image pixels from layout into rasterization without re-reading the source file.",
        ),
        feature(
            "external-stylesheet-render",
            "css",
            Implemented,
            "Fetches external stylesheets and applies simple display rules to text layout.",
        ),
        feature(
            "external-script-render",
            "runtime",
            Implemented,
            "Fetches external script resources and executes the tiny DOM mutation subset before layout.",
        ),
        feature(
            "inline-onclick-event",
            "events",
            Implemented,
            "Dispatches a tiny click event path for inline onclick DOM mutations.",
        ),
        feature(
            "event-listener-click",
            "events",
            Implemented,
            "Registers and dispatches tiny addEventListener('click', ...) handlers.",
        ),
        feature(
            "complex-query-selector",
            "dom",
            Implemented,
            "Resolves document.querySelector through the shared compound/attribute selector matcher.",
        ),
        feature(
            "complex-click-selector",
            "events",
            Implemented,
            "Targets click dispatch with the shared compound/attribute selector matcher.",
        ),
        feature(
            "simple-css-display",
            "css",
            Implemented,
            "Parses inline and linked display:none rules for static selector matches.",
        ),
        feature(
            "css-background-color",
            "css",
            Implemented,
            "Parses a tiny background/background-color subset into deterministic grayscale paint values.",
        ),
        feature(
            "css-color-property",
            "css",
            Implemented,
            "Parses a tiny CSS color property subset into inherited text paint values.",
        ),
        feature(
            "css-text-align",
            "css",
            Implemented,
            "Parses a tiny text-align subset for start/left, center, and end/right and offsets block text in the deterministic display-list layout path.",
        ),
        feature(
            "css-border-shorthand",
            "css",
            Implemented,
            "Parses a tiny border shorthand and border longhand subset into deterministic paint values.",
        ),
        feature(
            "css-padding-shorthand",
            "css",
            Implemented,
            "Parses a tiny padding shorthand and longhand subset into block box spacing.",
        ),
        feature(
            "css-margin-shorthand",
            "css",
            Implemented,
            "Parses a tiny margin shorthand and longhand subset into block box spacing.",
        ),
        feature(
            "css-size-properties",
            "css",
            Implemented,
            "Parses a tiny width, height, and min-height subset into block sizing constraints.",
        ),
        feature(
            "css-max-width-auto-margin-layout",
            "css",
            Implemented,
            "Parses a tiny max-width/max-inline-size subset plus horizontal margin auto for block boxes, constraining and centering deterministic document columns; this is not full CSS sizing, percentages, flex/grid alignment, or browser-accurate layout.",
        ),
        feature(
            "compound-css-selectors",
            "css",
            Implemented,
            "Matches performant compound, descendant, and child selectors for display-oriented CSS.",
        ),
        feature(
            "attribute-css-selectors",
            "css",
            Implemented,
            "Matches attribute existence and exact-value selectors through the indexed CSS cascade.",
        ),
        feature(
            "hidden-attribute",
            "html",
            Implemented,
            "Suppresses hidden elements in the visible text/layout tree.",
        ),
        feature(
            "block-text-layout",
            "layout",
            Implemented,
            "Runs deterministic block text wrapping for terminal-width output.",
        ),
        feature(
            "list-marker-layout",
            "layout",
            Implemented,
            "Renders unordered list bullets plus ordered list numeric markers, including ol start/reversed and li value attributes, in the deterministic document layout path.",
        ),
        feature(
            "nested-list-indent-layout",
            "layout",
            Implemented,
            "Indents nested ol/ul containers in the deterministic document layout path so child list markers render at deeper x offsets.",
        ),
        feature(
            "preformatted-text-layout",
            "layout",
            Implemented,
            "Preserves spaces, explicit newlines, and blank lines for pre elements and a tiny white-space: pre/pre-wrap CSS subset in the deterministic document layout path.",
        ),
        feature(
            "simple-table-row-layout",
            "layout",
            Implemented,
            "Flows simple table header/data cells across each tr row in the deterministic document layout path; this is not rowspan/colspan or full CSS table layout.",
        ),
        feature(
            "simple-table-column-layout",
            "layout",
            Implemented,
            "Precomputes simple per-column text widths and pads table cells so deterministic document table rows remain readable; this is not rowspan/colspan or full CSS table layout.",
        ),
        feature(
            "prose-indent-layout",
            "layout",
            Implemented,
            "Applies small UA-style default indentation for blockquote and definition-description elements in the deterministic document layout path.",
        ),
        feature(
            "inline-form-control-layout",
            "layout",
            Implemented,
            "Renders common input/select/textarea controls as deterministic inline widgets with hit targets in the document layout path.",
        ),
        feature(
            "display-list",
            "rendering",
            Implemented,
            "Emits deterministic text display-list commands.",
        ),
        feature(
            "display-list-hit-testing",
            "rendering",
            Implemented,
            "Reports the topmost display-list command under a point in local cell coordinates.",
        ),
        feature(
            "layer-tree-snapshot",
            "rendering",
            Implemented,
            "Builds a deterministic local layer-tree/debug snapshot from display-list commands with image layer promotion.",
        ),
        feature(
            "retained-layout-tree",
            "layout",
            Implemented,
            "Builds a deterministic retained layout-tree/debug snapshot from supported paint-backed element boxes, including parent/child links and bounds; this is not full CSS layout, anonymous boxes, inline fragmentation, scrolling boxes, or browser-accurate layout geometry.",
        ),
        feature(
            "text-color-paint",
            "rendering",
            Implemented,
            "Emits styled text display-list commands for non-default CSS text colors.",
        ),
        feature(
            "rect-paint-command",
            "rendering",
            Implemented,
            "Emits non-text rectangle paint commands for horizontal-rule display-list primitives.",
        ),
        feature(
            "block-background-paint",
            "rendering",
            Implemented,
            "Emits block background rectangles as underlay display-list paint commands.",
        ),
        feature(
            "block-border-paint",
            "rendering",
            Implemented,
            "Reserves block border space and emits border rectangles into the display list.",
        ),
        feature(
            "block-padding-layout",
            "layout",
            Implemented,
            "Applies block padding to text/image layout and block background extents.",
        ),
        feature(
            "block-margin-layout",
            "layout",
            Implemented,
            "Applies block margin to outer block layout spacing and box widths.",
        ),
        feature(
            "block-size-layout",
            "layout",
            Implemented,
            "Applies explicit block width and minimum height to text wrapping and paint extents.",
        ),
        feature(
            "image-placeholder-paint",
            "rendering",
            Implemented,
            "Emits deterministic image placeholder display-list commands for static img elements.",
        ),
        feature(
            "cpu-text-raster",
            "rendering",
            Implemented,
            "Rasterizes the text display list into a deterministic grayscale pixel buffer with stable screenshot hashes.",
        ),
        feature(
            "cpu-rect-raster",
            "rendering",
            Implemented,
            "Rasterizes rectangle display-list commands into deterministic grayscale pixels.",
        ),
        feature(
            "cpu-image-placeholder-raster",
            "rendering",
            Implemented,
            "Rasterizes image placeholder display-list commands into deterministic grayscale pixels.",
        ),
        feature(
            "cpu-decoded-image-raster",
            "rendering",
            Implemented,
            "Rasterizes decoded SVG and PNG image pixels through the deterministic CPU raster path.",
        ),
        feature(
            "rgba-screenshot-artifact",
            "rendering",
            Implemented,
            "Converts the deterministic CPU raster into RGBA8 pixels and writes PNG screenshot artifacts for Stage 1 visual evidence.",
        ),
        feature(
            "viewport-raster-culling",
            "rendering",
            Implemented,
            "Culls display-list commands and clips text, rect, and image raster work to an explicit whole-document CPU raster viewport offset and size; this is not JS/CSS layout scrolling, compositor tiling, async scrolling, or browser-accurate scroll containers.",
        ),
        feature(
            "browser-viewport-layout-state",
            "rendering",
            Implemented,
            "Reports clamped whole-document viewport offsets, max scroll extents, and visible retained layout boxes for the supported text viewport/shell path; this is not CSS overflow, scroll containers, scroll anchoring, async compositor scrolling, or browser-accurate viewport semantics.",
        ),
        feature(
            "browser-viewport-invalidation",
            "rendering",
            Implemented,
            "Reports deterministic viewport dirty regions, invalidated area, and reused area for supported whole-document scroll updates so future shells can avoid full repaint accounting; this is not a compositor damage tracker, scroll-container invalidation, CSS clip/transform damage, or GPU tile invalidation.",
        ),
        feature(
            "browser-viewport-frame-surface",
            "rendering",
            Implemented,
            "Combines clamped viewport state, deterministic RGBA frame pixels, and mapped dirty pixel rectangles for the supported CPU presentation path; this is not an OS window surface, GPU swapchain, compositor damage tracker, or browser-accurate scrolling model.",
        ),
        feature(
            "browser-app-state-surface",
            "platform",
            Implemented,
            "Provides a reusable Rust BrowserApp state model for tabs, navigation, viewport scrolling, input actions, and presentable RGBA viewport frames; this is not a native OS window, GUI browser chrome, process isolation, or full browser platform lifecycle.",
        ),
        feature(
            "browser-app-cli-surface",
            "platform",
            Implemented,
            "Adds a brutal-browser app command over BrowserApp for scripted tab/navigation/input actions, JSON cookie/localStorage profile files, and PNG frame output; this is still a deterministic CLI-driven app surface, not native OS windowing, browser chrome widgets, encrypted profile storage, or a GPU compositor.",
        ),
        feature(
            "browser-app-interactive-shell",
            "platform",
            Implemented,
            "Adds an interactive/stdin command stream over BrowserApp that keeps tab, history, profile, viewport, and frame-output state alive across commands; this is still terminal-driven control, not native browser chrome, OS windowing, or a GUI event loop.",
        ),
        feature(
            "browser-app-visible-viewport",
            "platform",
            Implemented,
            "Reports and prints the active BrowserApp visible text viewport alongside the RGBA frame contract so terminal/stdin app driving can inspect page content; this is not native text selection, accessibility output, or browser chrome rendering.",
        ),
        feature(
            "browser-app-profile-history-bookmarks",
            "platform",
            Implemented,
            "Adds a JSON BrowserApp profile file for persistent app-level visit history and bookmarks plus terminal commands to add/list/open/remove bookmarks and list/open profile history; this is not an encrypted profile directory, sync, omnibox autocomplete, or browser-grade history/bookmark UI.",
        ),
        feature(
            "browser-app-find-text",
            "platform",
            Implemented,
            "Adds BrowserApp-level find/find-next state with match counts, active match position, viewport scrolling, JSON report output, and terminal command wiring; this is not full browser find UI, highlight painting, selection, locale-aware search, or native text ranges.",
        ),
        feature(
            "browser-app-window-frame",
            "platform",
            Implemented,
            "Composes BrowserApp tab/title/location/status chrome with the presentable page viewport into one deterministic RGBA browser-window frame artifact for future native shells; this is not OS windowing, native widgets, GPU presentation, or product browser chrome.",
        ),
        feature(
            "browser-app-window-hit-testing",
            "platform",
            Implemented,
            "Adds BrowserApp window-coordinate hit testing and click routing for simple chrome controls, tabs, and page viewport clicks so a future native window backend can dispatch mouse input through the same state boundary; this is not full browser chrome UI, text selection, drag/drop, context menus, or platform event-loop integration.",
        ),
        feature(
            "browser-native-window-shell",
            "platform",
            Implemented,
            "Adds a feature-gated minifb native window shell that presents BrowserApp RGBA window frames and routes mouse, wheel, and basic keyboard input back through BrowserApp; build with the native-window Cargo feature. This is not product-grade browser chrome, GPU compositing, text input/selection, menus, downloads, settings, or process isolation.",
        ),
        feature(
            "browser-native-window-location-input",
            "platform",
            Implemented,
            "Adds feature-gated native-window location entry over the BrowserApp state boundary, including live location-bar chrome text, Enter-to-open, Escape cancel, Backspace edit, Ctrl/Cmd+L focus, resize-aware viewport updates, and printable-key routing into focused text controls. This is not a full omnibox, autocomplete, search-provider integration, IME, text selection, clipboard, or product browser chrome.",
        ),
        feature(
            "visual-baseline-runner",
            "testing",
            Implemented,
            "Verifies fixture raster baselines and writes deterministic PGM screenshot artifacts for visual regression gates.",
        ),
        feature(
            "visual-pixel-diff",
            "testing",
            Implemented,
            "Compares actual and baseline PGM screenshots with pixel-count and ratio thresholds plus diff artifacts.",
        ),
        feature(
            "stage1-document-page-corpus",
            "testing",
            Implemented,
            "Checks a small Stage 1 document-page corpus with expected visible text and RGBA screenshot hashes.",
        ),
        feature(
            "terminal-text-render",
            "rendering",
            Implemented,
            "Renders visible static text to terminal/plain output.",
        ),
        feature(
            "static-accessibility-tree",
            "accessibility",
            Implemented,
            "Builds a deterministic static accessibility snapshot with document, role, name, heading level, value, checked, disabled, and parent/child relationships for the supported DOM/CSS/tiny-script subset.",
        ),
        feature(
            "session-history",
            "navigation",
            Implemented,
            "Tracks back/forward session history for static navigations.",
        ),
        feature(
            "browser-session-reload",
            "navigation",
            Implemented,
            "Reloads the current BrowserSession entry from its target without pushing a new history entry, clearing transient form focus and filled state.",
        ),
        feature(
            "browser-shell-cli",
            "platform",
            Implemented,
            "Provides a local CLI browser shell over the existing static BrowserSession for open/back/forward/links/link/click/scroll/render-style interactions around the supported static layout/display-list/text viewport subset; this is not a GUI browser shell, OS windowing, browser chrome, tab/process isolation, accessibility, devtools, or Chromium parity.",
        ),
        feature(
            "browser-shell-visual-frame",
            "platform",
            Implemented,
            "Reports the current browse shell viewport as RGBA frame metadata in JSON and can write the final visible viewport as a PNG artifact; this is still a deterministic CPU-raster shell path, not a GPU compositor or OS window.",
        ),
        feature(
            "browser-shell-tabs",
            "platform",
            Implemented,
            "Adds local CLI browse-shell tab listing, new-tab, switch-tab, and close-tab commands over multiple BrowserSession instances; this is not GUI tab chrome, shared profile synchronization, process isolation, restore, or browser-grade tab lifecycle.",
        ),
        feature(
            "browser-shell-relative-open",
            "platform",
            Implemented,
            "Resolves local CLI browse-shell open/go targets against the current page source before navigating, so relative paths, query strings, and fragments behave like a narrow address-bar navigation path; this is not omnibox search, URL autocomplete, tab UI, security UI, or full browser chrome.",
        ),
        feature(
            "browser-shell-location-command",
            "platform",
            Implemented,
            "Adds local CLI browse-shell location/url/where commands that report the current source, title, history position, and text viewport without changing page state; this is not a real address bar, omnibox, browser chrome, or tab UI.",
        ),
        feature(
            "browser-shell-cookie-inspection",
            "platform",
            Implemented,
            "Adds local CLI browse-shell cookies/cookie commands that print the current in-memory BrowserSession cookie jar without changing page state; this is not persistent profile storage, cookie settings UI, permissions, partitioning, or browser chrome.",
        ),
        feature(
            "browser-shell-clear-cookies",
            "platform",
            Implemented,
            "Adds local CLI browse-shell clear-cookies/clear-cookie-jar commands that clear the current in-memory BrowserSession cookie jar without navigating or changing page state; this is not persistent profile clearing, storage partition clearing, permissions UI, settings UI, or browser chrome.",
        ),
        feature(
            "browser-shell-cookie-jar-file",
            "storage",
            Implemented,
            "Adds a local CLI browse-shell --cookie-jar JSON file that loads cookies before navigation and saves the current in-memory cookie jar after the shell run; this is not encrypted profile storage, cookie expiration persistence, partitioning, permissions UI, or browser chrome.",
        ),
        feature(
            "browser-shell-local-storage-file",
            "storage",
            Implemented,
            "Adds a local CLI browse-shell --local-storage JSON file that loads origin-scoped localStorage before navigation and saves it after the shell run; this is not IndexedDB, Cache API, quota management, private browsing, partitioning, or browser chrome.",
        ),
        feature(
            "browser-shell-local-storage-inspection",
            "storage",
            Implemented,
            "Adds local CLI browse-shell local-storage/storage/localstorage commands that print current origin-scoped localStorage session entries without changing page state; this is not devtools storage panels, IndexedDB, Cache API, quota management, partitioning, or browser chrome.",
        ),
        feature(
            "browser-shell-session-storage-inspection",
            "storage",
            Implemented,
            "Adds local CLI browse-shell session-storage/sessionstorage commands that print current in-memory origin-scoped sessionStorage entries without changing page state; this is not persistent profile storage, devtools storage panels, IndexedDB, Cache API, quota management, partitioning, or browser chrome.",
        ),
        feature(
            "browser-shell-clear-local-storage",
            "storage",
            Implemented,
            "Adds local CLI browse-shell clear-local-storage/clear-storage commands that clear current origin-scoped localStorage session state without navigating; this is not full site-data clearing, IndexedDB, Cache API, quota management, private browsing, partitioning, or browser chrome.",
        ),
        feature(
            "browser-shell-clear-session-storage",
            "storage",
            Implemented,
            "Adds a local CLI browse-shell clear-session-storage command that clears current in-memory origin-scoped sessionStorage state without navigating; this is not full site-data clearing, persistent profiles, IndexedDB, Cache API, quota management, private browsing, partitioning, or browser chrome.",
        ),
        feature(
            "browser-shell-link-activation",
            "navigation",
            Implemented,
            "Lists extracted anchors in the local CLI browser shell and navigates link targets by zero-based index, exact text, or anchor selector through the same BrowserSession history using resolved href targets; this is not full browser click/default-action semantics, pointer routing, event cancellation semantics, or SPA navigation.",
        ),
        feature(
            "browser-shell-reload",
            "navigation",
            Implemented,
            "Adds local CLI browse-shell reload/refresh commands over BrowserSession reload for the current entry; this is not full browser reload lifecycle, cache policy, POST replay UI, service worker handling, or BFCache policy.",
        ),
        feature(
            "browser-shell-anchor-click-default",
            "navigation",
            Implemented,
            "Lets normal local CLI shell click <selector> dispatch supported click handlers, honor the tiny return-false/preventDefault cancellation subset, and navigate an anchor href through BrowserSession history as a narrow default action; this is not full pointer routing, button/form defaults, SPA navigation, or browser event/default-action ordering.",
        ),
        feature(
            "browser-session-live-page-state",
            "runtime",
            Implemented,
            "Keeps a live DOM/CSS/runtime state per BrowserSession history entry so repeated supported clicks and event listeners accumulate on the current page instead of reparsing original HTML; this is not a complete browser event loop, BFCache, navigation lifecycle, or full JS/Web API runtime.",
        ),
        feature(
            "browser-shell-fragment-navigation",
            "navigation",
            Implemented,
            "Records rendered id and legacy anchor-name fragment targets and scrolls the local CLI text viewport to the matching target after supported open/link/click/default navigation; this is not full CSS scroll behavior, :target styling, history scroll restoration, or browser UI.",
        ),
        feature(
            "browser-shell-coordinate-click",
            "platform",
            Implemented,
            "Routes local CLI shell coordinate clicks through display-list hit testing into the supported pointerdown/mousedown/pointerup/mouseup/click-handler/default-action navigation path; this is terminal-cell scaffold evidence, not full browser pointer routing, text selection, transformed/scrolling hit testing, or complete event/default-action semantics.",
        ),
        feature(
            "browser-cli-click-at-viewport-offset",
            "platform",
            Implemented,
            "Applies explicit --viewport-x/--viewport-y offsets to one-shot brutal-browser click-at/tap coordinates before display-list hit testing and supported default actions; this is scripted viewport-coordinate routing only, not browser-accurate scrolling, transformed hit testing, or platform pointer integration.",
        ),
        feature(
            "browser-shell-wheel-events",
            "platform",
            Implemented,
            "Dispatches a narrow bubbling wheel event on the document before local CLI shell scroll/left/right viewport movement, exposes event.deltaX/event.deltaY, rerenders supported DOM mutations, and honors preventDefault by canceling the shell viewport offset; this is not full WheelEvent semantics, precise device deltas, scroll containers, CSS overflow, compositor scrolling, or platform input integration.",
        ),
        feature(
            "browser-shell-find-text",
            "platform",
            Implemented,
            "Adds local CLI browse-shell find/find-next commands that search rendered text viewport lines and scroll to the matching document line; this is not full browser find UI, highlight painting, selection, match count reporting, or locale-aware search.",
        ),
        feature(
            "browser-shell-form-fill-state",
            "forms",
            Implemented,
            "Remembers filled text-like form field values on the current BrowserSession entry and merges those values into later GET form submission navigation for the local CLI shell/session path; select controls use the separate narrow select-state gates. This is not full interactive form state, validation, focus/input events, autofill, POST, or browser UI.",
        ),
        feature(
            "browser-session-select-form-state",
            "forms",
            Implemented,
            "Extracts select option metadata, validates enabled option values, remembers the selected option in BrowserSession entries, and submits that value for GET/URL-encoded POST forms; this is not multi-select, optgroup inheritance, input/change events, validation, popup UI, or browser chrome.",
        ),
        feature(
            "browser-shell-select-form-choice",
            "forms",
            Implemented,
            "Adds local CLI browse-shell choose <value> and select <form> <control> <value> commands over BrowserSession select state, including focused select controls; this is not native select UI, keyboard events, input/change event dispatch, validation, or browser chrome.",
        ),
        feature(
            "browser-session-checkable-form-state",
            "forms",
            Implemented,
            "Remembers checkbox/radio checked state in BrowserSession entries, applies it after rerenders, and uses it for GET/URL-encoded POST form submission; this is not full form dirtiness, validation, indeterminate checkboxes, custom controls, or browser UI.",
        ),
        feature(
            "browser-session-form-state-visual-update",
            "forms",
            Implemented,
            "Updates the current rendered text/display-list widgets when supported BrowserSession text, select, checkbox, and radio form state changes; this is still deterministic document-shell rendering, not native platform form controls or input/change event dispatch.",
        ),
        feature(
            "browser-session-form-input-change-events",
            "forms",
            Implemented,
            "Writes supported BrowserSession text/select/checkable edits into the live DOM and dispatches narrow addEventListener input/change events so page handlers can read current value/checked state; this is not full composition/blur/change timing, constraint validation UI, or native platform controls.",
        ),
        feature(
            "browser-session-keyboard-events",
            "forms",
            Implemented,
            "Dispatches narrow bubbling keydown/keyup events for BrowserSession text insertion and Backspace deletion on focused editable controls, exposes event.type/event.key/event.target and supports keydown preventDefault blocking the text mutation; this is not full KeyboardEvent semantics, composition, selection ranges, shortcuts, repeat state, IME, or platform keyboard integration.",
        ),
        feature(
            "browser-session-beforeinput-events",
            "forms",
            Implemented,
            "Dispatches narrow bubbling beforeinput events for supported focused text insertion and Backspace deletion before live DOM/form mutation, exposes event.inputType/event.data, and honors preventDefault by skipping the edit and following input event; this is not full InputEvent semantics, composition, selection ranges, paste/drop/history input types, or native editor integration.",
        ),
        feature(
            "browser-session-pointer-events",
            "events",
            Implemented,
            "Dispatches narrow bubbling pointerdown and pointerup events around BrowserSession coordinate clicks before the existing click/default-action path, exposes event.clientX/event.clientY/event.button/event.pointerId/event.pointerType/event.isPrimary, and forwards coordinate readback to the generated click event; this is not full PointerEvent semantics, pointer capture, touch/pen input, pointercancel, CSS hit testing, or platform pointer integration.",
        ),
        feature(
            "browser-session-mouse-events",
            "events",
            Implemented,
            "Dispatches narrow bubbling mousedown and mouseup compatibility events around BrowserSession coordinate clicks between pointerdown/pointerup and the generated click event, exposing event.clientX/event.clientY/event.button; this is not full MouseEvent semantics, double-click/context-menu/aux-click handling, movement fields, hover events, CSS hit testing, or platform pointer integration.",
        ),
        feature(
            "browser-session-event-target-propagation",
            "events",
            Implemented,
            "Exposes the tiny event object event.type/event.target/event.currentTarget across supported bubbling DOM events and honors stopPropagation without suppressing later listeners on the same target; this is not composed paths, shadow DOM retargeting, passive listeners, or full Event API compatibility.",
        ),
        feature(
            "browser-session-stop-immediate-propagation",
            "events",
            Implemented,
            "Honors stopImmediatePropagation for supported BrowserSession event listeners by suppressing later listeners on the same event target and stopping propagation to remaining targets; this is not listener priority, composed paths, shadow DOM retargeting, or complete DOM Event compatibility.",
        ),
        feature(
            "browser-session-document-event-listeners",
            "events",
            Implemented,
            "Allows document addEventListener handlers for supported non-lifecycle bubbling events on the DOM document node, enabling narrow delegated click/keyboard/form/input handlers; this is not global event handler attributes, full listener lifecycle management, or full DOM EventTarget compatibility.",
        ),
        feature(
            "browser-session-capture-event-listeners",
            "events",
            Implemented,
            "Parses boolean and object-form addEventListener capture options for supported BrowserSession events, dispatches capture -> target -> bubble listener order, and exposes event.eventPhase; this is not passive/signal listener options, composed paths, shadow DOM retargeting, or a complete DOM EventTarget model.",
        ),
        feature(
            "browser-session-once-event-listeners",
            "events",
            Implemented,
            "Parses object-form addEventListener once options for supported BrowserSession events and removes those listeners after their first invocation across repeated session interactions; this is not passive/signal options, AbortSignal integration, or full listener lifecycle compatibility.",
        ),
        feature(
            "browser-session-remove-event-listener",
            "events",
            Implemented,
            "Supports a narrow removeEventListener path for supported session and lifecycle events by resolving const/let/var callable bindings and matching listener handler plus capture; this is not broad function declaration parsing, exact browser listener identity, dispatch-time removal ordering, passive/signal options, or complete DOM EventTarget compatibility.",
        ),
        feature(
            "browser-session-window-event-target",
            "events",
            Implemented,
            "Keeps supported window addEventListener handlers on a distinct Window event target at the top of the narrow session event path, with limited event.currentTarget/this identity readback for Window listeners; this is not the full Window API, global event handler attributes, cross-frame dispatch, or complete DOM EventTarget compatibility.",
        ),
        feature(
            "browser-session-submit-reset-events",
            "forms",
            Implemented,
            "Dispatches narrow bubbling submit/reset addEventListener handlers on supported forms before BrowserSession default navigation/reset, exposes the tiny event object, honors preventDefault, and lets submit handlers mutate supported live form values before submission; this is not full SubmitEvent.submitter, invalid events, requestSubmit semantics, custom elements, or broad browser event ordering.",
        ),
        feature(
            "browser-shell-checkable-form-toggle",
            "forms",
            Implemented,
            "Adds local CLI browse-shell toggle <form> <control>, space/toggle-focused, selector click default toggling, and narrow label activation for supported checkbox/radio controls; this is not full input/change event dispatch, full keyboard event dispatch, full label activation semantics, indeterminate state, custom controls, or browser UI.",
        ),
        feature(
            "browser-session-required-form-validation",
            "forms",
            Implemented,
            "Blocks supported BrowserSession and local CLI GET/URL-encoded POST submissions when enabled required text-like, select, checkbox, or radio controls are empty or unchecked, while honoring form novalidate and submitter formnovalidate; this is not full constraint validation, validation UI, input/change events, invalid events, custom validity, or broad browser compatibility.",
        ),
        feature(
            "browser-session-type-value-validation",
            "forms",
            Implemented,
            "Blocks supported BrowserSession and local CLI GET/URL-encoded POST submissions for non-empty email and URL controls with invalid values, while honoring form novalidate and submitter formnovalidate; this is not full type validation, IDNA/email grammar, validation UI, invalid events, custom validity, or broad browser compatibility.",
        ),
        feature(
            "browser-session-submitter-action-method-overrides",
            "forms",
            Implemented,
            "Honors submitter formaction and GET/POST formmethod overrides for supported BrowserSession and local CLI submit-control click/focused-submit paths; this is not full submitter semantics, external form ownership, target/enctype/dialog handling, event ordering, or broad form compatibility.",
        ),
        feature(
            "session-cookies",
            "network",
            Implemented,
            "Carries in-memory HTTP cookies between session navigations, honoring domain/path/secure matching plus immediate Max-Age deletion.",
        ),
        feature(
            "http-redirect-navigation",
            "network",
            Implemented,
            "Follows a bounded HTTP(S) redirect chain for document, form, and static resource loads, resolves relative Location headers, records the final URL as the session entry target, stores Set-Cookie headers from redirect responses before the next hop, and converts 303 plus POST 301/302 redirects to GET; this is not full navigation lifecycle, mixed-content/referrer/CORS policy, HSTS, redirect UI, or browser-grade error pages.",
        ),
        feature(
            "fixture-verifier",
            "testing",
            Implemented,
            "Verifies title, text, and display-list fixtures from JSON manifests.",
        ),
        feature(
            "css-cascade-complete",
            "css",
            Partial,
            "Only a tiny display-oriented CSS subset is parsed and applied.",
        ),
        feature(
            "navigation-complete",
            "navigation",
            Partial,
            "Supports simple local/HTTP navigation, history, reload, and CLI fragment target scrolling, but not full browser navigation semantics.",
        ),
        feature(
            "resource-loading-complete",
            "loading",
            Partial,
            "Loads static resources but does not integrate full fetch, preload, priority, CSP, or module semantics.",
        ),
        feature(
            "javascript",
            "runtime",
            Partial,
            "Tiny inline DOM/Web Storage/timer mutation subset only; no general parser, VM, full event loop, or broad Web API surface yet.",
        ),
        feature(
            "web-apis",
            "runtime",
            Partial,
            "Minimal DOM query, event listener, attribute, timer, localStorage, and sessionStorage APIs exist; broad Web API compatibility is still missing.",
        ),
        feature(
            "form-validation",
            "forms",
            Partial,
            "The supported BrowserSession/CLI submit paths enforce narrow required-control value-missing checks plus non-empty email/URL value checks, but full constraint validation, validation UI, custom validity, invalid events, and broad form semantics remain missing.",
        ),
        feature(
            "browser-session-urlencoded-post-form-submit",
            "forms",
            Implemented,
            "Submits application/x-www-form-urlencoded POST forms through BrowserSession and the local CLI shell, carrying filled text-like state, explicit overrides, cookies, and returned HTML rendering; this is not multipart forms, file upload, fetch/XHR, validation, full navigation lifecycle, browser UI, or broad browser compatibility.",
        ),
        feature(
            "browser-session-form-submit-button-click-default",
            "forms",
            Implemented,
            "Narrow BrowserSession/local CLI click default action for supported form submit controls from input/button elements submits the owning GET or application/x-www-form-urlencoded POST form with remembered text-like state and clicked submitter name/value; this is not form event dispatch, constraint validation, focus/input events, browser UI, full event/default-action ordering, or broad browser compatibility.",
        ),
        feature(
            "browser-session-form-reset-click-default",
            "forms",
            Implemented,
            "Narrow BrowserSession/local CLI click default action for supported reset controls clears remembered text-like state for the owning form and preserves same-page rendering; this is not full form event dispatch, constraint validation, focus/input events, browser UI, full form owner mapping, or broad browser compatibility.",
        ),
        feature(
            "post-form-submission",
            "forms",
            Missing,
            "Broad browser POST form submission remains missing; the separate implemented browser-session-urlencoded-post-form-submit gate is limited to BrowserSession/CLI application/x-www-form-urlencoded forms, browser-session-form-submit-button-click-default is limited to supported submit-control click defaults, browser-session-submitter-action-method-overrides is limited to submitter formaction/GET-or-POST formmethod overrides on those supported paths, and browser-session-form-reset-click-default is limited to supported reset-control click defaults.",
        ),
        feature(
            "font-shaping",
            "text",
            Missing,
            "No font selection, shaping, bidi, or glyph metrics.",
        ),
        feature(
            "paint-raster",
            "rendering",
            Partial,
            "Deterministic CPU text/styled-text/rectangle/background/border/padding/margin/sizing/image-placeholder/SVG/PNG-subset raster and whole-document viewport culling exist, but full image decode, stacking, CSS clipping, and GPU fallback are still missing.",
        ),
        feature(
            "compositor",
            "rendering",
            Partial,
            "Local layer-tree/debug snapshots exist, but GPU compositing, animation, compositor tiling, async scrolling, OS presentation, and frame scheduling are still missing.",
        ),
        feature(
            "sandbox",
            "security",
            Missing,
            "No process sandbox, site isolation, or permission model.",
        ),
        feature(
            "accessibility-tree",
            "accessibility",
            Partial,
            "A deterministic static accessibility snapshot exists for the supported DOM/CSS/tiny-script subset, but there is no full accessibility tree, focus model, live region support, keyboard navigation, or platform accessibility bridge.",
        ),
        feature(
            "persistent-cookies",
            "storage",
            Missing,
            "Browser-grade persistent cookie profiles remain missing; the separate browser-shell-cookie-jar-file gate is only local CLI JSON load/save around the in-memory session jar.",
        ),
        feature(
            "storage",
            "storage",
            Partial,
            "Origin-scoped localStorage and in-memory BrowserSession-scoped sessionStorage exist in browser sessions, with optional local CLI JSON load/save for localStorage only; no IndexedDB, Cache API, quota manager, browser profile persistence, or private mode.",
        ),
        feature(
            "media",
            "media",
            Missing,
            "No audio/video decode, playback, capture, or media pipeline.",
        ),
        feature(
            "canvas",
            "graphics",
            Missing,
            "No Canvas 2D/WebGL/WebGPU implementation.",
        ),
        feature(
            "devtools",
            "tooling",
            Missing,
            "No inspector protocol, timeline, console, or network panel.",
        ),
        feature(
            "extensions",
            "platform",
            Missing,
            "No browser extension system or extension permission policy.",
        ),
    ]
}

fn feature(
    id: &str,
    category: &str,
    status: BrowserFeatureState,
    evidence: &str,
) -> BrowserFeatureCoverage {
    BrowserFeatureCoverage {
        id: id.to_owned(),
        category: category.to_owned(),
        status,
        evidence: evidence.to_owned(),
    }
}
