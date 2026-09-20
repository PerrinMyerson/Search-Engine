use super::super::RequestTarget;
use super::{
    BrowserRouteError, BrowserSessionAction, BrowserSessionPayload, BrowserSessionPayloadOptions,
    BrowserSessionRegistry, BrowserWebSession, browser_session_payload_with_options,
    browser_session_target_viewport_x, browser_session_target_viewport_y,
    parse_optional_usize_param,
};
use std::time::{Duration, Instant};
use tokio::time::sleep;

const API_IN_FLIGHT_SNAPSHOT_WAIT: Duration = Duration::from_millis(500);
const API_IN_FLIGHT_SNAPSHOT_POLL: Duration = Duration::from_millis(5);

pub(super) async fn api_target(
    registry: &BrowserSessionRegistry,
    target: &RequestTarget,
) -> Result<BrowserSessionPayload, BrowserRouteError> {
    let id = target.param("id").unwrap_or_default();
    if id.trim().is_empty() {
        return Err(BrowserRouteError::BadRequest(
            "missing browser session id".to_owned(),
        ));
    }

    let mut web_session = api_session_snapshot(registry, &id).await?;

    apply_browser_session_api_viewport_target(target, &mut web_session);
    let mut payload = browser_session_payload_with_options(
        &id,
        &mut web_session,
        BrowserSessionPayloadOptions {
            render_viewport_image: false,
            fast_scroll: false,
        },
    )?;
    let mut sessions_view = {
        let sessions = registry.sessions.lock().await;
        sessions.clone()
    };
    sessions_view.insert(id.clone(), web_session);
    let closed_sessions = registry.closed_sessions.lock().await;
    let bookmarks = registry.bookmarks.lock().await;
    registry
        .attach_browser_session_registry_state(
            &mut payload,
            &sessions_view,
            &closed_sessions,
            &bookmarks,
            &id,
        )
        .await;
    Ok(payload)
}

async fn api_session_snapshot(
    registry: &BrowserSessionRegistry,
    id: &str,
) -> Result<BrowserWebSession, BrowserRouteError> {
    let started = Instant::now();
    loop {
        if let Some(web_session) = api_session_snapshot_once(registry, id).await {
            return Ok(web_session);
        }
        if !api_session_is_in_flight(registry, id).await {
            return Err(BrowserRouteError::NotFound(format!(
                "browser session {id} not found"
            )));
        }
        if started.elapsed() >= API_IN_FLIGHT_SNAPSHOT_WAIT {
            return Err(BrowserRouteError::Upstream(format!(
                "browser session {id} is busy; API snapshot was unavailable after {}ms",
                API_IN_FLIGHT_SNAPSHOT_WAIT.as_millis()
            )));
        }
        sleep(API_IN_FLIGHT_SNAPSHOT_POLL).await;
    }
}

async fn api_session_snapshot_once(
    registry: &BrowserSessionRegistry,
    id: &str,
) -> Option<BrowserWebSession> {
    let web_session = {
        let sessions = registry.sessions.lock().await;
        sessions.get(id).cloned()
    };
    if web_session.is_some() {
        return web_session;
    }
    let sessions = registry.in_flight_viewports.lock().await;
    sessions.get(id).cloned()
}

async fn api_session_is_in_flight(registry: &BrowserSessionRegistry, id: &str) -> bool {
    let sessions = registry.in_flight_sessions.lock().await;
    sessions.contains_key(id)
}

fn apply_browser_session_api_viewport_target(
    target: &RequestTarget,
    web_session: &mut BrowserWebSession,
) {
    web_session.width =
        parse_optional_usize_param(target, "width", 40, 160).unwrap_or(web_session.width);
    web_session.height =
        parse_optional_usize_param(target, "height", 16, 120).unwrap_or(web_session.height);
    web_session.max_bytes =
        parse_optional_usize_param(target, "max_bytes", 64 * 1024, 16 * 1024 * 1024)
            .unwrap_or(web_session.max_bytes);
    web_session.viewport_x =
        browser_session_target_viewport_x(target, &BrowserSessionAction::Current)
            .unwrap_or(web_session.viewport_x);
    web_session.viewport_y =
        browser_session_target_viewport_y(target, &BrowserSessionAction::Current)
            .unwrap_or(web_session.viewport_y);
}
