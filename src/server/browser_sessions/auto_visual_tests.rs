use super::super::RequestTarget;
use super::*;

#[tokio::test]
async fn auto_visual_initial_open_waits_past_generic_timeout_without_pending_retry() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut buf = [0u8; 1024];
        let _ = stream.read(&mut buf).await.unwrap();
        tokio::time::sleep(BROWSER_CREATE_TARGET_TIMEOUT + std::time::Duration::from_millis(20))
            .await;
        let body =
            "<!doctype html><title>Visual Slow Enough</title><p>first render should not retry</p>";
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        let _ = stream.write_all(response.as_bytes()).await;
    });

    let registry = BrowserSessionRegistry::default();
    let create = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![
            ("url".to_owned(), format!("http://{addr}/doc")),
            ("visual".to_owned(), "auto".to_owned()),
            ("width".to_owned(), "40".to_owned()),
            ("height".to_owned(), "10".to_owned()),
        ],
    };
    let (payload, _) = registry.create_target(&create).await.unwrap();

    assert_eq!(payload.pending_source, None);
    assert_eq!(payload.title, "Visual Slow Enough");
    assert!(payload.viewport_image.is_some());
    assert!(
        payload
            .action_feedback
            .as_deref()
            .is_some_and(|feedback| feedback.starts_with("Loaded first visual viewport"))
    );
    server.abort();
}

#[tokio::test]
async fn pending_auto_visual_open_completes_first_viewport_visual_on_retry() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let document_requests = Arc::new(AtomicUsize::new(0));
    let server_requests = document_requests.clone();
    let server = tokio::spawn(async move {
        for _ in 0..5 {
            let Ok((mut stream, _)) = listener.accept().await else {
                return;
            };
            let requests = server_requests.clone();
            tokio::spawn(async move {
                let mut buf = [0u8; 4096];
                let Ok(read) = stream.read(&mut buf).await else {
                    return;
                };
                let request = String::from_utf8_lossy(&buf[..read]);
                let request_line = request.lines().next().unwrap_or_default();
                let (body, content_type, slow) = if request_line.contains(" /app.css ") {
                    (
                        "img { display: block; width: 80px; height: 40px; }",
                        "text/css",
                        false,
                    )
                } else if request_line.contains(" /visible.svg ") {
                    (
                        r##"<svg xmlns="http://www.w3.org/2000/svg" width="2" height="2"><rect width="2" height="2" fill="#0088cc"/></svg>"##,
                        "image/svg+xml",
                        false,
                    )
                } else {
                    (
                        r#"<!doctype html><title>Retry Visual</title><link rel="stylesheet" href="/app.css"><img src="/visible.svg" alt="Visible image"><p>retry visual text</p>"#,
                        "text/html",
                        requests.fetch_add(1, Ordering::SeqCst) == 0,
                    )
                };
                if slow {
                    tokio::time::sleep(BROWSER_CREATE_TARGET_TIMEOUT * 3).await;
                }
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.write_all(response.as_bytes()).await;
            });
        }
    });

    let registry = BrowserSessionRegistry::default();
    let target_source = format!("http://{addr}/doc");
    let create = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![
            ("url".to_owned(), target_source.clone()),
            ("visual".to_owned(), "auto".to_owned()),
            ("width".to_owned(), "40".to_owned()),
            ("height".to_owned(), "10".to_owned()),
        ],
    };
    let (payload, _) = registry.create_target(&create).await.unwrap();
    assert_eq!(
        payload.pending_source.as_deref(),
        Some(target_source.as_str())
    );
    {
        let sessions = registry.sessions.lock().await;
        assert!(sessions.get(&payload.id).unwrap().visual_viewport_images);
    }

    let continue_load = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![
            ("id".to_owned(), payload.id.clone()),
            ("action".to_owned(), "current".to_owned()),
            ("width".to_owned(), "40".to_owned()),
            ("height".to_owned(), "10".to_owned()),
        ],
    };
    let (payload, _) = registry.apply_target(&continue_load).await.unwrap();
    assert_eq!(payload.id, continue_load.params[0].1);
    assert_eq!(payload.pending_source, None);
    assert_eq!(payload.title, "Retry Visual");
    assert!(payload.viewport_image.is_some());
    let report = payload.resource_report.as_ref().unwrap();
    assert_eq!(report.action, "Auto visual");
    assert_eq!(report.applied, Some(1));
    assert_eq!(report.decoded, Some(1));
    assert_eq!(report.attached, Some(1));
    assert!(
        payload
            .action_feedback
            .as_deref()
            .is_some_and(|feedback| feedback.starts_with("Loaded first visual viewport"))
    );
    server.abort();
}

#[tokio::test]
async fn concurrent_visual_current_requests_wait_for_same_session() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        for _ in 0..8 {
            let Ok((mut stream, _)) = listener.accept().await else {
                return;
            };
            tokio::spawn(async move {
                let mut buf = [0u8; 4096];
                let Ok(read) = stream.read(&mut buf).await else {
                    return;
                };
                let request = String::from_utf8_lossy(&buf[..read]);
                let request_line = request.lines().next().unwrap_or_default();
                let (body, content_type, slow) = if request_line.contains(" /app.css ") {
                    (
                        "img { display: block; width: 80px; height: 40px; }",
                        "text/css",
                        false,
                    )
                } else if request_line.contains(" /slow.svg ") {
                    (
                        r##"<svg xmlns="http://www.w3.org/2000/svg" width="2" height="2"><rect width="2" height="2" fill="#0088cc"/></svg>"##,
                        "image/svg+xml",
                        true,
                    )
                } else {
                    (
                        r#"<!doctype html><title>Concurrent Visual</title><link rel="stylesheet" href="/app.css"><img src="/slow.svg" alt="Slow image"><p>same session</p>"#,
                        "text/html",
                        false,
                    )
                };
                if slow {
                    tokio::time::sleep(BROWSER_VISUAL_VIEWPORT_CURRENT_REFRESH_TIMEOUT / 2).await;
                }
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.write_all(response.as_bytes()).await;
            });
        }
    });

    let registry = BrowserSessionRegistry::default();
    let create = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![
            ("url".to_owned(), format!("http://{addr}/doc")),
            ("visual".to_owned(), "auto".to_owned()),
            ("width".to_owned(), "40".to_owned()),
            ("height".to_owned(), "10".to_owned()),
        ],
    };
    let (payload, _) = registry.create_target(&create).await.unwrap();
    let current_a = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![
            ("id".to_owned(), payload.id.clone()),
            ("action".to_owned(), "current".to_owned()),
            ("width".to_owned(), "40".to_owned()),
            ("height".to_owned(), "10".to_owned()),
        ],
    };
    let current_b = RequestTarget {
        path: "/browser".to_owned(),
        params: current_a.params.clone(),
    };
    let (first, second) = tokio::join!(
        registry.apply_target(&current_a),
        registry.apply_target(&current_b)
    );
    let (first_payload, _) = first.unwrap();
    let (second_payload, _) = second.unwrap();
    assert_eq!(first_payload.id, payload.id);
    assert_eq!(second_payload.id, payload.id);
    assert!(
        !first_payload
            .action_feedback
            .as_deref()
            .unwrap_or_default()
            .contains("Recovered expired browser session")
    );
    assert!(
        !second_payload
            .action_feedback
            .as_deref()
            .unwrap_or_default()
            .contains("Recovered expired browser session")
    );
    assert_eq!(
        second_payload
            .resource_report
            .as_ref()
            .and_then(|report| report.attached),
        Some(1)
    );
    server.abort();
}

#[tokio::test]
async fn create_with_auto_visual_defers_slow_first_viewport_image_after_styles() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;
    use tokio::sync::oneshot;

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (image_requested_tx, image_requested_rx) = oneshot::channel::<()>();
    let server = tokio::spawn(async move {
        let mut image_requested_tx = Some(image_requested_tx);
        for _ in 0..3 {
            let Ok((mut stream, _)) = listener.accept().await else {
                return;
            };
            let mut buf = [0u8; 4096];
            let Ok(read) = stream.read(&mut buf).await else {
                continue;
            };
            let request = String::from_utf8_lossy(&buf[..read]);
            let request_line = request.lines().next().unwrap_or_default();
            let (body, content_type) = if request_line.contains(" /app.css ") {
                (
                    "img { display: block; width: 320px; height: 180px; }".to_owned(),
                    "text/css",
                )
            } else if request_line.contains(" /slow.svg ") {
                if let Some(tx) = image_requested_tx.take() {
                    let _ = tx.send(());
                }
                tokio::time::sleep(BROWSER_AUTO_VISUAL_FIRST_RESPONSE_TIMEOUT * 3).await;
                (
                    r##"<svg xmlns="http://www.w3.org/2000/svg" width="4" height="2"><rect width="4" height="2" fill="#0088cc"/></svg>"##.to_owned(),
                    "image/svg+xml",
                )
            } else {
                (
                    r#"<!doctype html><title>Slow Image</title><link rel="stylesheet" href="/app.css"><img src="/slow.svg" alt="Slow image"><p>Styled shell text remains usable.</p>"#.to_owned(),
                    "text/html",
                )
            };
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            let _ = stream.write_all(response.as_bytes()).await;
        }
    });

    let registry = BrowserSessionRegistry::default();
    let create = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![
            ("url".to_owned(), format!("http://{addr}/doc")),
            ("visual".to_owned(), "auto".to_owned()),
            ("width".to_owned(), "40".to_owned()),
            ("height".to_owned(), "10".to_owned()),
        ],
    };
    let started = std::time::Instant::now();
    let (payload, back_href) = registry.create_target(&create).await.unwrap();

    image_requested_rx.await.unwrap();
    assert!(
        started.elapsed() < BROWSER_AUTO_VISUAL_FIRST_IMAGE_TIMEOUT * 2,
        "slow first viewport image should not hold the first response open"
    );
    assert_eq!(payload.title, "Slow Image");
    let report = payload.resource_report.as_ref().unwrap();
    assert_eq!(report.action, "Apply styles");
    assert_eq!(report.applied, Some(1));
    assert_eq!(report.decoded, None);
    assert_eq!(report.attached, None);
    assert!(
        payload
            .action_feedback
            .as_deref()
            .is_some_and(|feedback| feedback
                .starts_with("Visual loading deferred; first viewport image work exceeded ")),
        "unexpected feedback: {:?}",
        payload.action_feedback
    );
    {
        let sessions = registry.sessions.lock().await;
        assert!(
            sessions
                .get(&payload.id)
                .is_some_and(|session| session.visual_viewport_images),
            "deferred first viewport image sessions should keep viewport image refresh enabled"
        );
    }

    let html = render_browser_session_page(&payload, &back_href);
    assert!(html.contains(r#"data-auto-visual-status"#));
    assert!(html.contains(r#"auto-visual-status" hidden data-auto-visual-status"#));
    assert!(html.contains("const preferFirstViewportVisual = true"));
    assert!(html.contains(
        "const backgroundFirstViewportVisual = preferFirstViewportVisual && !blockBrowserControls;"
    ));
    assert!(html.contains(r#"const applyStylesheetsUrl = "";"#));
    assert!(html.contains(r#"replacePartialFragment(doc, "[data-browser-chrome-status]")"#));
    assert!(html.contains("partial=viewport"));
    assert!(html.contains("const firstViewportVisualStillPending = (html) =>"));
    assert!(html.contains(r#"html.includes("Visual viewport refresh pending")"#));
    assert!(html.contains("for (const delayMs of [180, 320])"));
    assert!(html.contains("Loading first viewport..."));
    server.abort();
}

#[tokio::test]
async fn current_refresh_applies_styles_after_deferred_auto_visual_stylesheet() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let css_requests = Arc::new(AtomicUsize::new(0));
    let server_css_requests = css_requests.clone();
    let server = tokio::spawn(async move {
        for _ in 0..6 {
            let Ok((mut stream, _)) = listener.accept().await else {
                return;
            };
            let css_requests = server_css_requests.clone();
            tokio::spawn(async move {
                let mut buf = [0u8; 4096];
                let Ok(read) = stream.read(&mut buf).await else {
                    return;
                };
                let request = String::from_utf8_lossy(&buf[..read]);
                let request_line = request.lines().next().unwrap_or_default();
                let (body, content_type, slow) = if request_line.contains(" /slow.css ") {
                    let request_index = css_requests.fetch_add(1, Ordering::SeqCst);
                    (
                        ".hidden-menu { display: none; } img { display: block; width: 80px; height: 40px; }",
                        "text/css",
                        request_index == 0,
                    )
                } else if request_line.contains(" /visible.svg ") {
                    (
                        r##"<svg xmlns="http://www.w3.org/2000/svg" width="2" height="2"><rect width="2" height="2" fill="#0088cc"/></svg>"##,
                        "image/svg+xml",
                        false,
                    )
                } else {
                    (
                        r#"<!doctype html><title>Deferred Style</title><link rel="stylesheet" href="/slow.css"><nav class="hidden-menu">dropdown text should stay hidden</nav><img src="/visible.svg" alt="Visible image"><p>styled visual shell</p>"#,
                        "text/html",
                        false,
                    )
                };
                if slow {
                    tokio::time::sleep(BROWSER_AUTO_VISUAL_FIRST_RESPONSE_TIMEOUT * 2).await;
                }
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.write_all(response.as_bytes()).await;
            });
        }
    });

    let registry = BrowserSessionRegistry::default();
    let create = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![
            ("url".to_owned(), format!("http://{addr}/doc")),
            ("visual".to_owned(), "auto".to_owned()),
            ("width".to_owned(), "40".to_owned()),
            ("height".to_owned(), "10".to_owned()),
        ],
    };
    let (payload, _) = registry.create_target(&create).await.unwrap();
    assert_eq!(payload.title, "Deferred Style");
    assert!(payload.resource_report.is_none());
    {
        let sessions = registry.sessions.lock().await;
        let session = sessions.get(&payload.id).unwrap();
        assert!(session.visual_viewport_images);
        assert!(!session.visual_stylesheets_ready);
    }

    let current = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![
            ("id".to_owned(), payload.id.clone()),
            ("action".to_owned(), "current".to_owned()),
            ("width".to_owned(), "40".to_owned()),
            ("height".to_owned(), "10".to_owned()),
        ],
    };
    let (payload, _) = registry.apply_target(&current).await.unwrap();
    let report = payload.resource_report.as_ref().unwrap();
    assert_eq!(report.action, "Viewport visual");
    assert_eq!(report.applied, Some(1));
    assert_eq!(report.decoded, Some(1));
    assert_eq!(report.attached, Some(1));
    assert!(payload.viewport_image.is_some());
    assert!(
        payload
            .action_feedback
            .as_deref()
            .is_some_and(|feedback| feedback.starts_with("Loaded first visual viewport"))
    );
    {
        let sessions = registry.sessions.lock().await;
        assert!(
            sessions
                .get(&payload.id)
                .is_some_and(|session| session.visual_stylesheets_ready)
        );
    }
    server.abort();
}
