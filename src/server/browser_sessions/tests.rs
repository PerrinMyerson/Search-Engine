use super::super::RequestTarget;
use super::*;

#[tokio::test]
async fn browser_session_registry_keeps_history_across_link_navigation() {
    let dir = tempfile::tempdir().unwrap();
    let first = dir.path().join("first.html");
    let second = dir.path().join("second.html");
    std::fs::write(
        &first,
        format!(
            r#"<!doctype html><title>First</title><a href="{}">Second</a>"#,
            second.display()
        ),
    )
    .unwrap();
    std::fs::write(
        &second,
        r#"<!doctype html><title>Second</title><p>done</p>"#,
    )
    .unwrap();

    let registry = BrowserSessionRegistry::default();
    let create = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![
            ("url".to_owned(), first.display().to_string()),
            ("from".to_owned(), "/search?q=local".to_owned()),
        ],
    };
    let (payload, _) = registry.create_target(&create).await.unwrap();
    assert_eq!(payload.title, "First");
    assert_eq!(payload.history_len, 1);
    assert!(!payload.can_back);

    let follow = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![
            ("id".to_owned(), payload.id.clone()),
            ("action".to_owned(), "link".to_owned()),
            ("link".to_owned(), "0".to_owned()),
        ],
    };
    let (payload, _) = registry.apply_target(&follow).await.unwrap();
    assert_eq!(payload.title, "Second");
    assert_eq!(payload.history_len, 2);
    assert!(payload.can_back);

    let back = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![
            ("id".to_owned(), payload.id.clone()),
            ("action".to_owned(), "back".to_owned()),
        ],
    };
    let (payload, _) = registry.apply_target(&back).await.unwrap();
    assert_eq!(payload.title, "First");
    assert!(!payload.can_back);
    assert!(payload.can_forward);
}

#[tokio::test]
async fn browser_session_registry_opens_links_by_text_and_selector() {
    let dir = tempfile::tempdir().unwrap();
    let first = dir.path().join("first.html");
    let second = dir.path().join("second.html");
    std::fs::write(
        &first,
        format!(
            r#"<!doctype html><title>First</title><a id="go" href="{}">Open target</a>"#,
            second.display()
        ),
    )
    .unwrap();
    std::fs::write(
        &second,
        r#"<!doctype html><title>Second</title><p>linked by text or selector</p>"#,
    )
    .unwrap();

    let registry = BrowserSessionRegistry::default();
    let create = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![("url".to_owned(), first.display().to_string())],
    };
    let (payload, back_href) = registry.create_target(&create).await.unwrap();
    let html = render_browser_session_page(&payload, &back_href);
    assert!(html.contains(r#"action" value="link-text""#));
    assert!(html.contains(r#"action" value="link-selector""#));

    let open_text = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![
            ("id".to_owned(), payload.id.clone()),
            ("action".to_owned(), "link-text".to_owned()),
            ("text".to_owned(), "Open target".to_owned()),
        ],
    };
    let (payload, _) = registry.apply_target(&open_text).await.unwrap();
    assert_eq!(payload.title, "Second");
    assert_eq!(payload.history_len, 2);
    assert!(payload.viewport.contains("linked by text or selector"));

    let back = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![
            ("id".to_owned(), payload.id.clone()),
            ("action".to_owned(), "back".to_owned()),
        ],
    };
    let (payload, _) = registry.apply_target(&back).await.unwrap();
    assert_eq!(payload.title, "First");

    let open_selector = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![
            ("id".to_owned(), payload.id),
            ("action".to_owned(), "link-selector".to_owned()),
            ("selector".to_owned(), "#go".to_owned()),
        ],
    };
    let (payload, _) = registry.apply_target(&open_selector).await.unwrap();
    assert_eq!(payload.title, "Second");
    assert_eq!(payload.history_len, 2);
    assert!(payload.viewport.contains("linked by text or selector"));
}

#[tokio::test]
async fn browser_session_registry_opens_links_in_new_sessions() {
    let dir = tempfile::tempdir().unwrap();
    let first = dir.path().join("first.html");
    let second = dir.path().join("second.html");
    std::fs::write(
        &first,
        format!(
            r#"<!doctype html><title>First</title><a href="{}">Open target</a>"#,
            second.display()
        ),
    )
    .unwrap();
    std::fs::write(
        &second,
        r#"<!doctype html><title>Second</title><p>new session target</p>"#,
    )
    .unwrap();

    let registry = BrowserSessionRegistry::default();
    let create = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![
            ("url".to_owned(), first.display().to_string()),
            ("from".to_owned(), "/search?q=links".to_owned()),
        ],
    };
    let (payload, back_href) = registry.create_target(&create).await.unwrap();
    let html = render_browser_session_page(&payload, &back_href);
    assert!(html.contains("New session"));
    assert!(html.contains("link-actions"));
    let first_id = payload.id.clone();
    let new_session_href = payload.links[0].new_session_url.clone();
    assert!(new_session_href.contains("url="));
    assert!(new_session_href.contains("from=%2Fsearch%3Fq%3Dlinks"));

    let create_link_session = RequestTarget {
        path: "/browser".to_owned(),
        params: form_urlencoded::parse(new_session_href.trim_start_matches("/browser?").as_bytes())
            .map(|(key, value)| (key.into_owned(), value.into_owned()))
            .collect(),
    };
    let (payload, _) = registry.create_target(&create_link_session).await.unwrap();
    assert_eq!(payload.title, "Second");
    assert_ne!(payload.id, first_id);
    assert_eq!(payload.sessions.len(), 2);
    assert!(
        payload
            .sessions
            .iter()
            .any(|session| session.id == first_id)
    );
    assert!(
        payload
            .sessions
            .iter()
            .any(|session| session.id == payload.id && session.current)
    );
    assert!(payload.viewport.contains("new session target"));
}

#[tokio::test]
async fn browser_session_registry_opens_address_bar_url_in_new_session() {
    let dir = tempfile::tempdir().unwrap();
    let first = dir.path().join("first.html");
    let second = dir.path().join("second.html");
    std::fs::write(
        &first,
        r#"<!doctype html><title>First</title><p>source tab</p>"#,
    )
    .unwrap();
    std::fs::write(
        &second,
        r#"<!doctype html><title>Second</title><p>new address session</p>"#,
    )
    .unwrap();

    let registry = BrowserSessionRegistry::default();
    let create = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![
            ("url".to_owned(), first.display().to_string()),
            ("from".to_owned(), "/search?q=address".to_owned()),
        ],
    };
    let (payload, back_href) = registry.create_target(&create).await.unwrap();
    let html = render_browser_session_page(&payload, &back_href);
    assert!(html.contains(r#"name="action" value="open-new-session""#));
    assert!(html.contains("New tab"));

    let first_id = payload.id.clone();
    let first_source = payload.source.clone();
    let open_new = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![
            ("id".to_owned(), first_id.clone()),
            ("action".to_owned(), "open-new-session".to_owned()),
            ("url".to_owned(), second.display().to_string()),
        ],
    };
    let (payload, _) = registry.apply_target(&open_new).await.unwrap();
    assert_eq!(payload.title, "Second");
    assert_ne!(payload.id, first_id);
    assert_eq!(payload.sessions.len(), 2);
    assert!(payload.viewport.contains("new address session"));
    assert!(payload.sessions.iter().any(|session| {
        session.id == first_id && session.source == first_source && !session.current
    }));
    assert!(
        payload
            .sessions
            .iter()
            .any(|session| session.id == payload.id && session.current)
    );
}

#[tokio::test]
async fn browser_session_registry_opens_link_text_and_selector_in_new_sessions() {
    let dir = tempfile::tempdir().unwrap();
    let first = dir.path().join("first.html");
    let text_target = dir.path().join("text.html");
    let selector_target = dir.path().join("selector.html");
    std::fs::write(
        &first,
        format!(
            r#"<!doctype html><title>First</title><a id="by-text" href="{}">Text Target</a><a id="by-selector" href="{}">Selector Target</a>"#,
            text_target.display(),
            selector_target.display()
        ),
    )
    .unwrap();
    std::fs::write(
        &text_target,
        r#"<!doctype html><title>Text Target</title><p>new text session</p>"#,
    )
    .unwrap();
    std::fs::write(
        &selector_target,
        r#"<!doctype html><title>Selector Target</title><p>new selector session</p>"#,
    )
    .unwrap();

    let registry = BrowserSessionRegistry::default();
    let create = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![
            ("url".to_owned(), first.display().to_string()),
            ("from".to_owned(), "/search?q=new-link".to_owned()),
        ],
    };
    let (payload, back_href) = registry.create_target(&create).await.unwrap();
    let html = render_browser_session_page(&payload, &back_href);
    assert!(html.contains(r#"action" value="link-text-new-session""#));
    assert!(html.contains(r#"action" value="link-selector-new-session""#));

    let first_id = payload.id.clone();
    let first_source = payload.source.clone();
    let open_text = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![
            ("id".to_owned(), first_id.clone()),
            ("action".to_owned(), "link-text-new-session".to_owned()),
            ("text".to_owned(), "Text Target".to_owned()),
        ],
    };
    let (payload, _) = registry.apply_target(&open_text).await.unwrap();
    let text_id = payload.id.clone();
    assert_eq!(payload.title, "Text Target");
    assert_ne!(payload.id, first_id);
    assert_eq!(payload.sessions.len(), 2);
    assert!(payload.viewport.contains("new text session"));
    assert!(payload.sessions.iter().any(|session| {
        session.id == first_id && session.source == first_source && !session.current
    }));
    assert!(
        payload
            .sessions
            .iter()
            .any(|session| session.id == payload.id && session.current)
    );

    let switch_first = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![
            ("id".to_owned(), first_id.clone()),
            ("action".to_owned(), "current".to_owned()),
        ],
    };
    let (payload, _) = registry.apply_target(&switch_first).await.unwrap();
    assert_eq!(payload.id, first_id);
    assert_eq!(payload.title, "First");

    let open_selector = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![
            ("id".to_owned(), first_id.clone()),
            ("action".to_owned(), "link-selector-new-session".to_owned()),
            ("selector".to_owned(), "#by-selector".to_owned()),
        ],
    };
    let (payload, _) = registry.apply_target(&open_selector).await.unwrap();
    assert_eq!(payload.title, "Selector Target");
    assert_ne!(payload.id, first_id);
    assert_ne!(payload.id, text_id);
    assert_eq!(payload.sessions.len(), 3);
    assert!(payload.viewport.contains("new selector session"));
    assert!(payload.sessions.iter().any(|session| {
        session.id == first_id && session.source == first_source && !session.current
    }));
    assert!(
        payload
            .sessions
            .iter()
            .any(|session| session.id == text_id && !session.current)
    );
    assert!(
        payload
            .sessions
            .iter()
            .any(|session| session.id == payload.id && session.current)
    );
}

#[tokio::test]
async fn browser_session_registry_duplicates_existing_sessions() {
    let dir = tempfile::tempdir().unwrap();
    let first = dir.path().join("first.html");
    let second = dir.path().join("second.html");
    std::fs::write(
        &first,
        format!(
            r#"<!doctype html><title>Original</title><a href="{}">Second</a><p>duplicate me</p>"#,
            second.display()
        ),
    )
    .unwrap();
    std::fs::write(
        &second,
        r#"<!doctype html><title>Second</title><p>duplicate destination</p>"#,
    )
    .unwrap();

    let registry = BrowserSessionRegistry::default();
    let create = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![
            ("url".to_owned(), first.display().to_string()),
            ("from".to_owned(), "/search?q=duplicate".to_owned()),
        ],
    };
    let (payload, back_href) = registry.create_target(&create).await.unwrap();
    let html = render_browser_session_page(&payload, &back_href);
    assert!(html.contains("Duplicate"));

    let original_id = payload.id.clone();
    let follow = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![
            ("id".to_owned(), original_id.clone()),
            ("action".to_owned(), "link".to_owned()),
            ("link".to_owned(), "0".to_owned()),
        ],
    };
    let (payload, _) = registry.apply_target(&follow).await.unwrap();
    assert_eq!(payload.title, "Second");
    assert_eq!(payload.history_len, 2);
    assert!(payload.can_back);

    let duplicate_href = payload
        .sessions
        .iter()
        .find(|session| session.id == original_id)
        .unwrap()
        .duplicate_url
        .clone();
    assert!(duplicate_href.contains("action=duplicate-session"));
    assert!(duplicate_href.contains("session="));
    assert!(duplicate_href.contains("from=%2Fsearch%3Fq%3Dduplicate"));
    let duplicate = RequestTarget {
        path: "/browser".to_owned(),
        params: form_urlencoded::parse(duplicate_href.trim_start_matches("/browser?").as_bytes())
            .map(|(key, value)| (key.into_owned(), value.into_owned()))
            .collect(),
    };
    let (payload, _) = registry.apply_target(&duplicate).await.unwrap();

    assert_eq!(payload.title, "Second");
    assert_ne!(payload.id, original_id);
    assert_eq!(payload.sessions.len(), 2);
    assert_eq!(payload.history_len, 2);
    assert!(payload.can_back);
    assert!(
        payload
            .sessions
            .iter()
            .any(|session| session.id == original_id && !session.current)
    );
    assert!(
        payload
            .sessions
            .iter()
            .any(|session| session.id == payload.id && session.current)
    );
    assert!(payload.viewport.contains("duplicate destination"));
}

#[tokio::test]
async fn browser_session_registry_opens_history_entries_from_inspector() {
    let dir = tempfile::tempdir().unwrap();
    let first = dir.path().join("first.html");
    let second = dir.path().join("second.html");
    let third = dir.path().join("third.html");
    std::fs::write(
        &first,
        r#"<!doctype html><title>First</title><p>first page</p>"#,
    )
    .unwrap();
    std::fs::write(
        &second,
        r#"<!doctype html><title>Second</title><p>second page</p>"#,
    )
    .unwrap();
    std::fs::write(
        &third,
        r#"<!doctype html><title>Third</title><p>third page</p>"#,
    )
    .unwrap();

    let registry = BrowserSessionRegistry::default();
    let create = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![("url".to_owned(), first.display().to_string())],
    };
    let (payload, _) = registry.create_target(&create).await.unwrap();
    let open_second = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![
            ("id".to_owned(), payload.id.clone()),
            ("action".to_owned(), "open".to_owned()),
            ("url".to_owned(), second.display().to_string()),
        ],
    };
    let (payload, _) = registry.apply_target(&open_second).await.unwrap();
    let open_third = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![
            ("id".to_owned(), payload.id.clone()),
            ("action".to_owned(), "open".to_owned()),
            ("url".to_owned(), third.display().to_string()),
        ],
    };
    let (payload, _) = registry.apply_target(&open_third).await.unwrap();
    assert_eq!(payload.title, "Third");
    assert_eq!(payload.history_len, 3);
    assert_eq!(payload.current_history_index, Some(2));

    let first_history_href = payload.history[0].action_url.clone();
    assert!(first_history_href.contains("action=history"));
    let open_first_history = RequestTarget {
        path: "/browser".to_owned(),
        params: form_urlencoded::parse(
            first_history_href
                .trim_start_matches("/browser?")
                .as_bytes(),
        )
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect(),
    };
    let (payload, _) = registry.apply_target(&open_first_history).await.unwrap();
    assert_eq!(payload.title, "First");
    assert_eq!(payload.current_history_index, Some(0));
    assert!(!payload.can_back);
    assert!(payload.can_forward);
    assert!(payload.viewport.contains("first page"));

    let third_history_href = payload.history[2].action_url.clone();
    let open_third_history = RequestTarget {
        path: "/browser".to_owned(),
        params: form_urlencoded::parse(
            third_history_href
                .trim_start_matches("/browser?")
                .as_bytes(),
        )
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect(),
    };
    let (payload, _) = registry.apply_target(&open_third_history).await.unwrap();
    assert_eq!(payload.title, "Third");
    assert_eq!(payload.current_history_index, Some(2));
    assert!(payload.can_back);
    assert!(!payload.can_forward);
    assert!(payload.viewport.contains("third page"));

    let second_history_new_session = payload.history[1].new_session_url.clone();
    assert!(second_history_new_session.contains("url="));
    let open_second_new_session = RequestTarget {
        path: "/browser".to_owned(),
        params: form_urlencoded::parse(
            second_history_new_session
                .trim_start_matches("/browser?")
                .as_bytes(),
        )
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect(),
    };
    let (payload, _) = registry
        .create_target(&open_second_new_session)
        .await
        .unwrap();
    assert_eq!(payload.title, "Second");
    assert_eq!(payload.history_len, 1);
    assert_eq!(payload.sessions.len(), 2);
    assert!(
        payload
            .sessions
            .iter()
            .any(|session| session.title == "Third")
    );
    assert!(payload.viewport.contains("second page"));
}

#[tokio::test]
async fn browser_session_registry_finds_and_cycles_page_text() {
    let dir = tempfile::tempdir().unwrap();
    let page = dir.path().join("find.html");
    std::fs::write(
        &page,
        r#"<!doctype html><title>Find</title><p>intro</p><p>needle first</p><p>middle</p><p>needle second</p><p>tail</p>"#,
    )
    .unwrap();

    let registry = BrowserSessionRegistry::default();
    let create = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![("url".to_owned(), page.display().to_string())],
    };
    let (payload, _) = registry.create_target(&create).await.unwrap();

    let find = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![
            ("id".to_owned(), payload.id.clone()),
            ("action".to_owned(), "find".to_owned()),
            ("q".to_owned(), "needle".to_owned()),
        ],
    };
    let (payload, _) = registry.apply_target(&find).await.unwrap();
    assert_eq!(payload.find_query, "needle");
    assert_eq!(payload.find_match_count, 2);
    assert_eq!(payload.find_current_index, Some(0));
    assert!(payload.viewport.contains("needle first"));
    let html = render_browser_session_page(&payload, "/search?q=find");
    assert!(html.contains("<mark>needle</mark> first"));

    let next = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![
            ("id".to_owned(), payload.id.clone()),
            ("action".to_owned(), "find-next".to_owned()),
        ],
    };
    let (payload, _) = registry.apply_target(&next).await.unwrap();
    assert_eq!(payload.find_match_count, 2);
    assert_eq!(payload.find_current_index, Some(1));
    assert!(payload.viewport.contains("needle second"));

    let previous = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![
            ("id".to_owned(), payload.id.clone()),
            ("action".to_owned(), "find-prev".to_owned()),
        ],
    };
    let (payload, _) = registry.apply_target(&previous).await.unwrap();
    assert_eq!(payload.find_current_index, Some(0));
    assert!(payload.viewport.contains("needle first"));

    let clear = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![
            ("id".to_owned(), payload.id),
            ("action".to_owned(), "clear-find".to_owned()),
        ],
    };
    let (payload, _) = registry.apply_target(&clear).await.unwrap();
    assert!(payload.find_query.is_empty());
    assert_eq!(payload.find_match_count, 0);
    assert_eq!(payload.find_current_index, None);
}

#[tokio::test]
async fn browser_session_registry_scrolls_text_viewport_horizontally() {
    let dir = tempfile::tempdir().unwrap();
    let page = dir.path().join("wide.html");
    std::fs::write(
        &page,
        r#"<!doctype html><title>Wide</title><pre>ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789</pre>"#,
    )
    .unwrap();

    let registry = BrowserSessionRegistry::default();
    let create = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![
            ("url".to_owned(), page.display().to_string()),
            ("width".to_owned(), "40".to_owned()),
            ("height".to_owned(), "16".to_owned()),
        ],
    };
    let (payload, _) = registry.create_target(&create).await.unwrap();
    assert_eq!(payload.viewport_x, 0);
    assert!(payload.max_scroll_x > 0);
    assert!(payload.viewport.contains("ABCDEFGHIJKLMNOPQRSTUVWXYZ"));

    let scroll_right = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![
            ("id".to_owned(), payload.id.clone()),
            ("action".to_owned(), "scroll".to_owned()),
            ("dx".to_owned(), "8".to_owned()),
        ],
    };
    let (payload, back_href) = registry.apply_target(&scroll_right).await.unwrap();
    assert_eq!(payload.viewport_x, 8);
    assert!(payload.viewport.contains("IJKLMNOPQRSTUVWXYZ"));
    assert!(!payload.viewport.contains("ABCDEFGH"));

    let html = render_browser_session_page(&payload, &back_href);
    assert!(html.contains(">Left</a>"));
    assert!(html.contains(">Right</a>"));
    assert!(html.contains("viewport 40x16 at x=8 y=0"));
    assert!(html.contains(r#"name="viewport_x" value="8""#));

    let duplicate_href = browser_session_new_session_href(&payload.source, &payload);
    let duplicate = RequestTarget {
        path: "/browser".to_owned(),
        params: form_urlencoded::parse(duplicate_href.trim_start_matches("/browser?").as_bytes())
            .map(|(key, value)| (key.into_owned(), value.into_owned()))
            .collect(),
    };
    let (duplicate_payload, _) = registry.create_target(&duplicate).await.unwrap();
    assert_ne!(duplicate_payload.id, payload.id);
    assert_eq!(duplicate_payload.viewport_x, 8);
    assert!(duplicate_payload.viewport.contains("IJKLMNOPQRSTUVWXYZ"));

    let scroll_left = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![
            ("id".to_owned(), payload.id),
            ("action".to_owned(), "scroll".to_owned()),
            ("dx".to_owned(), "-8".to_owned()),
        ],
    };
    let (payload, _) = registry.apply_target(&scroll_left).await.unwrap();
    assert_eq!(payload.viewport_x, 0);
}

#[tokio::test]
async fn browser_session_registry_reports_and_switches_open_sessions() {
    let dir = tempfile::tempdir().unwrap();
    let first = dir.path().join("one.html");
    let second = dir.path().join("two.html");
    let third = dir.path().join("three.html");
    std::fs::write(
        &first,
        r#"<!doctype html><title>One</title><p>first session</p>"#,
    )
    .unwrap();
    std::fs::write(
        &second,
        r#"<!doctype html><title>Two</title><p>second session</p>"#,
    )
    .unwrap();
    std::fs::write(
        &third,
        r#"<!doctype html><title>Three</title><p>third session</p>"#,
    )
    .unwrap();

    let registry = BrowserSessionRegistry::default();
    let create_first = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![("url".to_owned(), first.display().to_string())],
    };
    let (payload, _) = registry.create_target(&create_first).await.unwrap();
    let first_id = payload.id.clone();
    assert_eq!(payload.sessions.len(), 1);
    assert_eq!(payload.sessions[0].id, first_id);
    assert!(payload.sessions[0].current);

    let create_second = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![("url".to_owned(), second.display().to_string())],
    };
    let (payload, back_href) = registry.create_target(&create_second).await.unwrap();
    let second_id = payload.id.clone();
    assert_eq!(payload.title, "Two");
    assert_eq!(payload.sessions.len(), 2);
    assert_eq!(payload.sessions[0].id, first_id);
    assert_eq!(payload.sessions[1].id, second_id);
    assert!(payload.sessions[0].can_close);
    assert!(payload.sessions[1].can_close);
    assert!(payload.sessions[0].reload_url.contains("action=reload"));
    assert!(payload.sessions[0].close_url.contains("close-session"));
    assert!(!payload.sessions[0].current);
    assert!(payload.sessions[1].current);
    let html = render_browser_session_page(&payload, &back_href);
    assert!(html.contains(">Duplicate tab</a>"));
    assert!(html.contains(">Close tab</a>"));
    assert!(html.contains(">Prev tab</a>"));
    assert!(html.contains(">Next tab</a>"));
    assert!(html.contains(">Reload</a>"));
    assert!(html.contains("action=close-session"));
    assert!(html.contains("close_id="));

    let first_href = payload.sessions[0].action_url.clone();
    let switch_first = RequestTarget {
        path: "/browser".to_owned(),
        params: form_urlencoded::parse(first_href.trim_start_matches("/browser?").as_bytes())
            .map(|(key, value)| (key.into_owned(), value.into_owned()))
            .collect(),
    };
    let (payload, _) = registry.apply_target(&switch_first).await.unwrap();
    assert_eq!(payload.id, first_id);
    assert_eq!(payload.title, "One");
    assert_eq!(payload.sessions.len(), 2);
    assert!(payload.sessions[0].current);
    assert!(!payload.sessions[1].current);
    assert!(payload.viewport.contains("first session"));

    let create_third = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![("url".to_owned(), third.display().to_string())],
    };
    let (payload, _) = registry.create_target(&create_third).await.unwrap();
    let third_id = payload.id.clone();
    assert_eq!(payload.sessions.len(), 3);
    assert!(payload.sessions[2].current);

    let close_second_href = payload.sessions[1].close_url.clone();
    let close_second = RequestTarget {
        path: "/browser".to_owned(),
        params: form_urlencoded::parse(
            close_second_href.trim_start_matches("/browser?").as_bytes(),
        )
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect(),
    };
    let (payload, _) = registry.apply_target(&close_second).await.unwrap();
    assert_eq!(payload.id, third_id);
    assert_eq!(payload.title, "Three");
    assert_eq!(payload.sessions.len(), 2);
    assert!(
        payload
            .sessions
            .iter()
            .all(|session| session.id != second_id)
    );
    assert!(payload.sessions[1].current);

    let close_current_href = payload.sessions[1].close_url.clone();
    let close_current = RequestTarget {
        path: "/browser".to_owned(),
        params: form_urlencoded::parse(
            close_current_href
                .trim_start_matches("/browser?")
                .as_bytes(),
        )
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect(),
    };
    let (payload, _) = registry.apply_target(&close_current).await.unwrap();
    assert_eq!(payload.id, first_id);
    assert_eq!(payload.title, "One");
    assert_eq!(payload.sessions.len(), 1);
    assert!(payload.sessions[0].current);
    assert!(!payload.sessions[0].can_close);
}

#[tokio::test]
async fn browser_session_registry_reloads_background_sessions_from_tab_card() {
    let dir = tempfile::tempdir().unwrap();
    let first = dir.path().join("one.html");
    let second = dir.path().join("two.html");
    std::fs::write(
        &first,
        r#"<!doctype html><title>Old one</title><p>stale first tab</p>"#,
    )
    .unwrap();
    std::fs::write(
        &second,
        r#"<!doctype html><title>Two</title><p>second tab</p>"#,
    )
    .unwrap();

    let registry = BrowserSessionRegistry::default();
    let create_first = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![("url".to_owned(), first.display().to_string())],
    };
    let (payload, _) = registry.create_target(&create_first).await.unwrap();
    let first_id = payload.id.clone();

    let create_second = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![("url".to_owned(), second.display().to_string())],
    };
    let (payload, _) = registry.create_target(&create_second).await.unwrap();
    assert_eq!(payload.title, "Two");
    assert_eq!(payload.sessions.len(), 2);
    assert_eq!(payload.sessions[0].id, first_id);
    assert!(!payload.sessions[0].current);
    assert!(payload.sessions[1].current);

    std::fs::write(
        &first,
        r#"<!doctype html><title>Fresh one</title><p>fresh first tab</p>"#,
    )
    .unwrap();

    let reload_first = RequestTarget {
        path: "/browser".to_owned(),
        params: form_urlencoded::parse(
            payload.sessions[0]
                .reload_url
                .trim_start_matches("/browser?")
                .as_bytes(),
        )
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect(),
    };
    let (payload, _) = registry.apply_target(&reload_first).await.unwrap();
    assert_eq!(payload.id, first_id);
    assert_eq!(payload.title, "Fresh one");
    assert!(payload.viewport.contains("fresh first tab"));
    assert_eq!(payload.sessions.len(), 2);
    assert!(payload.sessions[0].current);
    assert!(!payload.sessions[1].current);
}

#[tokio::test]
async fn browser_session_registry_cycles_open_sessions_from_toolbar() {
    let dir = tempfile::tempdir().unwrap();
    let first = dir.path().join("one.html");
    let second = dir.path().join("two.html");
    let third = dir.path().join("three.html");
    std::fs::write(
        &first,
        r#"<!doctype html><title>One</title><p>first tab</p>"#,
    )
    .unwrap();
    std::fs::write(
        &second,
        r#"<!doctype html><title>Two</title><p>second tab</p>"#,
    )
    .unwrap();
    std::fs::write(
        &third,
        r#"<!doctype html><title>Three</title><p>third tab</p>"#,
    )
    .unwrap();

    let registry = BrowserSessionRegistry::default();
    let create_first = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![("url".to_owned(), first.display().to_string())],
    };
    let (payload, _) = registry.create_target(&create_first).await.unwrap();
    let first_id = payload.id.clone();

    let create_second = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![("url".to_owned(), second.display().to_string())],
    };
    let (payload, _) = registry.create_target(&create_second).await.unwrap();
    let second_id = payload.id.clone();

    let create_third = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![("url".to_owned(), third.display().to_string())],
    };
    let (payload, _) = registry.create_target(&create_third).await.unwrap();
    let third_id = payload.id.clone();
    assert_eq!(payload.sessions.len(), 3);
    assert_eq!(payload.id, third_id);

    let next_from_third = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![
            ("id".to_owned(), third_id.clone()),
            ("action".to_owned(), "next-tab".to_owned()),
        ],
    };
    let (payload, _) = registry.apply_target(&next_from_third).await.unwrap();
    assert_eq!(payload.id, first_id);
    assert_eq!(payload.title, "One");
    assert!(payload.sessions[0].current);
    assert!(!payload.sessions[1].current);
    assert!(!payload.sessions[2].current);

    let previous_from_first = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![
            ("id".to_owned(), first_id.clone()),
            ("action".to_owned(), "previous-tab".to_owned()),
        ],
    };
    let (payload, _) = registry.apply_target(&previous_from_first).await.unwrap();
    assert_eq!(payload.id, third_id);
    assert_eq!(payload.title, "Three");
    assert!(!payload.sessions[0].current);
    assert!(!payload.sessions[1].current);
    assert!(payload.sessions[2].current);

    let previous_from_third = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![
            ("id".to_owned(), third_id),
            ("action".to_owned(), "prev-tab".to_owned()),
        ],
    };
    let (payload, _) = registry.apply_target(&previous_from_third).await.unwrap();
    assert_eq!(payload.id, second_id);
    assert_eq!(payload.title, "Two");
    assert!(!payload.sessions[0].current);
    assert!(payload.sessions[1].current);
    assert!(!payload.sessions[2].current);
}

#[tokio::test]
async fn browser_session_registry_closes_other_open_sessions() {
    let dir = tempfile::tempdir().unwrap();
    let first = dir.path().join("one.html");
    let second = dir.path().join("two.html");
    let third = dir.path().join("three.html");
    std::fs::write(
        &first,
        r#"<!doctype html><title>One</title><p>first tab</p>"#,
    )
    .unwrap();
    std::fs::write(
        &second,
        r#"<!doctype html><title>Two</title><p>second tab</p>"#,
    )
    .unwrap();
    std::fs::write(
        &third,
        r#"<!doctype html><title>Three</title><p>active tab</p>"#,
    )
    .unwrap();

    let registry = BrowserSessionRegistry::default();
    for page in [&first, &second, &third] {
        let create = RequestTarget {
            path: "/browser".to_owned(),
            params: vec![("url".to_owned(), page.display().to_string())],
        };
        registry.create_target(&create).await.unwrap();
    }

    let current = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![
            ("id".to_owned(), "s3".to_owned()),
            ("action".to_owned(), "current".to_owned()),
        ],
    };
    let (payload, back_href) = registry.apply_target(&current).await.unwrap();
    assert_eq!(payload.title, "Three");
    assert_eq!(payload.sessions.len(), 3);
    let html = render_browser_session_page(&payload, &back_href);
    assert!(html.contains(">Close others</a>"));
    assert!(html.contains("action=close-other-tabs"));

    let close_others = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![
            ("id".to_owned(), payload.id),
            ("action".to_owned(), "close-other-tabs".to_owned()),
        ],
    };
    let (payload, _) = registry.apply_target(&close_others).await.unwrap();
    assert_eq!(payload.title, "Three");
    assert_eq!(payload.sessions.len(), 1);
    assert_eq!(payload.sessions[0].id, "s3");
    assert!(payload.sessions[0].current);
    assert!(!payload.sessions[0].can_close);
    assert_eq!(payload.closed_sessions.len(), 2);
    assert!(
        payload
            .closed_sessions
            .iter()
            .any(|closed| closed.title == "One")
    );
    assert!(
        payload
            .closed_sessions
            .iter()
            .any(|closed| closed.title == "Two")
    );
}

#[tokio::test]
async fn browser_session_registry_closes_open_sessions_to_the_right() {
    let dir = tempfile::tempdir().unwrap();
    let first = dir.path().join("one.html");
    let second = dir.path().join("two.html");
    let third = dir.path().join("three.html");
    std::fs::write(
        &first,
        r#"<!doctype html><title>One</title><p>left tab</p>"#,
    )
    .unwrap();
    std::fs::write(
        &second,
        r#"<!doctype html><title>Two</title><p>active middle tab</p>"#,
    )
    .unwrap();
    std::fs::write(
        &third,
        r#"<!doctype html><title>Three</title><p>right tab</p>"#,
    )
    .unwrap();

    let registry = BrowserSessionRegistry::default();
    for page in [&first, &second, &third] {
        let create = RequestTarget {
            path: "/browser".to_owned(),
            params: vec![("url".to_owned(), page.display().to_string())],
        };
        registry.create_target(&create).await.unwrap();
    }

    let switch_second = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![
            ("id".to_owned(), "s2".to_owned()),
            ("action".to_owned(), "current".to_owned()),
        ],
    };
    let (payload, back_href) = registry.apply_target(&switch_second).await.unwrap();
    assert_eq!(payload.title, "Two");
    assert_eq!(payload.sessions.len(), 3);
    let html = render_browser_session_page(&payload, &back_href);
    assert!(html.contains(">Close right</a>"));
    assert!(html.contains("action=close-tabs-right"));

    let close_right = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![
            ("id".to_owned(), payload.id),
            ("action".to_owned(), "close-tabs-right".to_owned()),
        ],
    };
    let (payload, _) = registry.apply_target(&close_right).await.unwrap();
    assert_eq!(payload.title, "Two");
    assert_eq!(payload.sessions.len(), 2);
    assert!(payload.sessions.iter().any(|session| session.id == "s1"));
    assert!(payload.sessions.iter().any(|session| session.id == "s2"));
    assert!(payload.sessions.iter().all(|session| session.id != "s3"));
    assert_eq!(payload.closed_sessions.len(), 1);
    assert_eq!(payload.closed_sessions[0].title, "Three");
}

#[tokio::test]
async fn browser_session_registry_closes_open_sessions_to_the_left() {
    let dir = tempfile::tempdir().unwrap();
    let first = dir.path().join("one.html");
    let second = dir.path().join("two.html");
    let third = dir.path().join("three.html");
    std::fs::write(
        &first,
        r#"<!doctype html><title>One</title><p>left tab</p>"#,
    )
    .unwrap();
    std::fs::write(
        &second,
        r#"<!doctype html><title>Two</title><p>active middle tab</p>"#,
    )
    .unwrap();
    std::fs::write(
        &third,
        r#"<!doctype html><title>Three</title><p>right tab</p>"#,
    )
    .unwrap();

    let registry = BrowserSessionRegistry::default();
    for page in [&first, &second, &third] {
        let create = RequestTarget {
            path: "/browser".to_owned(),
            params: vec![("url".to_owned(), page.display().to_string())],
        };
        registry.create_target(&create).await.unwrap();
    }

    let switch_second = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![
            ("id".to_owned(), "s2".to_owned()),
            ("action".to_owned(), "current".to_owned()),
        ],
    };
    let (payload, back_href) = registry.apply_target(&switch_second).await.unwrap();
    assert_eq!(payload.title, "Two");
    assert_eq!(payload.sessions.len(), 3);
    let html = render_browser_session_page(&payload, &back_href);
    assert!(html.contains(">Close left</a>"));
    assert!(html.contains("action=close-tabs-left"));

    let close_left = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![
            ("id".to_owned(), payload.id),
            ("action".to_owned(), "close-tabs-left".to_owned()),
        ],
    };
    let (payload, _) = registry.apply_target(&close_left).await.unwrap();
    assert_eq!(payload.title, "Two");
    assert_eq!(payload.sessions.len(), 2);
    assert!(payload.sessions.iter().any(|session| session.id == "s2"));
    assert!(payload.sessions.iter().any(|session| session.id == "s3"));
    assert!(payload.sessions.iter().all(|session| session.id != "s1"));
    assert_eq!(payload.closed_sessions.len(), 1);
    assert_eq!(payload.closed_sessions[0].title, "One");
}

#[tokio::test]
async fn browser_session_registry_closes_duplicate_open_sessions() {
    let dir = tempfile::tempdir().unwrap();
    let first = dir.path().join("duplicate.html");
    let second = dir.path().join("unique.html");
    std::fs::write(
        &first,
        r#"<!doctype html><title>Duplicate</title><p>same page</p>"#,
    )
    .unwrap();
    std::fs::write(
        &second,
        r#"<!doctype html><title>Unique</title><p>different page</p>"#,
    )
    .unwrap();

    let registry = BrowserSessionRegistry::default();
    for page in [&first, &second, &first] {
        let create = RequestTarget {
            path: "/browser".to_owned(),
            params: vec![("url".to_owned(), page.display().to_string())],
        };
        registry.create_target(&create).await.unwrap();
    }

    let switch_first_duplicate = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![
            ("id".to_owned(), "s1".to_owned()),
            ("action".to_owned(), "current".to_owned()),
        ],
    };
    let (payload, back_href) = registry
        .apply_target(&switch_first_duplicate)
        .await
        .unwrap();
    assert_eq!(payload.title, "Duplicate");
    assert_eq!(payload.sessions.len(), 3);
    let html = render_browser_session_page(&payload, &back_href);
    assert!(html.contains(">Close duplicates</a>"));
    assert!(html.contains("action=close-duplicate-tabs"));

    let close_duplicates = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![
            ("id".to_owned(), payload.id),
            ("action".to_owned(), "close-duplicate-tabs".to_owned()),
        ],
    };
    let (payload, _) = registry.apply_target(&close_duplicates).await.unwrap();
    assert_eq!(payload.title, "Duplicate");
    assert_eq!(payload.sessions.len(), 2);
    assert!(payload.sessions.iter().any(|session| session.id == "s1"));
    assert!(payload.sessions.iter().any(|session| session.id == "s2"));
    assert!(payload.sessions.iter().all(|session| session.id != "s3"));
    assert_eq!(payload.closed_sessions.len(), 1);
    assert_eq!(payload.closed_sessions[0].title, "Duplicate");
}

#[tokio::test]
async fn browser_session_registry_restores_recently_closed_sessions() {
    let dir = tempfile::tempdir().unwrap();
    let first = dir.path().join("first.html");
    let second = dir.path().join("second.html");
    std::fs::write(
        &first,
        r#"<!doctype html><title>First Closed</title><p>restore me</p>"#,
    )
    .unwrap();
    std::fs::write(
        &second,
        r#"<!doctype html><title>Second Active</title><p>stay active</p>"#,
    )
    .unwrap();

    let registry = BrowserSessionRegistry::default();
    let create_first = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![("url".to_owned(), first.display().to_string())],
    };
    let (payload, _) = registry.create_target(&create_first).await.unwrap();
    let first_id = payload.id.clone();

    let create_second = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![("url".to_owned(), second.display().to_string())],
    };
    let (payload, _) = registry.create_target(&create_second).await.unwrap();
    let second_id = payload.id.clone();
    assert_eq!(payload.sessions.len(), 2);

    let close_first_href = payload
        .sessions
        .iter()
        .find(|session| session.id == first_id)
        .unwrap()
        .close_url
        .clone();
    let close_first = RequestTarget {
        path: "/browser".to_owned(),
        params: form_urlencoded::parse(close_first_href.trim_start_matches("/browser?").as_bytes())
            .map(|(key, value)| (key.into_owned(), value.into_owned()))
            .collect(),
    };
    let (payload, back_href) = registry.apply_target(&close_first).await.unwrap();
    assert_eq!(payload.id, second_id);
    assert_eq!(payload.title, "Second Active");
    assert_eq!(payload.sessions.len(), 1);
    assert_eq!(payload.closed_sessions.len(), 1);
    assert_eq!(payload.closed_sessions[0].id, first_id);
    assert_eq!(payload.closed_sessions[0].title, "First Closed");
    assert!(payload.closed_sessions[0].source.ends_with("first.html"));
    assert!(
        payload.closed_sessions[0]
            .restore_url
            .contains("action=restore-closed")
    );
    assert!(
        payload.closed_sessions[0]
            .forget_url
            .contains("action=forget-closed")
    );

    let html = render_browser_session_page(&payload, &back_href);
    assert!(html.contains("<h2>Recently closed</h2>"));
    assert!(html.contains(">Restore tab</a>"));
    assert!(html.contains(">Restore</a>"));
    assert!(html.contains(">Forget</a>"));

    let restore_href = payload.closed_sessions[0].restore_url.clone();
    let restore = RequestTarget {
        path: "/browser".to_owned(),
        params: form_urlencoded::parse(restore_href.trim_start_matches("/browser?").as_bytes())
            .map(|(key, value)| (key.into_owned(), value.into_owned()))
            .collect(),
    };
    let (payload, _) = registry.apply_target(&restore).await.unwrap();
    assert_eq!(payload.title, "First Closed");
    assert_ne!(payload.id, first_id);
    assert_eq!(payload.sessions.len(), 2);
    assert!(payload.closed_sessions.is_empty());
    assert_eq!(payload.history_len, 1);
    assert!(payload.viewport.contains("restore me"));
}

#[tokio::test]
async fn browser_session_registry_forgets_recently_closed_sessions() {
    let dir = tempfile::tempdir().unwrap();
    let first = dir.path().join("first.html");
    let second = dir.path().join("second.html");
    std::fs::write(
        &first,
        r#"<!doctype html><title>Forget Closed</title><p>forget me</p>"#,
    )
    .unwrap();
    std::fs::write(
        &second,
        r#"<!doctype html><title>Still Open</title><p>active tab remains</p>"#,
    )
    .unwrap();

    let registry = BrowserSessionRegistry::default();
    let create_first = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![("url".to_owned(), first.display().to_string())],
    };
    let (payload, _) = registry.create_target(&create_first).await.unwrap();
    let first_id = payload.id.clone();

    let create_second = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![("url".to_owned(), second.display().to_string())],
    };
    let (payload, _) = registry.create_target(&create_second).await.unwrap();
    let second_id = payload.id.clone();

    let close_first_href = payload
        .sessions
        .iter()
        .find(|session| session.id == first_id)
        .unwrap()
        .close_url
        .clone();
    let close_first = RequestTarget {
        path: "/browser".to_owned(),
        params: form_urlencoded::parse(close_first_href.trim_start_matches("/browser?").as_bytes())
            .map(|(key, value)| (key.into_owned(), value.into_owned()))
            .collect(),
    };
    let (payload, _) = registry.apply_target(&close_first).await.unwrap();
    assert_eq!(payload.id, second_id);
    assert_eq!(payload.closed_sessions.len(), 1);
    assert_eq!(payload.closed_sessions[0].id, first_id);
    assert_eq!(payload.closed_sessions[0].title, "Forget Closed");

    let forget = RequestTarget {
        path: "/browser".to_owned(),
        params: form_urlencoded::parse(
            payload.closed_sessions[0]
                .forget_url
                .trim_start_matches("/browser?")
                .as_bytes(),
        )
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect(),
    };
    let (payload, _) = registry.apply_target(&forget).await.unwrap();
    assert_eq!(payload.id, second_id);
    assert_eq!(payload.title, "Still Open");
    assert_eq!(payload.sessions.len(), 1);
    assert!(payload.closed_sessions.is_empty());
    assert!(payload.viewport.contains("active tab remains"));
}

#[tokio::test]
async fn browser_session_registry_persists_recently_closed_pages() {
    let dir = tempfile::tempdir().unwrap();
    let profile = dir.path().join("browser-profile.json");
    let first = dir.path().join("first.html");
    let second = dir.path().join("second.html");
    std::fs::write(
        &first,
        r#"<!doctype html><title>Persist Closed</title><p>closed page survived restart</p>"#,
    )
    .unwrap();
    std::fs::write(
        &second,
        r#"<!doctype html><title>Active After Close</title><p>active after close</p>"#,
    )
    .unwrap();

    let registry = BrowserSessionRegistry::with_profile_path(profile.clone());
    let create_first = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![("url".to_owned(), first.display().to_string())],
    };
    let (payload, _) = registry.create_target(&create_first).await.unwrap();
    let first_id = payload.id.clone();
    let create_second = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![("url".to_owned(), second.display().to_string())],
    };
    let (payload, _) = registry.create_target(&create_second).await.unwrap();

    let close_first_href = payload
        .sessions
        .iter()
        .find(|session| session.id == first_id)
        .unwrap()
        .close_url
        .clone();
    let close_first = RequestTarget {
        path: "/browser".to_owned(),
        params: form_urlencoded::parse(close_first_href.trim_start_matches("/browser?").as_bytes())
            .map(|(key, value)| (key.into_owned(), value.into_owned()))
            .collect(),
    };
    let (payload, _) = registry.apply_target(&close_first).await.unwrap();
    assert_eq!(payload.title, "Active After Close");
    assert_eq!(payload.closed_sessions.len(), 1);
    assert_eq!(payload.closed_sessions[0].title, "Persist Closed");
    assert!(!payload.closed_sessions[0].persisted);
    let saved = load_browser_session_profile(&profile).unwrap();
    assert_eq!(saved.closed.len(), 1);
    assert_eq!(saved.closed[0].title, "Persist Closed");
    assert!(saved.closed[0].closed_at_unix_secs > 0);
    drop(registry);

    let registry = BrowserSessionRegistry::with_profile_path(profile.clone());
    let create_active = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![("url".to_owned(), second.display().to_string())],
    };
    let (payload, back_href) = registry.create_target(&create_active).await.unwrap();
    assert_eq!(payload.title, "Active After Close");
    assert_eq!(payload.closed_sessions.len(), 1);
    assert_eq!(payload.closed_sessions[0].id, "p1");
    assert_eq!(payload.closed_sessions[0].title, "Persist Closed");
    assert!(payload.closed_sessions[0].persisted);
    assert!(payload.closed_sessions[0].closed_at.contains("UTC"));
    assert!(
        payload.closed_sessions[0]
            .restore_url
            .contains("action=open-profile-closed")
    );
    assert!(
        payload.closed_sessions[0]
            .forget_url
            .contains("action=forget-profile-closed")
    );
    let html = render_browser_session_page(&payload, &back_href);
    assert!(html.contains("<h2>Recently closed</h2>"));
    assert!(html.contains("saved"));
    assert!(html.contains(">Restore tab</a>"));
    assert!(html.contains(">Forget</a>"));

    let restore = RequestTarget {
        path: "/browser".to_owned(),
        params: form_urlencoded::parse(
            payload.closed_sessions[0]
                .restore_url
                .trim_start_matches("/browser?")
                .as_bytes(),
        )
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect(),
    };
    let (payload, _) = registry.apply_target(&restore).await.unwrap();
    assert_eq!(payload.title, "Persist Closed");
    assert!(payload.closed_sessions.is_empty());
    assert_eq!(payload.history_len, 2);
    assert!(payload.viewport.contains("closed page survived restart"));
    assert!(
        load_browser_session_profile(&profile)
            .unwrap()
            .closed
            .is_empty()
    );
}

#[tokio::test]
async fn browser_session_registry_forgets_persisted_recently_closed_pages() {
    let dir = tempfile::tempdir().unwrap();
    let profile = dir.path().join("browser-profile.json");
    let first = dir.path().join("first.html");
    let second = dir.path().join("second.html");
    std::fs::write(
        &first,
        r#"<!doctype html><title>Persisted Forget</title><p>forget persisted closed page</p>"#,
    )
    .unwrap();
    std::fs::write(
        &second,
        r#"<!doctype html><title>Forget Active</title><p>active tab remains after persisted forget</p>"#,
    )
    .unwrap();

    let registry = BrowserSessionRegistry::with_profile_path(profile.clone());
    let create_first = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![("url".to_owned(), first.display().to_string())],
    };
    let (payload, _) = registry.create_target(&create_first).await.unwrap();
    let first_id = payload.id.clone();
    let create_second = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![("url".to_owned(), second.display().to_string())],
    };
    let (payload, _) = registry.create_target(&create_second).await.unwrap();
    let close_first_href = payload
        .sessions
        .iter()
        .find(|session| session.id == first_id)
        .unwrap()
        .close_url
        .clone();
    let close_first = RequestTarget {
        path: "/browser".to_owned(),
        params: form_urlencoded::parse(close_first_href.trim_start_matches("/browser?").as_bytes())
            .map(|(key, value)| (key.into_owned(), value.into_owned()))
            .collect(),
    };
    registry.apply_target(&close_first).await.unwrap();
    assert_eq!(
        load_browser_session_profile(&profile).unwrap().closed.len(),
        1
    );
    drop(registry);

    let registry = BrowserSessionRegistry::with_profile_path(profile.clone());
    let create_active = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![("url".to_owned(), second.display().to_string())],
    };
    let (payload, _) = registry.create_target(&create_active).await.unwrap();
    assert_eq!(payload.title, "Forget Active");
    assert_eq!(payload.closed_sessions.len(), 1);
    assert!(payload.closed_sessions[0].persisted);
    assert_eq!(payload.closed_sessions[0].title, "Persisted Forget");

    let forget = RequestTarget {
        path: "/browser".to_owned(),
        params: form_urlencoded::parse(
            payload.closed_sessions[0]
                .forget_url
                .trim_start_matches("/browser?")
                .as_bytes(),
        )
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect(),
    };
    let (payload, _) = registry.apply_target(&forget).await.unwrap();
    assert_eq!(payload.title, "Forget Active");
    assert!(payload.closed_sessions.is_empty());
    assert!(payload.viewport.contains("active tab remains"));
    assert!(
        load_browser_session_profile(&profile)
            .unwrap()
            .closed
            .is_empty()
    );
}

#[tokio::test]
async fn browser_session_registry_clears_recently_closed_pages() {
    let dir = tempfile::tempdir().unwrap();
    let profile = dir.path().join("browser-profile.json");
    let first = dir.path().join("first.html");
    let second = dir.path().join("second.html");
    std::fs::write(
        &first,
        r#"<!doctype html><title>Closed Clear</title><p>clear this closed page</p>"#,
    )
    .unwrap();
    std::fs::write(
        &second,
        r#"<!doctype html><title>Clear Active</title><p>active remains open</p>"#,
    )
    .unwrap();

    let registry = BrowserSessionRegistry::with_profile_path(profile.clone());
    let create_first = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![("url".to_owned(), first.display().to_string())],
    };
    let (payload, _) = registry.create_target(&create_first).await.unwrap();
    let first_id = payload.id.clone();
    let create_second = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![("url".to_owned(), second.display().to_string())],
    };
    let (payload, _) = registry.create_target(&create_second).await.unwrap();

    let close_first_href = payload
        .sessions
        .iter()
        .find(|session| session.id == first_id)
        .unwrap()
        .close_url
        .clone();
    let close_first = RequestTarget {
        path: "/browser".to_owned(),
        params: form_urlencoded::parse(close_first_href.trim_start_matches("/browser?").as_bytes())
            .map(|(key, value)| (key.into_owned(), value.into_owned()))
            .collect(),
    };
    let (payload, back_href) = registry.apply_target(&close_first).await.unwrap();
    assert_eq!(payload.title, "Clear Active");
    assert_eq!(payload.closed_sessions.len(), 1);
    assert!(payload.closed_sessions_clear_url.is_some());
    assert_eq!(
        load_browser_session_profile(&profile).unwrap().closed.len(),
        1
    );
    let html = render_browser_session_page(&payload, &back_href);
    assert!(html.contains("<h2>Recently closed</h2>"));
    assert!(html.contains("action=clear-closed"));

    let clear_href = payload.closed_sessions_clear_url.clone().unwrap();
    let clear = RequestTarget {
        path: "/browser".to_owned(),
        params: form_urlencoded::parse(clear_href.trim_start_matches("/browser?").as_bytes())
            .map(|(key, value)| (key.into_owned(), value.into_owned()))
            .collect(),
    };
    let (payload, back_href) = registry.apply_target(&clear).await.unwrap();
    assert_eq!(payload.title, "Clear Active");
    assert_eq!(payload.sessions.len(), 1);
    assert!(payload.closed_sessions.is_empty());
    assert!(
        load_browser_session_profile(&profile)
            .unwrap()
            .closed
            .is_empty()
    );
    let html = render_browser_session_page(&payload, &back_href);
    assert!(!html.contains("<h2>Recently closed</h2>"));
    assert!(!html.contains("clear this closed page"));
}

#[tokio::test]
async fn browser_session_registry_bookmarks_current_pages() {
    let dir = tempfile::tempdir().unwrap();
    let first = dir.path().join("first.html");
    let second = dir.path().join("second.html");
    std::fs::write(
        &first,
        r#"<!doctype html><title>First Bookmark</title><p>first saved page</p>"#,
    )
    .unwrap();
    std::fs::write(
        &second,
        r#"<!doctype html><title>Second Page</title><p>second page</p>"#,
    )
    .unwrap();

    let registry = BrowserSessionRegistry::default();
    let create = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![("url".to_owned(), first.display().to_string())],
    };
    let (payload, back_href) = registry.create_target(&create).await.unwrap();
    assert!(!payload.current_bookmarked);
    assert!(payload.bookmarks.is_empty());
    let html = render_browser_session_page(&payload, &back_href);
    assert!(html.contains("<h2>Bookmarks</h2>"));
    assert!(html.contains("Add bookmark"));

    let add = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![
            ("id".to_owned(), payload.id.clone()),
            ("action".to_owned(), "add-bookmark".to_owned()),
        ],
    };
    let (payload, back_href) = registry.apply_target(&add).await.unwrap();
    assert!(payload.current_bookmarked);
    assert_eq!(payload.bookmarks.len(), 1);
    assert_eq!(payload.bookmarks[0].title, "First Bookmark");
    assert!(payload.bookmarks[0].source.ends_with("first.html"));
    assert!(payload.bookmarks[0].current);
    assert!(
        payload
            .bookmarks_clear_url
            .as_deref()
            .is_some_and(|href| href.contains("action=clear-bookmarks"))
    );
    let html = render_browser_session_page(&payload, &back_href);
    assert!(html.contains("Bookmarked"));
    assert!(html.contains("remove-bookmark"));
    assert!(html.contains("clear-bookmarks"));

    let open_second = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![
            ("id".to_owned(), payload.id.clone()),
            ("action".to_owned(), "open".to_owned()),
            ("url".to_owned(), second.display().to_string()),
        ],
    };
    let (payload, _) = registry.apply_target(&open_second).await.unwrap();
    assert_eq!(payload.title, "Second Page");
    assert!(!payload.current_bookmarked);
    assert_eq!(payload.bookmarks.len(), 1);
    assert!(!payload.bookmarks[0].current);

    let open_bookmark = RequestTarget {
        path: "/browser".to_owned(),
        params: form_urlencoded::parse(
            payload.bookmarks[0]
                .action_url
                .trim_start_matches("/browser?")
                .as_bytes(),
        )
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect(),
    };
    let (payload, _) = registry.apply_target(&open_bookmark).await.unwrap();
    assert_eq!(payload.title, "First Bookmark");
    assert!(payload.current_bookmarked);
    assert_eq!(payload.history_len, 3);
    assert!(payload.viewport.contains("first saved page"));

    let remove_bookmark = RequestTarget {
        path: "/browser".to_owned(),
        params: form_urlencoded::parse(
            payload.bookmarks[0]
                .remove_url
                .trim_start_matches("/browser?")
                .as_bytes(),
        )
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect(),
    };
    let (payload, _) = registry.apply_target(&remove_bookmark).await.unwrap();
    assert!(!payload.current_bookmarked);
    assert!(payload.bookmarks.is_empty());
}

#[tokio::test]
async fn browser_session_registry_clears_bookmarks() {
    let dir = tempfile::tempdir().unwrap();
    let first = dir.path().join("first.html");
    let second = dir.path().join("second.html");
    std::fs::write(
        &first,
        r#"<!doctype html><title>Clear First</title><p>first bookmark</p>"#,
    )
    .unwrap();
    std::fs::write(
        &second,
        r#"<!doctype html><title>Clear Second</title><p>second bookmark</p>"#,
    )
    .unwrap();

    let registry = BrowserSessionRegistry::default();
    let create = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![("url".to_owned(), first.display().to_string())],
    };
    let (payload, _) = registry.create_target(&create).await.unwrap();
    let add_first = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![
            ("id".to_owned(), payload.id.clone()),
            ("action".to_owned(), "add-bookmark".to_owned()),
        ],
    };
    let (payload, _) = registry.apply_target(&add_first).await.unwrap();

    let open_second = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![
            ("id".to_owned(), payload.id.clone()),
            ("action".to_owned(), "open".to_owned()),
            ("url".to_owned(), second.display().to_string()),
        ],
    };
    let (payload, _) = registry.apply_target(&open_second).await.unwrap();
    let add_second = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![
            ("id".to_owned(), payload.id.clone()),
            ("action".to_owned(), "add-bookmark".to_owned()),
        ],
    };
    let (payload, back_href) = registry.apply_target(&add_second).await.unwrap();
    assert!(payload.current_bookmarked);
    assert_eq!(payload.bookmarks.len(), 2);
    let html = render_browser_session_page(&payload, &back_href);
    assert!(html.contains("2 saved"));
    assert!(html.contains("action=clear-bookmarks"));

    let clear_href = payload.bookmarks_clear_url.clone().unwrap();
    let clear = RequestTarget {
        path: "/browser".to_owned(),
        params: form_urlencoded::parse(clear_href.trim_start_matches("/browser?").as_bytes())
            .map(|(key, value)| (key.into_owned(), value.into_owned()))
            .collect(),
    };
    let (payload, _) = registry.apply_target(&clear).await.unwrap();
    assert!(!payload.current_bookmarked);
    assert!(payload.bookmarks.is_empty());
    assert!(payload.bookmarks_clear_url.is_none());
    assert_eq!(payload.title, "Clear Second");
    assert!(payload.viewport.contains("second bookmark"));
}

#[tokio::test]
async fn browser_session_registry_persists_profile_bookmarks_and_history() {
    let dir = tempfile::tempdir().unwrap();
    let profile = dir.path().join("browser-profile.json");
    let first = dir.path().join("first.html");
    let second = dir.path().join("second.html");
    std::fs::write(
        &first,
        r#"<!doctype html><title>Persist One</title><p>saved across registries</p>"#,
    )
    .unwrap();
    std::fs::write(
        &second,
        r#"<!doctype html><title>Persist Two</title><p>second profile page</p>"#,
    )
    .unwrap();

    let registry = BrowserSessionRegistry::with_profile_path(profile.clone());
    let create_first = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![("url".to_owned(), first.display().to_string())],
    };
    let (payload, _) = registry.create_target(&create_first).await.unwrap();
    assert!(payload.profile_enabled);
    assert!(payload.profile_error.is_none());
    assert_eq!(payload.profile_history.len(), 1);
    assert_eq!(payload.profile_history[0].title, "Persist One");
    assert!(payload.profile_history[0].visited_at_unix_secs > 0);
    assert!(payload.profile_history[0].visited_at.contains("UTC"));

    let add_bookmark = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![
            ("id".to_owned(), payload.id),
            ("action".to_owned(), "add-bookmark".to_owned()),
        ],
    };
    let (payload, _) = registry.apply_target(&add_bookmark).await.unwrap();
    assert_eq!(payload.bookmarks.len(), 1);
    assert!(
        std::fs::read_to_string(&profile)
            .unwrap()
            .contains("Persist One")
    );
    drop(registry);

    let registry = BrowserSessionRegistry::with_profile_path(profile);
    let create_second = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![("url".to_owned(), second.display().to_string())],
    };
    let (payload, back_href) = registry.create_target(&create_second).await.unwrap();
    assert_eq!(payload.title, "Persist Two");
    assert_eq!(payload.bookmarks.len(), 1);
    assert_eq!(payload.bookmarks[0].title, "Persist One");
    assert!(payload.bookmarks[0].source.ends_with("first.html"));
    assert_eq!(payload.profile_history.len(), 2);
    assert_eq!(payload.profile_history[0].title, "Persist Two");
    assert_eq!(payload.profile_history[1].title, "Persist One");
    assert!(payload.profile_history_clear_url.is_some());
    let html = render_browser_session_page(&payload, &back_href);
    assert!(html.contains("<h2>Profile history</h2>"));
    assert!(html.contains("Persist One"));
    assert!(html.contains(">Remove</a>"));

    let open_bookmark = RequestTarget {
        path: "/browser".to_owned(),
        params: form_urlencoded::parse(
            payload.bookmarks[0]
                .action_url
                .trim_start_matches("/browser?")
                .as_bytes(),
        )
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect(),
    };
    let (payload, _) = registry.apply_target(&open_bookmark).await.unwrap();
    assert_eq!(payload.title, "Persist One");
    assert!(payload.current_bookmarked);
    assert!(payload.viewport.contains("saved across registries"));
}

#[tokio::test]
async fn browser_session_registry_restores_profile_tabs_without_url() {
    let dir = tempfile::tempdir().unwrap();
    let profile = dir.path().join("browser-profile.json");
    let first = dir.path().join("tab-one.html");
    let second = dir.path().join("tab-two.html");
    std::fs::write(
        &first,
        r#"<!doctype html><title>Tab One</title><p>first restored tab</p>"#,
    )
    .unwrap();
    std::fs::write(
        &second,
        r#"<!doctype html><title>Tab Two</title><p>second restored tab</p>"#,
    )
    .unwrap();

    let registry = BrowserSessionRegistry::with_profile_path(profile.clone());
    let create_first = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![("url".to_owned(), first.display().to_string())],
    };
    let (payload, _) = registry.create_target(&create_first).await.unwrap();
    let first_id = payload.id.clone();
    let create_second = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![("url".to_owned(), second.display().to_string())],
    };
    let (payload, _) = registry.create_target(&create_second).await.unwrap();
    assert_eq!(payload.title, "Tab Two");
    assert_eq!(payload.sessions.len(), 2);
    let saved = load_browser_session_profile(&profile).unwrap();
    assert_eq!(saved.tabs.len(), 2);
    assert!(saved.tabs[1].active);

    let first_href = payload
        .sessions
        .iter()
        .find(|session| session.id == first_id)
        .unwrap()
        .action_url
        .clone();
    let switch_first = RequestTarget {
        path: "/browser".to_owned(),
        params: form_urlencoded::parse(first_href.trim_start_matches("/browser?").as_bytes())
            .map(|(key, value)| (key.into_owned(), value.into_owned()))
            .collect(),
    };
    let (payload, _) = registry.apply_target(&switch_first).await.unwrap();
    assert_eq!(payload.title, "Tab One");
    let saved = load_browser_session_profile(&profile).unwrap();
    assert_eq!(saved.tabs.len(), 2);
    assert!(saved.tabs[0].active);
    assert!(!saved.tabs[1].active);
    drop(registry);

    let registry = BrowserSessionRegistry::with_profile_path(profile);
    let restore_tabs = RequestTarget {
        path: "/browser".to_owned(),
        params: Vec::new(),
    };
    let (payload, back_href) = registry.create_target(&restore_tabs).await.unwrap();
    assert_eq!(payload.title, "Tab One");
    assert_eq!(payload.sessions.len(), 2);
    assert!(
        payload
            .sessions
            .iter()
            .any(|session| session.title == "Tab Two")
    );
    assert!(
        payload
            .sessions
            .iter()
            .any(|session| session.title == "Tab One" && session.current)
    );
    assert!(payload.viewport.contains("first restored tab"));
    let html = render_browser_session_page(&payload, &back_href);
    assert!(html.contains("Tab Two"));
    assert!(html.contains("<h2>Sessions</h2>"));
}

#[tokio::test]
async fn browser_session_registry_clears_saved_profile_tabs() {
    let dir = tempfile::tempdir().unwrap();
    let profile = dir.path().join("browser-profile.json");
    let first = dir.path().join("saved-one.html");
    let second = dir.path().join("saved-two.html");
    std::fs::write(
        &first,
        r#"<!doctype html><title>Saved One</title><p>first saved tab</p>"#,
    )
    .unwrap();
    std::fs::write(
        &second,
        r#"<!doctype html><title>Saved Two</title><p>second saved tab</p>"#,
    )
    .unwrap();

    let registry = BrowserSessionRegistry::with_profile_path(profile.clone());
    let create_first = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![("url".to_owned(), first.display().to_string())],
    };
    registry.create_target(&create_first).await.unwrap();
    let create_second = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![("url".to_owned(), second.display().to_string())],
    };
    let (payload, back_href) = registry.create_target(&create_second).await.unwrap();
    assert_eq!(payload.sessions.len(), 2);
    assert_eq!(
        load_browser_session_profile(&profile).unwrap().tabs.len(),
        2
    );
    let clear_url = payload.profile_tabs_clear_url.clone().unwrap();
    let html = render_browser_session_page(&payload, &back_href);
    assert!(html.contains("Forget saved"));
    assert!(html.contains("action=clear-profile-tabs"));

    let clear = RequestTarget {
        path: "/browser".to_owned(),
        params: form_urlencoded::parse(clear_url.trim_start_matches("/browser?").as_bytes())
            .map(|(key, value)| (key.into_owned(), value.into_owned()))
            .collect(),
    };
    let (payload, _) = registry.apply_target(&clear).await.unwrap();
    assert_eq!(payload.title, "Saved Two");
    assert_eq!(payload.sessions.len(), 2);
    assert!(
        load_browser_session_profile(&profile)
            .unwrap()
            .tabs
            .is_empty()
    );
    drop(registry);

    let registry = BrowserSessionRegistry::with_profile_path(profile);
    let restore_tabs = RequestTarget {
        path: "/browser".to_owned(),
        params: Vec::new(),
    };
    assert!(registry.create_target(&restore_tabs).await.is_err());

    let create_first = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![("url".to_owned(), first.display().to_string())],
    };
    let (payload, _) = registry.create_target(&create_first).await.unwrap();
    assert_eq!(payload.title, "Saved One");
    assert_eq!(payload.sessions.len(), 1);
}

#[tokio::test]
async fn browser_session_registry_does_not_keep_partial_failed_profile_restore() {
    let dir = tempfile::tempdir().unwrap();
    let profile = dir.path().join("browser-profile.json");
    let first = dir.path().join("tab-one.html");
    let missing = dir.path().join("missing.html");
    std::fs::write(
        &first,
        r#"<!doctype html><title>Good Tab</title><p>good restored tab</p>"#,
    )
    .unwrap();
    save_browser_session_profile(
        &profile,
        &BrowserSessionProfileFile {
            version: 1,
            bookmarks: Vec::new(),
            tabs: vec![
                BrowserSessionProfileTabFile {
                    title: "Good Tab".to_owned(),
                    source: first.display().to_string(),
                    active: true,
                    updated_at_unix_secs: 1,
                },
                BrowserSessionProfileTabFile {
                    title: "Missing Tab".to_owned(),
                    source: missing.display().to_string(),
                    active: false,
                    updated_at_unix_secs: 1,
                },
            ],
            history: Vec::new(),
            closed: Vec::new(),
        },
    )
    .unwrap();

    let registry = BrowserSessionRegistry::with_profile_path(profile);
    let restore_tabs = RequestTarget {
        path: "/browser".to_owned(),
        params: Vec::new(),
    };
    assert!(registry.create_target(&restore_tabs).await.is_err());

    let create_good = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![("url".to_owned(), first.display().to_string())],
    };
    let (payload, _) = registry.create_target(&create_good).await.unwrap();
    assert_eq!(payload.title, "Good Tab");
    assert_eq!(payload.sessions.len(), 1);
    assert!(payload.sessions[0].current);
}

#[tokio::test]
async fn browser_session_registry_removes_and_clears_profile_history() {
    let dir = tempfile::tempdir().unwrap();
    let profile = dir.path().join("browser-profile.json");
    let first = dir.path().join("first.html");
    let second = dir.path().join("second.html");
    std::fs::write(
        &first,
        r#"<!doctype html><title>History One</title><p>old profile page</p>"#,
    )
    .unwrap();
    std::fs::write(
        &second,
        r#"<!doctype html><title>History Two</title><p>current profile page</p>"#,
    )
    .unwrap();

    let registry = BrowserSessionRegistry::with_profile_path(profile.clone());
    let create_first = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![("url".to_owned(), first.display().to_string())],
    };
    registry.create_target(&create_first).await.unwrap();

    let create_second = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![("url".to_owned(), second.display().to_string())],
    };
    let (payload, _) = registry.create_target(&create_second).await.unwrap();
    assert_eq!(payload.profile_history.len(), 2);
    assert_eq!(payload.profile_history[0].title, "History Two");
    assert_eq!(payload.profile_history[1].title, "History One");

    let remove_old = RequestTarget {
        path: "/browser".to_owned(),
        params: form_urlencoded::parse(
            payload.profile_history[1]
                .remove_url
                .trim_start_matches("/browser?")
                .as_bytes(),
        )
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect(),
    };
    let (payload, _) = registry.apply_target(&remove_old).await.unwrap();
    assert_eq!(payload.title, "History Two");
    assert_eq!(payload.profile_history.len(), 1);
    assert_eq!(payload.profile_history[0].title, "History Two");
    let saved = load_browser_session_profile(&profile).unwrap();
    assert_eq!(saved.history.len(), 1);
    assert_eq!(saved.history[0].title, "History Two");

    let clear_url = payload.profile_history_clear_url.clone().unwrap();
    let clear = RequestTarget {
        path: "/browser".to_owned(),
        params: form_urlencoded::parse(clear_url.trim_start_matches("/browser?").as_bytes())
            .map(|(key, value)| (key.into_owned(), value.into_owned()))
            .collect(),
    };
    let (payload, back_href) = registry.apply_target(&clear).await.unwrap();
    assert_eq!(payload.title, "History Two");
    assert!(payload.profile_history.is_empty());
    assert!(
        load_browser_session_profile(&profile)
            .unwrap()
            .history
            .is_empty()
    );
    let html = render_browser_session_page(&payload, &back_href);
    assert!(html.contains("No profile history"));
    assert!(!html.contains("old profile page"));

    assert!(html.contains("<h2>Profile history</h2>"));
}

#[tokio::test]
async fn browser_session_registry_keeps_session_after_bad_action_request() {
    let dir = tempfile::tempdir().unwrap();
    let page = dir.path().join("page.html");
    std::fs::write(
        &page,
        r#"<!doctype html><title>Stable</title><p>still here</p>"#,
    )
    .unwrap();

    let registry = BrowserSessionRegistry::default();
    let create = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![("url".to_owned(), page.display().to_string())],
    };
    let (payload, _) = registry.create_target(&create).await.unwrap();
    let session_id = payload.id.clone();

    let bad_action = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![
            ("id".to_owned(), session_id.clone()),
            ("action".to_owned(), "link".to_owned()),
        ],
    };
    assert!(registry.apply_target(&bad_action).await.is_err());

    let current = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![
            ("id".to_owned(), session_id),
            ("action".to_owned(), "current".to_owned()),
        ],
    };
    let (payload, _) = registry.apply_target(&current).await.unwrap();
    assert_eq!(payload.title, "Stable");
    assert!(payload.viewport.contains("still here"));
}

#[tokio::test]
async fn browser_session_registry_keeps_session_after_failed_action_application() {
    let dir = tempfile::tempdir().unwrap();
    let page = dir.path().join("page.html");
    std::fs::write(
        &page,
        r#"<!doctype html><title>Stable</title><a href="missing.html">bad link</a><p>still here</p>"#,
    )
    .unwrap();

    let registry = BrowserSessionRegistry::default();
    let create = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![("url".to_owned(), page.display().to_string())],
    };
    let (payload, _) = registry.create_target(&create).await.unwrap();
    let session_id = payload.id.clone();

    let bad_link = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![
            ("id".to_owned(), session_id.clone()),
            ("action".to_owned(), "link".to_owned()),
            ("link".to_owned(), "99".to_owned()),
        ],
    };
    assert!(registry.apply_target(&bad_link).await.is_err());

    let current = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![
            ("id".to_owned(), session_id),
            ("action".to_owned(), "current".to_owned()),
        ],
    };
    let (payload, _) = registry.apply_target(&current).await.unwrap();
    assert_eq!(payload.title, "Stable");
    assert_eq!(payload.sessions.len(), 1);
    assert!(payload.viewport.contains("still here"));
}

#[tokio::test]
async fn browser_session_registry_click_selector_defaults_can_navigate() {
    let dir = tempfile::tempdir().unwrap();
    let first = dir.path().join("first.html");
    let second = dir.path().join("second.html");
    std::fs::write(
        &first,
        format!(
            r#"<!doctype html><title>First</title><a id="go" href="{}">Second</a>"#,
            second.display()
        ),
    )
    .unwrap();
    std::fs::write(
        &second,
        r#"<!doctype html><title>Second</title><p>arrived</p>"#,
    )
    .unwrap();

    let registry = BrowserSessionRegistry::default();
    let create = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![("url".to_owned(), first.display().to_string())],
    };
    let (payload, _) = registry.create_target(&create).await.unwrap();

    let click = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![
            ("id".to_owned(), payload.id),
            ("action".to_owned(), "click-selector".to_owned()),
            ("selector".to_owned(), "#go".to_owned()),
        ],
    };
    let (payload, _) = registry.apply_target(&click).await.unwrap();
    assert_eq!(payload.title, "Second");
    assert_eq!(payload.history_len, 2);
    assert!(payload.can_back);
    assert!(payload.viewport.contains("arrived"));
}

#[tokio::test]
async fn browser_session_registry_click_at_uses_viewport_coordinates() {
    let dir = tempfile::tempdir().unwrap();
    let page = dir.path().join("button.html");
    std::fs::write(
        &page,
        r#"<!doctype html>
<html><head><title>Button</title></head><body>
<button onclick="document.querySelector('#out').innerText = 'Clicked'">Press</button>
<p id="out">Waiting</p>
</body></html>"#,
    )
    .unwrap();

    let registry = BrowserSessionRegistry::default();
    let create = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![("url".to_owned(), page.display().to_string())],
    };
    let (payload, _) = registry.create_target(&create).await.unwrap();

    let click = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![
            ("id".to_owned(), payload.id),
            ("action".to_owned(), "click-at".to_owned()),
            ("x".to_owned(), "0".to_owned()),
            ("y".to_owned(), "0".to_owned()),
        ],
    };
    let (payload, _) = registry.apply_target(&click).await.unwrap();
    assert_eq!(payload.title, "Button");
    assert_eq!(payload.history_len, 1);
    assert!(payload.viewport.contains("Clicked"));
}

#[test]
fn browser_session_find_highlighting_escapes_viewport_text() {
    let rendered =
        render_browser_session_highlighted_text("Alpha <Needle>\nneedle & tail", "needle");

    assert_eq!(
        rendered,
        "Alpha &lt;<mark>Needle</mark>&gt;\n<mark>needle</mark> &amp; tail"
    );
}

#[test]
fn browser_session_action_href_preserves_session_and_viewport() {
    let payload = BrowserSessionPayload {
        id: "s7".to_owned(),
        back_href: "/search?q=cat".to_owned(),
        title: "Example".to_owned(),
        source: "https://example.com".to_owned(),
        width: 90,
        height: 30,
        max_bytes: 1024 * 1024,
        viewport_x: 12,
        viewport_y: 0,
        document_width: 90,
        document_height: 30,
        max_scroll_x: 20,
        max_scroll_y: 0,
        dom_node_count: 1,
        link_count: 0,
        can_back: false,
        can_forward: false,
        history_len: 1,
        current_history_index: Some(0),
        profile_enabled: false,
        profile_error: None,
        current_bookmarked: false,
        bookmarks_clear_url: None,
        closed_sessions_clear_url: None,
        profile_tabs_clear_url: None,
        profile_history_clear_url: None,
        find_query: String::new(),
        find_match_count: 0,
        find_current_index: None,
        find_current_line: None,
        sessions: Vec::new(),
        closed_sessions: Vec::new(),
        bookmarks: Vec::new(),
        profile_history: Vec::new(),
        history: Vec::new(),
        viewport: String::new(),
        focused: None,
        links: Vec::new(),
        form_count: 0,
        forms: Vec::new(),
        cookies: Vec::new(),
        local_storage: Vec::new(),
        session_storage: Vec::new(),
        resource_count: 0,
        resources: Vec::new(),
        resource_report: None,
    };
    let href =
        browser_session_action_href(&payload.id, "scroll", &[("dy", "15".to_owned())], &payload);
    let target = RequestTarget {
        path: "/browser".to_owned(),
        params: form_urlencoded::parse(href.trim_start_matches("/browser?").as_bytes())
            .map(|(key, value)| (key.into_owned(), value.into_owned()))
            .collect(),
    };

    assert_eq!(target.param("id").as_deref(), Some("s7"));
    assert_eq!(target.param("action").as_deref(), Some("scroll"));
    assert_eq!(target.param("dy").as_deref(), Some("15"));
    assert_eq!(target.param("width").as_deref(), Some("90"));
    assert_eq!(target.param("height").as_deref(), Some("30"));
    assert_eq!(target.param("viewport_x").as_deref(), Some("12"));
    assert_eq!(target.param("from").as_deref(), Some("/search?q=cat"));
}

#[tokio::test]
async fn browser_session_registry_edits_and_submits_forms() {
    let dir = tempfile::tempdir().unwrap();
    let form_page = dir.path().join("form.html");
    let result_page = dir.path().join("result.html");
    std::fs::write(
        &form_page,
        r#"<!doctype html>
<title>Form</title>
<form action="result.html" method="get">
  <input name="q" value="old">
  <select name="kind">
<option value="docs">Docs</option>
<option value="news" selected>News</option>
  </select>
  <input type="checkbox" name="fast">
  <button>Go</button>
</form>"#,
    )
    .unwrap();
    std::fs::write(
        &result_page,
        r#"<!doctype html><title>Result</title><p>ok</p>"#,
    )
    .unwrap();

    let registry = BrowserSessionRegistry::default();
    let create = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![("url".to_owned(), form_page.display().to_string())],
    };
    let (payload, _) = registry.create_target(&create).await.unwrap();
    assert_eq!(payload.title, "Form");
    assert_eq!(payload.form_count, 1);
    assert_eq!(payload.forms[0].controls[0].value, "old");

    let fill = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![
            ("id".to_owned(), payload.id.clone()),
            ("action".to_owned(), "fill".to_owned()),
            ("form".to_owned(), "0".to_owned()),
            ("name".to_owned(), "q".to_owned()),
            ("value".to_owned(), "rust browser".to_owned()),
        ],
    };
    let (payload, _) = registry.apply_target(&fill).await.unwrap();
    assert_eq!(payload.forms[0].controls[0].value, "rust browser");

    let select = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![
            ("id".to_owned(), payload.id.clone()),
            ("action".to_owned(), "select".to_owned()),
            ("form".to_owned(), "0".to_owned()),
            ("control".to_owned(), "1".to_owned()),
            ("value".to_owned(), "docs".to_owned()),
        ],
    };
    let (payload, _) = registry.apply_target(&select).await.unwrap();
    assert!(
        payload.forms[0].controls[1]
            .options
            .iter()
            .any(|option| option.value == "docs" && option.selected)
    );

    let toggle = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![
            ("id".to_owned(), payload.id.clone()),
            ("action".to_owned(), "toggle".to_owned()),
            ("form".to_owned(), "0".to_owned()),
            ("control".to_owned(), "2".to_owned()),
        ],
    };
    let (payload, _) = registry.apply_target(&toggle).await.unwrap();
    assert!(payload.forms[0].controls[2].checked);

    let submit = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![
            ("id".to_owned(), payload.id.clone()),
            ("action".to_owned(), "submit".to_owned()),
            ("form".to_owned(), "0".to_owned()),
        ],
    };
    let (payload, _) = registry.apply_target(&submit).await.unwrap();
    assert_eq!(payload.title, "Result");
    assert_eq!(payload.history_len, 2);
    assert!(payload.can_back);
    assert!(payload.source.contains("result.html"));
    assert!(payload.source.contains("q=rust+browser"));
    assert!(payload.source.contains("kind=docs"));
    assert!(payload.source.contains("fast=on"));
}

#[tokio::test]
async fn browser_session_registry_submits_forms_in_new_sessions() {
    let dir = tempfile::tempdir().unwrap();
    let form_page = dir.path().join("form.html");
    let result_page = dir.path().join("result.html");
    std::fs::write(
        &form_page,
        r#"<!doctype html>
<title>Form</title>
<form action="result.html" method="get">
  <input name="q" value="old">
  <button>Go</button>
</form>"#,
    )
    .unwrap();
    std::fs::write(
        &result_page,
        r#"<!doctype html><title>Result</title><p>new tab result</p>"#,
    )
    .unwrap();

    let registry = BrowserSessionRegistry::default();
    let create = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![
            ("url".to_owned(), form_page.display().to_string()),
            ("from".to_owned(), "/search?q=forms".to_owned()),
        ],
    };
    let (payload, back_href) = registry.create_target(&create).await.unwrap();
    let first_id = payload.id.clone();
    let html = render_browser_session_page(&payload, &back_href);
    assert!(html.contains("action=submit-new-session"));

    let fill = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![
            ("id".to_owned(), first_id.clone()),
            ("action".to_owned(), "fill".to_owned()),
            ("form".to_owned(), "0".to_owned()),
            ("name".to_owned(), "q".to_owned()),
            ("value".to_owned(), "rust browser".to_owned()),
        ],
    };
    let (payload, _) = registry.apply_target(&fill).await.unwrap();
    assert_eq!(payload.forms[0].controls[0].value, "rust browser");
    let submit_href = payload.forms[0].submit_new_session_url.clone();

    let submit_new_session = RequestTarget {
        path: "/browser".to_owned(),
        params: form_urlencoded::parse(submit_href.trim_start_matches("/browser?").as_bytes())
            .map(|(key, value)| (key.into_owned(), value.into_owned()))
            .collect(),
    };
    let (payload, _) = registry.apply_target(&submit_new_session).await.unwrap();
    assert_eq!(payload.title, "Result");
    assert_ne!(payload.id, first_id);
    assert_eq!(payload.sessions.len(), 2);
    assert!(payload.source.contains("result.html"));
    assert!(payload.source.contains("q=rust+browser"));
    assert!(payload.viewport.contains("new tab result"));
    assert!(
        payload
            .sessions
            .iter()
            .any(|session| session.id == first_id && session.title == "Form")
    );

    let original = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![
            ("id".to_owned(), first_id),
            ("action".to_owned(), "current".to_owned()),
        ],
    };
    let (payload, _) = registry.apply_target(&original).await.unwrap();
    assert_eq!(payload.title, "Form");
    assert_eq!(payload.history_len, 1);
    assert_eq!(payload.forms[0].controls[0].value, "rust browser");
}

#[tokio::test]
async fn browser_session_registry_focuses_types_and_submits_forms() {
    let dir = tempfile::tempdir().unwrap();
    let form_page = dir.path().join("keyboard.html");
    let result_page = dir.path().join("result.html");
    std::fs::write(
        &form_page,
        r#"<!doctype html>
<title>Keyboard</title>
<form action="result.html" method="get">
  <input id="q" name="q" value="old">
  <select id="kind" name="kind">
<option value="docs">Docs</option>
<option value="news" selected>News</option>
  </select>
  <input id="fast" type="checkbox" name="fast">
  <button id="go">Go</button>
</form>"#,
    )
    .unwrap();
    std::fs::write(
        &result_page,
        r#"<!doctype html><title>Result</title><p>ok</p>"#,
    )
    .unwrap();

    let registry = BrowserSessionRegistry::default();
    let create = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![("url".to_owned(), form_page.display().to_string())],
    };
    let (payload, _) = registry.create_target(&create).await.unwrap();

    let focus_select = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![
            ("id".to_owned(), payload.id.clone()),
            ("action".to_owned(), "focus-selector".to_owned()),
            ("selector".to_owned(), "#kind".to_owned()),
        ],
    };
    let (payload, _) = registry.apply_target(&focus_select).await.unwrap();
    assert_eq!(payload.focused.as_ref().unwrap().name, "kind");

    let choose = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![
            ("id".to_owned(), payload.id.clone()),
            ("action".to_owned(), "choose".to_owned()),
            ("value".to_owned(), "docs".to_owned()),
        ],
    };
    let (payload, _) = registry.apply_target(&choose).await.unwrap();
    assert!(
        payload.forms[0].controls[1]
            .options
            .iter()
            .any(|option| option.value == "docs" && option.selected)
    );

    let focus_check = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![
            ("id".to_owned(), payload.id.clone()),
            ("action".to_owned(), "focus-selector".to_owned()),
            ("selector".to_owned(), "#fast".to_owned()),
        ],
    };
    let (payload, _) = registry.apply_target(&focus_check).await.unwrap();
    assert_eq!(payload.focused.as_ref().unwrap().name, "fast");

    let space = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![
            ("id".to_owned(), payload.id.clone()),
            ("action".to_owned(), "space".to_owned()),
        ],
    };
    let (payload, _) = registry.apply_target(&space).await.unwrap();
    assert!(payload.forms[0].controls[2].checked);

    let focus_input = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![
            ("id".to_owned(), payload.id.clone()),
            ("action".to_owned(), "focus-selector".to_owned()),
            ("selector".to_owned(), "#q".to_owned()),
        ],
    };
    let (payload, _) = registry.apply_target(&focus_input).await.unwrap();
    assert_eq!(payload.focused.as_ref().unwrap().name, "q");

    let type_text = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![
            ("id".to_owned(), payload.id.clone()),
            ("action".to_owned(), "type-text".to_owned()),
            ("text".to_owned(), " browser".to_owned()),
        ],
    };
    let (payload, _) = registry.apply_target(&type_text).await.unwrap();
    assert_eq!(payload.focused.as_ref().unwrap().value, "old browser");

    let backspace = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![
            ("id".to_owned(), payload.id.clone()),
            ("action".to_owned(), "backspace".to_owned()),
            ("count".to_owned(), "1".to_owned()),
        ],
    };
    let (payload, _) = registry.apply_target(&backspace).await.unwrap();
    assert_eq!(payload.focused.as_ref().unwrap().value, "old browse");

    let type_tail = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![
            ("id".to_owned(), payload.id.clone()),
            ("action".to_owned(), "type-text".to_owned()),
            ("text".to_owned(), "r".to_owned()),
        ],
    };
    let (payload, _) = registry.apply_target(&type_tail).await.unwrap();
    assert_eq!(payload.focused.as_ref().unwrap().value, "old browser");

    let enter = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![
            ("id".to_owned(), payload.id),
            ("action".to_owned(), "enter".to_owned()),
        ],
    };
    let (payload, _) = registry.apply_target(&enter).await.unwrap();
    assert_eq!(payload.title, "Result");
    assert_eq!(payload.history_len, 2);
    assert!(payload.source.contains("q=old+browser"));
    assert!(payload.source.contains("kind=docs"));
    assert!(payload.source.contains("fast=on"));
}

#[tokio::test]
async fn browser_session_inspector_fetches_and_applies_page_resources() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        for _ in 0..2 {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut buf = [0u8; 4096];
            let read = stream.read(&mut buf).await.unwrap();
            let request = String::from_utf8_lossy(&buf[..read]);
            let request_line = request.lines().next().unwrap_or_default();
            let (body, content_type) = if request_line.contains(" /app.css ") {
                ("p { color: #cc0000; }", "text/css")
            } else {
                (
                    r#"<!doctype html>
<html><head><title>Resources</title><link rel="stylesheet" href="/app.css"></head>
<body><p>resource page</p></body></html>"#,
                    "text/html",
                )
            };
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream.write_all(response.as_bytes()).await.unwrap();
        }
    });

    let registry = BrowserSessionRegistry::default();
    let create = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![("url".to_owned(), format!("http://{addr}/doc"))],
    };
    let (payload, back_href) = registry.create_target(&create).await.unwrap();
    assert_eq!(payload.title, "Resources");
    assert_eq!(payload.resource_count, 1);
    assert!(payload.resource_report.is_none());

    let html = render_browser_session_page(&payload, &back_href);
    assert!(html.contains("action=fetch-resources"));
    assert!(html.contains("action=apply-styles"));
    assert!(html.contains("action=run-scripts"));
    assert!(html.contains("action=load-images"));

    let apply_styles = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![
            ("id".to_owned(), payload.id),
            ("action".to_owned(), "apply-styles".to_owned()),
        ],
    };
    let (payload, back_href) = registry.apply_target(&apply_styles).await.unwrap();
    server.await.unwrap();

    let report = payload.resource_report.as_ref().unwrap();
    assert_eq!(report.action, "Apply styles");
    assert_eq!(report.total, 1);
    assert_eq!(report.fetched, 1);
    assert_eq!(report.failed, 0);
    assert_eq!(report.applied, Some(1));
    assert_eq!(report.resources[0].status, "fetched");
    assert_eq!(report.resources[0].kind, "stylesheet");

    let html = render_browser_session_page(&payload, &back_href);
    assert!(html.contains("Apply styles: total=1 fetched=1 cached=0 failed=0 skipped=0 applied=1"));
    assert!(html.contains("text/css"));
    assert!(html.contains("Clear report"));
    assert!(html.contains("action=clear-resource-report"));

    let clear_report = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![
            ("id".to_owned(), payload.id),
            ("action".to_owned(), "clear-resource-report".to_owned()),
        ],
    };
    let (payload, back_href) = registry.apply_target(&clear_report).await.unwrap();
    assert!(payload.resource_report.is_none());
    let html = render_browser_session_page(&payload, &back_href);
    assert!(!html.contains("Apply styles: total=1"));
    assert!(!html.contains("Clear report"));
}

#[tokio::test]
async fn browser_session_inspector_reports_and_clears_page_state() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut buf = [0u8; 4096];
        let _ = stream.read(&mut buf).await.unwrap();
        let body = r#"<!doctype html>
<html><head>
<title>State</title>
<link rel="stylesheet" href="/app.css" media="screen">
<script>localStorage.setItem("theme", "dark"); sessionStorage.setItem("nonce", "abc");</script>
</head><body><img src="/logo.png" alt="Logo"><p>state</p></body></html>"#;
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nSet-Cookie: sid=abc; Path=/; HttpOnly\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        stream.write_all(response.as_bytes()).await.unwrap();
    });

    let registry = BrowserSessionRegistry::default();
    let create = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![("url".to_owned(), format!("http://{addr}/state"))],
    };
    let (payload, back_href) = registry.create_target(&create).await.unwrap();
    server.await.unwrap();

    assert_eq!(payload.title, "State");
    assert_eq!(payload.history.len(), 1);
    assert!(payload.history[0].current);
    assert!(payload.cookies.iter().any(|cookie| cookie.name == "sid"));
    assert!(
        payload
            .local_storage
            .iter()
            .any(|entry| entry.key == "theme" && entry.value == "dark")
    );
    assert!(
        payload
            .session_storage
            .iter()
            .any(|entry| entry.key == "nonce" && entry.value == "abc")
    );
    assert!(
        payload
            .resources
            .iter()
            .any(|resource| resource.url == "/app.css")
    );
    assert!(
        payload
            .resources
            .iter()
            .any(|resource| resource.url == "/logo.png")
    );

    let html = render_browser_session_page(&payload, &back_href);
    assert!(html.contains("Resources (2)"));
    assert!(html.contains("resource-actions"));
    assert!(html.contains("action=open"));
    assert!(html.contains("New session"));

    let clear_cookies = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![
            ("id".to_owned(), payload.id.clone()),
            ("action".to_owned(), "clear-cookies".to_owned()),
        ],
    };
    let (payload, _) = registry.apply_target(&clear_cookies).await.unwrap();
    assert!(payload.cookies.is_empty());

    let clear_local_storage = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![
            ("id".to_owned(), payload.id.clone()),
            ("action".to_owned(), "clear-local-storage".to_owned()),
        ],
    };
    let (payload, _) = registry.apply_target(&clear_local_storage).await.unwrap();
    assert!(payload.local_storage.is_empty());

    let clear_session_storage = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![
            ("id".to_owned(), payload.id),
            ("action".to_owned(), "clear-session-storage".to_owned()),
        ],
    };
    let (payload, _) = registry.apply_target(&clear_session_storage).await.unwrap();
    assert!(payload.session_storage.is_empty());
}

#[tokio::test]
async fn browser_session_page_renders_form_controls() {
    let dir = tempfile::tempdir().unwrap();
    let page = dir.path().join("form.html");
    std::fs::write(
        &page,
        r#"<!doctype html><title>Form</title><form><input name="q" value="old"><button>Go</button></form>"#,
    )
    .unwrap();

    let registry = BrowserSessionRegistry::default();
    let create = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![
            ("url".to_owned(), page.display().to_string()),
            ("from".to_owned(), "/search?q=forms".to_owned()),
        ],
    };
    let (payload, back_href) = registry.create_target(&create).await.unwrap();
    let html = render_browser_session_page(&payload, &back_href);

    assert!(html.contains("<h2>Forms</h2>"));
    assert!(html.contains("<h2>Sessions</h2>"));
    assert!(html.contains("session-new"));
    assert!(html.contains("<h2>Click</h2>"));
    assert!(html.contains("<h2>Keyboard</h2>"));
    assert!(html.contains("<h2>Inspector</h2>"));
    assert!(html.contains(r#"name="action" value="click-selector""#));
    assert!(html.contains(r#"name="action" value="click-at""#));
    assert!(html.contains(r#"name="action" value="focus-selector""#));
    assert!(html.contains(r#"name="action" value="type-text""#));
    assert!(html.contains(r#"name="action" value="choose""#));
    assert!(html.contains(r#"name="action" value="find""#));
    assert!(html.contains("Find in page"));
    assert!(html.contains("clear-cookies"));
    assert!(html.contains("localStorage"));
    assert!(html.contains("Resources"));
    assert!(html.contains("action=history"));
    assert!(html.contains(">Open</a>"));
    assert!(html.contains(r#"name="action" value="fill""#));
    assert!(html.contains(r#"name="value" value="old""#));
    assert!(html.contains("Submit form"));
    assert!(html.contains("rust browser session"));
}
