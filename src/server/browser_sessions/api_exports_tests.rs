use super::super::RequestTarget;
use super::*;
use std::sync::Arc;
use tokio::sync::Notify;
use tokio::time::{Duration, sleep};

#[tokio::test]
async fn browser_session_api_viewport_export_does_not_mutate_live_session() {
    let dir = tempfile::tempdir().unwrap();
    let page = dir.path().join("api_viewport.html");
    let body = (0..80)
        .map(|index| format!("<p>line {index}</p>"))
        .collect::<String>();
    std::fs::write(
        &page,
        format!(r#"<!doctype html><title>API Viewport</title>{body}"#),
    )
    .unwrap();

    let registry = BrowserSessionRegistry::default();
    let create = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![
            ("url".to_owned(), page.display().to_string()),
            ("width".to_owned(), "100".to_owned()),
            ("height".to_owned(), "44".to_owned()),
        ],
    };
    let (payload, _) = registry.create_target(&create).await.unwrap();
    let id = payload.id.clone();
    assert_eq!(payload.width, 100);
    assert_eq!(payload.height, 44);
    assert_eq!(payload.viewport_y, 0);

    let api_export = RequestTarget {
        path: "/api/browser-session".to_owned(),
        params: vec![
            ("id".to_owned(), id.clone()),
            ("format".to_owned(), "session-state".to_owned()),
            ("width".to_owned(), "60".to_owned()),
            ("height".to_owned(), "20".to_owned()),
            ("viewport_y".to_owned(), "10".to_owned()),
        ],
    };
    let api_payload = api_exports::api_target(&registry, &api_export)
        .await
        .unwrap();
    assert_eq!(api_payload.width, 60);
    assert_eq!(api_payload.height, 20);
    assert_eq!(api_payload.viewport_y, 10);
    let response = browser_session_api_response(&api_export, &api_payload);
    assert_eq!(response.status, 200);
    let exported: serde_json::Value = serde_json::from_str(&response.body).unwrap();
    assert_eq!(exported["viewport"]["width"], 60);
    assert_eq!(exported["viewport"]["height"], 20);
    assert_eq!(exported["viewport"]["y"], 10);

    {
        let sessions = registry.sessions.lock().await;
        let stored = sessions.get(&id).unwrap();
        assert_eq!(stored.width, 100);
        assert_eq!(stored.height, 44);
        assert_eq!(stored.viewport_y, 0);
    }

    let current = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![
            ("id".to_owned(), id),
            ("action".to_owned(), "current".to_owned()),
        ],
    };
    let (current_payload, _) = registry.apply_target(&current).await.unwrap();
    assert_eq!(current_payload.width, 100);
    assert_eq!(current_payload.height, 44);
    assert_eq!(current_payload.viewport_y, 0);
}

#[tokio::test]
async fn browser_session_api_viewport_export_waits_for_in_flight_snapshot_gap() {
    let dir = tempfile::tempdir().unwrap();
    let page = dir.path().join("api_in_flight.html");
    let body = (0..40)
        .map(|index| format!("<p>visible snapshot {index}</p>"))
        .collect::<String>();
    std::fs::write(
        &page,
        format!(r#"<!doctype html><title>API In Flight</title>{body}"#),
    )
    .unwrap();

    let registry = BrowserSessionRegistry::default();
    let create = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![("url".to_owned(), page.display().to_string())],
    };
    let (payload, _) = registry.create_target(&create).await.unwrap();
    let id = payload.id.clone();
    let web_session = {
        let mut sessions = registry.sessions.lock().await;
        sessions.remove(&id).unwrap()
    };
    registry
        .in_flight_sessions
        .lock()
        .await
        .insert(id.clone(), Arc::new(Notify::new()));

    let api_export = RequestTarget {
        path: "/api/browser-session".to_owned(),
        params: vec![
            ("id".to_owned(), id.clone()),
            ("format".to_owned(), "session-state".to_owned()),
            ("width".to_owned(), "60".to_owned()),
            ("height".to_owned(), "20".to_owned()),
            ("viewport_y".to_owned(), "3".to_owned()),
        ],
    };

    let insert_snapshot = async {
        sleep(Duration::from_millis(20)).await;
        registry
            .in_flight_viewports
            .lock()
            .await
            .insert(id.clone(), web_session);
    };
    let read_snapshot = api_exports::api_target(&registry, &api_export);
    let (api_payload, _) = tokio::join!(read_snapshot, insert_snapshot);
    let api_payload = api_payload.unwrap();

    assert_eq!(api_payload.width, 60);
    assert_eq!(api_payload.height, 20);
    assert_eq!(api_payload.viewport_y, 3);
    assert_eq!(api_payload.title, "API In Flight");
}
