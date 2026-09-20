use super::BrowserSessionAction;

pub(super) fn browser_action_marks_session_in_flight(action: &BrowserSessionAction) -> bool {
    matches!(
        action,
        BrowserSessionAction::Current
            | BrowserSessionAction::Open(_)
            | BrowserSessionAction::Back
            | BrowserSessionAction::Forward
            | BrowserSessionAction::Reload
            | BrowserSessionAction::FetchResources
            | BrowserSessionAction::MakeVisual
            | BrowserSessionAction::ApplyStylesheets
            | BrowserSessionAction::RunScripts
            | BrowserSessionAction::LoadImages
            | BrowserSessionAction::Scroll { .. }
            | BrowserSessionAction::Top
            | BrowserSessionAction::Bottom
            | BrowserSessionAction::PageUp
            | BrowserSessionAction::PageDown
            | BrowserSessionAction::LineUp
            | BrowserSessionAction::LineDown
    )
}

pub(super) fn browser_action_clones_in_flight_viewport(action: &BrowserSessionAction) -> bool {
    matches!(
        action,
        BrowserSessionAction::FetchResources
            | BrowserSessionAction::MakeVisual
            | BrowserSessionAction::ApplyStylesheets
            | BrowserSessionAction::RunScripts
            | BrowserSessionAction::LoadImages
    )
}

pub(super) fn browser_pending_current_should_retry(source: &str) -> bool {
    source.starts_with("http://") || source.starts_with("https://")
}
