use super::*;

fn assert_feature(report: &BrowserCoverageReport, id: &str, expected_status: BrowserFeatureState) {
    assert!(
        report
            .features
            .iter()
            .any(|feature| feature.id == id && feature.status == expected_status),
        "expected {id} to be {expected_status:?}"
    );
}

#[test]
fn browser_coverage_report_tracks_implemented_and_missing_features() {
    let report = browser_coverage_report();
    assert!(report.feature_count > 20);
    assert!(report.implemented_count > 10);
    assert!(report.missing_count > 5);
    assert!(report.implemented_ratio > 0.0);

    let implemented = [
        "static-html-parse",
        "inline-script-dom-text",
        "inline-script-dom-create",
        "dom-tree-mutation",
        "dom-node-traversal",
        "dom-insertion-methods",
        "document-fragment",
        "dom-selector-methods",
        "dom-inner-html-mutation",
        "dom-form-control-properties",
        "dom-location-readback",
        "dom-set-attribute",
        "dom-get-attribute",
        "dom-style-property-mutation",
        "dom-class-list-mutation",
        "dom-query-collections",
        "local-storage-api",
        "session-storage-api",
        "timer-task-queue",
        "document-lifecycle-events",
        "external-script-render",
        "inline-onclick-event",
        "event-listener-click",
        "complex-query-selector",
        "complex-click-selector",
        "css-background-color",
        "css-color-property",
        "css-text-align",
        "css-border-shorthand",
        "css-padding-shorthand",
        "css-margin-shorthand",
        "css-size-properties",
        "css-max-width-auto-margin-layout",
        "compound-css-selectors",
        "attribute-css-selectors",
        "hidden-attribute",
        "list-marker-layout",
        "nested-list-indent-layout",
        "preformatted-text-layout",
        "simple-table-row-layout",
        "simple-table-column-layout",
        "prose-indent-layout",
        "inline-form-control-layout",
        "cpu-text-raster",
        "rect-paint-command",
        "display-list-hit-testing",
        "layer-tree-snapshot",
        "retained-layout-tree",
        "text-color-paint",
        "block-background-paint",
        "block-border-paint",
        "block-padding-layout",
        "block-margin-layout",
        "block-size-layout",
        "image-replaced-element",
        "responsive-image-selection",
        "network-image-render",
        "svg-image-decode",
        "png-image-decode",
        "data-url-image-decode",
        "image-decode-cache",
        "image-placeholder-paint",
        "cpu-rect-raster",
        "cpu-image-placeholder-raster",
        "cpu-decoded-image-raster",
        "rgba-screenshot-artifact",
        "viewport-raster-culling",
        "browser-viewport-layout-state",
        "browser-viewport-invalidation",
        "browser-viewport-frame-surface",
        "browser-app-state-surface",
        "browser-app-cli-surface",
        "browser-app-interactive-shell",
        "browser-app-visible-viewport",
        "browser-app-profile-history-bookmarks",
        "browser-app-find-text",
        "browser-app-window-frame",
        "browser-app-window-hit-testing",
        "browser-native-window-shell",
        "browser-native-window-location-input",
        "stage1-document-page-corpus",
        "browser-shell-cli",
        "browser-shell-visual-frame",
        "browser-shell-tabs",
        "browser-shell-relative-open",
        "browser-shell-location-command",
        "browser-shell-cookie-inspection",
        "browser-shell-clear-cookies",
        "browser-shell-cookie-jar-file",
        "browser-shell-local-storage-file",
        "browser-shell-local-storage-inspection",
        "browser-shell-session-storage-inspection",
        "browser-shell-clear-local-storage",
        "browser-shell-clear-session-storage",
        "browser-shell-link-activation",
        "browser-session-reload",
        "browser-shell-reload",
        "browser-shell-anchor-click-default",
        "browser-session-live-page-state",
        "browser-shell-fragment-navigation",
        "browser-shell-coordinate-click",
        "browser-cli-click-at-viewport-offset",
        "browser-shell-wheel-events",
        "browser-shell-find-text",
        "browser-shell-form-fill-state",
        "browser-session-select-form-state",
        "browser-shell-select-form-choice",
        "browser-session-checkable-form-state",
        "browser-session-form-state-visual-update",
        "browser-session-form-input-change-events",
        "browser-session-keyboard-events",
        "browser-session-beforeinput-events",
        "browser-session-pointer-events",
        "browser-session-mouse-events",
        "browser-session-event-target-propagation",
        "browser-session-stop-immediate-propagation",
        "browser-session-document-event-listeners",
        "browser-session-capture-event-listeners",
        "browser-session-once-event-listeners",
        "browser-session-remove-event-listener",
        "browser-session-window-event-target",
        "browser-session-submit-reset-events",
        "browser-shell-checkable-form-toggle",
        "browser-session-focused-form-control",
        "browser-session-focus-traversal",
        "browser-session-focus-events",
        "browser-shell-focused-text-input",
        "browser-shell-focus-traversal",
        "browser-session-focused-text-edit",
        "browser-shell-focused-text-edit",
        "browser-session-focused-form-submit",
        "browser-shell-enter-submit",
        "browser-session-urlencoded-post-form-submit",
        "browser-session-form-submit-button-click-default",
        "browser-session-submitter-action-method-overrides",
        "browser-session-form-reset-click-default",
        "http-redirect-navigation",
        "visual-baseline-runner",
        "visual-pixel-diff",
        "static-accessibility-tree",
    ];
    for id in implemented {
        assert_feature(&report, id, BrowserFeatureState::Implemented);
    }

    assert_feature(
        &report,
        "post-form-submission",
        BrowserFeatureState::Missing,
    );
    for id in ["paint-raster", "compositor", "javascript"] {
        assert_feature(&report, id, BrowserFeatureState::Partial);
    }

    let mut gated_report = browser_coverage_report();
    assert!(gated_report.apply_gate(BrowserCoverageGate {
        required_features: implemented.iter().map(|id| (*id).to_owned()).collect(),
        min_implemented_ratio: Some(0.40),
        max_missing_features: Some(20),
    }));
    assert_eq!(gated_report.passed, Some(true));

    assert!(!gated_report.apply_gate(BrowserCoverageGate {
        required_features: vec!["javascript".to_owned()],
        ..BrowserCoverageGate::default()
    }));
    assert_eq!(gated_report.missing_required_features, vec!["javascript"]);
    assert_eq!(gated_report.passed, Some(false));
}
