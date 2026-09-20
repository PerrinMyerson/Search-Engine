use std::fmt::Write as _;

use super::{
    BrowserSessionPayload, BrowserSessionResourceReportPayload, browser_resource_count_label,
};

pub(super) fn render_browser_session_visual_flow_status(payload: &BrowserSessionPayload) -> String {
    let Some(report) = payload.resource_report.as_ref() else {
        if payload.resource_stylesheet_count == 0 && payload.resource_image_count == 0 {
            if payload.viewport_image.is_some() {
                return r#"<span class="viewport-state-chip" data-browser-visual-flow-status>visual page ready</span>"#.to_owned();
            }
            return r#"<span class="viewport-state-chip" data-browser-visual-flow-status>visual actions unavailable</span>"#.to_owned();
        }
        return format!(
            r#"<span class="viewport-state-chip" data-browser-visual-flow-status>visual actions ready: {} · {}</span>"#,
            html_escape::encode_text(&browser_resource_count_label(
                payload.resource_stylesheet_count,
                "stylesheet",
                "stylesheets",
            )),
            html_escape::encode_text(&browser_resource_count_label(
                payload.resource_image_count,
                "image",
                "images",
            )),
        );
    };

    let mut chips = String::new();
    if payload.resource_stylesheet_count > 0 {
        let label = match report.applied {
            Some(count) if count > 0 => format!("styles applied: {count}"),
            Some(_) => "styles unchanged".to_owned(),
            None => "styles waiting".to_owned(),
        };
        let _ = write!(
            chips,
            r#"<span class="viewport-state-chip report" data-browser-visual-flow-status>{}</span>"#,
            html_escape::encode_text(&label),
        );
    }
    if payload.resource_image_count > 0 {
        let paintable_decoded = browser_session_paintable_decoded_image_count(report);
        let decoded_not_rendered = browser_session_decoded_not_rendered_image_count(report);
        let label = match (report.decoded, report.attached) {
            (Some(_), Some(attached)) if paintable_decoded > 0 && attached > 0 => {
                let mut label = format!("images painted: {attached}/{paintable_decoded}");
                if decoded_not_rendered > 0 {
                    let _ = write!(label, "; not rendered: {decoded_not_rendered}");
                }
                label
            }
            (Some(decoded), Some(_)) if decoded > 0 => {
                if decoded_not_rendered == decoded {
                    format!("images decoded but not rendered: {decoded}")
                } else {
                    format!("images decoded but not painted: {decoded}")
                }
            }
            (Some(count), None) if count > 0 => format!("images loaded: {count}"),
            (Some(_), _) => format!(
                "images not decoded: fetched {}, failed {}, skipped {}",
                report.fetched, report.failed, report.skipped
            ),
            (None, _) => "images waiting".to_owned(),
        };
        let _ = write!(
            chips,
            r#"<span class="viewport-state-chip report" data-browser-visual-flow-status>{}</span>"#,
            html_escape::encode_text(&label),
        );
    }
    if report.failed > 0 {
        let _ = write!(
            chips,
            r#"<span class="viewport-state-chip warning" data-browser-visual-flow-status>resource failures: {}</span>"#,
            report.failed,
        );
    }
    if chips.is_empty() {
        r#"<span class="viewport-state-chip" data-browser-visual-flow-status>visual actions complete</span>"#.to_owned()
    } else {
        chips
    }
}

fn browser_session_paintable_decoded_image_count(
    report: &BrowserSessionResourceReportPayload,
) -> usize {
    report
        .resources
        .iter()
        .filter(|resource| {
            matches!(
                resource.image_visibility_state.as_deref(),
                Some("render_attached" | "decoded_unattached")
            )
        })
        .count()
}

fn browser_session_decoded_not_rendered_image_count(
    report: &BrowserSessionResourceReportPayload,
) -> usize {
    report
        .resources
        .iter()
        .filter(|resource| {
            resource.image_visibility_state.as_deref() == Some("decoded_not_rendered")
        })
        .count()
}
