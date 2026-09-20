use std::time::{Duration, Instant};

use tokio::time::timeout;

use crate::browser::{BrowserImageLoadOptions, BrowserImageViewport};

use super::{
    BrowserRouteError, BrowserSessionAction, BrowserWebSession, RequestTarget,
    browser_session_feedback_excerpt, browser_session_resource_report_from_auto_visual,
    browser_session_resource_report_from_images, browser_session_resource_report_from_make_visual,
    browser_session_resource_report_from_stylesheets, normalize_browser_session_viewport,
};

#[cfg(not(test))]
pub(super) const BROWSER_AUTO_VISUAL_FIRST_RESPONSE_TIMEOUT: Duration = Duration::from_millis(220);
#[cfg(test)]
pub(super) const BROWSER_AUTO_VISUAL_FIRST_RESPONSE_TIMEOUT: Duration = Duration::from_millis(100);
#[cfg(not(test))]
pub(super) const BROWSER_VISUAL_VIEWPORT_REFRESH_TIMEOUT: Duration = Duration::from_millis(125);
#[cfg(test)]
pub(super) const BROWSER_VISUAL_VIEWPORT_REFRESH_TIMEOUT: Duration = Duration::from_millis(250);
#[cfg(not(test))]
pub(super) const BROWSER_VISUAL_VIEWPORT_CURRENT_REFRESH_TIMEOUT: Duration =
    Duration::from_millis(450);
#[cfg(test)]
pub(super) const BROWSER_VISUAL_VIEWPORT_CURRENT_REFRESH_TIMEOUT: Duration =
    Duration::from_millis(250);
#[cfg(not(test))]
pub(super) const BROWSER_AUTO_VISUAL_FIRST_IMAGE_TIMEOUT: Duration = Duration::from_millis(900);
#[cfg(test)]
pub(super) const BROWSER_AUTO_VISUAL_FIRST_IMAGE_TIMEOUT: Duration = Duration::from_millis(260);
#[cfg(not(test))]
pub(super) const BROWSER_AUTO_VISUAL_BACKGROUND_FIRST_VIEWPORT_TIMEOUT: Duration =
    Duration::from_millis(1000);
#[cfg(test)]
pub(super) const BROWSER_AUTO_VISUAL_BACKGROUND_FIRST_VIEWPORT_TIMEOUT: Duration =
    Duration::from_millis(500);

pub(super) fn browser_target_auto_visual(target: &RequestTarget) -> bool {
    target
        .param("visual")
        .as_deref()
        .is_some_and(|mode| mode.eq_ignore_ascii_case("auto"))
}

pub(super) async fn apply_browser_pending_open_success_visual_state(
    web_session: &mut BrowserWebSession,
    target_url: &str,
) -> Result<(), BrowserRouteError> {
    if web_session.visual_viewport_images {
        apply_browser_session_auto_visual_for_first_response(web_session).await
    } else {
        web_session.action_feedback = Some(format!(
            "Opened {}.",
            browser_session_feedback_excerpt(target_url)
        ));
        Ok(())
    }
}

pub(super) async fn apply_browser_session_auto_visual_for_first_response(
    web_session: &mut BrowserWebSession,
) -> Result<(), BrowserRouteError> {
    let stylesheet_report = match timeout(
        BROWSER_AUTO_VISUAL_FIRST_RESPONSE_TIMEOUT,
        web_session
            .session
            .render_current_with_stylesheets(web_session.max_bytes),
    )
    .await
    {
        Ok(Ok(report)) => report,
        Ok(Err(error)) => {
            return Err(BrowserRouteError::Upstream(format!(
                "browser auto visual stylesheet render failed: {error:#}"
            )));
        }
        Err(_) => {
            defer_browser_session_auto_visual(
                web_session,
                None,
                format!(
                    "Visual loading deferred; first visual pass exceeded {}ms.",
                    BROWSER_AUTO_VISUAL_FIRST_RESPONSE_TIMEOUT.as_millis()
                ),
            );
            return Ok(());
        }
    };

    let stylesheet_payload = browser_session_resource_report_from_stylesheets(stylesheet_report);
    web_session.visual_stylesheets_ready = true;
    let image_timeout = BROWSER_AUTO_VISUAL_FIRST_IMAGE_TIMEOUT;

    let image_report = match timeout(
        image_timeout,
        web_session
            .session
            .render_current_with_image_options(first_viewport_image_options(web_session)),
    )
    .await
    {
        Ok(Ok(report)) => report,
        Ok(Err(error)) => {
            return Err(BrowserRouteError::Upstream(format!(
                "browser auto visual image render failed: {error:#}"
            )));
        }
        Err(_) => {
            defer_browser_session_auto_visual(
                web_session,
                Some(stylesheet_payload),
                format!(
                    "Visual loading deferred; first viewport image work exceeded {}ms.",
                    image_timeout.as_millis()
                ),
            );
            return Ok(());
        }
    };

    web_session.resource_report = Some(browser_session_resource_report_from_auto_visual(
        stylesheet_payload,
        browser_session_resource_report_from_images(image_report),
    ));
    web_session.visual_viewport_images = true;
    normalize_browser_session_viewport(web_session);
    web_session.action_feedback = Some(format!(
        "Loaded first visual viewport at x {}, y {}.",
        web_session.viewport_x, web_session.viewport_y
    ));
    Ok(())
}

fn defer_browser_session_auto_visual(
    web_session: &mut BrowserWebSession,
    resource_report: Option<super::BrowserSessionResourceReportPayload>,
    message: String,
) {
    web_session.resource_report = resource_report;
    web_session.visual_viewport_images = true;
    if web_session.resource_report.is_some() {
        web_session.visual_stylesheets_ready = true;
    }
    normalize_browser_session_viewport(web_session);
    web_session.action_feedback = Some(message);
}

fn first_viewport_image_options(web_session: &BrowserWebSession) -> BrowserImageLoadOptions {
    BrowserImageLoadOptions::first_viewport(
        web_session.max_bytes,
        BrowserImageViewport {
            x: web_session.viewport_x,
            y: web_session.viewport_y,
            width: web_session.width,
            height: web_session.height,
        },
    )
}

pub(super) fn browser_visual_viewport_refresh_timeout(action: &BrowserSessionAction) -> Duration {
    if matches!(action, BrowserSessionAction::Current) {
        BROWSER_VISUAL_VIEWPORT_CURRENT_REFRESH_TIMEOUT
    } else {
        BROWSER_VISUAL_VIEWPORT_REFRESH_TIMEOUT
    }
}

pub(super) async fn refresh_browser_session_visual_viewport_images(
    action: &BrowserSessionAction,
    web_session: &mut BrowserWebSession,
) -> Result<(), BrowserRouteError> {
    if !web_session.visual_viewport_images
        || web_session.pending_source.is_some()
        || !browser_action_refreshes_visual_viewport_images(action)
    {
        return Ok(());
    }
    let refresh_timeout = browser_visual_viewport_refresh_timeout(action);
    let started = Instant::now();
    let mut stylesheet_payload = None;
    if !web_session.visual_stylesheets_ready {
        let stylesheet_report = match timeout(
            refresh_timeout,
            web_session
                .session
                .render_current_with_stylesheets(web_session.max_bytes),
        )
        .await
        {
            Ok(Ok(report)) => report,
            Ok(Err(error)) => {
                return Err(BrowserRouteError::Upstream(format!(
                    "browser visual viewport stylesheet render failed: {error:#}"
                )));
            }
            Err(_) => {
                web_session.action_feedback = Some(format!(
                    "Visual viewport refresh pending; stylesheet work exceeded {}ms.",
                    refresh_timeout.as_millis()
                ));
                return Ok(());
            }
        };
        let payload = browser_session_resource_report_from_stylesheets(stylesheet_report);
        web_session.visual_stylesheets_ready = true;
        web_session.resource_report = Some(payload.clone());
        stylesheet_payload = Some(payload);
    }
    let image_timeout = refresh_timeout
        .checked_sub(started.elapsed())
        .unwrap_or(Duration::ZERO);
    if image_timeout.is_zero() {
        web_session.action_feedback = Some(format!(
            "Visual viewport refresh pending; image work exceeded {}ms.",
            refresh_timeout.as_millis()
        ));
        normalize_browser_session_viewport(web_session);
        return Ok(());
    }
    let image_options = first_viewport_image_options(web_session);
    let image_report = match timeout(
        image_timeout,
        web_session
            .session
            .render_current_with_image_options(image_options),
    )
    .await
    {
        Ok(Ok(report)) => report,
        Ok(Err(error)) => {
            return Err(BrowserRouteError::Upstream(format!(
                "browser visual viewport image render failed: {error:#}"
            )));
        }
        Err(_) => {
            web_session.action_feedback = Some(format!(
                "Visual viewport refresh pending; image work exceeded {}ms.",
                image_timeout.as_millis()
            ));
            return Ok(());
        }
    };
    let mut report = browser_session_resource_report_from_images(image_report);
    report.action = "Viewport visual".to_owned();
    if let Some(stylesheet_payload) = stylesheet_payload {
        report = browser_session_resource_report_from_make_visual(stylesheet_payload, report);
        report.action = "Viewport visual".to_owned();
    } else {
        report.applied = web_session
            .resource_report
            .as_ref()
            .and_then(|report| report.applied);
    }
    web_session.resource_report = Some(report);
    normalize_browser_session_viewport(web_session);
    replace_visual_pending_feedback_with_loaded(action, web_session);
    Ok(())
}

fn browser_action_refreshes_visual_viewport_images(action: &BrowserSessionAction) -> bool {
    matches!(
        action,
        BrowserSessionAction::Current
            | BrowserSessionAction::Scroll { .. }
            | BrowserSessionAction::Top
            | BrowserSessionAction::Bottom
            | BrowserSessionAction::PageUp
            | BrowserSessionAction::PageDown
            | BrowserSessionAction::LineUp
            | BrowserSessionAction::LineDown
    )
}

fn replace_visual_pending_feedback_with_loaded(
    action: &BrowserSessionAction,
    web_session: &mut BrowserWebSession,
) {
    let _ = action;
    let Some(feedback) = web_session.action_feedback.as_deref() else {
        return;
    };
    if !feedback.starts_with("Visual loading deferred;")
        && !feedback.starts_with("Visual viewport refresh pending;")
    {
        return;
    }
    web_session.action_feedback = Some(format!(
        "Loaded first visual viewport at x {}, y {}.",
        web_session.viewport_x, web_session.viewport_y
    ));
}
