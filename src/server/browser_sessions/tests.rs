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
    assert!(html.contains("Links CSV"));
    assert!(html.contains("format=links-csv"));
    assert!(html.contains(">Background</a>"));

    let links_csv_export = RequestTarget {
        path: "/api/browser-session".to_owned(),
        params: vec![
            ("id".to_owned(), payload.id.clone()),
            ("format".to_owned(), "links-csv".to_owned()),
        ],
    };
    let response = browser_session_api_response(&links_csv_export, &payload);
    assert_eq!(response.status, 200);
    assert_eq!(response.content_type, "text/csv; charset=utf-8");
    assert!(response.body.starts_with(
        "index,label,url,action_url,new_session_url,background_session_url,session_id,source,total_link_count\n"
    ));
    assert_eq!(response.body.lines().count(), 2);
    assert!(response.body.contains("Open target"));
    assert!(response.body.contains(&payload.links[0].url));
    assert!(response.body.contains("action=link"));
    assert!(response.body.contains(&payload.links[0].new_session_url));
    assert!(
        response
            .body
            .contains(&payload.links[0].background_session_url)
    );

    let state_export = RequestTarget {
        path: "/api/browser-session".to_owned(),
        params: vec![
            ("id".to_owned(), payload.id.clone()),
            ("format".to_owned(), "session-state".to_owned()),
        ],
    };
    let response = browser_session_api_response(&state_export, &payload);
    assert_eq!(response.status, 200);
    let exported: serde_json::Value = serde_json::from_str(&response.body).unwrap();
    assert_eq!(exported["counts"]["links"], 1);
    assert_eq!(exported["links"].as_array().unwrap().len(), 1);
    assert_eq!(exported["links"][0]["index"], 0);
    assert_eq!(exported["links"][0]["label"], "Open target");
    assert_eq!(exported["links"][0]["url"], payload.links[0].url);
    assert!(
        exported["links"][0]["action_url"]
            .as_str()
            .unwrap()
            .contains("action=link")
    );
    assert!(
        exported["links"][0]["new_session_url"]
            .as_str()
            .unwrap()
            .contains("url=")
    );
    assert!(
        exported["links"][0]["background_session_url"]
            .as_str()
            .unwrap()
            .contains("action=link-background-session")
    );

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
async fn browser_session_registry_opens_links_in_background_sessions() {
    let dir = tempfile::tempdir().unwrap();
    let first = dir.path().join("first.html");
    let second = dir.path().join("second.html");
    std::fs::write(
        &first,
        format!(
            r#"<!doctype html><title>First</title><p>keep reading</p><a href="{}">Queue target</a>"#,
            second.display()
        ),
    )
    .unwrap();
    std::fs::write(
        &second,
        r#"<!doctype html><title>Second</title><p>queued background tab</p>"#,
    )
    .unwrap();

    let registry = BrowserSessionRegistry::default();
    let create = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![
            ("url".to_owned(), first.display().to_string()),
            ("from".to_owned(), "/search?q=background".to_owned()),
        ],
    };
    let (payload, back_href) = registry.create_target(&create).await.unwrap();
    let first_id = payload.id.clone();
    let html = render_browser_session_page(&payload, &back_href);
    assert!(html.contains(r#"value="open-background-session""#));
    assert!(html.contains(">Background</a>"));
    assert!(
        payload.links[0]
            .background_session_url
            .contains("action=link-background-session")
    );

    let open_background = RequestTarget {
        path: "/browser".to_owned(),
        params: form_urlencoded::parse(
            payload.links[0]
                .background_session_url
                .trim_start_matches("/browser?")
                .as_bytes(),
        )
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect(),
    };
    let (payload, _) = registry.apply_target(&open_background).await.unwrap();
    assert_eq!(payload.id, first_id);
    assert_eq!(payload.title, "First");
    assert_eq!(payload.sessions.len(), 2);
    assert!(payload.viewport.contains("keep reading"));
    assert!(payload.sessions[0].current);
    assert!(!payload.sessions[1].current);
    assert_eq!(payload.sessions[1].page_title, "Second");

    let next_tab = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![
            ("id".to_owned(), first_id),
            ("action".to_owned(), "next-tab".to_owned()),
        ],
    };
    let (payload, _) = registry.apply_target(&next_tab).await.unwrap();
    assert_eq!(payload.title, "Second");
    assert!(payload.viewport.contains("queued background tab"));
}

#[tokio::test]
async fn browser_session_registry_opens_page_links_in_background_sessions() {
    let dir = tempfile::tempdir().unwrap();
    let first = dir.path().join("bulk-links.html");
    let second = dir.path().join("bulk-link-second.html");
    let third = dir.path().join("bulk-link-third.html");
    std::fs::write(
        &first,
        format!(
            r#"<!doctype html><title>Bulk Links</title><p>active link hub</p><a href="{second}">Second</a><a href="{third}">Third</a><a href="{second}">Second duplicate</a>"#,
            second = second.display(),
            third = third.display(),
        ),
    )
    .unwrap();
    std::fs::write(
        &second,
        r#"<!doctype html><title>Bulk Link Second</title><p>second bulk link tab</p>"#,
    )
    .unwrap();
    std::fs::write(
        &third,
        r#"<!doctype html><title>Bulk Link Third</title><p>third bulk link tab</p>"#,
    )
    .unwrap();

    let registry = BrowserSessionRegistry::default();
    let create = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![("url".to_owned(), first.display().to_string())],
    };
    let (payload, back_href) = registry.create_target(&create).await.unwrap();
    let active_id = payload.id.clone();
    assert_eq!(payload.title, "Bulk Links");
    assert_eq!(payload.links.len(), 3);
    let bulk_href = payload.links_background_url.clone().unwrap();
    assert!(bulk_href.contains("action=open-links-background-sessions"));
    assert!(bulk_href.contains("limit=16"));
    let html = render_browser_session_page(&payload, &back_href);
    assert!(html.contains("Open links bg"));
    assert!(html.contains("open-links-background-sessions"));
    let state_export = RequestTarget {
        path: "/api/browser-session".to_owned(),
        params: vec![
            ("id".to_owned(), payload.id.clone()),
            ("format".to_owned(), "session-state".to_owned()),
        ],
    };
    let response = browser_session_api_response(&state_export, &payload);
    let exported: serde_json::Value = serde_json::from_str(&response.body).unwrap();
    assert!(
        exported["action_urls"]["open_links_background"]
            .as_str()
            .unwrap()
            .contains("action=open-links-background-sessions")
    );

    let open_two_links = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![
            ("id".to_owned(), payload.id.clone()),
            (
                "action".to_owned(),
                "open-links-background-sessions".to_owned(),
            ),
            ("limit".to_owned(), "2".to_owned()),
        ],
    };
    let (payload, _) = registry.apply_target(&open_two_links).await.unwrap();
    assert_eq!(payload.id, active_id);
    assert_eq!(payload.title, "Bulk Links");
    assert_eq!(payload.sessions.len(), 3);
    assert!(payload.sessions.iter().any(|session| session.current
        && session.id == active_id
        && session.title == "Bulk Links"));
    assert!(
        payload
            .sessions
            .iter()
            .any(|session| !session.current && session.title == "Bulk Link Second")
    );
    assert!(
        payload
            .sessions
            .iter()
            .any(|session| !session.current && session.title == "Bulk Link Third")
    );

    let open_links_again = RequestTarget {
        path: "/browser".to_owned(),
        params: form_urlencoded::parse(bulk_href.trim_start_matches("/browser?").as_bytes())
            .map(|(key, value)| (key.into_owned(), value.into_owned()))
            .collect(),
    };
    let (payload, _) = registry.apply_target(&open_links_again).await.unwrap();
    assert_eq!(payload.id, active_id);
    assert_eq!(payload.sessions.len(), 3);
}

#[tokio::test]
async fn browser_session_registry_opens_page_links_in_new_sessions() {
    let dir = tempfile::tempdir().unwrap();
    let first = dir.path().join("tab-links.html");
    let second = dir.path().join("tab-link-second.html");
    let third = dir.path().join("tab-link-third.html");
    std::fs::write(
        &first,
        format!(
            r#"<!doctype html><title>Tab Links</title><p>active tab link hub</p><a href="{second}">Second</a><a href="{third}">Third</a><a href="{second}">Second duplicate</a>"#,
            second = second.display(),
            third = third.display(),
        ),
    )
    .unwrap();
    std::fs::write(
        &second,
        r#"<!doctype html><title>Tab Link Second</title><p>second link tab</p>"#,
    )
    .unwrap();
    std::fs::write(
        &third,
        r#"<!doctype html><title>Tab Link Third</title><p>third link tab</p>"#,
    )
    .unwrap();

    let registry = BrowserSessionRegistry::default();
    let create = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![("url".to_owned(), first.display().to_string())],
    };
    let (payload, back_href) = registry.create_target(&create).await.unwrap();
    let hub_id = payload.id.clone();
    assert_eq!(payload.title, "Tab Links");
    assert_eq!(payload.links.len(), 3);
    let html = render_browser_session_page(&payload, &back_href);
    assert!(html.contains("Open links tabs"));
    assert!(html.contains("open-links-new-sessions"));
    let state_export = RequestTarget {
        path: "/api/browser-session".to_owned(),
        params: vec![
            ("id".to_owned(), payload.id.clone()),
            ("format".to_owned(), "session-state".to_owned()),
        ],
    };
    let response = browser_session_api_response(&state_export, &payload);
    assert_eq!(response.status, 200);
    let exported: serde_json::Value = serde_json::from_str(&response.body).unwrap();
    let open_links_href = exported["action_urls"]["open_links_new_sessions"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(open_links_href.contains("action=open-links-new-sessions"));
    assert!(open_links_href.contains("limit=16"));

    let open_two_links = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![
            ("id".to_owned(), payload.id.clone()),
            ("action".to_owned(), "open-links-new-sessions".to_owned()),
            ("limit".to_owned(), "2".to_owned()),
        ],
    };
    let (payload, _) = registry.apply_target(&open_two_links).await.unwrap();
    let first_link_id = payload.id.clone();
    assert_ne!(first_link_id, hub_id);
    assert_eq!(payload.title, "Tab Link Second");
    assert_eq!(payload.sessions.len(), 3);
    assert!(
        payload.sessions.iter().any(|session| !session.current
            && session.id == hub_id
            && session.title == "Tab Links")
    );
    assert!(payload.sessions.iter().any(|session| {
        session.current && session.id == first_link_id && session.title == "Tab Link Second"
    }));
    assert!(
        payload
            .sessions
            .iter()
            .any(|session| !session.current && session.title == "Tab Link Third")
    );

    let open_links_again = RequestTarget {
        path: "/browser".to_owned(),
        params: form_urlencoded::parse(open_links_href.trim_start_matches("/browser?").as_bytes())
            .map(|(key, value)| (key.into_owned(), value.into_owned()))
            .collect(),
    };
    let (payload, _) = registry.apply_target(&open_links_again).await.unwrap();
    assert_eq!(payload.id, hub_id);
    assert_eq!(payload.title, "Tab Links");
    assert_eq!(payload.sessions.len(), 3);
}

#[tokio::test]
async fn browser_session_registry_bookmarks_page_links() {
    let dir = tempfile::tempdir().unwrap();
    let first = dir.path().join("bookmark-links.html");
    let second = dir.path().join("bookmark-link-second.html");
    let third = dir.path().join("bookmark-link-third.html");
    std::fs::write(
        &first,
        format!(
            r#"<!doctype html><title>Bookmark Links</title><p>active link bookmark hub</p><a href="{second}">Second saved link</a><a href="{third}">Third saved link</a><a href="{second}">Second duplicate</a>"#,
            second = second.display(),
            third = third.display(),
        ),
    )
    .unwrap();
    std::fs::write(
        &second,
        r#"<!doctype html><title>Bookmark Link Second</title><p>second linked page</p>"#,
    )
    .unwrap();
    std::fs::write(
        &third,
        r#"<!doctype html><title>Bookmark Link Third</title><p>third linked page</p>"#,
    )
    .unwrap();

    let registry = BrowserSessionRegistry::default();
    let create = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![("url".to_owned(), first.display().to_string())],
    };
    let (payload, back_href) = registry.create_target(&create).await.unwrap();
    let active_id = payload.id.clone();
    assert_eq!(payload.title, "Bookmark Links");
    assert_eq!(payload.links.len(), 3);
    assert!(payload.bookmarks.is_empty());
    assert!(!payload.current_bookmarked);
    let html = render_browser_session_page(&payload, &back_href);
    assert!(html.contains(">Bookmark links</a>"));
    assert!(html.contains("action=bookmark-page-links"));

    let state_export = RequestTarget {
        path: "/api/browser-session".to_owned(),
        params: vec![
            ("id".to_owned(), payload.id.clone()),
            ("format".to_owned(), "session-state".to_owned()),
        ],
    };
    let response = browser_session_api_response(&state_export, &payload);
    assert_eq!(response.status, 200);
    let exported: serde_json::Value = serde_json::from_str(&response.body).unwrap();
    let bookmark_links_href = exported["action_urls"]["bookmark_page_links"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(bookmark_links_href.contains("action=bookmark-page-links"));

    let bookmark_links = RequestTarget {
        path: "/browser".to_owned(),
        params: form_urlencoded::parse(
            bookmark_links_href
                .trim_start_matches("/browser?")
                .as_bytes(),
        )
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect(),
    };
    let (payload, back_href) = registry.apply_target(&bookmark_links).await.unwrap();
    assert_eq!(payload.id, active_id);
    assert_eq!(payload.title, "Bookmark Links");
    assert_eq!(payload.sessions.len(), 1);
    assert_eq!(payload.bookmarks.len(), 2);
    assert!(!payload.current_bookmarked);
    assert!(
        payload
            .bookmarks
            .iter()
            .any(|bookmark| bookmark.title == "Second saved link"
                && bookmark.source.ends_with("bookmark-link-second.html"))
    );
    assert!(
        payload
            .bookmarks
            .iter()
            .any(|bookmark| bookmark.title == "Third saved link"
                && bookmark.source.ends_with("bookmark-link-third.html"))
    );
    let html = render_browser_session_page(&payload, &back_href);
    assert!(!html.contains("action=bookmark-page-links"));
    assert!(html.contains("action=remove-page-link-bookmarks"));
    let response = browser_session_api_response(&state_export, &payload);
    let exported: serde_json::Value = serde_json::from_str(&response.body).unwrap();
    assert!(exported["action_urls"]["bookmark_page_links"].is_null());
    let remove_link_bookmarks_href = exported["action_urls"]["remove_page_link_bookmarks"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(remove_link_bookmarks_href.contains("action=remove-page-link-bookmarks"));
    assert_eq!(exported["counts"]["bookmarks"], 2);

    let add_active_bookmark = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![
            ("id".to_owned(), payload.id.clone()),
            ("action".to_owned(), "add-bookmark".to_owned()),
        ],
    };
    let (payload, _) = registry.apply_target(&add_active_bookmark).await.unwrap();
    assert!(payload.current_bookmarked);
    assert_eq!(payload.bookmarks.len(), 3);

    let remove_link_bookmarks = RequestTarget {
        path: "/browser".to_owned(),
        params: form_urlencoded::parse(
            remove_link_bookmarks_href
                .trim_start_matches("/browser?")
                .as_bytes(),
        )
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect(),
    };
    let (payload, back_href) = registry.apply_target(&remove_link_bookmarks).await.unwrap();
    assert_eq!(payload.id, active_id);
    assert_eq!(payload.title, "Bookmark Links");
    assert!(payload.current_bookmarked);
    assert_eq!(payload.bookmarks.len(), 1);
    assert_eq!(payload.bookmarks[0].title, "Bookmark Links");
    assert!(payload.bookmarks[0].source.ends_with("bookmark-links.html"));
    assert!(payload.bookmarks.iter().all(|bookmark| {
        !bookmark.source.ends_with("bookmark-link-second.html")
            && !bookmark.source.ends_with("bookmark-link-third.html")
    }));
    let html = render_browser_session_page(&payload, &back_href);
    assert!(html.contains("action=bookmark-page-links"));
    assert!(!html.contains("action=remove-page-link-bookmarks"));
    let response = browser_session_api_response(&state_export, &payload);
    let exported: serde_json::Value = serde_json::from_str(&response.body).unwrap();
    assert!(
        exported["action_urls"]["bookmark_page_links"]
            .as_str()
            .unwrap()
            .contains("action=bookmark-page-links")
    );
    assert!(exported["action_urls"]["remove_page_link_bookmarks"].is_null());
    assert_eq!(exported["counts"]["bookmarks"], 1);
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
    assert!(html.contains(r#"name="action" value="open-background-session""#));
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
    assert!(html.contains(r#"action" value="link-text-background-session""#));
    assert!(html.contains(r#"action" value="link-selector-background-session""#));

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

    let switch_first = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![
            ("id".to_owned(), first_id.clone()),
            ("action".to_owned(), "current".to_owned()),
        ],
    };
    let (payload, _) = registry.apply_target(&switch_first).await.unwrap();
    assert_eq!(payload.id, first_id);
    let open_text_background = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![
            ("id".to_owned(), first_id.clone()),
            (
                "action".to_owned(),
                "link-text-background-session".to_owned(),
            ),
            ("text".to_owned(), "Text Target".to_owned()),
        ],
    };
    let (payload, _) = registry.apply_target(&open_text_background).await.unwrap();
    assert_eq!(payload.id, first_id);
    assert_eq!(payload.title, "First");
    assert_eq!(payload.sessions.len(), 4);
    assert!(
        payload
            .sessions
            .iter()
            .any(|session| { session.page_title == "Text Target" && !session.current })
    );

    let open_selector_background = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![
            ("id".to_owned(), first_id.clone()),
            (
                "action".to_owned(),
                "link-selector-background-session".to_owned(),
            ),
            ("selector".to_owned(), "#by-selector".to_owned()),
        ],
    };
    let (payload, _) = registry
        .apply_target(&open_selector_background)
        .await
        .unwrap();
    assert_eq!(payload.id, first_id);
    assert_eq!(payload.title, "First");
    assert_eq!(payload.sessions.len(), 5);
    assert!(
        payload
            .sessions
            .iter()
            .any(|session| { session.page_title == "Selector Target" && !session.current })
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
    assert!(html.contains("Duplicate bg"));

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
    let (payload, back_href) = registry.apply_target(&duplicate).await.unwrap();

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

    let html = render_browser_session_page(&payload, &back_href);
    let duplicate_marker = ">Duplicate tab</a>";
    let marker_index = html.find(duplicate_marker).unwrap();
    let href_start = html[..marker_index].rfind("href=\"").unwrap() + "href=\"".len();
    let href_end = href_start + html[href_start..marker_index].find('"').unwrap();
    let toolbar_duplicate_href = html[href_start..href_end].replace("&amp;", "&");
    assert!(toolbar_duplicate_href.contains("action=duplicate-session"));
    assert!(toolbar_duplicate_href.contains(&format!("session={}", payload.id)));
    let toolbar_duplicate = RequestTarget {
        path: "/browser".to_owned(),
        params: form_urlencoded::parse(
            toolbar_duplicate_href
                .trim_start_matches("/browser?")
                .as_bytes(),
        )
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect(),
    };
    let (payload, _) = registry.apply_target(&toolbar_duplicate).await.unwrap();
    assert_eq!(payload.title, "Second");
    assert_eq!(payload.sessions.len(), 3);
    assert_eq!(payload.history_len, 2);
    assert!(payload.can_back);
    assert!(payload.viewport.contains("duplicate destination"));

    let active_id = payload.id.clone();
    let background_duplicate_href = payload
        .sessions
        .iter()
        .find(|session| session.id == active_id)
        .unwrap()
        .duplicate_background_url
        .clone();
    assert!(background_duplicate_href.contains("action=duplicate-background-session"));
    let background_duplicate = RequestTarget {
        path: "/browser".to_owned(),
        params: form_urlencoded::parse(
            background_duplicate_href
                .trim_start_matches("/browser?")
                .as_bytes(),
        )
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect(),
    };
    let (payload, _) = registry.apply_target(&background_duplicate).await.unwrap();
    assert_eq!(payload.id, active_id);
    assert_eq!(payload.title, "Second");
    assert_eq!(payload.sessions.len(), 4);
    assert_eq!(payload.history_len, 2);
    assert!(payload.can_back);
    assert!(payload.viewport.contains("duplicate destination"));
    assert_eq!(
        payload
            .sessions
            .iter()
            .filter(|session| session.page_title == "Second")
            .count(),
        4
    );
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
    let (payload, back_href) = registry.apply_target(&open_third).await.unwrap();
    assert_eq!(payload.title, "Third");
    assert_eq!(payload.history_len, 3);
    assert_eq!(payload.current_history_index, Some(2));

    let html = render_browser_session_page(&payload, &back_href);
    assert!(html.contains("History CSV"));
    assert!(html.contains("format=history-csv"));
    let history_csv_export = RequestTarget {
        path: "/api/browser-session".to_owned(),
        params: vec![
            ("id".to_owned(), payload.id.clone()),
            ("format".to_owned(), "history-csv".to_owned()),
        ],
    };
    let response = browser_session_api_response(&history_csv_export, &payload);
    assert_eq!(response.status, 200);
    assert_eq!(response.content_type, "text/csv; charset=utf-8");
    assert!(response.body.starts_with(
        "index,title,source,target,current,action_url,new_session_url,background_session_url,session_id,active_source,history_len\n"
    ));
    assert_eq!(response.body.lines().count(), 4);
    assert!(response.body.contains(",First,"));
    assert!(response.body.contains(",Second,"));
    assert!(response.body.contains(",Third,"));
    assert!(response.body.contains(",true,"));
    assert!(response.body.contains("action=history"));
    assert!(response.body.contains("history_len"));

    let state_export = RequestTarget {
        path: "/api/browser-session".to_owned(),
        params: vec![
            ("id".to_owned(), payload.id.clone()),
            ("format".to_owned(), "session-state".to_owned()),
        ],
    };
    let response = browser_session_api_response(&state_export, &payload);
    assert_eq!(response.status, 200);
    let exported: serde_json::Value = serde_json::from_str(&response.body).unwrap();
    assert_eq!(exported["history"]["len"], 3);
    assert_eq!(exported["history"]["current_index"], 2);
    assert_eq!(exported["history_entries"].as_array().unwrap().len(), 3);
    assert_eq!(exported["history_entries"][0]["title"], "First");
    assert_eq!(exported["history_entries"][1]["title"], "Second");
    assert_eq!(exported["history_entries"][2]["title"], "Third");
    assert_eq!(exported["history_entries"][2]["current"], true);
    assert!(
        exported["history_entries"][0]["action_url"]
            .as_str()
            .unwrap()
            .contains("action=history")
    );
    assert!(
        exported["history_entries"][1]["new_session_url"]
            .as_str()
            .unwrap()
            .contains("url=")
    );
    assert!(
        exported["history_entries"][1]["background_session_url"]
            .as_str()
            .unwrap()
            .contains("action=open-background-session")
    );

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
async fn browser_session_registry_reports_and_jumps_page_anchors() {
    let dir = tempfile::tempdir().unwrap();
    let page = dir.path().join("anchors.html");
    let trailing = (0..24)
        .map(|index| format!("<p>Trailing section {index}</p>\n"))
        .collect::<String>();
    std::fs::write(
        &page,
        format!(
            r#"<!doctype html>
<title>Anchors</title>
<h1 id="top">Top</h1>
<p>Intro one</p>
<p>Intro two</p>
<p>Intro three</p>
<section id="details"><h2>Details</h2><p>Target section body</p></section>
<p>More one</p>
<p>More two</p>
<p id="summary">Summary</p>
{trailing}"#
        ),
    )
    .unwrap();

    let registry = BrowserSessionRegistry::default();
    let create = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![
            ("url".to_owned(), page.display().to_string()),
            ("height".to_owned(), "4".to_owned()),
        ],
    };
    let (payload, back_href) = registry.create_target(&create).await.unwrap();
    let original_id = payload.id.clone();
    assert_eq!(payload.title, "Anchors");
    assert_eq!(payload.anchor_count, 3);
    assert_eq!(payload.anchors.len(), 3);
    assert!(payload.anchors.iter().any(|anchor| anchor.name == "top"));
    let details = payload
        .anchors
        .iter()
        .find(|anchor| anchor.name == "details")
        .unwrap();
    assert!(details.y > 0);
    assert!(details.action_url.contains("action=anchor"));
    assert!(
        details
            .action_url
            .contains(&format!("anchor={}", details.index + 1))
    );
    assert!(
        details
            .new_session_url
            .contains("action=anchor-new-session")
    );
    assert!(
        details
            .new_session_url
            .contains(&format!("anchor={}", details.index + 1))
    );
    assert!(
        details
            .background_session_url
            .contains("action=anchor-background-session")
    );
    assert!(
        details
            .background_session_url
            .contains(&format!("anchor={}", details.index + 1))
    );
    let details_y = details.y;
    let details_action_url = details.action_url.clone();
    let summary = payload
        .anchors
        .iter()
        .find(|anchor| anchor.name == "summary")
        .unwrap();
    let summary_y = summary.y;
    let summary_new_session_url = summary.new_session_url.clone();
    let top = payload
        .anchors
        .iter()
        .find(|anchor| anchor.name == "top")
        .unwrap();
    let top_y = top.y;

    let html = render_browser_session_page(&payload, &back_href);
    assert!(html.contains("Page Anchors (3)"));
    assert!(html.contains("Anchors CSV"));
    assert!(html.contains("format=anchors-csv"));
    assert!(html.contains("action=anchor"));
    assert!(html.contains("action=anchor-new-session"));
    assert!(html.contains("action=anchor-background-session"));
    assert!(html.contains("New session"));
    assert!(html.contains("Background"));
    assert!(html.contains("details"));

    let anchors_csv_export = RequestTarget {
        path: "/api/browser-session".to_owned(),
        params: vec![
            ("id".to_owned(), payload.id.clone()),
            ("format".to_owned(), "anchors-csv".to_owned()),
        ],
    };
    let response = browser_session_api_response(&anchors_csv_export, &payload);
    assert_eq!(response.status, 200);
    assert_eq!(response.content_type, "text/csv; charset=utf-8");
    assert!(
        response
            .body
            .starts_with("index,name,y,action_url,new_session_url,background_session_url,session_id,source,total_anchor_count\n")
    );
    assert_eq!(response.body.lines().count(), 4);
    assert!(response.body.contains("details"));
    assert!(response.body.contains("action=anchor"));
    assert!(response.body.contains("action=anchor-new-session"));
    assert!(response.body.contains("action=anchor-background-session"));

    let state_export = RequestTarget {
        path: "/api/browser-session".to_owned(),
        params: vec![
            ("id".to_owned(), payload.id.clone()),
            ("format".to_owned(), "session-state".to_owned()),
        ],
    };
    let response = browser_session_api_response(&state_export, &payload);
    assert_eq!(response.status, 200);
    let exported: serde_json::Value = serde_json::from_str(&response.body).unwrap();
    assert_eq!(exported["counts"]["anchors"], 3);
    assert!(
        exported["export_urls"]["anchors_csv"]
            .as_str()
            .unwrap()
            .contains("format=anchors-csv")
    );
    assert!(
        exported["anchors"]
            .as_array()
            .unwrap()
            .iter()
            .any(|anchor| anchor["name"] == "details"
                && anchor["action_url"]
                    .as_str()
                    .unwrap()
                    .contains("action=anchor")
                && anchor["new_session_url"]
                    .as_str()
                    .unwrap()
                    .contains("action=anchor-new-session")
                && anchor["background_session_url"]
                    .as_str()
                    .unwrap()
                    .contains("action=anchor-background-session"))
    );

    let jump_anchor = RequestTarget {
        path: "/browser".to_owned(),
        params: form_urlencoded::parse(
            details_action_url
                .trim_start_matches("/browser?")
                .as_bytes(),
        )
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect(),
    };
    let (payload, _) = registry.apply_target(&jump_anchor).await.unwrap();
    assert_eq!(payload.viewport_x, 0);
    assert_eq!(payload.viewport_y, details_y);
    assert!(payload.viewport.contains("Details"));
    let top_background_session_url = payload
        .anchors
        .iter()
        .find(|anchor| anchor.name == "top")
        .unwrap()
        .background_session_url
        .clone();

    let open_summary_new_session = RequestTarget {
        path: "/browser".to_owned(),
        params: form_urlencoded::parse(
            summary_new_session_url
                .trim_start_matches("/browser?")
                .as_bytes(),
        )
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect(),
    };
    let (payload, _) = registry
        .apply_target(&open_summary_new_session)
        .await
        .unwrap();
    let summary_session_id = payload.id.clone();
    assert_ne!(summary_session_id, original_id);
    assert_eq!(payload.title, "Anchors");
    assert_eq!(payload.viewport_x, 0);
    assert_eq!(payload.viewport_y, summary_y);
    assert!(payload.viewport.contains("Summary"));
    assert_eq!(payload.sessions.len(), 2);
    assert!(
        payload
            .sessions
            .iter()
            .any(|session| session.id == original_id && !session.current)
    );

    let open_top_background_session = RequestTarget {
        path: "/browser".to_owned(),
        params: form_urlencoded::parse(
            top_background_session_url
                .trim_start_matches("/browser?")
                .as_bytes(),
        )
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect(),
    };
    let (payload, _) = registry
        .apply_target(&open_top_background_session)
        .await
        .unwrap();
    assert_eq!(payload.id, original_id);
    assert_eq!(payload.viewport_y, details_y);
    assert_eq!(payload.sessions.len(), 3);
    let background_session = payload
        .sessions
        .iter()
        .find(|session| !session.current && session.id != summary_session_id)
        .unwrap();
    let background_session_id = background_session.id.clone();
    let switch_background = RequestTarget {
        path: "/browser".to_owned(),
        params: form_urlencoded::parse(
            background_session
                .action_url
                .trim_start_matches("/browser?")
                .as_bytes(),
        )
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect(),
    };
    let (payload, _) = registry.apply_target(&switch_background).await.unwrap();
    assert_eq!(payload.id, background_session_id);
    assert_eq!(payload.viewport_x, 0);
    assert_eq!(payload.viewport_y, top_y);
    assert!(payload.viewport.contains("Top"));
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
    let original_id = payload.id.clone();

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
    assert_eq!(payload.find_matches.len(), 2);
    assert_eq!(payload.find_matches[0].line + 1, 2);
    assert!(payload.find_matches[0].current);
    assert!(payload.find_matches[0].text.contains("needle first"));
    assert_eq!(payload.find_matches[1].line + 1, 4);
    assert!(!payload.find_matches[1].current);
    assert!(
        payload.find_matches[1]
            .action_url
            .contains("action=find-match")
    );
    assert!(payload.find_matches[1].action_url.contains("match=2"));
    assert!(
        payload.find_matches[1]
            .new_session_url
            .contains("action=find-match-new-session")
    );
    assert!(payload.find_matches[1].new_session_url.contains("match=2"));
    assert!(
        payload.find_matches[1]
            .background_session_url
            .contains("action=find-match-background-session")
    );
    assert!(
        payload.find_matches[1]
            .background_session_url
            .contains("match=2")
    );
    assert!(payload.viewport.contains("needle first"));
    let html = render_browser_session_page(&payload, "/search?q=find");
    assert!(html.contains("<mark>needle</mark> first"));
    assert!(html.contains("Find JSON"));
    assert!(html.contains("format=find-json"));
    assert!(html.contains("Find CSV"));
    assert!(html.contains("format=find-csv"));
    assert!(html.contains(r#"class="find-match current""#));
    assert!(html.contains("action=find-match"));
    assert!(html.contains("action=find-match-new-session"));
    assert!(html.contains("action=find-match-background-session"));
    assert!(html.contains("action=open-find-matches-new-sessions"));
    assert!(html.contains("action=open-find-matches-background-sessions"));
    let find_json_export = RequestTarget {
        path: "/api/browser-session".to_owned(),
        params: vec![
            ("id".to_owned(), payload.id.clone()),
            ("format".to_owned(), "find-json".to_owned()),
        ],
    };
    let response = browser_session_api_response(&find_json_export, &payload);
    assert_eq!(response.status, 200);
    assert_eq!(response.content_type, "application/json; charset=utf-8");
    let exported_find: serde_json::Value = serde_json::from_str(&response.body).unwrap();
    assert_eq!(exported_find["format"], "browser-find");
    assert_eq!(exported_find["id"], payload.id);
    assert_eq!(exported_find["query"], "needle");
    assert_eq!(exported_find["match_count"], 2);
    assert_eq!(exported_find["current_index"], 0);
    assert_eq!(exported_find["current_line"], 1);
    assert_eq!(exported_find["matches"].as_array().unwrap().len(), 2);
    assert_eq!(exported_find["matches"][0]["text"], "needle first");
    assert_eq!(exported_find["matches"][0]["current"], true);
    assert!(
        exported_find["matches"][1]["action_url"]
            .as_str()
            .unwrap()
            .contains("action=find-match")
    );
    assert!(
        exported_find["matches"][1]["new_session_url"]
            .as_str()
            .unwrap()
            .contains("action=find-match-new-session")
    );
    assert!(
        exported_find["matches"][1]["background_session_url"]
            .as_str()
            .unwrap()
            .contains("action=find-match-background-session")
    );
    assert!(
        exported_find["csv_url"]
            .as_str()
            .unwrap()
            .contains("format=find-csv")
    );
    assert!(
        exported_find["session_state_url"]
            .as_str()
            .unwrap()
            .contains("format=session-state")
    );
    let find_csv_export = RequestTarget {
        path: "/api/browser-session".to_owned(),
        params: vec![
            ("id".to_owned(), payload.id.clone()),
            ("format".to_owned(), "find-csv".to_owned()),
        ],
    };
    let response = browser_session_api_response(&find_csv_export, &payload);
    assert_eq!(response.status, 200);
    assert_eq!(response.content_type, "text/csv; charset=utf-8");
    assert!(response.body.starts_with("match_index,line,current,query,text,action_url,new_session_url,background_session_url,session_id,source,match_count,current_match_index,current_line\n"));
    assert_eq!(response.body.lines().count(), 3);
    assert!(response.body.contains("1,2,true,needle,needle first"));
    assert!(response.body.contains("2,4,false,needle,needle second"));
    assert!(response.body.contains("action=find-match"));
    assert!(response.body.contains("action=find-match-new-session"));
    assert!(
        response
            .body
            .contains("action=find-match-background-session")
    );
    assert!(response.body.contains(",2,1,2"));
    let state_export = RequestTarget {
        path: "/api/browser-session".to_owned(),
        params: vec![
            ("id".to_owned(), payload.id.clone()),
            ("format".to_owned(), "session-state".to_owned()),
        ],
    };
    let response = browser_session_api_response(&state_export, &payload);
    assert_eq!(response.status, 200);
    let exported: serde_json::Value = serde_json::from_str(&response.body).unwrap();
    assert_eq!(exported["find"]["query"], "needle");
    assert_eq!(exported["find"]["match_count"], 2);
    assert_eq!(exported["find"]["current_index"], 0);
    assert_eq!(exported["find"]["current_line"], 1);
    assert_eq!(exported["find"]["matches"].as_array().unwrap().len(), 2);
    assert_eq!(exported["find"]["matches"][0]["text"], "needle first");
    assert_eq!(exported["find"]["matches"][0]["current"], true);
    assert!(
        exported["find"]["matches"][1]["action_url"]
            .as_str()
            .unwrap()
            .contains("action=find-match")
    );
    assert!(
        exported["find"]["matches"][1]["new_session_url"]
            .as_str()
            .unwrap()
            .contains("action=find-match-new-session")
    );
    assert!(
        exported["find"]["matches"][1]["background_session_url"]
            .as_str()
            .unwrap()
            .contains("action=find-match-background-session")
    );
    assert!(
        exported["action_urls"]["open_find_matches_new_sessions"]
            .as_str()
            .unwrap()
            .contains("action=open-find-matches-new-sessions")
    );
    assert!(
        exported["action_urls"]["open_find_matches_background"]
            .as_str()
            .unwrap()
            .contains("action=open-find-matches-background-sessions")
    );
    assert!(
        exported["action_urls"]["clear_find"]
            .as_str()
            .unwrap()
            .contains("action=clear-find")
    );
    let state_csv_export = RequestTarget {
        path: "/api/browser-session".to_owned(),
        params: vec![
            ("id".to_owned(), payload.id.clone()),
            ("format".to_owned(), "session-state-csv".to_owned()),
        ],
    };
    let response = browser_session_api_response(&state_csv_export, &payload);
    assert_eq!(response.status, 200);
    assert_eq!(response.content_type, "text/csv; charset=utf-8");
    assert!(response.body.contains("find,,needle,match_count,2,1,2,"));
    assert!(response.body.contains("action=clear-find"));

    let jump_second = RequestTarget {
        path: "/browser".to_owned(),
        params: form_urlencoded::parse(
            payload.find_matches[1]
                .action_url
                .trim_start_matches("/browser?")
                .as_bytes(),
        )
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect(),
    };
    let (payload, _) = registry.apply_target(&jump_second).await.unwrap();
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

    let second_new_session_url = payload.find_matches[1].new_session_url.clone();
    let first_background_session_url = payload.find_matches[0].background_session_url.clone();
    let open_second_new_session = RequestTarget {
        path: "/browser".to_owned(),
        params: form_urlencoded::parse(
            second_new_session_url
                .trim_start_matches("/browser?")
                .as_bytes(),
        )
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect(),
    };
    let (payload, _) = registry
        .apply_target(&open_second_new_session)
        .await
        .unwrap();
    let second_session_id = payload.id.clone();
    assert_ne!(second_session_id, original_id);
    assert_eq!(payload.title, "Find");
    assert_eq!(payload.find_query, "needle");
    assert_eq!(payload.find_current_index, Some(1));
    assert!(payload.viewport.contains("needle second"));
    assert_eq!(payload.sessions.len(), 2);
    assert!(
        payload
            .sessions
            .iter()
            .any(|session| session.id == original_id && !session.current)
    );

    let open_first_background_session = RequestTarget {
        path: "/browser".to_owned(),
        params: form_urlencoded::parse(
            first_background_session_url
                .trim_start_matches("/browser?")
                .as_bytes(),
        )
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect(),
    };
    let (payload, _) = registry
        .apply_target(&open_first_background_session)
        .await
        .unwrap();
    assert_eq!(payload.id, original_id);
    assert_eq!(payload.find_current_index, Some(1));
    assert!(payload.viewport.contains("needle second"));
    assert_eq!(payload.sessions.len(), 3);
    let background_session = payload
        .sessions
        .iter()
        .find(|session| !session.current && session.id != second_session_id)
        .unwrap();
    let background_session_id = background_session.id.clone();
    let switch_background = RequestTarget {
        path: "/browser".to_owned(),
        params: form_urlencoded::parse(
            background_session
                .action_url
                .trim_start_matches("/browser?")
                .as_bytes(),
        )
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect(),
    };
    let (payload, _) = registry.apply_target(&switch_background).await.unwrap();
    assert_eq!(payload.id, background_session_id);
    assert_eq!(payload.find_query, "needle");
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
async fn browser_session_registry_opens_find_matches_in_bulk_sessions() {
    let dir = tempfile::tempdir().unwrap();
    let page = dir.path().join("find-bulk.html");
    std::fs::write(
        &page,
        r#"<!doctype html><title>Find Bulk</title><p>needle first</p><p>middle</p><p>needle second</p><p>spacer</p><p>needle third</p>"#,
    )
    .unwrap();

    let registry = BrowserSessionRegistry::default();
    let create = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![("url".to_owned(), page.display().to_string())],
    };
    let (payload, _) = registry.create_target(&create).await.unwrap();
    let original_id = payload.id.clone();
    let find = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![
            ("id".to_owned(), payload.id),
            ("action".to_owned(), "find".to_owned()),
            ("q".to_owned(), "needle".to_owned()),
        ],
    };
    let (payload, back_href) = registry.apply_target(&find).await.unwrap();
    assert_eq!(payload.find_current_index, Some(0));
    assert_eq!(payload.find_match_count, 3);
    let html = render_browser_session_page(&payload, &back_href);
    assert!(html.contains("Open matches tabs"));
    assert!(html.contains("action=open-find-matches-new-sessions"));
    assert!(html.contains("Open matches bg"));
    assert!(html.contains("action=open-find-matches-background-sessions"));

    let open_matches_new_sessions = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![
            ("id".to_owned(), payload.id),
            (
                "action".to_owned(),
                "open-find-matches-new-sessions".to_owned(),
            ),
            ("limit".to_owned(), "2".to_owned()),
        ],
    };
    let (payload, _) = registry
        .apply_target(&open_matches_new_sessions)
        .await
        .unwrap();
    assert_ne!(payload.id, original_id);
    assert_eq!(payload.title, "Find Bulk");
    assert_eq!(payload.find_query, "needle");
    assert_eq!(payload.find_current_index, Some(1));
    assert!(payload.viewport.contains("needle second"));
    assert_eq!(payload.sessions.len(), 3);
    let third_session = payload
        .sessions
        .iter()
        .find(|session| !session.current && session.id != original_id)
        .unwrap();
    let third_session_id = third_session.id.clone();
    let switch_third = RequestTarget {
        path: "/browser".to_owned(),
        params: form_urlencoded::parse(
            third_session
                .action_url
                .trim_start_matches("/browser?")
                .as_bytes(),
        )
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect(),
    };
    let (payload, _) = registry.apply_target(&switch_third).await.unwrap();
    assert_eq!(payload.id, third_session_id);
    assert_eq!(payload.find_current_index, Some(2));
    assert!(payload.viewport.contains("needle third"));

    let background_registry = BrowserSessionRegistry::default();
    let (payload, _) = background_registry.create_target(&create).await.unwrap();
    let original_id = payload.id.clone();
    let find = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![
            ("id".to_owned(), payload.id),
            ("action".to_owned(), "find".to_owned()),
            ("q".to_owned(), "needle".to_owned()),
        ],
    };
    let (payload, _) = background_registry.apply_target(&find).await.unwrap();
    let open_matches_background = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![
            ("id".to_owned(), payload.id),
            (
                "action".to_owned(),
                "open-find-matches-background-sessions".to_owned(),
            ),
            ("limit".to_owned(), "2".to_owned()),
        ],
    };
    let (payload, _) = background_registry
        .apply_target(&open_matches_background)
        .await
        .unwrap();
    assert_eq!(payload.id, original_id);
    assert_eq!(payload.find_current_index, Some(0));
    assert!(payload.viewport.contains("needle first"));
    assert_eq!(payload.sessions.len(), 3);
    let background_actions = payload
        .sessions
        .iter()
        .filter(|session| !session.current)
        .map(|session| session.action_url.clone())
        .collect::<Vec<_>>();
    let mut background_match_indexes = Vec::new();
    for action_url in background_actions {
        let switch_background = RequestTarget {
            path: "/browser".to_owned(),
            params: form_urlencoded::parse(action_url.trim_start_matches("/browser?").as_bytes())
                .map(|(key, value)| (key.into_owned(), value.into_owned()))
                .collect(),
        };
        let (payload, _) = background_registry
            .apply_target(&switch_background)
            .await
            .unwrap();
        background_match_indexes.push(payload.find_current_index.unwrap());
    }
    background_match_indexes.sort_unstable();
    assert_eq!(background_match_indexes, vec![1, 2]);
}

#[tokio::test]
async fn browser_session_registry_scrolls_text_viewport_horizontally() {
    let dir = tempfile::tempdir().unwrap();
    let page = dir.path().join("wide.html");
    let wide_lines = (0..30)
        .map(|index| {
            format!("{index:02} ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789")
        })
        .collect::<Vec<_>>()
        .join("\n");
    std::fs::write(
        &page,
        format!(r#"<!doctype html><title>Wide</title><pre>{wide_lines}</pre>"#),
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
    assert_eq!(payload.viewport_y, 0);
    assert!(payload.max_scroll_x > 0);
    assert!(payload.max_scroll_y > 0);
    assert!(payload.viewport.contains("ABCDEFGHIJKLMNOPQRSTUVWXYZ"));
    let html = render_browser_session_page(&payload, "/search?q=wide");
    assert!(html.contains("<span>Top</span>"));
    assert!(html.contains("<span>Up</span>"));
    assert!(html.contains(">Down</a>"));
    assert!(html.contains(">Bottom</a>"));
    let state_export = RequestTarget {
        path: "/api/browser-session".to_owned(),
        params: vec![
            ("id".to_owned(), payload.id.clone()),
            ("format".to_owned(), "session-state".to_owned()),
        ],
    };
    let response = browser_session_api_response(&state_export, &payload);
    let exported: serde_json::Value = serde_json::from_str(&response.body).unwrap();
    assert!(exported["action_urls"]["top"].is_null());
    assert!(exported["action_urls"]["scroll_up"].is_null());
    assert!(
        exported["action_urls"]["bottom"]
            .as_str()
            .unwrap()
            .contains("action=bottom")
    );
    assert!(
        exported["action_urls"]["scroll_down"]
            .as_str()
            .unwrap()
            .contains("action=scroll")
    );

    let scroll_right = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![
            ("id".to_owned(), payload.id.clone()),
            ("action".to_owned(), "scroll".to_owned()),
            ("dx".to_owned(), "8".to_owned()),
        ],
    };
    let (payload, _) = registry.apply_target(&scroll_right).await.unwrap();
    assert_eq!(payload.viewport_x, 8);
    assert!(payload.viewport.contains("IJKLMNOPQRSTUVWXYZ"));
    assert!(!payload.viewport.contains("ABCDEFGH"));

    let scroll_down = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![
            ("id".to_owned(), payload.id.clone()),
            ("action".to_owned(), "scroll".to_owned()),
            ("viewport_x".to_owned(), payload.viewport_x.to_string()),
            ("dy".to_owned(), "4".to_owned()),
        ],
    };
    let (payload, back_href) = registry.apply_target(&scroll_down).await.unwrap();
    assert_eq!(payload.viewport_x, 8);
    assert_eq!(payload.viewport_y, 4);

    let html = render_browser_session_page(&payload, &back_href);
    assert!(html.contains(">Left</a>"));
    assert!(html.contains(">Top</a>"));
    assert!(html.contains(">Up</a>"));
    assert!(html.contains(">Down</a>"));
    assert!(html.contains(">Right</a>"));
    assert!(html.contains("viewport 40x16 at x=8 y=4"));
    assert!(html.contains(r#"name="viewport_x" value="8""#));
    assert!(html.contains(r#"name="viewport_y" value="4""#));
    assert!(html.contains("viewport-jump"));
    assert!(html.contains(r#"name="action" value="current""#));
    assert!(html.contains(r#"name="x" value="8""#));
    assert!(html.contains(r#"name="y" value="4""#));
    let response = browser_session_api_response(&state_export, &payload);
    let exported: serde_json::Value = serde_json::from_str(&response.body).unwrap();
    assert!(
        exported["action_urls"]["top"]
            .as_str()
            .unwrap()
            .contains("action=top")
    );
    assert!(
        exported["action_urls"]["scroll_up"]
            .as_str()
            .unwrap()
            .contains("action=scroll")
    );
    assert!(
        exported["action_urls"]["bottom"]
            .as_str()
            .unwrap()
            .contains("action=bottom")
    );
    assert!(
        exported["action_urls"]["scroll_down"]
            .as_str()
            .unwrap()
            .contains("action=scroll")
    );

    let jump_viewport = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![
            ("id".to_owned(), payload.id.clone()),
            ("action".to_owned(), "current".to_owned()),
            ("x".to_owned(), "12".to_owned()),
            ("y".to_owned(), "12".to_owned()),
        ],
    };
    let (payload, _) = registry.apply_target(&jump_viewport).await.unwrap();
    assert_eq!(payload.viewport_x, 12);
    assert_eq!(payload.viewport_y, 12);

    let bottom = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![
            ("id".to_owned(), payload.id.clone()),
            ("action".to_owned(), "bottom".to_owned()),
            ("viewport_x".to_owned(), payload.viewport_x.to_string()),
        ],
    };
    let (payload, _) = registry.apply_target(&bottom).await.unwrap();
    assert_eq!(payload.viewport_x, 12);
    assert_eq!(payload.viewport_y, payload.max_scroll_y);
    let response = browser_session_api_response(&state_export, &payload);
    let exported: serde_json::Value = serde_json::from_str(&response.body).unwrap();
    assert!(
        exported["action_urls"]["top"]
            .as_str()
            .unwrap()
            .contains("action=top")
    );
    assert!(
        exported["action_urls"]["scroll_up"]
            .as_str()
            .unwrap()
            .contains("action=scroll")
    );
    assert!(exported["action_urls"]["bottom"].is_null());
    assert!(exported["action_urls"]["scroll_down"].is_null());

    let duplicate_href = browser_session_new_session_href(&payload.source, &payload);
    let duplicate = RequestTarget {
        path: "/browser".to_owned(),
        params: form_urlencoded::parse(duplicate_href.trim_start_matches("/browser?").as_bytes())
            .map(|(key, value)| (key.into_owned(), value.into_owned()))
            .collect(),
    };
    let (duplicate_payload, _) = registry.create_target(&duplicate).await.unwrap();
    assert_ne!(duplicate_payload.id, payload.id);
    assert_eq!(duplicate_payload.viewport_x, payload.viewport_x);
    assert_eq!(duplicate_payload.viewport_y, payload.viewport_y);
    assert!(duplicate_payload.viewport.contains("JKLMNOPQRSTUVWXYZ"));

    let scroll_left = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![
            ("id".to_owned(), payload.id),
            ("action".to_owned(), "scroll".to_owned()),
            ("dx".to_owned(), "-12".to_owned()),
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
    assert_eq!(payload.sessions[0].position, 1);
    assert_eq!(payload.sessions[1].position, 2);
    assert!(payload.sessions[0].can_close);
    assert!(payload.sessions[1].can_close);
    assert!(!payload.sessions[0].can_move_left);
    assert!(payload.sessions[0].can_move_right);
    assert!(payload.sessions[1].can_move_left);
    assert!(!payload.sessions[1].can_move_right);
    assert!(!payload.sessions[0].pinned);
    assert!(!payload.sessions[1].pinned);
    assert!(payload.sessions[0].reload_url.contains("action=reload"));
    assert!(
        payload.sessions[0]
            .move_right_url
            .contains("action=move-tab-right")
    );
    assert!(
        payload.sessions[1]
            .move_left_url
            .contains("action=move-tab-left")
    );
    assert!(payload.sessions[0].close_url.contains("close-session"));
    assert!(payload.sessions[0].pin_url.contains("action=pin-tab"));
    assert!(payload.sessions[0].unpin_url.contains("action=unpin-tab"));
    assert!(!payload.sessions[0].current);
    assert!(payload.sessions[1].current);
    let html = render_browser_session_page(&payload, &back_href);
    assert!(html.contains(">Duplicate tab</a>"));
    assert!(html.contains(">Pin tab</a>"));
    assert!(html.contains(">Pin</a>"));
    assert!(html.contains(">Close tab</a>"));
    assert!(html.contains(">Prev tab</a>"));
    assert!(html.contains(">Next tab</a>"));
    assert!(html.contains(">Move left</a>"));
    assert!(html.contains(">Reload</a>"));
    assert!(html.contains(">Right</a>"));
    assert!(html.contains("action=close-session"));
    assert!(html.contains("close_id="));
    assert!(html.contains("Tabs CSV"));
    assert!(html.contains("format=tabs-csv"));
    assert!(html.contains("Jump tab"));
    assert!(html.contains("value=\"jump-tab\""));

    let pin_first = RequestTarget {
        path: "/browser".to_owned(),
        params: form_urlencoded::parse(
            payload.sessions[0]
                .pin_url
                .trim_start_matches("/browser?")
                .as_bytes(),
        )
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect(),
    };
    let (payload, back_href) = registry.apply_target(&pin_first).await.unwrap();
    assert_eq!(payload.id, second_id);
    assert_eq!(payload.sessions[0].id, first_id);
    assert!(payload.sessions[0].pinned);
    assert!(!payload.sessions[1].pinned);
    assert!(payload.sessions[1].current);
    let html = render_browser_session_page(&payload, &back_href);
    assert!(html.contains("Pinned · One"));
    assert!(html.contains(">Unpin</a>"));
    assert!(html.contains(">Pin tab</a>"));
    assert!(html.contains("Label current tab"));

    let label_first = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![
            ("id".to_owned(), payload.id.clone()),
            ("action".to_owned(), "label-tab".to_owned()),
            ("session".to_owned(), first_id.clone()),
            ("label".to_owned(), "Research one".to_owned()),
        ],
    };
    let (payload, back_href) = registry.apply_target(&label_first).await.unwrap();
    assert_eq!(payload.id, second_id);
    assert_eq!(payload.sessions[0].title, "Research one");
    assert_eq!(payload.sessions[0].page_title, "One");
    assert_eq!(payload.sessions[0].label.as_deref(), Some("Research one"));
    assert!(payload.sessions[0].label_url.contains("action=label-tab"));
    assert!(
        payload.sessions[0]
            .clear_label_url
            .contains("action=clear-tab-label")
    );
    let html = render_browser_session_page(&payload, &back_href);
    assert!(html.contains("Pinned · Research one"));
    assert!(html.contains(">Clear label</a>"));
    assert!(html.contains("Search tabs"));

    let search_tabs = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![
            ("id".to_owned(), payload.id.clone()),
            ("action".to_owned(), "search-tabs".to_owned()),
            ("q".to_owned(), "first session".to_owned()),
        ],
    };
    let (payload, back_href) = registry.apply_target(&search_tabs).await.unwrap();
    assert_eq!(payload.id, second_id);
    assert_eq!(payload.tab_search_query, "first session");
    assert!(
        payload
            .tab_search_results
            .iter()
            .any(|result| result.id == first_id
                && result.title == "Research one"
                && result.page_title == "One"
                && result.field == "text"
                && result.text.contains("first session"))
    );
    let html = render_browser_session_page(&payload, &back_href);
    assert!(html.contains("Tab Search JSON"));
    assert!(html.contains("format=tab-search-json"));
    assert!(html.contains("Tab Search CSV"));
    assert!(html.contains("format=tab-search-csv"));
    assert!(html.contains("Research one"));
    assert!(html.contains("first session"));
    assert!(html.contains(">Reload</a>"));
    assert!(html.contains(">Duplicate</a>"));
    assert!(html.contains(">Duplicate bg</a>"));
    assert!(html.contains(">Unpin</a>"));
    assert!(html.contains(">Close</a>"));

    let tab_search_json_export = RequestTarget {
        path: "/api/browser-session".to_owned(),
        params: vec![
            ("id".to_owned(), payload.id.clone()),
            ("format".to_owned(), "tab-search-json".to_owned()),
        ],
    };
    let response = browser_session_api_response(&tab_search_json_export, &payload);
    assert_eq!(response.status, 200);
    assert_eq!(response.content_type, "application/json; charset=utf-8");
    let exported_tab_search: serde_json::Value = serde_json::from_str(&response.body).unwrap();
    assert_eq!(exported_tab_search["format"], "browser-tab-search");
    assert_eq!(exported_tab_search["id"], payload.id);
    assert_eq!(exported_tab_search["query"], "first session");
    assert!(exported_tab_search["result_count"].as_u64().unwrap() > 0);
    let exported_tab_search_result = exported_tab_search["results"]
        .as_array()
        .unwrap()
        .iter()
        .find(|result| result["id"] == first_id && result["field"] == "text")
        .unwrap();
    assert_eq!(exported_tab_search_result["title"], "Research one");
    assert_eq!(exported_tab_search_result["page_title"], "One");
    assert_eq!(exported_tab_search_result["pinned"], true);
    assert!(
        exported_tab_search_result["reload_url"]
            .as_str()
            .unwrap()
            .contains("action=reload")
    );
    assert!(
        exported_tab_search_result["duplicate_url"]
            .as_str()
            .unwrap()
            .contains("action=duplicate-session")
    );
    assert!(
        exported_tab_search_result["duplicate_background_url"]
            .as_str()
            .unwrap()
            .contains("action=duplicate-background-session")
    );
    assert!(
        exported_tab_search_result["pin_url"]
            .as_str()
            .unwrap()
            .contains("action=pin-tab")
    );
    assert!(
        exported_tab_search_result["unpin_url"]
            .as_str()
            .unwrap()
            .contains("action=unpin-tab")
    );
    assert!(
        exported_tab_search_result["close_url"]
            .as_str()
            .unwrap()
            .contains("action=close-session")
    );
    assert!(
        exported_tab_search["csv_url"]
            .as_str()
            .unwrap()
            .contains("format=tab-search-csv")
    );
    assert!(
        exported_tab_search["session_state_url"]
            .as_str()
            .unwrap()
            .contains("format=session-state")
    );

    let tab_search_csv_export = RequestTarget {
        path: "/api/browser-session".to_owned(),
        params: vec![
            ("id".to_owned(), payload.id.clone()),
            ("format".to_owned(), "tab-search-csv".to_owned()),
        ],
    };
    let response = browser_session_api_response(&tab_search_csv_export, &payload);
    assert_eq!(response.status, 200);
    assert_eq!(response.content_type, "text/csv; charset=utf-8");
    assert!(response.body.starts_with("session_id,title,page_title,label,source,current,pinned,field,line,text,action_url,reload_url,duplicate_url,duplicate_background_url,pin_url,unpin_url,close_url,active_session_id,query,result_count\n"));
    assert!(response.body.contains(&first_id));
    assert!(response.body.contains("Research one"));
    assert!(response.body.contains("first session"));
    assert!(response.body.contains("action=current"));
    assert!(response.body.contains("action=reload"));
    assert!(response.body.contains("action=duplicate-session"));
    assert!(
        response
            .body
            .contains("action=duplicate-background-session")
    );
    assert!(response.body.contains("action=pin-tab"));
    assert!(response.body.contains("action=unpin-tab"));
    assert!(response.body.contains("action=close-session"));

    let tabs_csv_export = RequestTarget {
        path: "/api/browser-session".to_owned(),
        params: vec![
            ("id".to_owned(), payload.id.clone()),
            ("format".to_owned(), "tabs-csv".to_owned()),
        ],
    };
    let response = browser_session_api_response(&tabs_csv_export, &payload);
    assert_eq!(response.status, 200);
    assert_eq!(response.content_type, "text/csv; charset=utf-8");
    assert!(response.body.starts_with("id,position,order,title,page_title,label,source,current,pinned,can_close,can_move_left,can_move_right,action_url,reload_url,move_left_url,move_right_url,duplicate_url,duplicate_background_url,label_url,clear_label_url,pin_url,unpin_url,close_url,active_session_id,back_href\n"));
    assert_eq!(response.body.lines().count(), 3);
    assert!(response.body.contains(&first_id));
    assert!(response.body.contains(&second_id));
    assert!(response.body.contains(",Research one,One,Research one,"));
    assert!(response.body.contains(",Two,"));
    assert!(response.body.contains(",false,true,true,false,true,"));
    assert!(response.body.contains(",true,false,true,true,false,"));
    assert!(response.body.contains("action=reload"));
    assert!(
        response
            .body
            .contains("action=duplicate-background-session")
    );
    assert!(response.body.contains("action=move-tab-left"));
    assert!(response.body.contains("action=move-tab-right"));
    assert!(response.body.contains("action=label-tab"));
    assert!(response.body.contains("action=clear-tab-label"));
    assert!(response.body.contains("action=pin-tab"));
    assert!(response.body.contains("action=unpin-tab"));
    assert!(response.body.contains("close-session"));

    let state_export = RequestTarget {
        path: "/api/browser-session".to_owned(),
        params: vec![
            ("id".to_owned(), payload.id.clone()),
            ("format".to_owned(), "session-state".to_owned()),
        ],
    };
    let response = browser_session_api_response(&state_export, &payload);
    assert_eq!(response.status, 200);
    let exported: serde_json::Value = serde_json::from_str(&response.body).unwrap();
    assert_eq!(exported["counts"]["open_sessions"], 2);
    assert_eq!(exported["counts"]["pinned_tabs"], 1);
    assert!(exported["counts"]["tab_search_results"].as_u64().unwrap() > 0);
    assert_eq!(exported["tab_search"]["query"], "first session");
    assert!(exported["tab_search"]["result_count"].as_u64().unwrap() > 0);
    let exported_tab_search_result = exported["tab_search"]["results"]
        .as_array()
        .unwrap()
        .iter()
        .find(|result| result["id"] == first_id && result["field"] == "text")
        .unwrap();
    assert_eq!(exported_tab_search_result["pinned"], true);
    assert!(
        exported_tab_search_result["reload_url"]
            .as_str()
            .unwrap()
            .contains("action=reload")
    );
    assert!(
        exported_tab_search_result["duplicate_url"]
            .as_str()
            .unwrap()
            .contains("action=duplicate-session")
    );
    assert!(
        exported_tab_search_result["duplicate_background_url"]
            .as_str()
            .unwrap()
            .contains("action=duplicate-background-session")
    );
    assert!(
        exported_tab_search_result["pin_url"]
            .as_str()
            .unwrap()
            .contains("action=pin-tab")
    );
    assert!(
        exported_tab_search_result["unpin_url"]
            .as_str()
            .unwrap()
            .contains("action=unpin-tab")
    );
    assert!(
        exported_tab_search_result["close_url"]
            .as_str()
            .unwrap()
            .contains("action=close-session")
    );
    assert!(
        exported["export_urls"]["tab_search_json"]
            .as_str()
            .unwrap()
            .contains("format=tab-search-json")
    );
    assert!(
        exported["export_urls"]["tab_search_csv"]
            .as_str()
            .unwrap()
            .contains("format=tab-search-csv")
    );
    assert_eq!(exported["tabs"].as_array().unwrap().len(), 2);
    assert_eq!(exported["tabs"][0]["id"], first_id);
    assert_eq!(exported["tabs"][1]["id"], second_id);
    assert_eq!(exported["tabs"][0]["position"], 1);
    assert_eq!(exported["tabs"][1]["position"], 2);
    assert_eq!(exported["tabs"][0]["current"], false);
    assert_eq!(exported["tabs"][1]["current"], true);
    assert_eq!(exported["tabs"][0]["can_move_left"], false);
    assert_eq!(exported["tabs"][0]["can_move_right"], true);
    assert_eq!(exported["tabs"][1]["can_move_left"], true);
    assert_eq!(exported["tabs"][1]["can_move_right"], false);
    assert_eq!(exported["tabs"][0]["title"], "Research one");
    assert_eq!(exported["tabs"][0]["page_title"], "One");
    assert_eq!(exported["tabs"][0]["label"], "Research one");
    assert!(exported["tabs"][1]["label"].is_null());
    assert_eq!(exported["tabs"][0]["pinned"], true);
    assert_eq!(exported["tabs"][1]["pinned"], false);
    assert!(
        exported["tabs"][0]["action_url"]
            .as_str()
            .unwrap()
            .contains("action=current")
    );
    assert!(
        exported["tabs"][1]["close_url"]
            .as_str()
            .unwrap()
            .contains("action=close-session")
    );
    assert!(
        exported["tabs"][0]["unpin_url"]
            .as_str()
            .unwrap()
            .contains("action=unpin-tab")
    );
    assert!(
        exported["tabs"][0]["clear_label_url"]
            .as_str()
            .unwrap()
            .contains("action=clear-tab-label")
    );
    assert!(
        exported["tabs"][1]["move_left_url"]
            .as_str()
            .unwrap()
            .contains("action=move-tab-left")
    );
    assert!(
        exported["action_urls"]["move_tab_left"]
            .as_str()
            .unwrap()
            .contains("action=move-tab-left")
    );
    assert!(exported["action_urls"]["move_tab_right"].is_null());
    let exported_close_tab_url = exported["action_urls"]["close_tab"].as_str().unwrap();
    assert!(exported_close_tab_url.contains("action=close-session"));
    assert!(exported_close_tab_url.contains(&format!("close_id={second_id}")));
    assert!(
        exported["action_urls"]["clear_tab_search"]
            .as_str()
            .unwrap()
            .contains("action=clear-tab-search")
    );

    let search_result = payload
        .tab_search_results
        .iter()
        .find(|result| result.id == first_id && result.field == "text")
        .unwrap()
        .clone();
    let unpin_from_search_result = RequestTarget {
        path: "/browser".to_owned(),
        params: form_urlencoded::parse(
            search_result
                .unpin_url
                .trim_start_matches("/browser?")
                .as_bytes(),
        )
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect(),
    };
    let (payload, _) = registry
        .apply_target(&unpin_from_search_result)
        .await
        .unwrap();
    assert!(!payload.sessions[0].pinned);
    let pin_from_search_result = RequestTarget {
        path: "/browser".to_owned(),
        params: form_urlencoded::parse(
            search_result
                .pin_url
                .trim_start_matches("/browser?")
                .as_bytes(),
        )
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect(),
    };
    let (payload, _) = registry
        .apply_target(&pin_from_search_result)
        .await
        .unwrap();
    assert!(payload.sessions[0].pinned);

    let clear_tab_search = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![
            ("id".to_owned(), payload.id.clone()),
            ("action".to_owned(), "clear-tab-search".to_owned()),
        ],
    };
    let (payload, _) = registry.apply_target(&clear_tab_search).await.unwrap();
    assert!(payload.tab_search_query.is_empty());
    assert!(payload.tab_search_results.is_empty());

    let jump_label = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![
            ("id".to_owned(), payload.id.clone()),
            ("action".to_owned(), "jump-tab".to_owned()),
            ("q".to_owned(), "research".to_owned()),
        ],
    };
    let (payload, _) = registry.apply_target(&jump_label).await.unwrap();
    assert_eq!(payload.id, first_id);
    assert_eq!(payload.title, "One");

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
    assert!(payload.sessions[0].pinned);
    assert_eq!(payload.sessions[0].label.as_deref(), Some("Research one"));
    assert!(!payload.sessions[1].current);
    assert!(payload.viewport.contains("first session"));

    let unpin_first = RequestTarget {
        path: "/browser".to_owned(),
        params: form_urlencoded::parse(
            payload.sessions[0]
                .unpin_url
                .trim_start_matches("/browser?")
                .as_bytes(),
        )
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect(),
    };
    let (payload, _) = registry.apply_target(&unpin_first).await.unwrap();
    assert_eq!(payload.id, first_id);
    assert!(!payload.sessions[0].pinned);

    let clear_label = RequestTarget {
        path: "/browser".to_owned(),
        params: form_urlencoded::parse(
            payload.sessions[0]
                .clear_label_url
                .trim_start_matches("/browser?")
                .as_bytes(),
        )
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect(),
    };
    let (payload, _) = registry.apply_target(&clear_label).await.unwrap();
    assert_eq!(payload.sessions[0].title, "One");
    assert!(payload.sessions[0].label.is_none());

    let create_third = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![("url".to_owned(), third.display().to_string())],
    };
    let (payload, _) = registry.create_target(&create_third).await.unwrap();
    let third_id = payload.id.clone();
    assert_eq!(payload.sessions.len(), 3);
    assert!(payload.sessions[2].current);

    let jump_second = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![
            ("id".to_owned(), payload.id.clone()),
            ("action".to_owned(), "jump-tab".to_owned()),
            ("q".to_owned(), "two".to_owned()),
        ],
    };
    let (payload, _) = registry.apply_target(&jump_second).await.unwrap();
    assert_eq!(payload.id, second_id);
    assert_eq!(payload.title, "Two");
    assert_eq!(payload.sessions.len(), 3);
    assert!(!payload.sessions[0].current);
    assert!(payload.sessions[1].current);
    assert!(!payload.sessions[2].current);
    assert!(payload.viewport.contains("second session"));

    let state_export = RequestTarget {
        path: "/api/browser-session".to_owned(),
        params: vec![
            ("id".to_owned(), payload.id.clone()),
            ("format".to_owned(), "session-state".to_owned()),
        ],
    };
    let response = browser_session_api_response(&state_export, &payload);
    let exported: serde_json::Value = serde_json::from_str(&response.body).unwrap();
    let close_second_href = exported["action_urls"]["close_tab"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(close_second_href.contains("action=close-session"));
    assert!(close_second_href.contains(&format!("close_id={second_id}")));
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
async fn browser_session_registry_pins_and_unpins_all_open_tabs() {
    let dir = tempfile::tempdir().unwrap();
    let first = dir.path().join("pin-one.html");
    let second = dir.path().join("pin-two.html");
    let third = dir.path().join("pin-three.html");
    std::fs::write(
        &first,
        r#"<!doctype html><title>Pin One</title><p>first tab</p>"#,
    )
    .unwrap();
    std::fs::write(
        &second,
        r#"<!doctype html><title>Pin Two</title><p>second tab</p>"#,
    )
    .unwrap();
    std::fs::write(
        &third,
        r#"<!doctype html><title>Pin Three</title><p>third active tab</p>"#,
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
    assert_eq!(payload.id, "s3");
    assert_eq!(payload.sessions.len(), 3);
    assert!(payload.sessions.iter().all(|session| !session.pinned));
    let html = render_browser_session_page(&payload, &back_href);
    assert!(html.contains(">Pin all</"));
    assert!(html.contains("action=pin-all-tabs"));
    let state_export = RequestTarget {
        path: "/api/browser-session".to_owned(),
        params: vec![
            ("id".to_owned(), payload.id.clone()),
            ("format".to_owned(), "session-state".to_owned()),
        ],
    };
    let response = browser_session_api_response(&state_export, &payload);
    let exported: serde_json::Value = serde_json::from_str(&response.body).unwrap();
    assert_eq!(exported["counts"]["pinned_tabs"], 0);
    assert!(
        exported["action_urls"]["pin_all_tabs"]
            .as_str()
            .unwrap()
            .contains("action=pin-all-tabs")
    );
    assert!(exported["action_urls"]["unpin_all_tabs"].is_null());

    let pin_all = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![
            ("id".to_owned(), payload.id.clone()),
            ("action".to_owned(), "pin-all-tabs".to_owned()),
        ],
    };
    let (payload, back_href) = registry.apply_target(&pin_all).await.unwrap();
    assert_eq!(payload.id, "s3");
    assert!(payload.sessions.iter().all(|session| session.pinned));
    assert!(payload.sessions[2].current);
    let html = render_browser_session_page(&payload, &back_href);
    assert!(html.contains(">Unpin all</"));
    assert!(html.contains("action=unpin-all-tabs"));
    let response = browser_session_api_response(&state_export, &payload);
    let exported: serde_json::Value = serde_json::from_str(&response.body).unwrap();
    assert_eq!(exported["counts"]["pinned_tabs"], 3);
    assert!(exported["action_urls"]["pin_all_tabs"].is_null());
    assert!(
        exported["action_urls"]["unpin_all_tabs"]
            .as_str()
            .unwrap()
            .contains("action=unpin-all-tabs")
    );

    let unpin_all = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![
            ("id".to_owned(), payload.id),
            ("action".to_owned(), "unpin-all-tabs".to_owned()),
        ],
    };
    let (payload, _) = registry.apply_target(&unpin_all).await.unwrap();
    assert_eq!(payload.id, "s3");
    assert!(payload.sessions.iter().all(|session| !session.pinned));
    assert!(payload.sessions[2].current);
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
async fn browser_session_registry_moves_tabs_and_closes_by_display_order() {
    let dir = tempfile::tempdir().unwrap();
    let first = dir.path().join("one.html");
    let second = dir.path().join("two.html");
    let third = dir.path().join("three.html");
    std::fs::write(
        &first,
        r#"<!doctype html><title>One</title><p>first ordered tab</p>"#,
    )
    .unwrap();
    std::fs::write(
        &second,
        r#"<!doctype html><title>Two</title><p>second ordered tab</p>"#,
    )
    .unwrap();
    std::fs::write(
        &third,
        r#"<!doctype html><title>Three</title><p>third ordered tab</p>"#,
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
    assert_eq!(payload.id, "s3");
    assert_eq!(
        payload
            .sessions
            .iter()
            .map(|session| session.id.as_str())
            .collect::<Vec<_>>(),
        vec!["s1", "s2", "s3"]
    );
    assert_eq!(payload.sessions[2].position, 3);
    assert!(payload.sessions[2].can_move_left);
    assert!(!payload.sessions[2].can_move_right);
    let html = render_browser_session_page(&payload, &back_href);
    assert!(html.contains(">Move left</a>"));
    assert!(html.contains("action=move-tab-left"));

    let move_third_left = RequestTarget {
        path: "/browser".to_owned(),
        params: form_urlencoded::parse(
            payload.sessions[2]
                .move_left_url
                .trim_start_matches("/browser?")
                .as_bytes(),
        )
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect(),
    };
    let (payload, back_href) = registry.apply_target(&move_third_left).await.unwrap();
    assert_eq!(payload.id, "s3");
    assert_eq!(payload.title, "Three");
    assert_eq!(
        payload
            .sessions
            .iter()
            .map(|session| session.id.as_str())
            .collect::<Vec<_>>(),
        vec!["s1", "s3", "s2"]
    );
    assert!(payload.sessions[1].current);
    assert_eq!(payload.sessions[1].position, 2);
    assert!(payload.sessions[1].can_move_left);
    assert!(payload.sessions[1].can_move_right);
    let html = render_browser_session_page(&payload, &back_href);
    assert!(html.contains(">Move left</a>"));
    assert!(html.contains(">Move right</a>"));
    assert!(html.contains(">Close left</a>"));
    assert!(html.contains(">Close right</a>"));

    let move_first_right = RequestTarget {
        path: "/browser".to_owned(),
        params: form_urlencoded::parse(
            payload.sessions[0]
                .move_right_url
                .trim_start_matches("/browser?")
                .as_bytes(),
        )
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect(),
    };
    let (payload, _) = registry.apply_target(&move_first_right).await.unwrap();
    assert_eq!(payload.id, "s3");
    assert_eq!(
        payload
            .sessions
            .iter()
            .map(|session| session.id.as_str())
            .collect::<Vec<_>>(),
        vec!["s3", "s1", "s2"]
    );
    assert!(payload.sessions[0].current);

    let move_active_right = RequestTarget {
        path: "/browser".to_owned(),
        params: form_urlencoded::parse(
            payload.sessions[0]
                .move_right_url
                .trim_start_matches("/browser?")
                .as_bytes(),
        )
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect(),
    };
    let (payload, _) = registry.apply_target(&move_active_right).await.unwrap();
    assert_eq!(
        payload
            .sessions
            .iter()
            .map(|session| session.id.as_str())
            .collect::<Vec<_>>(),
        vec!["s1", "s3", "s2"]
    );
    assert!(payload.sessions[1].current);

    let state_export = RequestTarget {
        path: "/api/browser-session".to_owned(),
        params: vec![
            ("id".to_owned(), payload.id.clone()),
            ("format".to_owned(), "session-state".to_owned()),
        ],
    };
    let response = browser_session_api_response(&state_export, &payload);
    assert_eq!(response.status, 200);
    let exported: serde_json::Value = serde_json::from_str(&response.body).unwrap();
    assert_eq!(exported["tabs"][0]["id"], "s1");
    assert_eq!(exported["tabs"][1]["id"], "s3");
    assert_eq!(exported["tabs"][2]["id"], "s2");
    assert_eq!(exported["tabs"][1]["position"], 2);
    assert!(
        exported["action_urls"]["move_tab_left"]
            .as_str()
            .unwrap()
            .contains("action=move-tab-left")
    );
    assert!(
        exported["action_urls"]["move_tab_right"]
            .as_str()
            .unwrap()
            .contains("action=move-tab-right")
    );

    let close_left = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![
            ("id".to_owned(), payload.id),
            ("action".to_owned(), "close-tabs-left".to_owned()),
        ],
    };
    let (payload, _) = registry.apply_target(&close_left).await.unwrap();
    assert_eq!(payload.id, "s3");
    assert_eq!(payload.title, "Three");
    assert_eq!(
        payload
            .sessions
            .iter()
            .map(|session| session.id.as_str())
            .collect::<Vec<_>>(),
        vec!["s3", "s2"]
    );
    assert!(payload.sessions[0].current);
    assert!(payload.sessions.iter().all(|session| session.id != "s1"));
    assert_eq!(payload.closed_sessions.len(), 1);
    assert_eq!(payload.closed_sessions[0].title, "One");
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
async fn browser_session_registry_keeps_pinned_tabs_when_closing_others() {
    let dir = tempfile::tempdir().unwrap();
    let first = dir.path().join("one.html");
    let second = dir.path().join("two.html");
    let third = dir.path().join("three.html");
    std::fs::write(
        &first,
        r#"<!doctype html><title>One</title><p>pinned tab</p>"#,
    )
    .unwrap();
    std::fs::write(
        &second,
        r#"<!doctype html><title>Two</title><p>unpinned tab</p>"#,
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
    let (payload, _) = registry.apply_target(&current).await.unwrap();
    let pin_first = RequestTarget {
        path: "/browser".to_owned(),
        params: form_urlencoded::parse(
            payload.sessions[0]
                .pin_url
                .trim_start_matches("/browser?")
                .as_bytes(),
        )
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect(),
    };
    let (payload, _) = registry.apply_target(&pin_first).await.unwrap();
    assert_eq!(payload.id, "s3");
    assert!(payload.sessions[0].pinned);
    assert_eq!(payload.sessions[0].id, "s1");
    assert_eq!(payload.sessions[2].id, "s3");

    let close_others = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![
            ("id".to_owned(), payload.id),
            ("action".to_owned(), "close-other-tabs".to_owned()),
        ],
    };
    let (payload, _) = registry.apply_target(&close_others).await.unwrap();
    assert_eq!(payload.title, "Three");
    assert_eq!(payload.sessions.len(), 2);
    assert!(
        payload
            .sessions
            .iter()
            .any(|session| { session.id == "s1" && session.pinned && !session.current })
    );
    assert!(
        payload
            .sessions
            .iter()
            .any(|session| { session.id == "s3" && !session.pinned && session.current })
    );
    assert!(payload.sessions.iter().all(|session| session.id != "s2"));
    assert_eq!(payload.closed_sessions.len(), 1);
    assert_eq!(payload.closed_sessions[0].title, "Two");
}

#[tokio::test]
async fn browser_session_registry_closes_unpinned_tabs_except_active() {
    let dir = tempfile::tempdir().unwrap();
    let first = dir.path().join("one.html");
    let second = dir.path().join("two.html");
    let third = dir.path().join("three.html");
    let fourth = dir.path().join("four.html");
    std::fs::write(
        &first,
        r#"<!doctype html><title>One</title><p>pinned left tab</p>"#,
    )
    .unwrap();
    std::fs::write(
        &second,
        r#"<!doctype html><title>Two</title><p>active unpinned tab</p>"#,
    )
    .unwrap();
    std::fs::write(
        &third,
        r#"<!doctype html><title>Three</title><p>unpinned close target</p>"#,
    )
    .unwrap();
    std::fs::write(
        &fourth,
        r#"<!doctype html><title>Four</title><p>pinned right tab</p>"#,
    )
    .unwrap();

    let registry = BrowserSessionRegistry::default();
    for page in [&first, &second, &third, &fourth] {
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
    let (payload, _) = registry.apply_target(&switch_second).await.unwrap();
    let pin_first = RequestTarget {
        path: "/browser".to_owned(),
        params: form_urlencoded::parse(
            payload.sessions[0]
                .pin_url
                .trim_start_matches("/browser?")
                .as_bytes(),
        )
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect(),
    };
    let (payload, _) = registry.apply_target(&pin_first).await.unwrap();
    let pin_fourth = RequestTarget {
        path: "/browser".to_owned(),
        params: form_urlencoded::parse(
            payload.sessions[3]
                .pin_url
                .trim_start_matches("/browser?")
                .as_bytes(),
        )
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect(),
    };
    let (payload, back_href) = registry.apply_target(&pin_fourth).await.unwrap();
    assert_eq!(payload.id, "s2");
    assert!(payload.sessions[0].pinned);
    assert!(!payload.sessions[1].pinned);
    assert!(!payload.sessions[2].pinned);
    assert!(payload.sessions[3].pinned);
    let html = render_browser_session_page(&payload, &back_href);
    assert!(html.contains(">Close unpinned</a>"));
    assert!(html.contains("action=close-unpinned-tabs"));
    let state_export = RequestTarget {
        path: "/api/browser-session".to_owned(),
        params: vec![
            ("id".to_owned(), payload.id.clone()),
            ("format".to_owned(), "session-state".to_owned()),
        ],
    };
    let response = browser_session_api_response(&state_export, &payload);
    let exported: serde_json::Value = serde_json::from_str(&response.body).unwrap();
    assert!(
        exported["action_urls"]["close_unpinned_tabs"]
            .as_str()
            .unwrap()
            .contains("action=close-unpinned-tabs")
    );

    let close_unpinned = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![
            ("id".to_owned(), payload.id),
            ("action".to_owned(), "close-unpinned-tabs".to_owned()),
        ],
    };
    let (payload, _) = registry.apply_target(&close_unpinned).await.unwrap();
    assert_eq!(payload.id, "s2");
    assert_eq!(payload.title, "Two");
    assert_eq!(payload.sessions.len(), 3);
    assert!(
        payload
            .sessions
            .iter()
            .any(|session| { session.id == "s1" && session.pinned && !session.current })
    );
    assert!(
        payload
            .sessions
            .iter()
            .any(|session| { session.id == "s2" && !session.pinned && session.current })
    );
    assert!(
        payload
            .sessions
            .iter()
            .any(|session| { session.id == "s4" && session.pinned && !session.current })
    );
    assert!(payload.sessions.iter().all(|session| session.id != "s3"));
    assert_eq!(payload.closed_sessions.len(), 1);
    assert_eq!(payload.closed_sessions[0].title, "Three");
}

#[tokio::test]
async fn browser_session_registry_closes_tab_search_matches_except_active_and_pinned() {
    let dir = tempfile::tempdir().unwrap();
    let active = dir.path().join("active.html");
    let first_match = dir.path().join("first-match.html");
    let second_match = dir.path().join("second-match.html");
    let pinned_match = dir.path().join("pinned-match.html");
    std::fs::write(
        &active,
        r#"<!doctype html><title>Needle Active</title><p>active needle tab stays open</p>"#,
    )
    .unwrap();
    std::fs::write(
        &first_match,
        r#"<!doctype html><title>Needle First</title><p>first needle tab closes</p>"#,
    )
    .unwrap();
    std::fs::write(
        &second_match,
        r#"<!doctype html><title>Needle Second</title><p>second needle tab closes</p>"#,
    )
    .unwrap();
    std::fs::write(
        &pinned_match,
        r#"<!doctype html><title>Needle Pinned</title><p>pinned needle tab stays open</p>"#,
    )
    .unwrap();

    let registry = BrowserSessionRegistry::default();
    for page in [&active, &first_match, &second_match, &pinned_match] {
        let create = RequestTarget {
            path: "/browser".to_owned(),
            params: vec![("url".to_owned(), page.display().to_string())],
        };
        registry.create_target(&create).await.unwrap();
    }

    let switch_active = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![
            ("id".to_owned(), "s1".to_owned()),
            ("action".to_owned(), "current".to_owned()),
        ],
    };
    let (payload, _) = registry.apply_target(&switch_active).await.unwrap();
    let pin_pinned_match = RequestTarget {
        path: "/browser".to_owned(),
        params: form_urlencoded::parse(
            payload.sessions[3]
                .pin_url
                .trim_start_matches("/browser?")
                .as_bytes(),
        )
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect(),
    };
    let (payload, _) = registry.apply_target(&pin_pinned_match).await.unwrap();

    let search_tabs = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![
            ("id".to_owned(), payload.id.clone()),
            ("action".to_owned(), "search-tabs".to_owned()),
            ("q".to_owned(), "needle".to_owned()),
        ],
    };
    let (payload, back_href) = registry.apply_target(&search_tabs).await.unwrap();
    assert_eq!(payload.id, "s1");
    assert_eq!(payload.tab_search_query, "needle");
    assert!(
        payload
            .tab_search_results
            .iter()
            .any(|result| result.id == "s1" && result.current)
    );
    assert!(
        payload
            .tab_search_results
            .iter()
            .any(|result| result.id == "s2" && !result.current && !result.pinned)
    );
    assert!(
        payload
            .tab_search_results
            .iter()
            .any(|result| result.id == "s3" && !result.current && !result.pinned)
    );
    assert!(
        payload
            .tab_search_results
            .iter()
            .any(|result| result.id == "s4" && result.pinned)
    );
    let html = render_browser_session_page(&payload, &back_href);
    assert!(html.contains(">Close matches</a>"));
    assert!(html.contains("action=close-tab-search-results"));

    let state_export = RequestTarget {
        path: "/api/browser-session".to_owned(),
        params: vec![
            ("id".to_owned(), payload.id.clone()),
            ("format".to_owned(), "session-state".to_owned()),
        ],
    };
    let response = browser_session_api_response(&state_export, &payload);
    let exported: serde_json::Value = serde_json::from_str(&response.body).unwrap();
    assert!(
        exported["action_urls"]["close_tab_search_results"]
            .as_str()
            .unwrap()
            .contains("action=close-tab-search-results")
    );

    let close_matches = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![
            ("id".to_owned(), payload.id.clone()),
            ("action".to_owned(), "close-tab-search-results".to_owned()),
        ],
    };
    let (payload, _) = registry.apply_target(&close_matches).await.unwrap();
    assert_eq!(payload.id, "s1");
    assert_eq!(payload.title, "Needle Active");
    assert_eq!(payload.sessions.len(), 2);
    assert!(
        payload
            .sessions
            .iter()
            .any(|session| { session.id == "s1" && session.current && !session.pinned })
    );
    assert!(
        payload
            .sessions
            .iter()
            .any(|session| { session.id == "s4" && !session.current && session.pinned })
    );
    assert!(payload.sessions.iter().all(|session| session.id != "s2"));
    assert!(payload.sessions.iter().all(|session| session.id != "s3"));
    assert_eq!(payload.closed_sessions.len(), 2);
    assert!(
        payload
            .closed_sessions
            .iter()
            .any(|session| session.title == "Needle First")
    );
    assert!(
        payload
            .closed_sessions
            .iter()
            .any(|session| session.title == "Needle Second")
    );
    assert_eq!(payload.tab_search_query, "needle");
    assert!(
        payload
            .tab_search_results
            .iter()
            .any(|result| result.id == "s1" && result.current)
    );
    assert!(
        payload
            .tab_search_results
            .iter()
            .any(|result| result.id == "s4" && result.pinned)
    );
    assert!(
        payload
            .tab_search_results
            .iter()
            .all(|result| result.id != "s2" && result.id != "s3")
    );

    let response = browser_session_api_response(&state_export, &payload);
    let exported: serde_json::Value = serde_json::from_str(&response.body).unwrap();
    assert!(exported["action_urls"]["close_tab_search_results"].is_null());
}

#[tokio::test]
async fn browser_session_registry_closes_tab_search_nonmatches_except_active_and_pinned() {
    let dir = tempfile::tempdir().unwrap();
    let active = dir.path().join("active.html");
    let first_match = dir.path().join("first-match.html");
    let close_target = dir.path().join("close-target.html");
    let pinned_nonmatch = dir.path().join("pinned-nonmatch.html");
    let second_match = dir.path().join("second-match.html");
    std::fs::write(
        &active,
        r#"<!doctype html><title>Control</title><p>active control tab stays open</p>"#,
    )
    .unwrap();
    std::fs::write(
        &first_match,
        r#"<!doctype html><title>Needle First</title><p>first needle tab stays open</p>"#,
    )
    .unwrap();
    std::fs::write(
        &close_target,
        r#"<!doctype html><title>Close Target</title><p>ordinary tab closes</p>"#,
    )
    .unwrap();
    std::fs::write(
        &pinned_nonmatch,
        r#"<!doctype html><title>Pinned Ordinary</title><p>pinned ordinary tab stays open</p>"#,
    )
    .unwrap();
    std::fs::write(
        &second_match,
        r#"<!doctype html><title>Needle Second</title><p>second needle tab stays open</p>"#,
    )
    .unwrap();

    let registry = BrowserSessionRegistry::default();
    for page in [
        &active,
        &first_match,
        &close_target,
        &pinned_nonmatch,
        &second_match,
    ] {
        let create = RequestTarget {
            path: "/browser".to_owned(),
            params: vec![("url".to_owned(), page.display().to_string())],
        };
        registry.create_target(&create).await.unwrap();
    }

    let switch_active = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![
            ("id".to_owned(), "s1".to_owned()),
            ("action".to_owned(), "current".to_owned()),
        ],
    };
    let (payload, _) = registry.apply_target(&switch_active).await.unwrap();
    let pin_nonmatch = RequestTarget {
        path: "/browser".to_owned(),
        params: form_urlencoded::parse(
            payload.sessions[3]
                .pin_url
                .trim_start_matches("/browser?")
                .as_bytes(),
        )
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect(),
    };
    let (payload, _) = registry.apply_target(&pin_nonmatch).await.unwrap();

    let search_tabs = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![
            ("id".to_owned(), payload.id.clone()),
            ("action".to_owned(), "search-tabs".to_owned()),
            ("q".to_owned(), "needle".to_owned()),
        ],
    };
    let (payload, back_href) = registry.apply_target(&search_tabs).await.unwrap();
    assert_eq!(payload.id, "s1");
    assert!(
        payload
            .tab_search_results
            .iter()
            .any(|result| result.id == "s2")
    );
    assert!(
        payload
            .tab_search_results
            .iter()
            .any(|result| result.id == "s5")
    );
    assert!(
        payload
            .tab_search_results
            .iter()
            .all(|result| result.id != "s1" && result.id != "s3" && result.id != "s4")
    );
    let html = render_browser_session_page(&payload, &back_href);
    assert!(html.contains(">Close nonmatches</a>"));
    assert!(html.contains("action=close-tab-search-nonmatches"));

    let state_export = RequestTarget {
        path: "/api/browser-session".to_owned(),
        params: vec![
            ("id".to_owned(), payload.id.clone()),
            ("format".to_owned(), "session-state".to_owned()),
        ],
    };
    let response = browser_session_api_response(&state_export, &payload);
    let exported: serde_json::Value = serde_json::from_str(&response.body).unwrap();
    assert!(
        exported["action_urls"]["close_tab_search_nonmatches"]
            .as_str()
            .unwrap()
            .contains("action=close-tab-search-nonmatches")
    );

    let close_nonmatches = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![
            ("id".to_owned(), payload.id.clone()),
            (
                "action".to_owned(),
                "close-tab-search-nonmatches".to_owned(),
            ),
        ],
    };
    let (payload, back_href) = registry.apply_target(&close_nonmatches).await.unwrap();
    assert_eq!(payload.id, "s1");
    assert_eq!(payload.sessions.len(), 4);
    assert!(
        payload
            .sessions
            .iter()
            .any(|session| { session.id == "s1" && session.current && !session.pinned })
    );
    assert!(
        payload
            .sessions
            .iter()
            .any(|session| { session.id == "s2" && !session.current && !session.pinned })
    );
    assert!(
        payload
            .sessions
            .iter()
            .any(|session| { session.id == "s4" && !session.current && session.pinned })
    );
    assert!(
        payload
            .sessions
            .iter()
            .any(|session| { session.id == "s5" && !session.current && !session.pinned })
    );
    assert!(payload.sessions.iter().all(|session| session.id != "s3"));
    assert_eq!(payload.closed_sessions.len(), 1);
    assert_eq!(payload.closed_sessions[0].title, "Close Target");
    assert!(
        payload
            .tab_search_results
            .iter()
            .all(|result| result.id == "s2" || result.id == "s5")
    );
    let html = render_browser_session_page(&payload, &back_href);
    assert!(!html.contains(">Close nonmatches</a>"));
    let response = browser_session_api_response(&state_export, &payload);
    let exported: serde_json::Value = serde_json::from_str(&response.body).unwrap();
    assert!(exported["action_urls"]["close_tab_search_nonmatches"].is_null());
}

#[tokio::test]
async fn browser_session_registry_pins_and_unpins_tab_search_matches() {
    let dir = tempfile::tempdir().unwrap();
    let first = dir.path().join("first.html");
    let second = dir.path().join("second.html");
    let active = dir.path().join("active.html");
    std::fs::write(
        &first,
        r#"<!doctype html><title>Group First</title><p>matching research group tab</p>"#,
    )
    .unwrap();
    std::fs::write(
        &second,
        r#"<!doctype html><title>Group Second</title><p>another matching group tab</p>"#,
    )
    .unwrap();
    std::fs::write(
        &active,
        r#"<!doctype html><title>Control</title><p>active control tab</p>"#,
    )
    .unwrap();

    let registry = BrowserSessionRegistry::default();
    for page in [&first, &second, &active] {
        let create = RequestTarget {
            path: "/browser".to_owned(),
            params: vec![("url".to_owned(), page.display().to_string())],
        };
        registry.create_target(&create).await.unwrap();
    }

    let search_tabs = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![
            ("id".to_owned(), "s3".to_owned()),
            ("action".to_owned(), "search-tabs".to_owned()),
            ("q".to_owned(), "group".to_owned()),
        ],
    };
    let (payload, back_href) = registry.apply_target(&search_tabs).await.unwrap();
    assert_eq!(payload.id, "s3");
    assert_eq!(payload.sessions.len(), 3);
    assert_eq!(payload.tab_search_query, "group");
    assert!(
        payload
            .tab_search_results
            .iter()
            .any(|result| result.id == "s1" && !result.pinned)
    );
    assert!(
        payload
            .tab_search_results
            .iter()
            .any(|result| result.id == "s2" && !result.pinned)
    );
    assert!(
        payload
            .tab_search_results
            .iter()
            .all(|result| result.id != "s3")
    );
    let html = render_browser_session_page(&payload, &back_href);
    assert!(html.contains(">Pin matches</a>"));
    assert!(html.contains("action=pin-tab-search-results"));
    assert!(!html.contains(">Unpin matches</a>"));

    let state_export = RequestTarget {
        path: "/api/browser-session".to_owned(),
        params: vec![
            ("id".to_owned(), payload.id.clone()),
            ("format".to_owned(), "session-state".to_owned()),
        ],
    };
    let response = browser_session_api_response(&state_export, &payload);
    let exported: serde_json::Value = serde_json::from_str(&response.body).unwrap();
    assert!(
        exported["action_urls"]["pin_tab_search_results"]
            .as_str()
            .unwrap()
            .contains("action=pin-tab-search-results")
    );
    assert!(exported["action_urls"]["unpin_tab_search_results"].is_null());

    let pin_matches = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![
            ("id".to_owned(), payload.id.clone()),
            ("action".to_owned(), "pin-tab-search-results".to_owned()),
        ],
    };
    let (payload, back_href) = registry.apply_target(&pin_matches).await.unwrap();
    assert_eq!(payload.id, "s3");
    assert!(
        payload
            .sessions
            .iter()
            .any(|session| { session.id == "s1" && session.pinned })
    );
    assert!(
        payload
            .sessions
            .iter()
            .any(|session| { session.id == "s2" && session.pinned })
    );
    assert!(
        payload
            .sessions
            .iter()
            .any(|session| { session.id == "s3" && session.current && !session.pinned })
    );
    assert!(
        payload
            .tab_search_results
            .iter()
            .all(|result| result.pinned)
    );
    let html = render_browser_session_page(&payload, &back_href);
    assert!(!html.contains(">Pin matches</a>"));
    assert!(html.contains(">Unpin matches</a>"));
    assert!(html.contains("action=unpin-tab-search-results"));
    let response = browser_session_api_response(&state_export, &payload);
    let exported: serde_json::Value = serde_json::from_str(&response.body).unwrap();
    assert!(exported["action_urls"]["pin_tab_search_results"].is_null());
    assert!(
        exported["action_urls"]["unpin_tab_search_results"]
            .as_str()
            .unwrap()
            .contains("action=unpin-tab-search-results")
    );

    let unpin_matches = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![
            ("id".to_owned(), payload.id.clone()),
            ("action".to_owned(), "unpin-tab-search-results".to_owned()),
        ],
    };
    let (payload, _) = registry.apply_target(&unpin_matches).await.unwrap();
    assert_eq!(payload.id, "s3");
    assert!(payload.sessions.iter().all(|session| !session.pinned));
    assert!(
        payload
            .tab_search_results
            .iter()
            .all(|result| !result.pinned)
    );
}

#[tokio::test]
async fn browser_session_registry_labels_and_clears_tab_search_matches() {
    let dir = tempfile::tempdir().unwrap();
    let first = dir.path().join("first.html");
    let second = dir.path().join("second.html");
    let active = dir.path().join("active.html");
    std::fs::write(
        &first,
        r#"<!doctype html><title>Group First</title><p>matching research group tab</p>"#,
    )
    .unwrap();
    std::fs::write(
        &second,
        r#"<!doctype html><title>Group Second</title><p>another matching group tab</p>"#,
    )
    .unwrap();
    std::fs::write(
        &active,
        r#"<!doctype html><title>Control</title><p>active control tab</p>"#,
    )
    .unwrap();

    let registry = BrowserSessionRegistry::default();
    for page in [&first, &second, &active] {
        let create = RequestTarget {
            path: "/browser".to_owned(),
            params: vec![("url".to_owned(), page.display().to_string())],
        };
        registry.create_target(&create).await.unwrap();
    }

    let search_tabs = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![
            ("id".to_owned(), "s3".to_owned()),
            ("action".to_owned(), "search-tabs".to_owned()),
            ("q".to_owned(), "group".to_owned()),
        ],
    };
    let (payload, back_href) = registry.apply_target(&search_tabs).await.unwrap();
    assert_eq!(payload.id, "s3");
    assert_eq!(payload.tab_search_query, "group");
    assert_eq!(payload.sessions.len(), 3);
    assert!(
        payload
            .tab_search_results
            .iter()
            .any(|result| result.id == "s1" && result.label.is_none())
    );
    assert!(
        payload
            .tab_search_results
            .iter()
            .any(|result| result.id == "s2" && result.label.is_none())
    );
    assert!(
        payload
            .tab_search_results
            .iter()
            .all(|result| result.id != "s3")
    );
    let html = render_browser_session_page(&payload, &back_href);
    assert!(html.contains("Label matches"));
    assert!(html.contains(r#"name="action" value="label-tab-search-results""#));
    assert!(!html.contains(">Clear labels</a>"));

    let state_export = RequestTarget {
        path: "/api/browser-session".to_owned(),
        params: vec![
            ("id".to_owned(), payload.id.clone()),
            ("format".to_owned(), "session-state".to_owned()),
        ],
    };
    let response = browser_session_api_response(&state_export, &payload);
    let exported: serde_json::Value = serde_json::from_str(&response.body).unwrap();
    let label_matches_href = exported["action_urls"]["label_tab_search_results"]
        .as_str()
        .unwrap();
    assert!(label_matches_href.contains("action=label-tab-search-results"));
    assert!(label_matches_href.contains("label=group"));
    assert!(exported["action_urls"]["clear_tab_search_labels"].is_null());

    let label_matches_default = RequestTarget {
        path: "/browser".to_owned(),
        params: form_urlencoded::parse(
            label_matches_href
                .trim_start_matches("/browser?")
                .as_bytes(),
        )
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect(),
    };
    let (payload, _) = registry.apply_target(&label_matches_default).await.unwrap();
    assert!(
        payload
            .tab_search_results
            .iter()
            .filter(|result| result.id == "s1" || result.id == "s2")
            .all(|result| result.label.as_deref() == Some("group"))
    );

    let label_matches = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![
            ("id".to_owned(), payload.id.clone()),
            ("action".to_owned(), "label-tab-search-results".to_owned()),
            ("label".to_owned(), "Research Group".to_owned()),
        ],
    };
    let (payload, back_href) = registry.apply_target(&label_matches).await.unwrap();
    assert_eq!(payload.id, "s3");
    assert!(payload.sessions.iter().any(|session| {
        session.id == "s1" && session.label.as_deref() == Some("Research Group")
    }));
    assert!(payload.sessions.iter().any(|session| {
        session.id == "s2" && session.label.as_deref() == Some("Research Group")
    }));
    assert!(
        payload
            .sessions
            .iter()
            .any(|session| { session.id == "s3" && session.current && session.label.is_none() })
    );
    assert!(
        payload
            .tab_search_results
            .iter()
            .filter(|result| result.id == "s1" || result.id == "s2")
            .all(|result| result.label.as_deref() == Some("Research Group"))
    );
    let html = render_browser_session_page(&payload, &back_href);
    assert!(html.contains(">Clear labels</a>"));
    assert!(html.contains("action=clear-tab-search-labels"));
    let response = browser_session_api_response(&state_export, &payload);
    let exported: serde_json::Value = serde_json::from_str(&response.body).unwrap();
    assert!(
        exported["action_urls"]["clear_tab_search_labels"]
            .as_str()
            .unwrap()
            .contains("action=clear-tab-search-labels")
    );

    let clear_labels = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![
            ("id".to_owned(), payload.id.clone()),
            ("action".to_owned(), "clear-tab-search-labels".to_owned()),
        ],
    };
    let (payload, back_href) = registry.apply_target(&clear_labels).await.unwrap();
    assert_eq!(payload.id, "s3");
    assert!(
        payload
            .sessions
            .iter()
            .all(|session| session.label.is_none())
    );
    assert!(
        payload
            .tab_search_results
            .iter()
            .all(|result| result.label.is_none())
    );
    let html = render_browser_session_page(&payload, &back_href);
    assert!(html.contains("Label matches"));
    assert!(!html.contains(">Clear labels</a>"));
    let response = browser_session_api_response(&state_export, &payload);
    let exported: serde_json::Value = serde_json::from_str(&response.body).unwrap();
    let label_matches_href = exported["action_urls"]["label_tab_search_results"]
        .as_str()
        .unwrap();
    assert!(label_matches_href.contains("action=label-tab-search-results"));
    assert!(label_matches_href.contains("label=group"));
    assert!(exported["action_urls"]["clear_tab_search_labels"].is_null());
}

#[tokio::test]
async fn browser_session_registry_reloads_tab_search_matches_without_switching_active_tab() {
    let dir = tempfile::tempdir().unwrap();
    let first = dir.path().join("first.html");
    let second = dir.path().join("second.html");
    let active = dir.path().join("active.html");
    std::fs::write(
        &first,
        r#"<!doctype html><title>Group First</title><p>old group first tab</p>"#,
    )
    .unwrap();
    std::fs::write(
        &second,
        r#"<!doctype html><title>Group Second</title><p>old group second tab</p>"#,
    )
    .unwrap();
    std::fs::write(
        &active,
        r#"<!doctype html><title>Control</title><p>active control tab</p>"#,
    )
    .unwrap();

    let registry = BrowserSessionRegistry::default();
    for page in [&first, &second, &active] {
        let create = RequestTarget {
            path: "/browser".to_owned(),
            params: vec![("url".to_owned(), page.display().to_string())],
        };
        registry.create_target(&create).await.unwrap();
    }

    let search_tabs = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![
            ("id".to_owned(), "s3".to_owned()),
            ("action".to_owned(), "search-tabs".to_owned()),
            ("q".to_owned(), "group".to_owned()),
        ],
    };
    let (payload, back_href) = registry.apply_target(&search_tabs).await.unwrap();
    assert_eq!(payload.id, "s3");
    assert_eq!(payload.title, "Control");
    assert!(payload.viewport.contains("active control tab"));
    assert!(
        payload
            .tab_search_results
            .iter()
            .any(|result| result.id == "s1" && result.text.contains("Group First"))
    );
    assert!(
        payload
            .tab_search_results
            .iter()
            .any(|result| result.id == "s2" && result.text.contains("Group Second"))
    );
    let html = render_browser_session_page(&payload, &back_href);
    assert!(html.contains(">Reload matches</a>"));
    assert!(html.contains("action=reload-tab-search-results"));

    let state_export = RequestTarget {
        path: "/api/browser-session".to_owned(),
        params: vec![
            ("id".to_owned(), payload.id.clone()),
            ("format".to_owned(), "session-state".to_owned()),
        ],
    };
    let response = browser_session_api_response(&state_export, &payload);
    let exported: serde_json::Value = serde_json::from_str(&response.body).unwrap();
    assert!(
        exported["action_urls"]["reload_tab_search_results"]
            .as_str()
            .unwrap()
            .contains("action=reload-tab-search-results")
    );

    std::fs::write(
        &first,
        r#"<!doctype html><title>Fresh Group First</title><p>fresh group first tab</p>"#,
    )
    .unwrap();
    std::fs::write(
        &second,
        r#"<!doctype html><title>Fresh Group Second</title><p>fresh group second tab</p>"#,
    )
    .unwrap();

    let reload_matches = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![
            ("id".to_owned(), payload.id.clone()),
            ("action".to_owned(), "reload-tab-search-results".to_owned()),
        ],
    };
    let (payload, _) = registry.apply_target(&reload_matches).await.unwrap();
    assert_eq!(payload.id, "s3");
    assert_eq!(payload.title, "Control");
    assert!(payload.viewport.contains("active control tab"));
    assert!(
        payload
            .sessions
            .iter()
            .any(|session| { session.id == "s1" && session.title == "Fresh Group First" })
    );
    assert!(
        payload
            .sessions
            .iter()
            .any(|session| { session.id == "s2" && session.title == "Fresh Group Second" })
    );
    assert!(
        payload
            .sessions
            .iter()
            .any(|session| { session.id == "s3" && session.current && session.title == "Control" })
    );
    assert!(
        payload
            .tab_search_results
            .iter()
            .any(|result| result.id == "s1" && result.text.contains("Fresh Group First"))
    );
    assert!(
        payload
            .tab_search_results
            .iter()
            .any(|result| result.id == "s2" && result.text.contains("Fresh Group Second"))
    );
    assert!(
        payload
            .tab_search_results
            .iter()
            .all(|result| !result.text.contains("old group"))
    );
}

#[tokio::test]
async fn browser_session_registry_duplicates_tab_search_matches_without_switching_active_tab() {
    let dir = tempfile::tempdir().unwrap();
    let first = dir.path().join("first.html");
    let second = dir.path().join("second.html");
    let active = dir.path().join("active.html");
    std::fs::write(
        &first,
        r#"<!doctype html><title>Group First</title><p>matching research group tab</p>"#,
    )
    .unwrap();
    std::fs::write(
        &second,
        r#"<!doctype html><title>Group Second</title><p>another matching group tab</p>"#,
    )
    .unwrap();
    std::fs::write(
        &active,
        r#"<!doctype html><title>Control</title><p>active control tab</p>"#,
    )
    .unwrap();

    let registry = BrowserSessionRegistry::default();
    for page in [&first, &second, &active] {
        let create = RequestTarget {
            path: "/browser".to_owned(),
            params: vec![("url".to_owned(), page.display().to_string())],
        };
        registry.create_target(&create).await.unwrap();
    }

    let search_tabs = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![
            ("id".to_owned(), "s3".to_owned()),
            ("action".to_owned(), "search-tabs".to_owned()),
            ("q".to_owned(), "group".to_owned()),
        ],
    };
    let (payload, back_href) = registry.apply_target(&search_tabs).await.unwrap();
    assert_eq!(payload.id, "s3");
    assert_eq!(payload.title, "Control");
    assert_eq!(payload.sessions.len(), 3);
    assert!(
        payload
            .tab_search_results
            .iter()
            .any(|result| result.id == "s1")
    );
    assert!(
        payload
            .tab_search_results
            .iter()
            .any(|result| result.id == "s2")
    );
    let html = render_browser_session_page(&payload, &back_href);
    assert!(html.contains(">Duplicate matches</a>"));
    assert!(html.contains("action=duplicate-tab-search-results"));

    let state_export = RequestTarget {
        path: "/api/browser-session".to_owned(),
        params: vec![
            ("id".to_owned(), payload.id.clone()),
            ("format".to_owned(), "session-state".to_owned()),
        ],
    };
    let response = browser_session_api_response(&state_export, &payload);
    let exported: serde_json::Value = serde_json::from_str(&response.body).unwrap();
    assert!(
        exported["action_urls"]["duplicate_tab_search_results"]
            .as_str()
            .unwrap()
            .contains("action=duplicate-tab-search-results")
    );

    let label_matches = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![
            ("id".to_owned(), payload.id.clone()),
            ("action".to_owned(), "label-tab-search-results".to_owned()),
            ("label".to_owned(), "Research Group".to_owned()),
        ],
    };
    let (payload, _) = registry.apply_target(&label_matches).await.unwrap();
    let pin_matches = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![
            ("id".to_owned(), payload.id.clone()),
            ("action".to_owned(), "pin-tab-search-results".to_owned()),
        ],
    };
    let (payload, _) = registry.apply_target(&pin_matches).await.unwrap();
    assert!(payload.sessions.iter().any(|session| {
        session.id == "s1" && session.pinned && session.label.as_deref() == Some("Research Group")
    }));
    assert!(payload.sessions.iter().any(|session| {
        session.id == "s2" && session.pinned && session.label.as_deref() == Some("Research Group")
    }));

    let duplicate_matches = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![
            ("id".to_owned(), payload.id.clone()),
            (
                "action".to_owned(),
                "duplicate-tab-search-results".to_owned(),
            ),
        ],
    };
    let (payload, _) = registry.apply_target(&duplicate_matches).await.unwrap();
    assert_eq!(payload.id, "s3");
    assert_eq!(payload.title, "Control");
    assert_eq!(payload.sessions.len(), 5);
    assert!(
        payload
            .sessions
            .iter()
            .any(|session| { session.id == "s3" && session.current && session.title == "Control" })
    );
    assert!(payload.sessions.iter().any(|session| {
        session.id == "s4"
            && session.title == "Group First"
            && !session.pinned
            && session.label.is_none()
    }));
    assert!(payload.sessions.iter().any(|session| {
        session.id == "s5"
            && session.title == "Group Second"
            && !session.pinned
            && session.label.is_none()
    }));
    assert!(
        payload
            .tab_search_results
            .iter()
            .any(|result| result.id == "s4")
    );
    assert!(
        payload
            .tab_search_results
            .iter()
            .any(|result| result.id == "s5")
    );
}

#[tokio::test]
async fn browser_session_registry_bookmarks_tab_search_matches_without_switching_active_tab() {
    let dir = tempfile::tempdir().unwrap();
    let first = dir.path().join("first.html");
    let second = dir.path().join("second.html");
    let active = dir.path().join("active.html");
    std::fs::write(
        &first,
        r#"<!doctype html><title>Group First</title><p>matching research group tab</p>"#,
    )
    .unwrap();
    std::fs::write(
        &second,
        r#"<!doctype html><title>Group Second</title><p>another matching group tab</p>"#,
    )
    .unwrap();
    std::fs::write(
        &active,
        r#"<!doctype html><title>Control</title><p>active control tab</p>"#,
    )
    .unwrap();

    let registry = BrowserSessionRegistry::default();
    for page in [&first, &second, &active] {
        let create = RequestTarget {
            path: "/browser".to_owned(),
            params: vec![("url".to_owned(), page.display().to_string())],
        };
        registry.create_target(&create).await.unwrap();
    }

    let search_tabs = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![
            ("id".to_owned(), "s3".to_owned()),
            ("action".to_owned(), "search-tabs".to_owned()),
            ("q".to_owned(), "group".to_owned()),
        ],
    };
    let (payload, back_href) = registry.apply_target(&search_tabs).await.unwrap();
    assert_eq!(payload.id, "s3");
    assert_eq!(payload.title, "Control");
    assert_eq!(payload.sessions.len(), 3);
    assert!(payload.bookmarks.is_empty());
    assert!(
        payload
            .tab_search_results
            .iter()
            .any(|result| result.id == "s1")
    );
    assert!(
        payload
            .tab_search_results
            .iter()
            .any(|result| result.id == "s2")
    );
    let html = render_browser_session_page(&payload, &back_href);
    assert!(html.contains(">Bookmark matches</a>"));
    assert!(html.contains("action=bookmark-tab-search-results"));

    let state_export = RequestTarget {
        path: "/api/browser-session".to_owned(),
        params: vec![
            ("id".to_owned(), payload.id.clone()),
            ("format".to_owned(), "session-state".to_owned()),
        ],
    };
    let response = browser_session_api_response(&state_export, &payload);
    let exported: serde_json::Value = serde_json::from_str(&response.body).unwrap();
    assert!(
        exported["action_urls"]["bookmark_tab_search_results"]
            .as_str()
            .unwrap()
            .contains("action=bookmark-tab-search-results")
    );

    let bookmark_matches = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![
            ("id".to_owned(), payload.id.clone()),
            (
                "action".to_owned(),
                "bookmark-tab-search-results".to_owned(),
            ),
        ],
    };
    let (payload, back_href) = registry.apply_target(&bookmark_matches).await.unwrap();
    assert_eq!(payload.id, "s3");
    assert_eq!(payload.title, "Control");
    assert!(!payload.current_bookmarked);
    assert_eq!(payload.bookmarks.len(), 2);
    assert!(
        payload
            .bookmarks
            .iter()
            .any(|bookmark| bookmark.title == "Group First"
                && bookmark.source.ends_with("first.html"))
    );
    assert!(payload.bookmarks.iter().any(
        |bookmark| bookmark.title == "Group Second" && bookmark.source.ends_with("second.html")
    ));
    let html = render_browser_session_page(&payload, &back_href);
    assert!(!html.contains(">Bookmark matches</a>"));
    let response = browser_session_api_response(&state_export, &payload);
    let exported: serde_json::Value = serde_json::from_str(&response.body).unwrap();
    assert!(exported["action_urls"]["bookmark_tab_search_results"].is_null());
    assert!(
        exported["action_urls"]["open_bookmarks_background"]
            .as_str()
            .unwrap()
            .contains("action=open-bookmarks-background-sessions")
    );

    let (payload, _) = registry.apply_target(&bookmark_matches).await.unwrap();
    assert_eq!(payload.bookmarks.len(), 2);
}

#[tokio::test]
async fn browser_session_registry_removes_bookmarks_for_tab_search_matches_only() {
    let dir = tempfile::tempdir().unwrap();
    let first = dir.path().join("first.html");
    let second = dir.path().join("second.html");
    let active = dir.path().join("active.html");
    std::fs::write(
        &first,
        r#"<!doctype html><title>Group First</title><p>matching research group tab</p>"#,
    )
    .unwrap();
    std::fs::write(
        &second,
        r#"<!doctype html><title>Group Second</title><p>another matching group tab</p>"#,
    )
    .unwrap();
    std::fs::write(
        &active,
        r#"<!doctype html><title>Control</title><p>active control tab</p>"#,
    )
    .unwrap();

    let registry = BrowserSessionRegistry::default();
    for page in [&first, &second, &active] {
        let create = RequestTarget {
            path: "/browser".to_owned(),
            params: vec![("url".to_owned(), page.display().to_string())],
        };
        registry.create_target(&create).await.unwrap();
    }

    let search_tabs = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![
            ("id".to_owned(), "s3".to_owned()),
            ("action".to_owned(), "search-tabs".to_owned()),
            ("q".to_owned(), "group".to_owned()),
        ],
    };
    let (payload, _) = registry.apply_target(&search_tabs).await.unwrap();
    let bookmark_matches = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![
            ("id".to_owned(), payload.id.clone()),
            (
                "action".to_owned(),
                "bookmark-tab-search-results".to_owned(),
            ),
        ],
    };
    let (payload, _) = registry.apply_target(&bookmark_matches).await.unwrap();
    assert_eq!(payload.bookmarks.len(), 2);

    let add_active_bookmark = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![
            ("id".to_owned(), payload.id.clone()),
            ("action".to_owned(), "add-bookmark".to_owned()),
        ],
    };
    let (payload, back_href) = registry.apply_target(&add_active_bookmark).await.unwrap();
    assert_eq!(payload.id, "s3");
    assert!(payload.current_bookmarked);
    assert_eq!(payload.bookmarks.len(), 3);
    let html = render_browser_session_page(&payload, &back_href);
    assert!(html.contains(">Remove bookmarks</a>"));
    assert!(html.contains("action=remove-tab-search-bookmarks"));

    let state_export = RequestTarget {
        path: "/api/browser-session".to_owned(),
        params: vec![
            ("id".to_owned(), payload.id.clone()),
            ("format".to_owned(), "session-state".to_owned()),
        ],
    };
    let response = browser_session_api_response(&state_export, &payload);
    let exported: serde_json::Value = serde_json::from_str(&response.body).unwrap();
    assert!(
        exported["action_urls"]["remove_tab_search_bookmarks"]
            .as_str()
            .unwrap()
            .contains("action=remove-tab-search-bookmarks")
    );

    let remove_match_bookmarks = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![
            ("id".to_owned(), payload.id.clone()),
            (
                "action".to_owned(),
                "remove-tab-search-bookmarks".to_owned(),
            ),
        ],
    };
    let (payload, back_href) = registry
        .apply_target(&remove_match_bookmarks)
        .await
        .unwrap();
    assert_eq!(payload.id, "s3");
    assert!(payload.current_bookmarked);
    assert_eq!(payload.bookmarks.len(), 1);
    assert_eq!(payload.bookmarks[0].title, "Control");
    assert!(payload.bookmarks[0].source.ends_with("active.html"));
    assert!(
        payload
            .bookmarks
            .iter()
            .all(|bookmark| !bookmark.source.ends_with("first.html")
                && !bookmark.source.ends_with("second.html"))
    );
    let html = render_browser_session_page(&payload, &back_href);
    assert!(!html.contains(">Remove bookmarks</a>"));
    assert!(html.contains(">Bookmark matches</a>"));
    let response = browser_session_api_response(&state_export, &payload);
    let exported: serde_json::Value = serde_json::from_str(&response.body).unwrap();
    assert!(exported["action_urls"]["remove_tab_search_bookmarks"].is_null());
    assert!(
        exported["action_urls"]["bookmark_tab_search_results"]
            .as_str()
            .unwrap()
            .contains("action=bookmark-tab-search-results")
    );
}

#[tokio::test]
async fn browser_session_registry_moves_tab_search_matches_to_front_and_back() {
    let dir = tempfile::tempdir().unwrap();
    let first = dir.path().join("first.html");
    let group_one = dir.path().join("group-one.html");
    let active = dir.path().join("active.html");
    let group_two = dir.path().join("group-two.html");
    let last = dir.path().join("last.html");
    std::fs::write(
        &first,
        r#"<!doctype html><title>First</title><p>first nonmatching tab</p>"#,
    )
    .unwrap();
    std::fs::write(
        &group_one,
        r#"<!doctype html><title>Group One</title><p>first matching group tab</p>"#,
    )
    .unwrap();
    std::fs::write(
        &active,
        r#"<!doctype html><title>Control</title><p>active control tab</p>"#,
    )
    .unwrap();
    std::fs::write(
        &group_two,
        r#"<!doctype html><title>Group Two</title><p>second matching group tab</p>"#,
    )
    .unwrap();
    std::fs::write(
        &last,
        r#"<!doctype html><title>Last</title><p>last nonmatching tab</p>"#,
    )
    .unwrap();

    let registry = BrowserSessionRegistry::default();
    for page in [&first, &group_one, &active, &group_two, &last] {
        let create = RequestTarget {
            path: "/browser".to_owned(),
            params: vec![("url".to_owned(), page.display().to_string())],
        };
        registry.create_target(&create).await.unwrap();
    }

    let search_tabs = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![
            ("id".to_owned(), "s3".to_owned()),
            ("action".to_owned(), "search-tabs".to_owned()),
            ("q".to_owned(), "group".to_owned()),
        ],
    };
    let (payload, back_href) = registry.apply_target(&search_tabs).await.unwrap();
    assert_eq!(payload.id, "s3");
    assert_eq!(
        payload
            .sessions
            .iter()
            .map(|session| session.id.as_str())
            .collect::<Vec<_>>(),
        vec!["s1", "s2", "s3", "s4", "s5"]
    );
    let html = render_browser_session_page(&payload, &back_href);
    assert!(html.contains(">Move matches front</a>"));
    assert!(html.contains(">Move matches end</a>"));
    assert!(html.contains("action=move-tab-search-results-front"));
    assert!(html.contains("action=move-tab-search-results-back"));

    let state_export = RequestTarget {
        path: "/api/browser-session".to_owned(),
        params: vec![
            ("id".to_owned(), payload.id.clone()),
            ("format".to_owned(), "session-state".to_owned()),
        ],
    };
    let response = browser_session_api_response(&state_export, &payload);
    let exported: serde_json::Value = serde_json::from_str(&response.body).unwrap();
    assert!(
        exported["action_urls"]["move_tab_search_results_front"]
            .as_str()
            .unwrap()
            .contains("action=move-tab-search-results-front")
    );
    assert!(
        exported["action_urls"]["move_tab_search_results_back"]
            .as_str()
            .unwrap()
            .contains("action=move-tab-search-results-back")
    );

    let move_front = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![
            ("id".to_owned(), payload.id.clone()),
            (
                "action".to_owned(),
                "move-tab-search-results-front".to_owned(),
            ),
        ],
    };
    let (payload, back_href) = registry.apply_target(&move_front).await.unwrap();
    assert_eq!(payload.id, "s3");
    assert_eq!(payload.title, "Control");
    assert!(
        payload
            .sessions
            .iter()
            .any(|session| { session.id == "s3" && session.current })
    );
    assert_eq!(
        payload
            .sessions
            .iter()
            .map(|session| session.id.as_str())
            .collect::<Vec<_>>(),
        vec!["s2", "s4", "s1", "s3", "s5"]
    );
    let html = render_browser_session_page(&payload, &back_href);
    assert!(!html.contains(">Move matches front</a>"));
    assert!(html.contains(">Move matches end</a>"));
    let response = browser_session_api_response(&state_export, &payload);
    let exported: serde_json::Value = serde_json::from_str(&response.body).unwrap();
    assert!(exported["action_urls"]["move_tab_search_results_front"].is_null());
    assert!(
        exported["action_urls"]["move_tab_search_results_back"]
            .as_str()
            .unwrap()
            .contains("action=move-tab-search-results-back")
    );

    let move_back = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![
            ("id".to_owned(), payload.id.clone()),
            (
                "action".to_owned(),
                "move-tab-search-results-back".to_owned(),
            ),
        ],
    };
    let (payload, back_href) = registry.apply_target(&move_back).await.unwrap();
    assert_eq!(payload.id, "s3");
    assert_eq!(
        payload
            .sessions
            .iter()
            .map(|session| session.id.as_str())
            .collect::<Vec<_>>(),
        vec!["s1", "s3", "s5", "s2", "s4"]
    );
    let html = render_browser_session_page(&payload, &back_href);
    assert!(html.contains(">Move matches front</a>"));
    assert!(!html.contains(">Move matches end</a>"));
    let response = browser_session_api_response(&state_export, &payload);
    let exported: serde_json::Value = serde_json::from_str(&response.body).unwrap();
    assert!(
        exported["action_urls"]["move_tab_search_results_front"]
            .as_str()
            .unwrap()
            .contains("action=move-tab-search-results-front")
    );
    assert!(exported["action_urls"]["move_tab_search_results_back"].is_null());
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
            .background_restore_url
            .contains("action=restore-closed-background-session")
    );
    assert!(
        payload.closed_sessions[0]
            .forget_url
            .contains("action=forget-closed")
    );

    let html = render_browser_session_page(&payload, &back_href);
    assert!(html.contains("<h2>Recently closed</h2>"));
    assert!(html.contains("Closed CSV"));
    assert!(html.contains("format=closed-sessions-csv"));
    assert!(html.contains(">Restore tab</a>"));
    assert!(html.contains(">Restore</a>"));
    assert!(html.contains(">Background</a>"));
    assert!(html.contains(">Forget</a>"));
    let closed_csv_export = RequestTarget {
        path: "/api/browser-session".to_owned(),
        params: vec![
            ("id".to_owned(), payload.id.clone()),
            ("format".to_owned(), "closed-sessions-csv".to_owned()),
        ],
    };
    let response = browser_session_api_response(&closed_csv_export, &payload);
    assert_eq!(response.status, 200);
    assert_eq!(response.content_type, "text/csv; charset=utf-8");
    assert!(response.body.starts_with(
        "id,title,source,persisted,closed_at_unix_secs,closed_at,restore_url,new_session_url,background_restore_url,forget_url,session_id,active_source,closed_count\n"
    ));
    assert_eq!(response.body.lines().count(), 2);
    assert!(response.body.contains(",First Closed,"));
    assert!(response.body.contains(",false,"));
    assert!(response.body.contains("action=restore-closed"));
    assert!(
        response
            .body
            .contains("action=restore-closed-background-session")
    );
    assert!(response.body.contains("action=forget-closed"));
    assert!(response.body.ends_with(",1\n"));
    let state_export = RequestTarget {
        path: "/api/browser-session".to_owned(),
        params: vec![
            ("id".to_owned(), payload.id.clone()),
            ("format".to_owned(), "session-state".to_owned()),
        ],
    };
    let response = browser_session_api_response(&state_export, &payload);
    assert_eq!(response.status, 200);
    assert_eq!(response.content_type, "application/json; charset=utf-8");
    let exported: serde_json::Value = serde_json::from_str(&response.body).unwrap();
    let closed_sessions = exported["closed_sessions"].as_array().unwrap();
    assert_eq!(closed_sessions.len(), 1);
    assert_eq!(closed_sessions[0]["id"], first_id);
    assert_eq!(closed_sessions[0]["title"], "First Closed");
    assert_eq!(closed_sessions[0]["persisted"], false);
    assert!(
        closed_sessions[0]["source"]
            .as_str()
            .unwrap()
            .ends_with("first.html")
    );
    assert!(
        closed_sessions[0]["restore_url"]
            .as_str()
            .unwrap()
            .contains("action=restore-closed")
    );
    assert!(
        closed_sessions[0]["background_restore_url"]
            .as_str()
            .unwrap()
            .contains("action=restore-closed-background-session")
    );
    assert!(
        closed_sessions[0]["forget_url"]
            .as_str()
            .unwrap()
            .contains("action=forget-closed")
    );

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
async fn browser_session_registry_restores_recently_closed_sessions_in_background() {
    let dir = tempfile::tempdir().unwrap();
    let first = dir.path().join("first.html");
    let second = dir.path().join("second.html");
    std::fs::write(
        &first,
        r#"<!doctype html><title>First Closed</title><p>background restore me</p>"#,
    )
    .unwrap();
    std::fs::write(
        &second,
        r#"<!doctype html><title>Second Active</title><p>keep this tab active</p>"#,
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
    let restore_background = RequestTarget {
        path: "/browser".to_owned(),
        params: form_urlencoded::parse(
            payload.closed_sessions[0]
                .background_restore_url
                .trim_start_matches("/browser?")
                .as_bytes(),
        )
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect(),
    };
    let (payload, _) = registry.apply_target(&restore_background).await.unwrap();
    assert_eq!(payload.id, second_id);
    assert_eq!(payload.title, "Second Active");
    assert_eq!(payload.sessions.len(), 2);
    assert!(payload.closed_sessions.is_empty());
    assert!(payload.sessions[0].current);
    assert_eq!(payload.sessions[1].page_title, "First Closed");
    assert!(!payload.sessions[1].current);
    assert!(payload.viewport.contains("keep this tab active"));

    let next_tab = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![
            ("id".to_owned(), second_id),
            ("action".to_owned(), "next-tab".to_owned()),
        ],
    };
    let (payload, _) = registry.apply_target(&next_tab).await.unwrap();
    assert_eq!(payload.title, "First Closed");
    assert!(payload.viewport.contains("background restore me"));
}

#[tokio::test]
async fn browser_session_registry_restores_all_recently_closed_sessions_in_background() {
    let dir = tempfile::tempdir().unwrap();
    let first = dir.path().join("first.html");
    let second = dir.path().join("second.html");
    let third = dir.path().join("third.html");
    std::fs::write(
        &first,
        r#"<!doctype html><title>Bulk Closed One</title><p>restore first in bulk</p>"#,
    )
    .unwrap();
    std::fs::write(
        &second,
        r#"<!doctype html><title>Bulk Closed Two</title><p>restore second in bulk</p>"#,
    )
    .unwrap();
    std::fs::write(
        &third,
        r#"<!doctype html><title>Bulk Active</title><p>keep bulk active</p>"#,
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
    let active_id = payload.id.clone();
    assert_eq!(payload.sessions.len(), 3);

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
    assert_eq!(payload.id, active_id);
    assert_eq!(payload.closed_sessions.len(), 1);

    let close_second_href = payload
        .sessions
        .iter()
        .find(|session| session.id == second_id)
        .unwrap()
        .close_url
        .clone();
    let close_second = RequestTarget {
        path: "/browser".to_owned(),
        params: form_urlencoded::parse(
            close_second_href.trim_start_matches("/browser?").as_bytes(),
        )
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect(),
    };
    let (payload, back_href) = registry.apply_target(&close_second).await.unwrap();
    assert_eq!(payload.id, active_id);
    assert_eq!(payload.title, "Bulk Active");
    assert_eq!(payload.sessions.len(), 1);
    assert_eq!(payload.closed_sessions.len(), 2);

    let html = render_browser_session_page(&payload, &back_href);
    assert!(html.contains(">Restore all bg</a>"));
    assert!(html.contains("action=restore-all-closed-background"));
    let state_export = RequestTarget {
        path: "/api/browser-session".to_owned(),
        params: vec![
            ("id".to_owned(), payload.id.clone()),
            ("format".to_owned(), "session-state".to_owned()),
        ],
    };
    let response = browser_session_api_response(&state_export, &payload);
    assert_eq!(response.status, 200);
    let exported: serde_json::Value = serde_json::from_str(&response.body).unwrap();
    let restore_all_href = exported["action_urls"]["restore_closed_background_sessions"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(restore_all_href.contains("action=restore-all-closed-background"));

    let restore_all = RequestTarget {
        path: "/browser".to_owned(),
        params: form_urlencoded::parse(restore_all_href.trim_start_matches("/browser?").as_bytes())
            .map(|(key, value)| (key.into_owned(), value.into_owned()))
            .collect(),
    };
    let (payload, back_href) = registry.apply_target(&restore_all).await.unwrap();
    assert_eq!(payload.id, active_id);
    assert_eq!(payload.title, "Bulk Active");
    assert_eq!(payload.sessions.len(), 3);
    assert!(payload.closed_sessions.is_empty());
    assert!(payload.sessions.iter().any(|session| session.current
        && session.id == active_id
        && session.title == "Bulk Active"));
    assert!(
        payload
            .sessions
            .iter()
            .any(|session| !session.current && session.page_title == "Bulk Closed One")
    );
    assert!(
        payload
            .sessions
            .iter()
            .any(|session| !session.current && session.page_title == "Bulk Closed Two")
    );
    let html = render_browser_session_page(&payload, &back_href);
    assert!(!html.contains("<h2>Recently closed</h2>"));

    let response = browser_session_api_response(&state_export, &payload);
    let exported: serde_json::Value = serde_json::from_str(&response.body).unwrap();
    assert!(exported["action_urls"]["restore_closed_background_sessions"].is_null());
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
            .background_restore_url
            .contains("action=open-profile-closed-background-session")
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
    assert!(html.contains(">Background</a>"));
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
async fn browser_session_registry_restores_persisted_closed_pages_in_background() {
    let dir = tempfile::tempdir().unwrap();
    let profile = dir.path().join("browser-profile.json");
    let closed_page = dir.path().join("closed.html");
    let active_page = dir.path().join("active.html");
    std::fs::write(
        &closed_page,
        r#"<!doctype html><title>Persist Closed</title><p>background persisted closed</p>"#,
    )
    .unwrap();
    std::fs::write(
        &active_page,
        r#"<!doctype html><title>Active Page</title><p>stay active persisted</p>"#,
    )
    .unwrap();
    save_browser_session_profile(
        &profile,
        &BrowserSessionProfileFile {
            version: 1,
            bookmarks: Vec::new(),
            tabs: Vec::new(),
            history: Vec::new(),
            closed: vec![BrowserSessionProfileClosedFile {
                title: "Persist Closed".to_owned(),
                source: closed_page.display().to_string(),
                closed_at_unix_secs: 1,
            }],
        },
    )
    .unwrap();

    let registry = BrowserSessionRegistry::with_profile_path(profile.clone());
    let create_active = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![("url".to_owned(), active_page.display().to_string())],
    };
    let (payload, _) = registry.create_target(&create_active).await.unwrap();
    let active_id = payload.id.clone();
    assert_eq!(payload.title, "Active Page");
    assert_eq!(payload.closed_sessions.len(), 1);
    assert!(
        payload.closed_sessions[0]
            .background_restore_url
            .contains("action=open-profile-closed-background-session")
    );

    let restore_background = RequestTarget {
        path: "/browser".to_owned(),
        params: form_urlencoded::parse(
            payload.closed_sessions[0]
                .background_restore_url
                .trim_start_matches("/browser?")
                .as_bytes(),
        )
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect(),
    };
    let (payload, _) = registry.apply_target(&restore_background).await.unwrap();
    assert_eq!(payload.id, active_id);
    assert_eq!(payload.title, "Active Page");
    assert_eq!(payload.sessions.len(), 2);
    assert!(payload.closed_sessions.is_empty());
    assert!(payload.sessions[0].current);
    assert_eq!(payload.sessions[1].page_title, "Persist Closed");
    assert!(!payload.sessions[1].current);
    assert!(
        load_browser_session_profile(&profile)
            .unwrap()
            .closed
            .is_empty()
    );
}

#[tokio::test]
async fn browser_session_registry_restores_all_persisted_closed_pages_in_background() {
    let dir = tempfile::tempdir().unwrap();
    let profile = dir.path().join("browser-profile.json");
    let first = dir.path().join("closed-one.html");
    let second = dir.path().join("closed-two.html");
    let active = dir.path().join("active.html");
    std::fs::write(
        &first,
        r#"<!doctype html><title>Persist Bulk One</title><p>bulk persisted one</p>"#,
    )
    .unwrap();
    std::fs::write(
        &second,
        r#"<!doctype html><title>Persist Bulk Two</title><p>bulk persisted two</p>"#,
    )
    .unwrap();
    std::fs::write(
        &active,
        r#"<!doctype html><title>Persist Bulk Active</title><p>keep persisted active</p>"#,
    )
    .unwrap();
    save_browser_session_profile(
        &profile,
        &BrowserSessionProfileFile {
            version: 1,
            bookmarks: Vec::new(),
            tabs: Vec::new(),
            history: Vec::new(),
            closed: vec![
                BrowserSessionProfileClosedFile {
                    title: "Persist Bulk One".to_owned(),
                    source: first.display().to_string(),
                    closed_at_unix_secs: 1,
                },
                BrowserSessionProfileClosedFile {
                    title: "Persist Bulk Two".to_owned(),
                    source: second.display().to_string(),
                    closed_at_unix_secs: 2,
                },
            ],
        },
    )
    .unwrap();

    let registry = BrowserSessionRegistry::with_profile_path(profile.clone());
    let create_active = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![("url".to_owned(), active.display().to_string())],
    };
    let (payload, back_href) = registry.create_target(&create_active).await.unwrap();
    let active_id = payload.id.clone();
    assert_eq!(payload.title, "Persist Bulk Active");
    assert_eq!(payload.closed_sessions.len(), 2);
    let html = render_browser_session_page(&payload, &back_href);
    assert!(html.contains(">Restore all bg</a>"));

    let state_export = RequestTarget {
        path: "/api/browser-session".to_owned(),
        params: vec![
            ("id".to_owned(), active_id.clone()),
            ("format".to_owned(), "session-state".to_owned()),
        ],
    };
    let response = browser_session_api_response(&state_export, &payload);
    let exported: serde_json::Value = serde_json::from_str(&response.body).unwrap();
    let restore_all_href = exported["action_urls"]["restore_closed_background_sessions"]
        .as_str()
        .unwrap()
        .to_owned();
    let restore_all = RequestTarget {
        path: "/browser".to_owned(),
        params: form_urlencoded::parse(restore_all_href.trim_start_matches("/browser?").as_bytes())
            .map(|(key, value)| (key.into_owned(), value.into_owned()))
            .collect(),
    };
    let (payload, _) = registry.apply_target(&restore_all).await.unwrap();
    assert_eq!(payload.id, active_id);
    assert_eq!(payload.title, "Persist Bulk Active");
    assert_eq!(payload.sessions.len(), 3);
    assert!(payload.closed_sessions.is_empty());
    assert!(
        payload
            .sessions
            .iter()
            .any(|session| session.current && session.title == "Persist Bulk Active")
    );
    assert!(
        payload
            .sessions
            .iter()
            .any(|session| !session.current && session.page_title == "Persist Bulk One")
    );
    assert!(
        payload
            .sessions
            .iter()
            .any(|session| !session.current && session.page_title == "Persist Bulk Two")
    );
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
        payload.bookmarks[0]
            .rename_url
            .contains("action=rename-bookmark")
    );
    assert!(
        payload
            .bookmarks_clear_url
            .as_deref()
            .is_some_and(|href| href.contains("action=clear-bookmarks"))
    );
    let html = render_browser_session_page(&payload, &back_href);
    assert!(html.contains("Bookmarked"));
    assert!(html.contains("Bookmarks CSV"));
    assert!(html.contains("format=bookmarks-csv"));
    assert!(html.contains("rename-bookmark"));
    assert!(html.contains("Bookmark title"));
    assert!(html.contains("remove-bookmark"));
    assert!(html.contains("clear-bookmarks"));
    let bookmarks_csv_export = RequestTarget {
        path: "/api/browser-session".to_owned(),
        params: vec![
            ("id".to_owned(), payload.id.clone()),
            ("format".to_owned(), "bookmarks-csv".to_owned()),
        ],
    };
    let response = browser_session_api_response(&bookmarks_csv_export, &payload);
    assert_eq!(response.status, 200);
    assert_eq!(response.content_type, "text/csv; charset=utf-8");
    assert!(response.body.starts_with(
        "id,title,source,current,action_url,new_session_url,background_session_url,rename_url,remove_url,session_id,active_source,bookmark_count\n"
    ));
    assert_eq!(response.body.lines().count(), 2);
    assert!(response.body.contains(",First Bookmark,"));
    assert!(response.body.contains(",true,"));
    assert!(response.body.contains("action=open-bookmark"));
    assert!(response.body.contains("action=rename-bookmark"));
    assert!(response.body.contains("action=remove-bookmark"));
    assert!(response.body.ends_with(",1\n"));
    let state_export = RequestTarget {
        path: "/api/browser-session".to_owned(),
        params: vec![
            ("id".to_owned(), payload.id.clone()),
            ("format".to_owned(), "session-state".to_owned()),
        ],
    };
    let response = browser_session_api_response(&state_export, &payload);
    assert_eq!(response.status, 200);
    assert_eq!(response.content_type, "application/json; charset=utf-8");
    let exported: serde_json::Value = serde_json::from_str(&response.body).unwrap();
    assert_eq!(exported["profile"]["enabled"], false);
    assert_eq!(exported["profile"]["current_bookmarked"], true);
    assert_eq!(exported["counts"]["bookmarks"], 1);
    assert_eq!(exported["counts"]["profile_history"], 0);
    let bookmarks = exported["bookmarks"].as_array().unwrap();
    assert_eq!(bookmarks.len(), 1);
    assert_eq!(bookmarks[0]["title"], "First Bookmark");
    assert_eq!(bookmarks[0]["current"], true);
    assert!(
        bookmarks[0]["source"]
            .as_str()
            .unwrap()
            .ends_with("first.html")
    );
    assert!(
        bookmarks[0]["action_url"]
            .as_str()
            .unwrap()
            .contains("action=open-bookmark")
    );
    assert!(
        bookmarks[0]["remove_url"]
            .as_str()
            .unwrap()
            .contains("action=remove-bookmark")
    );
    assert!(
        bookmarks[0]["rename_url"]
            .as_str()
            .unwrap()
            .contains("action=rename-bookmark")
    );
    assert!(
        bookmarks[0]["background_session_url"]
            .as_str()
            .unwrap()
            .contains("action=open-background-session")
    );
    assert!(
        exported["clear_urls"]["bookmarks"]
            .as_str()
            .unwrap()
            .contains("clear-bookmarks")
    );
    assert!(exported["clear_urls"]["profile_history"].is_null());

    let rename_bookmark = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![
            ("id".to_owned(), payload.id.clone()),
            ("action".to_owned(), "rename-bookmark".to_owned()),
            ("bookmark".to_owned(), payload.bookmarks[0].id.clone()),
            ("title".to_owned(), "Reading Queue".to_owned()),
        ],
    };
    let (payload, _) = registry.apply_target(&rename_bookmark).await.unwrap();
    assert!(payload.current_bookmarked);
    assert_eq!(payload.bookmarks.len(), 1);
    assert_eq!(payload.bookmarks[0].title, "Reading Queue");

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
    assert_eq!(payload.bookmarks[0].title, "Reading Queue");
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
async fn browser_session_registry_bookmarks_all_open_tabs() {
    let dir = tempfile::tempdir().unwrap();
    let first = dir.path().join("first.html");
    let second = dir.path().join("second.html");
    let third = dir.path().join("third.html");
    std::fs::write(
        &first,
        r#"<!doctype html><title>Bookmark All First</title><p>first all-tab bookmark</p>"#,
    )
    .unwrap();
    std::fs::write(
        &second,
        r#"<!doctype html><title>Bookmark All Second</title><p>second all-tab bookmark</p>"#,
    )
    .unwrap();
    std::fs::write(
        &third,
        r#"<!doctype html><title>Bookmark All Third</title><p>third all-tab bookmark</p>"#,
    )
    .unwrap();

    let registry = BrowserSessionRegistry::default();
    let create_first = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![("url".to_owned(), first.display().to_string())],
    };
    registry.create_target(&create_first).await.unwrap();
    let create_second = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![("url".to_owned(), second.display().to_string())],
    };
    registry.create_target(&create_second).await.unwrap();
    let create_third = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![("url".to_owned(), third.display().to_string())],
    };
    let (payload, back_href) = registry.create_target(&create_third).await.unwrap();
    let active_id = payload.id.clone();
    assert_eq!(payload.title, "Bookmark All Third");
    assert_eq!(payload.sessions.len(), 3);
    assert!(payload.bookmarks.is_empty());
    assert!(!payload.current_bookmarked);

    let html = render_browser_session_page(&payload, &back_href);
    assert!(html.contains(">Add all tabs</a>"));
    assert!(html.contains("action=bookmark-all-tabs"));
    let state_export = RequestTarget {
        path: "/api/browser-session".to_owned(),
        params: vec![
            ("id".to_owned(), payload.id.clone()),
            ("format".to_owned(), "session-state".to_owned()),
        ],
    };
    let response = browser_session_api_response(&state_export, &payload);
    assert_eq!(response.status, 200);
    let exported: serde_json::Value = serde_json::from_str(&response.body).unwrap();
    let bookmark_all_href = exported["action_urls"]["bookmark_all_tabs"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(bookmark_all_href.contains("action=bookmark-all-tabs"));

    let bookmark_all = RequestTarget {
        path: "/browser".to_owned(),
        params: form_urlencoded::parse(
            bookmark_all_href.trim_start_matches("/browser?").as_bytes(),
        )
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect(),
    };
    let (payload, back_href) = registry.apply_target(&bookmark_all).await.unwrap();
    assert_eq!(payload.id, active_id);
    assert_eq!(payload.title, "Bookmark All Third");
    assert_eq!(payload.sessions.len(), 3);
    assert_eq!(payload.bookmarks.len(), 3);
    assert!(payload.current_bookmarked);
    assert!(
        payload
            .bookmarks
            .iter()
            .any(|bookmark| bookmark.title == "Bookmark All First"
                && bookmark.source.ends_with("first.html"))
    );
    assert!(
        payload
            .bookmarks
            .iter()
            .any(|bookmark| bookmark.title == "Bookmark All Second"
                && bookmark.source.ends_with("second.html"))
    );
    assert!(
        payload
            .bookmarks
            .iter()
            .any(|bookmark| bookmark.title == "Bookmark All Third"
                && bookmark.source.ends_with("third.html")
                && bookmark.current)
    );
    let html = render_browser_session_page(&payload, &back_href);
    assert!(!html.contains("action=bookmark-all-tabs"));
    let response = browser_session_api_response(&state_export, &payload);
    let exported: serde_json::Value = serde_json::from_str(&response.body).unwrap();
    assert!(exported["action_urls"]["bookmark_all_tabs"].is_null());
    assert_eq!(exported["counts"]["bookmarks"], 3);
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
async fn browser_session_registry_opens_bookmarks_in_background_sessions() {
    let dir = tempfile::tempdir().unwrap();
    let first = dir.path().join("bulk-first.html");
    let second = dir.path().join("bulk-second.html");
    let third = dir.path().join("bulk-third.html");
    std::fs::write(
        &first,
        r#"<!doctype html><title>Bulk First</title><p>first saved bulk page</p>"#,
    )
    .unwrap();
    std::fs::write(
        &second,
        r#"<!doctype html><title>Bulk Second</title><p>second saved bulk page</p>"#,
    )
    .unwrap();
    std::fs::write(
        &third,
        r#"<!doctype html><title>Bulk Third</title><p>active bulk page</p>"#,
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
    let (payload, _) = registry.apply_target(&add_second).await.unwrap();
    let open_third = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![
            ("id".to_owned(), payload.id.clone()),
            ("action".to_owned(), "open".to_owned()),
            ("url".to_owned(), third.display().to_string()),
        ],
    };
    let (payload, back_href) = registry.apply_target(&open_third).await.unwrap();
    let active_id = payload.id.clone();
    assert_eq!(payload.title, "Bulk Third");
    assert_eq!(payload.sessions.len(), 1);
    assert_eq!(payload.bookmarks.len(), 2);
    assert!(!payload.current_bookmarked);
    let bulk_href = payload.bookmarks_background_url.clone().unwrap();
    assert!(bulk_href.contains("action=open-bookmarks-background-sessions"));
    let html = render_browser_session_page(&payload, &back_href);
    assert!(html.contains("Open all tabs"));
    assert!(html.contains("open-bookmarks-new-sessions"));
    assert!(html.contains("Open all bg"));
    assert!(html.contains("open-bookmarks-background-sessions"));
    let state_export = RequestTarget {
        path: "/api/browser-session".to_owned(),
        params: vec![
            ("id".to_owned(), payload.id.clone()),
            ("format".to_owned(), "session-state".to_owned()),
        ],
    };
    let response = browser_session_api_response(&state_export, &payload);
    let exported: serde_json::Value = serde_json::from_str(&response.body).unwrap();
    let open_bookmarks_tabs_href = exported["action_urls"]["open_bookmarks_new_sessions"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(open_bookmarks_tabs_href.contains("action=open-bookmarks-new-sessions"));
    assert!(
        exported["action_urls"]["open_bookmarks_background"]
            .as_str()
            .unwrap()
            .contains("action=open-bookmarks-background-sessions")
    );

    let open_bookmarks_tabs = RequestTarget {
        path: "/browser".to_owned(),
        params: form_urlencoded::parse(
            open_bookmarks_tabs_href
                .trim_start_matches("/browser?")
                .as_bytes(),
        )
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect(),
    };
    let (payload, _) = registry.apply_target(&open_bookmarks_tabs).await.unwrap();
    let first_bookmark_tab_id = payload.id.clone();
    assert_ne!(first_bookmark_tab_id, active_id);
    assert_eq!(payload.title, "Bulk First");
    assert_eq!(payload.sessions.len(), 3);
    assert!(payload.sessions.iter().any(|session| !session.current
        && session.id == active_id
        && session.title == "Bulk Third"));
    assert!(payload.sessions.iter().any(|session| {
        session.current && session.id == first_bookmark_tab_id && session.title == "Bulk First"
    }));
    assert!(
        payload
            .sessions
            .iter()
            .any(|session| !session.current && session.title == "Bulk Second")
    );

    let open_bookmarks = RequestTarget {
        path: "/browser".to_owned(),
        params: form_urlencoded::parse(bulk_href.trim_start_matches("/browser?").as_bytes())
            .map(|(key, value)| (key.into_owned(), value.into_owned()))
            .collect(),
    };
    let (payload, _) = registry.apply_target(&open_bookmarks).await.unwrap();
    assert_eq!(payload.id, active_id);
    assert_eq!(payload.title, "Bulk Third");
    assert_eq!(payload.sessions.len(), 3);
    assert!(payload.sessions.iter().any(|session| session.current
        && session.id == active_id
        && session.title == "Bulk Third"));
    assert!(
        payload
            .sessions
            .iter()
            .any(|session| !session.current && session.title == "Bulk First")
    );
    assert!(
        payload
            .sessions
            .iter()
            .any(|session| !session.current && session.title == "Bulk Second")
    );

    let open_bookmarks_again = RequestTarget {
        path: "/browser".to_owned(),
        params: form_urlencoded::parse(
            open_bookmarks_tabs_href
                .trim_start_matches("/browser?")
                .as_bytes(),
        )
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect(),
    };
    let (payload, _) = registry.apply_target(&open_bookmarks_again).await.unwrap();
    assert_eq!(payload.id, active_id);
    assert_eq!(payload.title, "Bulk Third");
    assert_eq!(payload.sessions.len(), 3);
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
    let rename_bookmark = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![
            ("id".to_owned(), payload.id.clone()),
            ("action".to_owned(), "rename-bookmark".to_owned()),
            ("bookmark".to_owned(), payload.bookmarks[0].id.clone()),
            ("title".to_owned(), "Saved Persist One".to_owned()),
        ],
    };
    let (payload, _) = registry.apply_target(&rename_bookmark).await.unwrap();
    assert_eq!(payload.bookmarks.len(), 1);
    assert_eq!(payload.bookmarks[0].title, "Saved Persist One");
    assert!(
        std::fs::read_to_string(&profile)
            .unwrap()
            .contains("Saved Persist One")
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
    assert_eq!(payload.bookmarks[0].title, "Saved Persist One");
    assert!(payload.bookmarks[0].source.ends_with("first.html"));
    assert_eq!(payload.profile_history.len(), 2);
    assert_eq!(payload.profile_history[0].title, "Persist Two");
    assert_eq!(payload.profile_history[1].title, "Persist One");
    assert!(payload.profile_history_clear_url.is_some());
    let html = render_browser_session_page(&payload, &back_href);
    assert!(html.contains("<h2>Profile history</h2>"));
    assert!(html.contains("Saved Persist One"));
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
async fn browser_session_registry_opens_profile_history_in_background_sessions() {
    let dir = tempfile::tempdir().unwrap();
    let profile = dir.path().join("browser-profile.json");
    let first = dir.path().join("history-one.html");
    let second = dir.path().join("history-two.html");
    let third = dir.path().join("history-three.html");
    std::fs::write(
        &first,
        r#"<!doctype html><title>History Bulk One</title><p>first history bulk page</p>"#,
    )
    .unwrap();
    std::fs::write(
        &second,
        r#"<!doctype html><title>History Bulk Two</title><p>second history bulk page</p>"#,
    )
    .unwrap();
    std::fs::write(
        &third,
        r#"<!doctype html><title>History Bulk Three</title><p>current history bulk page</p>"#,
    )
    .unwrap();

    let registry = BrowserSessionRegistry::with_profile_path(profile);
    let create_first = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![("url".to_owned(), first.display().to_string())],
    };
    let (payload, _) = registry.create_target(&create_first).await.unwrap();
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
    let (payload, back_href) = registry.apply_target(&open_third).await.unwrap();
    let active_id = payload.id.clone();
    assert_eq!(payload.title, "History Bulk Three");
    assert_eq!(payload.sessions.len(), 1);
    assert_eq!(payload.profile_history.len(), 3);
    assert_eq!(payload.profile_history[0].title, "History Bulk Three");
    assert_eq!(payload.profile_history[1].title, "History Bulk Two");
    assert_eq!(payload.profile_history[2].title, "History Bulk One");
    let html = render_browser_session_page(&payload, &back_href);
    assert!(html.contains(">Open history tabs</a>"));
    assert!(html.contains("action=open-profile-history-new-sessions"));
    assert!(html.contains(">Open history bg</a>"));
    assert!(html.contains("action=open-profile-history-background-sessions"));

    let state_export = RequestTarget {
        path: "/api/browser-session".to_owned(),
        params: vec![
            ("id".to_owned(), payload.id.clone()),
            ("format".to_owned(), "session-state".to_owned()),
        ],
    };
    let response = browser_session_api_response(&state_export, &payload);
    assert_eq!(response.status, 200);
    let exported: serde_json::Value = serde_json::from_str(&response.body).unwrap();
    let open_history_tabs_href = exported["action_urls"]["open_profile_history_new_sessions"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(open_history_tabs_href.contains("action=open-profile-history-new-sessions"));
    assert!(open_history_tabs_href.contains("limit=16"));
    let open_history_href = exported["action_urls"]["open_profile_history_background"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(open_history_href.contains("action=open-profile-history-background-sessions"));
    assert!(open_history_href.contains("limit=16"));

    let open_history_tabs = RequestTarget {
        path: "/browser".to_owned(),
        params: form_urlencoded::parse(
            open_history_tabs_href
                .trim_start_matches("/browser?")
                .as_bytes(),
        )
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect(),
    };
    let (payload, _) = registry.apply_target(&open_history_tabs).await.unwrap();
    let first_history_tab_id = payload.id.clone();
    assert_ne!(first_history_tab_id, active_id);
    assert_eq!(payload.title, "History Bulk Two");
    assert_eq!(payload.sessions.len(), 3);
    assert!(payload.sessions.iter().any(|session| !session.current
        && session.id == active_id
        && session.title == "History Bulk Three"));
    assert!(payload.sessions.iter().any(|session| {
        session.current && session.id == first_history_tab_id && session.title == "History Bulk Two"
    }));
    assert!(
        payload
            .sessions
            .iter()
            .any(|session| !session.current && session.title == "History Bulk One")
    );

    let open_history = RequestTarget {
        path: "/browser".to_owned(),
        params: form_urlencoded::parse(
            open_history_href.trim_start_matches("/browser?").as_bytes(),
        )
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect(),
    };
    let (payload, _) = registry.apply_target(&open_history).await.unwrap();
    assert_eq!(payload.id, active_id);
    assert_eq!(payload.title, "History Bulk Three");
    assert_eq!(payload.sessions.len(), 3);
    assert!(payload.sessions.iter().any(|session| session.current
        && session.id == active_id
        && session.title == "History Bulk Three"));
    assert!(
        payload
            .sessions
            .iter()
            .any(|session| !session.current && session.title == "History Bulk Two")
    );
    assert!(
        payload
            .sessions
            .iter()
            .any(|session| !session.current && session.title == "History Bulk One")
    );

    let open_history_again = RequestTarget {
        path: "/browser".to_owned(),
        params: form_urlencoded::parse(
            open_history_href.trim_start_matches("/browser?").as_bytes(),
        )
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect(),
    };
    let (payload, _) = registry.apply_target(&open_history_again).await.unwrap();
    assert_eq!(payload.id, active_id);
    assert_eq!(payload.sessions.len(), 3);

    let open_history_tabs_again = RequestTarget {
        path: "/browser".to_owned(),
        params: form_urlencoded::parse(
            open_history_tabs_href
                .trim_start_matches("/browser?")
                .as_bytes(),
        )
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect(),
    };
    let (payload, _) = registry
        .apply_target(&open_history_tabs_again)
        .await
        .unwrap();
    assert_eq!(payload.id, active_id);
    assert_eq!(payload.title, "History Bulk Three");
    assert_eq!(payload.sessions.len(), 3);
}

#[tokio::test]
async fn browser_session_registry_bookmarks_profile_history_entries() {
    let dir = tempfile::tempdir().unwrap();
    let profile = dir.path().join("browser-profile.json");
    let first = dir.path().join("history-bookmark-one.html");
    let second = dir.path().join("history-bookmark-two.html");
    std::fs::write(
        &first,
        r#"<!doctype html><title>History Bookmark Old</title><p>first history bookmark page</p>"#,
    )
    .unwrap();
    std::fs::write(
        &second,
        r#"<!doctype html><title>History Bookmark Two</title><p>second history bookmark page</p>"#,
    )
    .unwrap();

    let registry = BrowserSessionRegistry::with_profile_path(profile.clone());
    let create_first = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![("url".to_owned(), first.display().to_string())],
    };
    let (payload, _) = registry.create_target(&create_first).await.unwrap();
    std::fs::write(
        &first,
        r#"<!doctype html><title>History Bookmark Current</title><p>updated first history bookmark page</p>"#,
    )
    .unwrap();

    let open_second = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![
            ("id".to_owned(), payload.id.clone()),
            ("action".to_owned(), "open".to_owned()),
            ("url".to_owned(), second.display().to_string()),
        ],
    };
    let (payload, _) = registry.apply_target(&open_second).await.unwrap();
    let open_first = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![
            ("id".to_owned(), payload.id.clone()),
            ("action".to_owned(), "open".to_owned()),
            ("url".to_owned(), first.display().to_string()),
        ],
    };
    let (payload, back_href) = registry.apply_target(&open_first).await.unwrap();
    assert_eq!(payload.title, "History Bookmark Current");
    assert_eq!(payload.sessions.len(), 1);
    assert_eq!(payload.profile_history.len(), 3);
    assert_eq!(payload.profile_history[0].title, "History Bookmark Current");
    assert_eq!(payload.profile_history[1].title, "History Bookmark Two");
    assert_eq!(payload.profile_history[2].title, "History Bookmark Old");
    let html = render_browser_session_page(&payload, &back_href);
    assert!(html.contains(">Bookmark history</a>"));
    assert!(html.contains("action=bookmark-profile-history"));

    let state_export = RequestTarget {
        path: "/api/browser-session".to_owned(),
        params: vec![
            ("id".to_owned(), payload.id.clone()),
            ("format".to_owned(), "session-state".to_owned()),
        ],
    };
    let response = browser_session_api_response(&state_export, &payload);
    assert_eq!(response.status, 200);
    let exported: serde_json::Value = serde_json::from_str(&response.body).unwrap();
    let bookmark_history_href = exported["action_urls"]["bookmark_profile_history"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(bookmark_history_href.contains("action=bookmark-profile-history"));

    let bookmark_history = RequestTarget {
        path: "/browser".to_owned(),
        params: form_urlencoded::parse(
            bookmark_history_href
                .trim_start_matches("/browser?")
                .as_bytes(),
        )
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect(),
    };
    let (payload, back_href) = registry.apply_target(&bookmark_history).await.unwrap();
    assert_eq!(payload.title, "History Bookmark Current");
    assert_eq!(payload.sessions.len(), 1);
    assert!(payload.current_bookmarked);
    assert_eq!(payload.bookmarks.len(), 2);
    assert!(
        payload
            .bookmarks
            .iter()
            .any(|bookmark| bookmark.title == "History Bookmark Current"
                && bookmark.source.ends_with("history-bookmark-one.html"))
    );
    assert!(
        payload
            .bookmarks
            .iter()
            .any(|bookmark| bookmark.title == "History Bookmark Two"
                && bookmark.source.ends_with("history-bookmark-two.html"))
    );
    assert!(
        payload
            .bookmarks
            .iter()
            .all(|bookmark| bookmark.title != "History Bookmark Old")
    );
    let saved = load_browser_session_profile(&profile).unwrap();
    assert_eq!(saved.bookmarks.len(), 2);
    assert!(
        saved
            .bookmarks
            .iter()
            .any(|bookmark| bookmark.title == "History Bookmark Current"
                && bookmark.source.ends_with("history-bookmark-one.html"))
    );

    let response = browser_session_api_response(&state_export, &payload);
    assert_eq!(response.status, 200);
    let exported: serde_json::Value = serde_json::from_str(&response.body).unwrap();
    assert!(exported["action_urls"]["bookmark_profile_history"].is_null());
    let remove_history_bookmarks_href = exported["action_urls"]["remove_profile_history_bookmarks"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(remove_history_bookmarks_href.contains("action=remove-profile-history-bookmarks"));
    let html = render_browser_session_page(&payload, &back_href);
    assert!(!html.contains("action=bookmark-profile-history"));
    assert!(html.contains(">Remove history bookmarks</a>"));
    assert!(html.contains("action=remove-profile-history-bookmarks"));

    let remove_history_bookmarks = RequestTarget {
        path: "/browser".to_owned(),
        params: form_urlencoded::parse(
            remove_history_bookmarks_href
                .trim_start_matches("/browser?")
                .as_bytes(),
        )
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect(),
    };
    let (payload, back_href) = registry
        .apply_target(&remove_history_bookmarks)
        .await
        .unwrap();
    assert_eq!(payload.title, "History Bookmark Current");
    assert_eq!(payload.sessions.len(), 1);
    assert!(!payload.current_bookmarked);
    assert!(payload.bookmarks.is_empty());
    assert!(
        load_browser_session_profile(&profile)
            .unwrap()
            .bookmarks
            .is_empty()
    );
    let response = browser_session_api_response(&state_export, &payload);
    assert_eq!(response.status, 200);
    let exported: serde_json::Value = serde_json::from_str(&response.body).unwrap();
    assert!(
        exported["action_urls"]["bookmark_profile_history"]
            .as_str()
            .unwrap()
            .contains("action=bookmark-profile-history")
    );
    assert!(exported["action_urls"]["remove_profile_history_bookmarks"].is_null());
    let html = render_browser_session_page(&payload, &back_href);
    assert!(html.contains(">Bookmark history</a>"));
    assert!(!html.contains("action=remove-profile-history-bookmarks"));

    let (payload, _) = registry.apply_target(&bookmark_history).await.unwrap();
    assert_eq!(payload.bookmarks.len(), 2);
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
    let pin_first = RequestTarget {
        path: "/browser".to_owned(),
        params: form_urlencoded::parse(
            payload
                .sessions
                .iter()
                .find(|session| session.id == first_id)
                .unwrap()
                .pin_url
                .trim_start_matches("/browser?")
                .as_bytes(),
        )
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect(),
    };
    let (payload, _) = registry.apply_target(&pin_first).await.unwrap();
    let label_first = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![
            ("id".to_owned(), payload.id.clone()),
            ("action".to_owned(), "label-tab".to_owned()),
            ("session".to_owned(), first_id.clone()),
            ("label".to_owned(), "Saved workspace".to_owned()),
        ],
    };
    let (payload, _) = registry.apply_target(&label_first).await.unwrap();
    let saved = load_browser_session_profile(&profile).unwrap();
    assert_eq!(saved.tabs.len(), 2);
    assert!(saved.tabs[0].pinned);
    assert_eq!(saved.tabs[0].label.as_deref(), Some("Saved workspace"));
    assert!(!saved.tabs[1].pinned);
    assert!(saved.tabs[1].label.is_none());
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
    assert!(saved.tabs[0].pinned);
    assert_eq!(saved.tabs[0].label.as_deref(), Some("Saved workspace"));
    assert!(!saved.tabs[1].active);
    assert!(!saved.tabs[1].pinned);
    assert!(saved.tabs[1].label.is_none());
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
            .any(|session| session.title == "Saved workspace"
                && session.page_title == "Tab One"
                && session.current
                && session.pinned)
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
                    pinned: false,
                    label: None,
                    updated_at_unix_secs: 1,
                },
                BrowserSessionProfileTabFile {
                    title: "Missing Tab".to_owned(),
                    source: missing.display().to_string(),
                    active: false,
                    pinned: false,
                    label: None,
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
    let (payload, back_href) = registry.create_target(&create_second).await.unwrap();
    assert_eq!(payload.profile_history.len(), 2);
    assert_eq!(payload.profile_history[0].title, "History Two");
    assert_eq!(payload.profile_history[1].title, "History One");
    let html = render_browser_session_page(&payload, &back_href);
    assert!(html.contains("Profile History CSV"));
    assert!(html.contains("format=profile-history-csv"));
    let profile_history_csv_export = RequestTarget {
        path: "/api/browser-session".to_owned(),
        params: vec![
            ("id".to_owned(), payload.id.clone()),
            ("format".to_owned(), "profile-history-csv".to_owned()),
        ],
    };
    let response = browser_session_api_response(&profile_history_csv_export, &payload);
    assert_eq!(response.status, 200);
    assert_eq!(response.content_type, "text/csv; charset=utf-8");
    assert!(response.body.starts_with(
        "index,title,source,visited_at_unix_secs,visited_at,action_url,new_session_url,background_session_url,remove_url,session_id,active_source,profile_history_count\n"
    ));
    assert_eq!(response.body.lines().count(), 3);
    assert!(response.body.contains("1,History Two,"));
    assert!(response.body.contains("2,History One,"));
    assert!(response.body.contains("action=open"));
    assert!(response.body.contains("action=remove-profile-history"));
    assert!(response.body.ends_with(",2\n"));
    let state_export = RequestTarget {
        path: "/api/browser-session".to_owned(),
        params: vec![
            ("id".to_owned(), payload.id.clone()),
            ("format".to_owned(), "session-state".to_owned()),
        ],
    };
    let response = browser_session_api_response(&state_export, &payload);
    assert_eq!(response.status, 200);
    assert_eq!(response.content_type, "application/json; charset=utf-8");
    let exported: serde_json::Value = serde_json::from_str(&response.body).unwrap();
    assert_eq!(exported["profile"]["enabled"], true);
    assert_eq!(exported["profile"]["current_bookmarked"], false);
    assert_eq!(exported["counts"]["profile_history"], 2);
    assert_eq!(exported["counts"]["bookmarks"], 0);
    let profile_history = exported["profile_history"].as_array().unwrap();
    assert_eq!(profile_history.len(), 2);
    assert_eq!(profile_history[0]["index"], 0);
    assert_eq!(profile_history[0]["title"], "History Two");
    assert_eq!(profile_history[1]["index"], 1);
    assert_eq!(profile_history[1]["title"], "History One");
    assert!(
        profile_history[0]["action_url"]
            .as_str()
            .unwrap()
            .contains("action=open")
    );
    assert!(
        profile_history[0]["remove_url"]
            .as_str()
            .unwrap()
            .contains("action=remove-profile-history")
    );
    assert!(
        exported["clear_urls"]["profile_history"]
            .as_str()
            .unwrap()
            .contains("clear-profile-history")
    );
    assert!(
        exported["clear_urls"]["profile_tabs"]
            .as_str()
            .unwrap()
            .contains("clear-profile-tabs")
    );

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
        viewport_y: 7,
        document_width: 90,
        document_height: 30,
        max_scroll_x: 20,
        max_scroll_y: 0,
        dom_node_count: 1,
        link_count: 0,
        anchor_count: 0,
        can_back: false,
        can_forward: false,
        history_len: 1,
        current_history_index: Some(0),
        profile_enabled: false,
        profile_error: None,
        current_bookmarked: false,
        bookmarks_clear_url: None,
        bookmarks_background_url: None,
        links_background_url: None,
        closed_sessions_clear_url: None,
        profile_tabs_clear_url: None,
        profile_history_clear_url: None,
        find_query: String::new(),
        find_match_count: 0,
        find_current_index: None,
        find_current_line: None,
        find_matches: Vec::new(),
        tab_search_query: String::new(),
        tab_search_results: Vec::new(),
        sessions: Vec::new(),
        closed_sessions: Vec::new(),
        bookmarks: Vec::new(),
        profile_history: Vec::new(),
        history: Vec::new(),
        viewport: String::new(),
        page_text: String::new(),
        focused: None,
        anchors: Vec::new(),
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
    assert_eq!(target.param("viewport_y").as_deref(), Some("7"));
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
    assert!(
        payload.forms[0].controls[1].options[0]
            .select_url
            .as_deref()
            .is_some_and(|href| href.contains("action=select") && href.contains("value=docs"))
    );

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
        params: form_urlencoded::parse(
            payload.forms[0].controls[1].options[0]
                .select_url
                .as_deref()
                .unwrap()
                .trim_start_matches("/browser?")
                .as_bytes(),
        )
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect(),
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
async fn browser_session_registry_fills_form_controls_by_index() {
    let dir = tempfile::tempdir().unwrap();
    let form_page = dir.path().join("controls.html");
    std::fs::write(
        &form_page,
        r#"<!doctype html>
<title>Indexed Controls</title>
<form>
  <input value="anonymous">
  <input name="title" value="old">
  <input name="dup" value="one">
  <input name="dup" value="two">
  <textarea name="notes">old notes</textarea>
  <input type="hidden" name="token" value="secret">
</form>"#,
    )
    .unwrap();

    let registry = BrowserSessionRegistry::default();
    let create = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![("url".to_owned(), form_page.display().to_string())],
    };
    let (payload, _) = registry.create_target(&create).await.unwrap();
    assert!(payload.forms[0].controls[0].name.is_empty());
    assert_eq!(payload.forms[0].controls[0].value, "anonymous");
    assert_eq!(payload.forms[0].controls[1].value, "old");
    assert_eq!(payload.forms[0].controls[2].value, "one");
    assert_eq!(payload.forms[0].controls[3].value, "two");
    assert_eq!(payload.forms[0].controls[4].value, "old notes");
    assert!(payload.forms[0].controls[0].fill_url.is_none());
    assert!(payload.forms[0].controls[0].type_url.is_none());
    assert!(payload.forms[0].controls[0].clear_url.is_none());
    assert!(payload.forms[0].controls[0].focus_url.is_none());
    assert!(payload.forms[0].controls[1].fill_url.is_some());
    assert!(payload.forms[0].controls[1].type_url.is_some());
    assert!(payload.forms[0].controls[1].clear_url.is_some());
    assert!(payload.forms[0].controls[2].fill_url.is_none());
    assert!(payload.forms[0].controls[2].type_url.is_none());
    assert!(payload.forms[0].controls[2].clear_url.is_none());
    assert!(payload.forms[0].controls[3].fill_url.is_none());
    assert!(payload.forms[0].controls[3].type_url.is_none());
    assert!(payload.forms[0].controls[3].clear_url.is_none());
    assert!(payload.forms[0].controls[4].fill_url.is_some());
    assert!(payload.forms[0].controls[4].type_url.is_some());
    assert!(payload.forms[0].controls[4].clear_url.is_some());
    assert!(payload.forms[0].controls[5].fill_url.is_none());
    assert!(payload.forms[0].controls[5].type_url.is_none());
    assert!(payload.forms[0].controls[5].clear_url.is_none());

    let duplicate_fill = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![
            ("id".to_owned(), payload.id.clone()),
            ("action".to_owned(), "fill-control".to_owned()),
            ("form".to_owned(), "0".to_owned()),
            ("control".to_owned(), "2".to_owned()),
            ("value".to_owned(), "ambiguous".to_owned()),
        ],
    };
    assert!(registry.apply_target(&duplicate_fill).await.is_err());
    let mut params = form_urlencoded::parse(
        payload.forms[0].controls[1]
            .type_url
            .as_deref()
            .unwrap()
            .trim_start_matches("/browser?")
            .as_bytes(),
    )
    .map(|(key, value)| (key.into_owned(), value.into_owned()))
    .collect::<Vec<_>>();
    params.push(("value".to_owned(), "typed draft".to_owned()));
    let type_unique = RequestTarget {
        path: "/browser".to_owned(),
        params,
    };
    let (payload, back_href) = registry.apply_target(&type_unique).await.unwrap();
    assert_eq!(payload.forms[0].controls[0].value, "anonymous");
    assert_eq!(payload.forms[0].controls[1].value, "typed draft");
    assert_eq!(payload.forms[0].controls[2].value, "one");
    assert_eq!(payload.forms[0].controls[3].value, "two");
    let html = render_browser_session_page(&payload, &back_href);
    assert!(html.contains(r#"name="action" value="type-control""#));
    assert!(html.contains(">Type</button>"));
    assert!(html.contains("action=clear-control"));
    assert!(html.contains(">Clear</a>"));

    let mut params = form_urlencoded::parse(
        payload.forms[0].controls[1]
            .fill_url
            .as_deref()
            .unwrap()
            .trim_start_matches("/browser?")
            .as_bytes(),
    )
    .map(|(key, value)| (key.into_owned(), value.into_owned()))
    .collect::<Vec<_>>();
    params.push(("value".to_owned(), "draft".to_owned()));
    let fill_input = RequestTarget {
        path: "/browser".to_owned(),
        params,
    };
    let (payload, _) = registry.apply_target(&fill_input).await.unwrap();
    assert_eq!(payload.forms[0].controls[0].value, "anonymous");
    assert_eq!(payload.forms[0].controls[1].value, "draft");
    assert_eq!(payload.forms[0].controls[2].value, "one");
    assert_eq!(payload.forms[0].controls[3].value, "two");
    assert_eq!(payload.forms[0].controls[4].value, "old notes");

    let mut params = form_urlencoded::parse(
        payload.forms[0].controls[4]
            .fill_url
            .as_deref()
            .unwrap()
            .trim_start_matches("/browser?")
            .as_bytes(),
    )
    .map(|(key, value)| (key.into_owned(), value.into_owned()))
    .collect::<Vec<_>>();
    params.push(("value".to_owned(), "updated notes".to_owned()));
    let fill_textarea = RequestTarget {
        path: "/browser".to_owned(),
        params,
    };
    let (payload, _) = registry.apply_target(&fill_textarea).await.unwrap();
    assert_eq!(payload.forms[0].controls[0].value, "anonymous");
    assert_eq!(payload.forms[0].controls[1].value, "draft");
    assert_eq!(payload.forms[0].controls[2].value, "one");
    assert_eq!(payload.forms[0].controls[3].value, "two");
    assert_eq!(payload.forms[0].controls[4].value, "updated notes");

    let clear_textarea = RequestTarget {
        path: "/browser".to_owned(),
        params: form_urlencoded::parse(
            payload.forms[0].controls[4]
                .clear_url
                .as_deref()
                .unwrap()
                .trim_start_matches("/browser?")
                .as_bytes(),
        )
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect(),
    };
    let (payload, _) = registry.apply_target(&clear_textarea).await.unwrap();
    assert_eq!(payload.forms[0].controls[0].value, "anonymous");
    assert_eq!(payload.forms[0].controls[1].value, "draft");
    assert_eq!(payload.forms[0].controls[2].value, "one");
    assert_eq!(payload.forms[0].controls[3].value, "two");
    assert_eq!(payload.forms[0].controls[4].value, "");
    assert_eq!(payload.focused.as_ref().unwrap().name, "notes");

    let clear_duplicate = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![
            ("id".to_owned(), payload.id.clone()),
            ("action".to_owned(), "clear-control".to_owned()),
            ("form".to_owned(), "0".to_owned()),
            ("control".to_owned(), "2".to_owned()),
        ],
    };
    assert!(registry.apply_target(&clear_duplicate).await.is_err());
}

#[tokio::test]
async fn browser_session_registry_activates_form_action_controls() {
    let dir = tempfile::tempdir().unwrap();
    let form_page = dir.path().join("actions.html");
    let result_page = dir.path().join("result.html");
    std::fs::write(
        &form_page,
        r#"<!doctype html>
<title>Action Controls</title>
<form action="result.html" method="get">
  <input name="q" value="old">
  <input type="checkbox" name="fast" checked>
  <button type="reset" id="reset">Reset</button>
  <button type="submit" id="go" name="commit" value="yes">Go</button>
</form>"#,
    )
    .unwrap();
    std::fs::write(
        &result_page,
        r#"<!doctype html><title>Result</title><p>activated</p>"#,
    )
    .unwrap();

    let registry = BrowserSessionRegistry::default();
    let create = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![("url".to_owned(), form_page.display().to_string())],
    };
    let (payload, _) = registry.create_target(&create).await.unwrap();
    assert_eq!(payload.forms[0].controls[2].kind, "reset");
    assert_eq!(payload.forms[0].controls[3].kind, "submit");
    assert!(
        payload.forms[0].controls[2]
            .activate_new_session_url
            .is_none()
    );
    assert!(
        payload.forms[0].controls[2]
            .activate_background_session_url
            .is_none()
    );
    assert!(
        payload.forms[0].controls[2]
            .activate_url
            .as_deref()
            .is_some_and(|href| href.contains("action=activate-control"))
    );
    assert!(
        payload.forms[0].controls[3]
            .activate_new_session_url
            .as_deref()
            .is_some_and(|href| href.contains("action=activate-control-new-session"))
    );
    assert!(
        payload.forms[0].controls[3]
            .activate_background_session_url
            .as_deref()
            .is_some_and(|href| href.contains("action=activate-control-background-session"))
    );

    let fill = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![
            ("id".to_owned(), payload.id.clone()),
            ("action".to_owned(), "fill".to_owned()),
            ("form".to_owned(), "0".to_owned()),
            ("name".to_owned(), "q".to_owned()),
            ("value".to_owned(), "changed".to_owned()),
        ],
    };
    let (payload, _) = registry.apply_target(&fill).await.unwrap();
    let toggle = RequestTarget {
        path: "/browser".to_owned(),
        params: form_urlencoded::parse(
            payload.forms[0].controls[1]
                .toggle_url
                .as_deref()
                .unwrap()
                .trim_start_matches("/browser?")
                .as_bytes(),
        )
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect(),
    };
    let (payload, _) = registry.apply_target(&toggle).await.unwrap();
    assert_eq!(payload.forms[0].controls[0].value, "changed");
    assert!(!payload.forms[0].controls[1].checked);

    let reset = RequestTarget {
        path: "/browser".to_owned(),
        params: form_urlencoded::parse(
            payload.forms[0].controls[2]
                .activate_url
                .as_deref()
                .unwrap()
                .trim_start_matches("/browser?")
                .as_bytes(),
        )
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect(),
    };
    let (payload, _) = registry.apply_target(&reset).await.unwrap();
    assert_eq!(payload.title, "Action Controls");
    assert_eq!(payload.forms[0].controls[0].value, "old");
    assert!(payload.forms[0].controls[1].checked);

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
    let original_id = payload.id.clone();
    let submit_new_session = RequestTarget {
        path: "/browser".to_owned(),
        params: form_urlencoded::parse(
            payload.forms[0].controls[3]
                .activate_new_session_url
                .as_deref()
                .unwrap()
                .trim_start_matches("/browser?")
                .as_bytes(),
        )
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect(),
    };
    let (new_payload, _) = registry.apply_target(&submit_new_session).await.unwrap();
    assert_ne!(new_payload.id, original_id);
    assert_eq!(new_payload.title, "Result");
    assert_eq!(new_payload.history_len, 2);
    assert!(new_payload.source.contains("result.html"));
    assert!(new_payload.source.contains("q=rust+browser"));
    assert!(new_payload.source.contains("fast=on"));
    assert!(new_payload.source.contains("commit=yes"));

    let submit_background = RequestTarget {
        path: "/browser".to_owned(),
        params: form_urlencoded::parse(
            payload.forms[0].controls[3]
                .activate_background_session_url
                .as_deref()
                .unwrap()
                .trim_start_matches("/browser?")
                .as_bytes(),
        )
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect(),
    };
    let (background_payload, _) = registry.apply_target(&submit_background).await.unwrap();
    assert_eq!(background_payload.id, original_id);
    assert_eq!(background_payload.title, "Action Controls");
    assert_eq!(background_payload.sessions.len(), 3);
    assert!(
        background_payload
            .sessions
            .iter()
            .any(|session| { session.page_title == "Result" && !session.current })
    );

    let submit = RequestTarget {
        path: "/browser".to_owned(),
        params: form_urlencoded::parse(
            payload.forms[0].controls[3]
                .activate_url
                .as_deref()
                .unwrap()
                .trim_start_matches("/browser?")
                .as_bytes(),
        )
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect(),
    };
    let (payload, _) = registry.apply_target(&submit).await.unwrap();
    assert_eq!(payload.id, original_id);
    assert_eq!(payload.title, "Result");
    assert_eq!(payload.history_len, 2);
    assert!(payload.source.contains("result.html"));
    assert!(payload.source.contains("q=rust+browser"));
    assert!(payload.source.contains("fast=on"));
    assert!(payload.source.contains("commit=yes"));
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
    assert!(html.contains("action=submit-background-session"));

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
            ("id".to_owned(), first_id.clone()),
            ("action".to_owned(), "current".to_owned()),
        ],
    };
    let (payload, _) = registry.apply_target(&original).await.unwrap();
    assert_eq!(payload.title, "Form");
    assert_eq!(payload.history_len, 1);
    assert_eq!(payload.forms[0].controls[0].value, "rust browser");

    let submit_background = RequestTarget {
        path: "/browser".to_owned(),
        params: form_urlencoded::parse(
            payload.forms[0]
                .submit_background_session_url
                .trim_start_matches("/browser?")
                .as_bytes(),
        )
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect(),
    };
    let (payload, _) = registry.apply_target(&submit_background).await.unwrap();
    assert_eq!(payload.id, first_id);
    assert_eq!(payload.title, "Form");
    assert_eq!(payload.history_len, 1);
    assert_eq!(payload.sessions.len(), 3);
    assert!(payload.forms[0].controls[0].value == "rust browser");
    assert!(
        payload
            .sessions
            .iter()
            .any(|session| { session.page_title == "Result" && !session.current })
    );
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
    let focus_select_href = payload.forms[0].controls[1].focus_url.clone().unwrap();
    assert!(focus_select_href.contains("action=focus-control"));

    let focus_select = RequestTarget {
        path: "/browser".to_owned(),
        params: form_urlencoded::parse(
            focus_select_href.trim_start_matches("/browser?").as_bytes(),
        )
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect(),
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

    let state_export = RequestTarget {
        path: "/api/browser-session".to_owned(),
        params: vec![
            ("id".to_owned(), payload.id.clone()),
            ("format".to_owned(), "session-state".to_owned()),
        ],
    };
    let response = browser_session_api_response(&state_export, &payload);
    assert_eq!(response.status, 200);
    let exported: serde_json::Value = serde_json::from_str(&response.body).unwrap();
    assert_eq!(exported["focused"]["name"], "q");
    assert_eq!(exported["focused"]["kind"], "text");
    assert_eq!(exported["focused"]["value"], "old browser");
    assert_eq!(exported["focused"]["form_index"], 0);
    assert_eq!(exported["focused"]["control_index"], 0);

    let state_csv_export = RequestTarget {
        path: "/api/browser-session".to_owned(),
        params: vec![
            ("id".to_owned(), payload.id.clone()),
            ("format".to_owned(), "session-state-csv".to_owned()),
        ],
    };
    let response = browser_session_api_response(&state_csv_export, &payload);
    assert_eq!(response.status, 200);
    assert_eq!(response.content_type, "text/csv; charset=utf-8");
    assert!(response.body.contains("focused,,q,"));
    assert!(response.body.contains("form=0; control=0"));
    assert!(response.body.contains(",old browser,text,,"));
    assert!(response.body.contains("action=clear-input"));

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
    assert!(html.contains(r#"<span class="meta">0 images, 1 stylesheet</span>"#));
    assert!(!html.contains("action=load-images"));
    assert!(!html.contains(">Load images</a>"));

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
    assert!(html.contains("Report JSON"));
    assert!(html.contains("format=resource-report-json"));
    assert!(html.contains("Report CSV"));
    assert!(html.contains("format=resource-report-csv"));
    assert!(html.contains("Clear report"));
    assert!(html.contains("action=clear-resource-report"));
    let resource_report_json_export = RequestTarget {
        path: "/api/browser-session".to_owned(),
        params: vec![
            ("id".to_owned(), payload.id.clone()),
            ("format".to_owned(), "resource-report-json".to_owned()),
        ],
    };
    let response = browser_session_api_response(&resource_report_json_export, &payload);
    assert_eq!(response.status, 200);
    assert_eq!(response.content_type, "application/json; charset=utf-8");
    let exported_report: serde_json::Value = serde_json::from_str(&response.body).unwrap();
    assert_eq!(exported_report["format"], "browser-resource-report");
    assert_eq!(exported_report["id"], payload.id);
    assert_eq!(exported_report["resource_report"]["action"], "Apply styles");
    assert_eq!(exported_report["resource_report"]["applied"], 1);
    assert_eq!(
        exported_report["resource_report"]["resources"][0]["kind"],
        "stylesheet"
    );
    assert!(
        exported_report["csv_url"]
            .as_str()
            .unwrap()
            .contains("format=resource-report-csv")
    );
    assert!(
        exported_report["clear_url"]
            .as_str()
            .unwrap()
            .contains("clear-resource-report")
    );
    let resource_report_csv_export = RequestTarget {
        path: "/api/browser-session".to_owned(),
        params: vec![
            ("id".to_owned(), payload.id.clone()),
            ("format".to_owned(), "resource-report-csv".to_owned()),
        ],
    };
    let response = browser_session_api_response(&resource_report_csv_export, &payload);
    assert_eq!(response.status, 200);
    assert_eq!(response.content_type, "text/csv; charset=utf-8");
    assert!(response.body.starts_with(
        "action,page_source,total,fetched,cached,failed,skipped,applied,decoded,index,status,kind,url,resolved,source,bytes,content_type,error,session_id,active_source\n"
    ));
    assert_eq!(response.body.lines().count(), 2);
    assert!(response.body.contains("Apply styles,"));
    assert!(
        response
            .body
            .contains(",1,1,0,0,0,1,,1,fetched,stylesheet,")
    );
    assert!(response.body.contains("/app.css"));
    assert!(response.body.contains("text/css"));
    let state_export = RequestTarget {
        path: "/api/browser-session".to_owned(),
        params: vec![
            ("id".to_owned(), payload.id.clone()),
            ("format".to_owned(), "session-state".to_owned()),
        ],
    };
    let response = browser_session_api_response(&state_export, &payload);
    assert_eq!(response.status, 200);
    let exported: serde_json::Value = serde_json::from_str(&response.body).unwrap();
    assert_eq!(exported["resource_report"]["action"], "Apply styles");
    assert_eq!(exported["resource_report"]["total"], 1);
    assert_eq!(exported["resource_report"]["fetched"], 1);
    assert_eq!(exported["resource_report"]["failed"], 0);
    assert_eq!(exported["resource_report"]["applied"], 1);
    assert_eq!(exported["resource_report"]["resources"], 1);
    assert_eq!(
        exported["resource_report"]["fetches"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        exported["resource_report"]["fetches"][0]["status"],
        "fetched"
    );
    assert_eq!(
        exported["resource_report"]["fetches"][0]["kind"],
        "stylesheet"
    );
    assert!(
        exported["resource_report"]["fetches"][0]["resolved"]
            .as_str()
            .unwrap()
            .ends_with("/app.css")
    );
    assert_eq!(
        exported["resource_report"]["fetches"][0]["content_type"],
        "text/css"
    );
    assert!(
        exported["resource_report"]["csv_url"]
            .as_str()
            .unwrap()
            .contains("format=resource-report-csv")
    );
    assert!(
        exported["resource_report"]["clear_url"]
            .as_str()
            .unwrap()
            .contains("clear-resource-report")
    );

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
    assert!(!html.contains("Report CSV"));
    assert!(!html.contains("Clear report"));
}

#[tokio::test]
async fn browser_session_inspector_loads_images_and_exports_decode_report() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    let image_body = br##"<svg xmlns="http://www.w3.org/2000/svg" width="2" height="2"><rect width="2" height="2" fill="#000"/></svg>"##.to_vec();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        for _ in 0..2 {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut buf = [0u8; 4096];
            let read = stream.read(&mut buf).await.unwrap();
            let request = String::from_utf8_lossy(&buf[..read]);
            let request_line = request.lines().next().unwrap_or_default();
            let (body, content_type) = if request_line.contains(" /tile.svg ") {
                (image_body.clone(), "image/svg+xml")
            } else {
                (
                    br#"<!doctype html><title>Images</title><p>before image</p><img src="/tile.svg" alt="Tile image"><p>after image</p>"#.to_vec(),
                    "text/html",
                )
            };
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            stream.write_all(response.as_bytes()).await.unwrap();
            stream.write_all(&body).await.unwrap();
        }
    });

    let registry = BrowserSessionRegistry::default();
    let create = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![("url".to_owned(), format!("http://{addr}/images"))],
    };
    let (payload, back_href) = registry.create_target(&create).await.unwrap();
    assert_eq!(payload.title, "Images");
    assert_eq!(payload.resource_count, 1);
    assert_eq!(payload.resources[0].kind, "image");
    assert_eq!(payload.resources[0].alt.as_deref(), Some("Tile image"));
    assert!(payload.resource_report.is_none());
    let html = render_browser_session_page(&payload, &back_href);
    assert!(html.contains(">Load 1 image</a>"));
    assert!(html.contains(r#"<span class="meta">1 image</span>"#));
    assert!(html.contains("action=load-images"));

    let load_images = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![
            ("id".to_owned(), payload.id.clone()),
            ("action".to_owned(), "load-images".to_owned()),
        ],
    };
    let (payload, back_href) = registry.apply_target(&load_images).await.unwrap();
    server.await.unwrap();

    let report = payload.resource_report.as_ref().unwrap();
    assert_eq!(report.action, "Load images");
    assert_eq!(report.total, 1);
    assert_eq!(report.fetched, 1);
    assert_eq!(report.cached, 0);
    assert_eq!(report.failed, 0);
    assert_eq!(report.skipped, 0);
    assert_eq!(report.applied, None);
    assert_eq!(report.decoded, Some(1));
    assert_eq!(report.resources.len(), 1);
    assert_eq!(report.resources[0].status, "fetched");
    assert_eq!(report.resources[0].kind, "image");
    assert_eq!(
        report.resources[0].content_type.as_deref(),
        Some("image/svg+xml")
    );

    let html = render_browser_session_page(&payload, &back_href);
    assert!(html.contains("Load images: total=1 fetched=1 cached=0 failed=0 skipped=0 decoded=1"));
    assert!(html.contains("<th>Source</th>"));
    assert!(html.contains("<th>Content Type</th>"));
    assert!(html.contains("<th>Error</th>"));
    assert!(html.contains(&format!("http://{addr}/tile.svg")));
    assert!(html.contains("image/svg+xml"));
    assert!(html.contains("Report JSON"));
    assert!(html.contains("format=resource-report-json"));
    assert!(html.contains("Report CSV"));

    let resource_report_json_export = RequestTarget {
        path: "/api/browser-session".to_owned(),
        params: vec![
            ("id".to_owned(), payload.id.clone()),
            ("format".to_owned(), "resource-report-json".to_owned()),
        ],
    };
    let response = browser_session_api_response(&resource_report_json_export, &payload);
    assert_eq!(response.status, 200);
    assert_eq!(response.content_type, "application/json; charset=utf-8");
    let exported_report: serde_json::Value = serde_json::from_str(&response.body).unwrap();
    assert_eq!(exported_report["format"], "browser-resource-report");
    assert_eq!(exported_report["resource_report"]["action"], "Load images");
    assert_eq!(exported_report["resource_report"]["decoded"], 1);
    assert_eq!(
        exported_report["resource_report"]["resources"][0]["kind"],
        "image"
    );
    assert_eq!(
        exported_report["resource_report"]["resources"][0]["content_type"],
        "image/svg+xml"
    );

    let resource_report_csv_export = RequestTarget {
        path: "/api/browser-session".to_owned(),
        params: vec![
            ("id".to_owned(), payload.id.clone()),
            ("format".to_owned(), "resource-report-csv".to_owned()),
        ],
    };
    let response = browser_session_api_response(&resource_report_csv_export, &payload);
    assert_eq!(response.status, 200);
    assert_eq!(response.content_type, "text/csv; charset=utf-8");
    assert_eq!(response.body.lines().count(), 2);
    assert!(response.body.contains("Load images,"));
    assert!(response.body.contains(",1,1,0,0,0,,1,1,fetched,image,"));
    assert!(response.body.contains("/tile.svg"));
    assert!(response.body.contains("image/svg+xml"));

    let state_export = RequestTarget {
        path: "/api/browser-session".to_owned(),
        params: vec![
            ("id".to_owned(), payload.id.clone()),
            ("format".to_owned(), "session-state".to_owned()),
        ],
    };
    let response = browser_session_api_response(&state_export, &payload);
    assert_eq!(response.status, 200);
    let exported: serde_json::Value = serde_json::from_str(&response.body).unwrap();
    assert_eq!(exported["resource_report"]["action"], "Load images");
    assert_eq!(exported["resource_report"]["total"], 1);
    assert_eq!(exported["resource_report"]["fetched"], 1);
    assert_eq!(exported["resource_report"]["decoded"], 1);
    assert!(exported["resource_report"]["applied"].is_null());
    assert_eq!(exported["resource_report"]["resources"], 1);
    assert_eq!(
        exported["resource_report"]["fetches"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        exported["resource_report"]["fetches"][0]["status"],
        "fetched"
    );
    assert_eq!(exported["resource_report"]["fetches"][0]["kind"], "image");
    assert!(
        exported["resource_report"]["fetches"][0]["resolved"]
            .as_str()
            .unwrap()
            .ends_with("/tile.svg")
    );
    assert_eq!(
        exported["resource_report"]["fetches"][0]["content_type"],
        "image/svg+xml"
    );
    assert!(
        exported["action_urls"]["load_images"]
            .as_str()
            .unwrap()
            .contains("action=load-images")
    );
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
<link rel="icon" href="/favicon.ico">
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
    assert!(
        payload
            .resources
            .iter()
            .any(|resource| resource.url == "/favicon.ico")
    );

    let html = render_browser_session_page(&payload, &back_href);
    assert!(html.contains("Session State"));
    assert!(html.contains("State JSON"));
    assert!(html.contains("State CSV"));
    assert!(html.contains("Viewport Text"));
    assert!(html.contains("Page Text"));
    assert!(html.contains("/api/browser-session?"));
    assert!(html.contains("format=session-state"));
    assert!(html.contains("format=session-state-csv"));
    assert!(html.contains("format=viewport-text"));
    assert!(html.contains("format=page-text"));
    assert!(html.contains("Resources (3)"));
    assert!(html.contains(r#"<span class="meta">1 image, 1 stylesheet, 1 other resource</span>"#));
    assert!(html.contains("Resources JSON"));
    assert!(html.contains("format=resources-json"));
    assert!(html.contains("Resources CSV"));
    assert!(html.contains("format=resources-csv"));
    assert!(html.contains("resource-actions"));
    assert!(html.contains("action=resource"));
    assert!(html.contains(">Load 1 image</a>"));
    assert!(html.contains("action=resource-new-session"));
    assert!(html.contains("New session"));

    let state_export = RequestTarget {
        path: "/api/browser-session".to_owned(),
        params: vec![
            ("id".to_owned(), payload.id.clone()),
            ("format".to_owned(), "session-state".to_owned()),
        ],
    };
    let response = browser_session_api_response(&state_export, &payload);
    assert_eq!(response.status, 200);
    assert_eq!(response.content_type, "application/json; charset=utf-8");
    let exported: serde_json::Value = serde_json::from_str(&response.body).unwrap();
    assert_eq!(exported["format"], "browser-session-state");
    assert_eq!(exported["id"], payload.id);
    assert_eq!(exported["counts"]["cookies"], 1);
    assert_eq!(exported["counts"]["local_storage"], 1);
    assert_eq!(exported["counts"]["session_storage"], 1);
    assert_eq!(exported["viewport"]["width"], payload.width);
    assert_eq!(exported["history"]["len"], 1);
    for (field, format) in [
        ("payload_json", "json"),
        ("session_state_json", "session-state"),
        ("session_state_csv", "session-state-csv"),
        ("tabs_csv", "tabs-csv"),
        ("closed_sessions_csv", "closed-sessions-csv"),
        ("bookmarks_csv", "bookmarks-csv"),
        ("anchors_csv", "anchors-csv"),
        ("links_csv", "links-csv"),
        ("forms_json", "forms-json"),
        ("forms_csv", "forms-csv"),
        ("history_csv", "history-csv"),
        ("profile_history_csv", "profile-history-csv"),
        ("resources_json", "resources-json"),
        ("resources_csv", "resources-csv"),
        ("resource_report_json", "resource-report-json"),
        ("resource_report_csv", "resource-report-csv"),
        ("find_json", "find-json"),
        ("find_csv", "find-csv"),
        ("tab_search_json", "tab-search-json"),
        ("tab_search_csv", "tab-search-csv"),
        ("viewport_text", "viewport-text"),
        ("page_text", "page-text"),
    ] {
        let href = exported["export_urls"][field].as_str().unwrap();
        assert!(href.contains("/api/browser-session?"));
        assert!(href.contains(&format!("id={}", payload.id)));
        assert!(href.contains(&format!("format={format}")));
    }
    assert!(exported["action_urls"]["back"].is_null());
    assert!(exported["action_urls"]["forward"].is_null());
    assert!(
        exported["action_urls"]["reload"]
            .as_str()
            .unwrap()
            .contains("action=reload")
    );
    assert_eq!(payload.viewport_y, 0);
    assert_eq!(payload.max_scroll_y, 0);
    assert!(exported["action_urls"]["top"].is_null());
    assert!(exported["action_urls"]["bottom"].is_null());
    assert!(exported["action_urls"]["scroll_up"].is_null());
    assert!(exported["action_urls"]["scroll_down"].is_null());
    assert!(
        exported["action_urls"]["duplicate_tab"]
            .as_str()
            .unwrap()
            .contains("action=duplicate-session")
    );
    assert!(
        exported["action_urls"]["duplicate_tab_background"]
            .as_str()
            .unwrap()
            .contains("action=duplicate-background-session")
    );
    assert!(exported["action_urls"]["close_tab"].is_null());
    assert!(
        exported["action_urls"]["add_bookmark"]
            .as_str()
            .unwrap()
            .contains("action=add-bookmark")
    );
    assert!(exported["action_urls"]["clear_find"].is_null());
    assert!(
        exported["action_urls"]["fetch_resources"]
            .as_str()
            .unwrap()
            .contains("action=fetch-resources")
    );
    assert!(
        exported["action_urls"]["apply_stylesheets"]
            .as_str()
            .unwrap()
            .contains("action=apply-styles")
    );
    assert!(exported["action_urls"]["clear_resource_report"].is_null());
    assert!(
        exported["clear_urls"]["cookies"]
            .as_str()
            .unwrap()
            .contains("clear-cookies")
    );
    assert!(
        exported["cookies"]
            .as_array()
            .unwrap()
            .iter()
            .any(|cookie| cookie["name"] == "sid")
    );
    assert!(
        exported["local_storage"]
            .as_array()
            .unwrap()
            .iter()
            .any(|entry| entry["key"] == "theme" && entry["value"] == "dark")
    );
    assert!(
        exported["session_storage"]
            .as_array()
            .unwrap()
            .iter()
            .any(|entry| entry["key"] == "nonce" && entry["value"] == "abc")
    );
    let resources = exported["resources"].as_array().unwrap();
    assert_eq!(resources.len(), 3);
    let stylesheet = resources
        .iter()
        .find(|resource| resource["url"] == "/app.css")
        .unwrap();
    assert_eq!(stylesheet["index"], 0);
    assert_eq!(stylesheet["kind"], "stylesheet");
    assert_eq!(stylesheet["media"], "screen");
    assert_eq!(stylesheet["details"], "rel=stylesheet · media=screen");
    assert!(
        stylesheet["open_url"]
            .as_str()
            .unwrap()
            .contains("action=resource")
    );
    assert!(
        stylesheet["new_session_url"]
            .as_str()
            .unwrap()
            .contains("action=resource-new-session")
    );
    assert!(
        stylesheet["background_session_url"]
            .as_str()
            .unwrap()
            .contains("action=resource-background-session")
    );
    let image = resources
        .iter()
        .find(|resource| resource["url"] == "/logo.png")
        .unwrap();
    assert_eq!(image["kind"], "image");
    assert_eq!(image["alt"], "Logo");
    assert_eq!(image["details"], "alt=Logo");
    let icon = resources
        .iter()
        .find(|resource| resource["url"] == "/favicon.ico")
        .unwrap();
    assert_eq!(icon["kind"], "icon");
    assert_eq!(icon["details"], "rel=icon");

    let resources_json_export = RequestTarget {
        path: "/api/browser-session".to_owned(),
        params: vec![
            ("id".to_owned(), payload.id.clone()),
            ("format".to_owned(), "resources-json".to_owned()),
        ],
    };
    let response = browser_session_api_response(&resources_json_export, &payload);
    assert_eq!(response.status, 200);
    assert_eq!(response.content_type, "application/json; charset=utf-8");
    let exported_resources: serde_json::Value = serde_json::from_str(&response.body).unwrap();
    assert_eq!(exported_resources["format"], "browser-resources");
    assert_eq!(exported_resources["id"], payload.id);
    assert_eq!(exported_resources["resource_count"], 3);
    assert_eq!(exported_resources["resources"].as_array().unwrap().len(), 3);
    assert!(
        exported_resources["resources"]
            .as_array()
            .unwrap()
            .iter()
            .any(|resource| resource["url"] == "/app.css"
                && resource["kind"] == "stylesheet"
                && resource["media"] == "screen")
    );
    assert!(
        exported_resources["resources"]
            .as_array()
            .unwrap()
            .iter()
            .any(|resource| resource["url"] == "/logo.png"
                && resource["kind"] == "image"
                && resource["alt"] == "Logo")
    );
    assert!(
        exported_resources["resources"]
            .as_array()
            .unwrap()
            .iter()
            .any(|resource| resource["url"] == "/favicon.ico" && resource["kind"] == "icon")
    );
    assert!(
        exported_resources["csv_url"]
            .as_str()
            .unwrap()
            .contains("format=resources-csv")
    );
    assert!(
        exported_resources["session_state_url"]
            .as_str()
            .unwrap()
            .contains("format=session-state")
    );

    let state_csv_export = RequestTarget {
        path: "/api/browser-session".to_owned(),
        params: vec![
            ("id".to_owned(), payload.id.clone()),
            ("format".to_owned(), "session-state-csv".to_owned()),
        ],
    };
    let response = browser_session_api_response(&state_csv_export, &payload);
    assert_eq!(response.status, 200);
    assert_eq!(response.content_type, "text/csv; charset=utf-8");
    assert!(
        response.body.starts_with(
            "kind,origin,name,key,value,domain,path,flags,clear_url,session_id,source\n"
        )
    );
    assert!(response.body.contains("cookie,,sid,,abc,127.0.0.1,/"));
    assert!(response.body.contains("localStorage,"));
    assert!(response.body.contains(",theme,dark,"));
    assert!(response.body.contains("sessionStorage,"));
    assert!(response.body.contains(",nonce,abc,"));
    assert!(response.body.contains("clear-cookies"));
    assert!(response.body.contains("clear-local-storage"));
    assert!(response.body.contains("clear-session-storage"));

    let resources_csv_export = RequestTarget {
        path: "/api/browser-session".to_owned(),
        params: vec![
            ("id".to_owned(), payload.id.clone()),
            ("format".to_owned(), "resources-csv".to_owned()),
        ],
    };
    let response = browser_session_api_response(&resources_csv_export, &payload);
    assert_eq!(response.status, 200);
    assert_eq!(response.content_type, "text/csv; charset=utf-8");
    assert!(response.body.starts_with("index,kind,initiator,url,resolved,details,open_url,new_session_url,background_session_url,session_id,source,total_resource_count\n"));
    assert_eq!(response.body.lines().count(), 4);
    assert!(response.body.contains("stylesheet"));
    assert!(response.body.contains("/app.css"));
    assert!(response.body.contains("media=screen"));
    assert!(response.body.contains("image"));
    assert!(response.body.contains("/logo.png"));
    assert!(response.body.contains("alt=Logo"));
    assert!(response.body.contains("icon"));
    assert!(response.body.contains("/favicon.ico"));
    assert!(response.body.contains("rel=icon"));
    assert!(response.body.contains("action=resource"));
    assert!(response.body.contains("action=resource-new-session"));
    assert!(response.body.contains("action=resource-background-session"));
    assert!(response.body.contains("total_resource_count"));

    let viewport_text_export = RequestTarget {
        path: "/api/browser-session".to_owned(),
        params: vec![
            ("id".to_owned(), payload.id.clone()),
            ("format".to_owned(), "viewport-text".to_owned()),
        ],
    };
    let response = browser_session_api_response(&viewport_text_export, &payload);
    assert_eq!(response.status, 200);
    assert_eq!(response.content_type, "text/plain; charset=utf-8");
    assert_eq!(response.body, payload.viewport);

    let page_text_export = RequestTarget {
        path: "/api/browser-session".to_owned(),
        params: vec![
            ("id".to_owned(), payload.id.clone()),
            ("format".to_owned(), "page-text".to_owned()),
        ],
    };
    let response = browser_session_api_response(&page_text_export, &payload);
    assert_eq!(response.status, 200);
    assert_eq!(response.content_type, "text/plain; charset=utf-8");
    assert!(response.body.contains("state"));
    assert_eq!(response.body, payload.page_text);

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
async fn browser_session_registry_opens_resources_by_index() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        for _ in 0..4 {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut buf = [0u8; 4096];
            let read = stream.read(&mut buf).await.unwrap();
            let request = String::from_utf8_lossy(&buf[..read]);
            let request_line = request.lines().next().unwrap_or_default();
            let body = if request_line.contains(" /resource.html ") {
                r#"<!doctype html><title>Resource Target</title><p>resource body</p>"#
            } else {
                r#"<!doctype html><title>Resource Index</title><script src="/resource.html"></script><p>host page</p>"#
            };
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream.write_all(response.as_bytes()).await.unwrap();
        }
    });

    let registry = BrowserSessionRegistry::default();
    let create = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![("url".to_owned(), format!("http://{addr}/index.html"))],
    };
    let (payload, _) = registry.create_target(&create).await.unwrap();
    let original_id = payload.id.clone();
    assert_eq!(payload.title, "Resource Index");
    assert_eq!(payload.resources.len(), 1);
    assert_eq!(payload.resources[0].index, 0);
    assert_eq!(payload.resources[0].kind, "script");
    assert!(payload.resources[0].resolved.ends_with("/resource.html"));
    assert!(payload.resources[0].open_url.contains("action=resource"));
    assert!(
        payload.resources[0]
            .new_session_url
            .contains("action=resource-new-session")
    );
    assert!(
        payload.resources[0]
            .background_session_url
            .contains("action=resource-background-session")
    );

    let resource_background_session = RequestTarget {
        path: "/browser".to_owned(),
        params: form_urlencoded::parse(
            payload.resources[0]
                .background_session_url
                .trim_start_matches("/browser?")
                .as_bytes(),
        )
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect(),
    };
    let (payload, _) = registry
        .apply_target(&resource_background_session)
        .await
        .unwrap();
    assert_eq!(payload.id, original_id);
    assert_eq!(payload.title, "Resource Index");
    assert_eq!(payload.sessions.len(), 2);
    assert!(payload.sessions[0].current);
    assert_eq!(payload.sessions[1].page_title, "Resource Target");

    let resource_new_session = RequestTarget {
        path: "/browser".to_owned(),
        params: form_urlencoded::parse(
            payload.resources[0]
                .new_session_url
                .trim_start_matches("/browser?")
                .as_bytes(),
        )
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect(),
    };
    let (payload, _) = registry.apply_target(&resource_new_session).await.unwrap();
    assert_ne!(payload.id, original_id);
    assert_eq!(payload.title, "Resource Target");
    assert!(payload.source.ends_with("/resource.html"));
    assert_eq!(payload.sessions.len(), 3);
    assert!(
        payload
            .sessions
            .iter()
            .any(|session| session.id == original_id && session.title == "Resource Index")
    );

    let open_original_resource = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![
            ("id".to_owned(), original_id.clone()),
            ("action".to_owned(), "resource".to_owned()),
            ("resource".to_owned(), "0".to_owned()),
        ],
    };
    let (payload, _) = registry
        .apply_target(&open_original_resource)
        .await
        .unwrap();
    assert_eq!(payload.id, original_id);
    assert_eq!(payload.title, "Resource Target");
    assert!(payload.source.ends_with("/resource.html"));
    assert_eq!(payload.history_len, 2);
    assert!(payload.can_back);

    server.await.unwrap();
}

#[tokio::test]
async fn browser_session_registry_opens_resources_in_bulk_sessions() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        for _ in 0..6 {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut buf = [0u8; 4096];
            let read = stream.read(&mut buf).await.unwrap();
            let request = String::from_utf8_lossy(&buf[..read]);
            let request_line = request.lines().next().unwrap_or_default();
            let body = if request_line.contains(" /resource-one.html ") {
                r#"<!doctype html><title>Resource One</title><p>one</p>"#
            } else if request_line.contains(" /resource-two.html ") {
                r#"<!doctype html><title>Resource Two</title><p>two</p>"#
            } else {
                r#"<!doctype html><title>Resource Bulk</title><script src="/resource-one.html"></script><script src="/resource-two.html"></script><p>host page</p>"#
            };
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream.write_all(response.as_bytes()).await.unwrap();
        }
    });

    let registry = BrowserSessionRegistry::default();
    let create = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![("url".to_owned(), format!("http://{addr}/index.html"))],
    };
    let (payload, back_href) = registry.create_target(&create).await.unwrap();
    let original_id = payload.id.clone();
    assert_eq!(payload.title, "Resource Bulk");
    assert_eq!(payload.resources.len(), 2);

    let html = render_browser_session_page(&payload, &back_href);
    assert!(html.contains("Open resources tabs"));
    assert!(html.contains("action=open-resources-new-sessions"));
    assert!(html.contains("Open resources bg"));
    assert!(html.contains("action=open-resources-background-sessions"));

    let state_export = RequestTarget {
        path: "/api/browser-session".to_owned(),
        params: vec![
            ("id".to_owned(), payload.id.clone()),
            ("format".to_owned(), "session-state".to_owned()),
        ],
    };
    let response = browser_session_api_response(&state_export, &payload);
    let exported: serde_json::Value = serde_json::from_str(&response.body).unwrap();
    assert!(
        exported["action_urls"]["open_resources_new_sessions"]
            .as_str()
            .unwrap()
            .contains("action=open-resources-new-sessions")
    );
    assert!(
        exported["action_urls"]["open_resources_background"]
            .as_str()
            .unwrap()
            .contains("action=open-resources-background-sessions")
    );

    let open_resources_new_sessions = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![
            ("id".to_owned(), payload.id),
            (
                "action".to_owned(),
                "open-resources-new-sessions".to_owned(),
            ),
            ("limit".to_owned(), "2".to_owned()),
        ],
    };
    let (payload, _) = registry
        .apply_target(&open_resources_new_sessions)
        .await
        .unwrap();
    assert_ne!(payload.id, original_id);
    assert_eq!(payload.title, "Resource One");
    assert_eq!(payload.sessions.len(), 3);
    assert!(
        payload
            .sessions
            .iter()
            .any(|session| session.id == original_id && session.page_title == "Resource Bulk")
    );
    assert!(
        payload
            .sessions
            .iter()
            .any(|session| session.page_title == "Resource Two")
    );

    let background_registry = BrowserSessionRegistry::default();
    let (payload, _) = background_registry.create_target(&create).await.unwrap();
    let original_id = payload.id.clone();
    let open_resources_background = RequestTarget {
        path: "/browser".to_owned(),
        params: vec![
            ("id".to_owned(), payload.id),
            (
                "action".to_owned(),
                "open-resources-background-sessions".to_owned(),
            ),
            ("limit".to_owned(), "2".to_owned()),
        ],
    };
    let (payload, _) = background_registry
        .apply_target(&open_resources_background)
        .await
        .unwrap();
    assert_eq!(payload.id, original_id);
    assert_eq!(payload.title, "Resource Bulk");
    assert_eq!(payload.sessions.len(), 3);
    assert!(
        payload
            .sessions
            .iter()
            .any(|session| session.current && session.id == original_id)
    );
    assert!(
        payload
            .sessions
            .iter()
            .any(|session| !session.current && session.page_title == "Resource One")
    );
    assert!(
        payload
            .sessions
            .iter()
            .any(|session| !session.current && session.page_title == "Resource Two")
    );

    server.await.unwrap();
}

#[tokio::test]
async fn browser_session_page_renders_form_controls() {
    let dir = tempfile::tempdir().unwrap();
    let page = dir.path().join("form.html");
    std::fs::write(
        &page,
        r#"<!doctype html><title>Form</title><form><input name="q" value="old"><select name="kind"><option value="docs">Docs</option><option value="news" selected>News</option></select><button>Go</button></form>"#,
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
    assert!(html.contains("Forms JSON"));
    assert!(html.contains("format=forms-json"));
    assert!(html.contains("Forms CSV"));
    assert!(html.contains("format=forms-csv"));
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
    assert!(html.contains("action=focus-control"));
    assert!(html.contains("action=activate-control"));
    assert!(html.contains("action=activate-control-new-session"));
    assert!(html.contains(">Focus</a>"));
    assert!(html.contains(">Activate</a>"));
    assert!(html.contains(r#"name="action" value="find""#));
    assert!(html.contains("Find in page"));
    assert!(html.contains("State JSON"));
    assert!(html.contains("State CSV"));
    assert!(html.contains("clear-cookies"));
    assert!(html.contains("localStorage"));
    assert!(html.contains("Resources"));
    assert!(html.contains("action=history"));
    assert!(html.contains(">Open</a>"));
    assert!(html.contains(r#"name="action" value="type-control""#));
    assert!(html.contains(">Type</button>"));
    assert!(html.contains(r#"name="action" value="select""#));
    assert!(html.contains("Choose Docs"));
    assert!(html.contains(r#"name="value" value="old""#));
    assert!(html.contains("Submit form"));
    assert!(html.contains("rust browser session"));

    let state_export = RequestTarget {
        path: "/api/browser-session".to_owned(),
        params: vec![
            ("id".to_owned(), payload.id.clone()),
            ("format".to_owned(), "session-state".to_owned()),
        ],
    };
    let response = browser_session_api_response(&state_export, &payload);
    assert_eq!(response.status, 200);
    let exported: serde_json::Value = serde_json::from_str(&response.body).unwrap();
    assert_eq!(exported["counts"]["forms"], 1);
    assert_eq!(exported["forms"].as_array().unwrap().len(), 1);
    assert_eq!(exported["forms"][0]["index"], 0);
    assert_eq!(exported["forms"][0]["method"], "GET");
    assert!(
        exported["forms"][0]["submit_url"]
            .as_str()
            .unwrap()
            .contains("action=submit")
    );
    assert!(
        exported["forms"][0]["submit_new_session_url"]
            .as_str()
            .unwrap()
            .contains("action=submit-new-session")
    );
    assert!(
        exported["forms"][0]["submit_background_session_url"]
            .as_str()
            .unwrap()
            .contains("action=submit-background-session")
    );
    assert_eq!(
        exported["forms"][0]["controls"].as_array().unwrap().len(),
        3
    );
    assert_eq!(exported["forms"][0]["controls"][0]["name"], "q");
    assert_eq!(exported["forms"][0]["controls"][0]["value"], "old");
    assert!(
        exported["forms"][0]["controls"][0]["fill_url"]
            .as_str()
            .unwrap()
            .contains("action=fill-control")
    );
    assert!(
        exported["forms"][0]["controls"][0]["type_url"]
            .as_str()
            .unwrap()
            .contains("action=type-control")
    );
    assert_eq!(exported["forms"][0]["controls"][1]["kind"], "select");
    assert_eq!(
        exported["forms"][0]["controls"][1]["options"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert!(
        exported["forms"][0]["controls"][1]["options"][0]["select_url"]
            .as_str()
            .unwrap()
            .contains("action=select")
    );
    assert_eq!(exported["forms"][0]["controls"][2]["kind"], "submit");
    assert!(
        exported["forms"][0]["controls"][2]["activate_new_session_url"]
            .as_str()
            .unwrap()
            .contains("action=activate-control-new-session")
    );
    assert!(
        exported["forms"][0]["controls"][2]["activate_background_session_url"]
            .as_str()
            .unwrap()
            .contains("action=activate-control-background-session")
    );

    let forms_json_export = RequestTarget {
        path: "/api/browser-session".to_owned(),
        params: vec![
            ("id".to_owned(), payload.id.clone()),
            ("format".to_owned(), "forms-json".to_owned()),
        ],
    };
    let response = browser_session_api_response(&forms_json_export, &payload);
    assert_eq!(response.status, 200);
    assert_eq!(response.content_type, "application/json; charset=utf-8");
    let exported_forms: serde_json::Value = serde_json::from_str(&response.body).unwrap();
    assert_eq!(exported_forms["format"], "browser-forms");
    assert_eq!(exported_forms["id"], payload.id);
    assert_eq!(exported_forms["form_count"], 1);
    assert_eq!(exported_forms["forms"].as_array().unwrap().len(), 1);
    assert_eq!(exported_forms["forms"][0]["method"], "GET");
    assert_eq!(
        exported_forms["forms"][0]["controls"]
            .as_array()
            .unwrap()
            .len(),
        3
    );
    assert_eq!(exported_forms["forms"][0]["controls"][0]["name"], "q");
    assert_eq!(exported_forms["forms"][0]["controls"][0]["value"], "old");
    assert!(
        exported_forms["forms"][0]["controls"][0]["type_url"]
            .as_str()
            .unwrap()
            .contains("action=type-control")
    );
    assert!(
        exported_forms["csv_url"]
            .as_str()
            .unwrap()
            .contains("format=forms-csv")
    );
    assert!(
        exported_forms["session_state_url"]
            .as_str()
            .unwrap()
            .contains("format=session-state")
    );

    let forms_csv_export = RequestTarget {
        path: "/api/browser-session".to_owned(),
        params: vec![
            ("id".to_owned(), payload.id.clone()),
            ("format".to_owned(), "forms-csv".to_owned()),
        ],
    };
    let response = browser_session_api_response(&forms_csv_export, &payload);
    assert_eq!(response.status, 200);
    assert_eq!(response.content_type, "text/csv; charset=utf-8");
    assert!(response.body.starts_with("form_index,control_index,method,action,resolved_action,control_name,control_kind,value,disabled,required,checked,options,option_select_urls,fill_url,type_url,clear_url,focus_url,activate_url,activate_new_session_url,activate_background_session_url,toggle_url,submit_url,submit_new_session_url,submit_background_session_url,session_id,source\n"));
    assert!(response.body.contains(",q,"));
    assert!(response.body.contains(",old,"));
    assert!(response.body.contains("action=fill-control"));
    assert!(response.body.contains("action=clear-control"));
    assert!(response.body.contains("action=select"));
    assert!(response.body.contains("action=focus-control"));
    assert!(response.body.contains("action=activate-control"));
    assert!(
        response
            .body
            .contains("action=activate-control-new-session")
    );
    assert!(
        response
            .body
            .contains("action=activate-control-background-session")
    );
    assert!(response.body.contains("action=submit"));
    assert!(response.body.contains("action=submit-new-session"));
    assert!(response.body.contains("action=submit-background-session"));
}
