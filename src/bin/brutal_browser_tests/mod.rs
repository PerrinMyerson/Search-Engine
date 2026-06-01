use super::*;

#[test]
fn browser_cli_parser_handles_coordinate_click_aliases() {
    let cli = Cli::try_parse_from(["brutal-browser", "click-at", "page.html", "12", "4"]).unwrap();
    match cli.command {
        Command::ClickAt {
            target,
            x,
            y,
            viewport_x,
            viewport_y,
            width,
            max_bytes,
            json,
            display_list,
        } => {
            assert_eq!(target, "page.html");
            assert_eq!(x, 12);
            assert_eq!(y, 4);
            assert_eq!(viewport_x, 0);
            assert_eq!(viewport_y, 0);
            assert_eq!(width, 100);
            assert_eq!(max_bytes, 4 * 1024 * 1024);
            assert!(!json);
            assert!(!display_list);
        }
        other => panic!("expected coordinate click command, got {other:?}"),
    }

    let cli = Cli::try_parse_from(["brutal-browser", "tap", "page.html", "7", "8"]).unwrap();
    match cli.command {
        Command::ClickAt { x, y, .. } => {
            assert_eq!(x, 7);
            assert_eq!(y, 8);
        }
        other => panic!("expected tap alias to parse as coordinate click, got {other:?}"),
    }

    let cli = Cli::try_parse_from([
        "brutal-browser",
        "click-at",
        "page.html",
        "1",
        "2",
        "--viewport-x",
        "3",
        "--scroll-y",
        "4",
    ])
    .unwrap();
    match cli.command {
        Command::ClickAt {
            x,
            y,
            viewport_x,
            viewport_y,
            ..
        } => {
            assert_eq!(x, 1);
            assert_eq!(y, 2);
            assert_eq!(viewport_x, 3);
            assert_eq!(viewport_y, 4);
        }
        other => panic!("expected coordinate click command, got {other:?}"),
    }
}

#[test]
fn browser_cli_parser_handles_form_submit_command_names() {
    let cli = Cli::try_parse_from(["brutal-browser", "submit", "page.html", "--field", "q=rust"])
        .unwrap();
    match cli.command {
        Command::Submit { target, fields, .. } => {
            assert_eq!(target, "page.html");
            assert_eq!(fields, vec!["q=rust".to_owned()]);
        }
        other => panic!("expected submit command, got {other:?}"),
    }

    let cli = Cli::try_parse_from([
        "brutal-browser",
        "submit-get",
        "page.html",
        "--field",
        "q=rust",
    ])
    .unwrap();
    match cli.command {
        Command::SubmitGet { target, fields, .. } => {
            assert_eq!(target, "page.html");
            assert_eq!(fields, vec!["q=rust".to_owned()]);
        }
        other => panic!("expected submit-get command, got {other:?}"),
    }

    let cli = Cli::try_parse_from([
        "brutal-browser",
        "submit-post",
        "page.html",
        "--field",
        "token=a=b",
    ])
    .unwrap();
    match cli.command {
        Command::SubmitPost { target, fields, .. } => {
            assert_eq!(target, "page.html");
            assert_eq!(fields, vec!["token=a=b".to_owned()]);
        }
        other => panic!("expected submit-post command, got {other:?}"),
    }

    let cli = Cli::try_parse_from(["brutal-browser", "get-form-url", "page.html"]).unwrap();
    match cli.command {
        Command::FormUrl { target, .. } => assert_eq!(target, "page.html"),
        other => panic!("expected form-url command, got {other:?}"),
    }
}

#[test]
fn browser_cli_parser_handles_browse_cookie_jar() {
    let cli = Cli::try_parse_from([
        "brutal-browser",
        "browse",
        "http://example.test",
        "--cookie-jar",
        "profile/cookies.json",
        "--local-storage",
        "profile/local-storage.json",
        "--screenshot-output",
        "profile/final-frame.png",
        "--no-interactive",
    ])
    .unwrap();
    match cli.command {
        Command::Browse {
            target,
            cookie_jar,
            local_storage,
            screenshot_output,
            no_interactive,
            ..
        } => {
            assert_eq!(target, "http://example.test");
            assert_eq!(cookie_jar, Some(PathBuf::from("profile/cookies.json")));
            assert_eq!(
                local_storage,
                Some(PathBuf::from("profile/local-storage.json"))
            );
            assert_eq!(
                screenshot_output,
                Some(PathBuf::from("profile/final-frame.png"))
            );
            assert!(no_interactive);
        }
        other => panic!("expected browse command, got {other:?}"),
    }
}

#[test]
fn browser_cli_parser_handles_app_command() {
    let cli = Cli::try_parse_from([
        "brutal-browser",
        "app",
        "page.html",
        "--viewport-width",
        "40",
        "--viewport-height",
        "12",
        "--scroll-y",
        "3",
        "--cell-width",
        "10",
        "--cell-height",
        "14",
        "--cmd",
        "down 2",
        "--stdin",
        "--no-interactive",
        "--cookie-jar",
        "profile/cookies.json",
        "--local-storage",
        "profile/local-storage.json",
        "--profile",
        "profile/app-profile.json",
        "--output",
        "profile/app-frame.png",
        "--window-output",
        "profile/app-window.png",
        "--json",
    ])
    .unwrap();
    match cli.command {
        Command::App(args) => {
            assert_eq!(args.target, "page.html");
            assert_eq!(args.viewport_width, Some(40));
            assert_eq!(args.viewport_height, 12);
            assert_eq!(args.viewport_y, 3);
            assert_eq!(args.cell_width, 10);
            assert_eq!(args.cell_height, 14);
            assert_eq!(args.commands, vec!["down 2".to_owned()]);
            assert!(args.stdin);
            assert!(args.no_interactive);
            assert_eq!(args.cookie_jar, Some(PathBuf::from("profile/cookies.json")));
            assert_eq!(
                args.local_storage,
                Some(PathBuf::from("profile/local-storage.json"))
            );
            assert_eq!(
                args.profile,
                Some(PathBuf::from("profile/app-profile.json"))
            );
            assert_eq!(args.output, Some(PathBuf::from("profile/app-frame.png")));
            assert_eq!(
                args.window_output,
                Some(PathBuf::from("profile/app-window.png"))
            );
            assert!(args.json);
        }
        other => panic!("expected app command, got {other:?}"),
    }
}

#[test]
fn browser_cli_parser_handles_window_command() {
    let cli = Cli::try_parse_from([
        "brutal-browser",
        "window",
        "page.html",
        "--viewport-width",
        "80",
        "--viewport-height",
        "30",
        "--scroll-y",
        "4",
        "--cell-width",
        "10",
        "--cell-height",
        "14",
        "--cookie-jar",
        "profile/cookies.json",
        "--local-storage",
        "profile/local-storage.json",
    ])
    .unwrap();

    match cli.command {
        Command::Window(args) => {
            assert_eq!(args.target, "page.html");
            assert_eq!(args.viewport_width, Some(80));
            assert_eq!(args.viewport_height, 30);
            assert_eq!(args.viewport_y, 4);
            assert_eq!(args.cell_width, 10);
            assert_eq!(args.cell_height, 14);
            assert_eq!(args.cookie_jar, Some(PathBuf::from("profile/cookies.json")));
            assert_eq!(
                args.local_storage,
                Some(PathBuf::from("profile/local-storage.json"))
            );
        }
        other => panic!("expected window command, got {other:?}"),
    }
}

#[test]
fn browser_cli_parser_handles_screenshot_commands() {
    let cli = Cli::try_parse_from([
        "brutal-browser",
        "screenshot",
        "page.html",
        "--output",
        "page.png",
        "--viewport-width",
        "40",
        "--viewport-height",
        "12",
    ])
    .unwrap();
    match cli.command {
        Command::Screenshot {
            target,
            output,
            viewport_width,
            viewport_height,
            ..
        } => {
            assert_eq!(target, "page.html");
            assert_eq!(output, Some(PathBuf::from("page.png")));
            assert_eq!(viewport_width, Some(40));
            assert_eq!(viewport_height, Some(12));
        }
        other => panic!("expected screenshot command, got {other:?}"),
    }

    let cli = Cli::try_parse_from(["brutal-browser", "screenshot-file", "page.html"]).unwrap();
    match cli.command {
        Command::ScreenshotFile { path, .. } => assert_eq!(path, PathBuf::from("page.html")),
        other => panic!("expected screenshot-file command, got {other:?}"),
    }
}

#[test]
fn browser_shell_parser_handles_navigation_and_clicks() {
    assert_eq!(
        parse_browser_shell_command("open https://example.com").unwrap(),
        BrowserShellCommand::Open("https://example.com".to_owned())
    );
    assert_eq!(
        parse_browser_shell_command("click main nav a.primary").unwrap(),
        BrowserShellCommand::Click("main nav a.primary".to_owned())
    );
    assert_eq!(
        parse_browser_shell_command("reload").unwrap(),
        BrowserShellCommand::Reload
    );
    assert_eq!(
        parse_browser_shell_command("refresh").unwrap(),
        BrowserShellCommand::Reload
    );
    assert_eq!(
        parse_browser_shell_command("location").unwrap(),
        BrowserShellCommand::Location
    );
    assert_eq!(
        parse_browser_shell_command("url").unwrap(),
        BrowserShellCommand::Location
    );
    assert_eq!(
        parse_browser_shell_command("cookies").unwrap(),
        BrowserShellCommand::Cookies
    );
    assert_eq!(
        parse_browser_shell_command("local-storage").unwrap(),
        BrowserShellCommand::LocalStorage
    );
    assert_eq!(
        parse_browser_shell_command("storage").unwrap(),
        BrowserShellCommand::LocalStorage
    );
    assert_eq!(
        parse_browser_shell_command("localstorage").unwrap(),
        BrowserShellCommand::LocalStorage
    );
    assert_eq!(
        parse_browser_shell_command("session-storage").unwrap(),
        BrowserShellCommand::SessionStorage
    );
    assert_eq!(
        parse_browser_shell_command("sessionstorage").unwrap(),
        BrowserShellCommand::SessionStorage
    );
    assert_eq!(
        parse_browser_shell_command("tabs").unwrap(),
        BrowserShellCommand::Tabs
    );
    assert_eq!(
        parse_browser_shell_command("new-tab second.html").unwrap(),
        BrowserShellCommand::NewTab("second.html".to_owned())
    );
    assert_eq!(
        parse_browser_shell_command("open-tab https://example.test").unwrap(),
        BrowserShellCommand::NewTab("https://example.test".to_owned())
    );
    assert_eq!(
        parse_browser_shell_command("switch-tab 2").unwrap(),
        BrowserShellCommand::SwitchTab(2)
    );
    assert_eq!(
        parse_browser_shell_command("close-tab").unwrap(),
        BrowserShellCommand::CloseTab(None)
    );
    assert_eq!(
        parse_browser_shell_command("close-tab 1").unwrap(),
        BrowserShellCommand::CloseTab(Some(1))
    );
    assert_eq!(
        parse_browser_shell_command("clear-cookies").unwrap(),
        BrowserShellCommand::ClearCookies
    );
    assert_eq!(
        parse_browser_shell_command("clear-local-storage").unwrap(),
        BrowserShellCommand::ClearLocalStorage
    );
    assert_eq!(
        parse_browser_shell_command("clear-storage").unwrap(),
        BrowserShellCommand::ClearLocalStorage
    );
    assert_eq!(
        parse_browser_shell_command("clear-session-storage").unwrap(),
        BrowserShellCommand::ClearSessionStorage
    );
    assert_eq!(
        parse_browser_shell_command("render").unwrap(),
        BrowserShellCommand::Render
    );
    assert_eq!(
        parse_browser_shell_command("").unwrap(),
        BrowserShellCommand::Render
    );
    assert_eq!(
        parse_browser_shell_command("links").unwrap(),
        BrowserShellCommand::Links
    );
    assert_eq!(
        parse_browser_shell_command("forms").unwrap(),
        BrowserShellCommand::Forms
    );
    assert_eq!(
        parse_browser_shell_command("form").unwrap(),
        BrowserShellCommand::Forms
    );
    assert_eq!(
        parse_browser_shell_command("link 3").unwrap(),
        BrowserShellCommand::Link(BrowserShellLinkTarget::Index(3))
    );
    assert_eq!(
        parse_browser_shell_command("follow text Second page").unwrap(),
        BrowserShellCommand::Link(BrowserShellLinkTarget::Text("Second page".to_owned()))
    );
    assert_eq!(
        parse_browser_shell_command("activate selector nav a.primary").unwrap(),
        BrowserShellCommand::Link(BrowserShellLinkTarget::Selector("nav a.primary".to_owned()))
    );
    assert_eq!(
        parse_browser_shell_command("focus form input[name=q]").unwrap(),
        BrowserShellCommand::Focus("form input[name=q]".to_owned())
    );
    assert_eq!(
        parse_browser_shell_command("tab").unwrap(),
        BrowserShellCommand::FocusNext
    );
    assert_eq!(
        parse_browser_shell_command("focus-next").unwrap(),
        BrowserShellCommand::FocusNext
    );
    assert_eq!(
        parse_browser_shell_command("shift-tab").unwrap(),
        BrowserShellCommand::FocusPrevious
    );
    assert_eq!(
        parse_browser_shell_command("focus-prev").unwrap(),
        BrowserShellCommand::FocusPrevious
    );
    assert_eq!(
        parse_browser_shell_command("type rust browser").unwrap(),
        BrowserShellCommand::TypeText("rust browser".to_owned())
    );
    assert_eq!(
        parse_browser_shell_command("backspace").unwrap(),
        BrowserShellCommand::DeleteTextBackward(1)
    );
    assert_eq!(
        parse_browser_shell_command("backspace 3").unwrap(),
        BrowserShellCommand::DeleteTextBackward(3)
    );
    assert_eq!(
        parse_browser_shell_command("clear-input").unwrap(),
        BrowserShellCommand::ClearText
    );
    assert_eq!(
        parse_browser_shell_command("enter").unwrap(),
        BrowserShellCommand::SubmitFocused
    );
    assert_eq!(
        parse_browser_shell_command("submit-focused").unwrap(),
        BrowserShellCommand::SubmitFocused
    );
    assert_eq!(
        parse_browser_shell_command("space").unwrap(),
        BrowserShellCommand::ToggleFocused
    );
    assert_eq!(
        parse_browser_shell_command("toggle 0 2").unwrap(),
        BrowserShellCommand::ToggleControl {
            form_index: 0,
            control_index: 2,
        }
    );
    assert_eq!(
        parse_browser_shell_command("choose docs").unwrap(),
        BrowserShellCommand::SelectFocused("docs".to_owned())
    );
    assert_eq!(
        parse_browser_shell_command("select 0 1 docs value").unwrap(),
        BrowserShellCommand::SelectControl {
            form_index: 0,
            control_index: 1,
            value: "docs value".to_owned(),
        }
    );
    assert_eq!(
        parse_browser_shell_command("find target phrase").unwrap(),
        BrowserShellCommand::Find {
            query: "target phrase".to_owned(),
            next: false,
        }
    );
    assert_eq!(
        parse_browser_shell_command("/target phrase").unwrap(),
        BrowserShellCommand::Find {
            query: "target phrase".to_owned(),
            next: false,
        }
    );
    assert_eq!(
        parse_browser_shell_command("find-next target phrase").unwrap(),
        BrowserShellCommand::Find {
            query: "target phrase".to_owned(),
            next: true,
        }
    );
    assert_eq!(
        parse_browser_shell_command("submit 0 q=rust fast=1").unwrap(),
        BrowserShellCommand::Submit {
            mode: BrowserFormSubmitMode::Auto,
            form_index: 0,
            fields: vec![
                ("q".to_owned(), "rust".to_owned()),
                ("fast".to_owned(), "1".to_owned())
            ],
        }
    );
    assert!(parse_browser_shell_command("focus").is_err());
    assert!(parse_browser_shell_command("tab now").is_err());
    assert!(parse_browser_shell_command("shift-tab now").is_err());
    assert!(parse_browser_shell_command("type").is_err());
    assert!(parse_browser_shell_command("backspace nope").is_err());
    assert!(parse_browser_shell_command("clear-input q").is_err());
    assert!(parse_browser_shell_command("enter now").is_err());
    assert!(parse_browser_shell_command("space now").is_err());
    assert!(parse_browser_shell_command("location now").is_err());
    assert!(parse_browser_shell_command("cookies now").is_err());
    assert!(parse_browser_shell_command("local-storage now").is_err());
    assert!(parse_browser_shell_command("session-storage now").is_err());
    assert!(parse_browser_shell_command("tabs now").is_err());
    assert!(parse_browser_shell_command("new-tab").is_err());
    assert!(parse_browser_shell_command("switch-tab").is_err());
    assert!(parse_browser_shell_command("switch-tab one").is_err());
    assert!(parse_browser_shell_command("switch-tab 1 2").is_err());
    assert!(parse_browser_shell_command("close-tab one").is_err());
    assert!(parse_browser_shell_command("close-tab 1 2").is_err());
    assert!(parse_browser_shell_command("clear-cookies now").is_err());
    assert!(parse_browser_shell_command("clear-local-storage now").is_err());
    assert!(parse_browser_shell_command("clear-session-storage now").is_err());
    assert!(parse_browser_shell_command("render now").is_err());
    assert!(parse_browser_shell_command("toggle").is_err());
    assert!(parse_browser_shell_command("toggle 0").is_err());
    assert!(parse_browser_shell_command("toggle x 0").is_err());
    assert!(parse_browser_shell_command("find").is_err());
    assert!(parse_browser_shell_command("/").is_err());
}

#[test]
fn browser_shell_parser_handles_form_fill_commands() {
    assert_eq!(
        parse_browser_shell_command("fill 0 q=rust").unwrap(),
        BrowserShellCommand::Fill {
            form_index: 0,
            name: "q".to_owned(),
            value: "rust".to_owned(),
        }
    );
    assert_eq!(
        parse_browser_shell_command("field 2 token=a=b").unwrap(),
        BrowserShellCommand::Fill {
            form_index: 2,
            name: "token".to_owned(),
            value: "a=b".to_owned(),
        }
    );
    assert_eq!(
        parse_browser_shell_command("submit 0").unwrap(),
        BrowserShellCommand::Submit {
            mode: BrowserFormSubmitMode::Auto,
            form_index: 0,
            fields: Vec::new(),
        }
    );
    assert!(parse_browser_shell_command("fill").is_err());
    assert!(parse_browser_shell_command("fill 0").is_err());
    assert!(parse_browser_shell_command("field 0 =rust").is_err());
    assert!(parse_browser_shell_command("fill x q=rust").is_err());
    assert!(parse_browser_shell_command("fill 0 q=rust fast=1").is_err());
}

#[test]
fn browser_shell_parser_handles_form_submit_command_names() {
    assert_eq!(
        parse_browser_shell_command("submit-form 0 q=rust").unwrap(),
        BrowserShellCommand::Submit {
            mode: BrowserFormSubmitMode::Auto,
            form_index: 0,
            fields: vec![("q".to_owned(), "rust".to_owned())],
        }
    );
    assert_eq!(
        parse_browser_shell_command("submit-get 1 q=rust").unwrap(),
        BrowserShellCommand::Submit {
            mode: BrowserFormSubmitMode::Get,
            form_index: 1,
            fields: vec![("q".to_owned(), "rust".to_owned())],
        }
    );
    assert_eq!(
        parse_browser_shell_command("post-submit 2 token=a=b").unwrap(),
        BrowserShellCommand::Submit {
            mode: BrowserFormSubmitMode::Post,
            form_index: 2,
            fields: vec![("token".to_owned(), "a=b".to_owned())],
        }
    );
    assert!(parse_browser_shell_command("submit-post").is_err());
}

#[test]
fn browser_shell_parser_handles_coordinate_clicks() {
    assert_eq!(
        parse_browser_shell_command("click-at 12 4").unwrap(),
        BrowserShellCommand::ClickAt { x: 12, y: 4 }
    );
    assert_eq!(
        parse_browser_shell_command("tap 7 8").unwrap(),
        BrowserShellCommand::ClickAt { x: 7, y: 8 }
    );
    assert_eq!(
        parse_browser_shell_command("click main nav a.primary").unwrap(),
        BrowserShellCommand::Click("main nav a.primary".to_owned())
    );
    assert!(parse_browser_shell_command("click-at 12").is_err());
    assert!(parse_browser_shell_command("tap 7 8 9").is_err());
    assert!(parse_browser_shell_command("click-at x 4").is_err());
}

#[test]
fn browser_shell_parser_handles_viewport_commands() {
    assert_eq!(
        parse_browser_shell_command("down").unwrap(),
        BrowserShellCommand::Scroll(23)
    );
    assert_eq!(
        parse_browser_shell_command("up 5").unwrap(),
        BrowserShellCommand::Scroll(-5)
    );
    assert_eq!(
        parse_browser_shell_command("scroll -3").unwrap(),
        BrowserShellCommand::Scroll(-3)
    );
    assert_eq!(
        parse_browser_shell_command("right 7").unwrap(),
        BrowserShellCommand::HorizontalScroll(7)
    );
}

#[test]
fn signed_offsets_saturate_at_zero() {
    let mut value = 2usize;
    apply_signed_offset(&mut value, -10);
    assert_eq!(value, 0);
    apply_signed_offset(&mut value, 4);
    assert_eq!(value, 4);
}

#[tokio::test]
async fn browser_shell_link_command_navigates_current_session() {
    let dir = tempfile::tempdir().unwrap();
    let first = dir.path().join("first.html");
    let second = dir.path().join("second.html");
    std::fs::write(
            &first,
            r#"<html><head><title>First</title></head><body><a href="second.html">Second</a></body></html>"#,
        )
        .unwrap();
    std::fs::write(
        &second,
        r#"<html><head><title>Second</title></head><body>Arrived</body></html>"#,
    )
    .unwrap();

    let mut session = BrowserSession::new(BrowserRenderOptions::default());
    session
        .navigate(&first.display().to_string())
        .await
        .unwrap();
    let mut state = BrowserShellState {
        viewport_x: 3,
        viewport_y: 4,
        viewport_width: 80,
        viewport_height: 24,
    };

    let keep_running = apply_browser_shell_command(&mut session, &mut state, 1024, "link 0")
        .await
        .unwrap();

    assert!(keep_running);
    assert_eq!(session.current().unwrap().title, "Second");
    assert_eq!(session.current().unwrap().text, "Arrived");
    assert_eq!(state.viewport_x, 0);
    assert_eq!(state.viewport_y, 0);
}

#[tokio::test]
async fn browser_shell_reload_command_refreshes_current_entry() {
    let dir = tempfile::tempdir().unwrap();
    let page = dir.path().join("page.html");
    std::fs::write(
        &page,
        "<html><head><title>First</title></head><body>one</body></html>",
    )
    .unwrap();

    let mut session = BrowserSession::new(BrowserRenderOptions::default());
    session.navigate(&page.display().to_string()).await.unwrap();
    let mut state = BrowserShellState {
        viewport_x: 5,
        viewport_y: 9,
        viewport_width: 80,
        viewport_height: 24,
    };
    std::fs::write(
        &page,
        "<html><head><title>Reloaded</title></head><body>updated</body></html>",
    )
    .unwrap();

    let keep_running = apply_browser_shell_command(&mut session, &mut state, 1024, "refresh")
        .await
        .unwrap();

    assert!(keep_running);
    assert_eq!(session.current().unwrap().title, "Reloaded");
    assert_eq!(session.current().unwrap().text, "updated");
    assert_eq!(session.snapshot().entries.len(), 1);
    assert_eq!(state.viewport_x, 0);
    assert_eq!(state.viewport_y, 0);
}

#[tokio::test]
async fn browser_shell_location_command_preserves_current_page_and_viewport() {
    let dir = tempfile::tempdir().unwrap();
    let page = dir.path().join("page.html");
    std::fs::write(
        &page,
        r#"<html><head><title>Where</title></head><body>Here</body></html>"#,
    )
    .unwrap();

    let mut session = BrowserSession::new(BrowserRenderOptions::default());
    session.navigate(&page.display().to_string()).await.unwrap();
    let mut state = BrowserShellState {
        viewport_x: 2,
        viewport_y: 3,
        viewport_width: 80,
        viewport_height: 24,
    };

    let keep_running = apply_browser_shell_command(&mut session, &mut state, 1024, "location")
        .await
        .unwrap();

    assert!(keep_running);
    assert_eq!(session.current().unwrap().title, "Where");
    assert_eq!(state.viewport_x, 2);
    assert_eq!(state.viewport_y, 3);
}

#[tokio::test]
async fn browser_shell_cookies_command_preserves_session_state() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut buf = [0u8; 1024];
        let _ = stream.read(&mut buf).await.unwrap();
        let body = "<html><head><title>Cookie</title></head><body>set</body></html>";
        let response = format!(
            "HTTP/1.1 200 OK\r\nSet-Cookie: sid=abc; Path=/; HttpOnly\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        stream.write_all(response.as_bytes()).await.unwrap();
    });

    let mut session = BrowserSession::new(BrowserRenderOptions::default());
    session
        .navigate(&format!("http://{addr}/cookie"))
        .await
        .unwrap();
    server.await.unwrap();
    let mut state = BrowserShellState {
        viewport_x: 2,
        viewport_y: 3,
        viewport_width: 80,
        viewport_height: 24,
    };

    let keep_running = apply_browser_shell_command(&mut session, &mut state, 1024, "cookies")
        .await
        .unwrap();

    assert!(keep_running);
    assert_eq!(session.current().unwrap().title, "Cookie");
    assert_eq!(session.cookies_snapshot()[0].name, "sid");
    assert_eq!(state.viewport_x, 2);
    assert_eq!(state.viewport_y, 3);
}

#[tokio::test]
async fn browser_shell_open_resolves_relative_to_current_page() {
    let dir = tempfile::tempdir().unwrap();
    let start_page = dir.path().join("start.html");
    let next_page = dir.path().join("next.html");
    std::fs::write(
        &start_page,
        "<html><head><title>Start</title></head><body>start</body></html>",
    )
    .unwrap();
    std::fs::write(
        &next_page,
        "<html><head><title>Next</title></head><body>next</body></html>",
    )
    .unwrap();

    let mut session = BrowserSession::new(BrowserRenderOptions::default());
    session
        .navigate(start_page.to_str().unwrap())
        .await
        .unwrap();
    let mut state = BrowserShellState {
        viewport_x: 2,
        viewport_y: 3,
        viewport_width: 80,
        viewport_height: 24,
    };

    let keep_running =
        apply_browser_shell_command(&mut session, &mut state, 1024, "open next.html")
            .await
            .unwrap();

    assert!(keep_running);
    let current = session.current().unwrap();
    assert_eq!(current.title, "Next");
    assert_eq!(current.source, next_page.display().to_string());
    assert_eq!(session.snapshot().entries[1].target, current.source);
    assert_eq!(state.viewport_x, 0);
    assert_eq!(state.viewport_y, 0);

    let keep_running =
        apply_browser_shell_command(&mut session, &mut state, 1024, "open ?mode=debug")
            .await
            .unwrap();

    assert!(keep_running);
    let current = session.current().unwrap();
    assert_eq!(current.title, "Next");
    assert_eq!(
        current.source,
        format!("{}?mode=debug", next_page.display())
    );
}

#[tokio::test]
async fn browser_shell_clear_cookies_command_preserves_page_state() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut buf = [0u8; 1024];
        let _ = stream.read(&mut buf).await.unwrap();
        let body = "<html><head><title>Cookie</title></head><body>set</body></html>";
        let response = format!(
            "HTTP/1.1 200 OK\r\nSet-Cookie: sid=abc; Path=/; HttpOnly\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        stream.write_all(response.as_bytes()).await.unwrap();
    });

    let mut session = BrowserSession::new(BrowserRenderOptions::default());
    session
        .navigate(&format!("http://{addr}/cookie"))
        .await
        .unwrap();
    server.await.unwrap();
    let mut state = BrowserShellState {
        viewport_x: 2,
        viewport_y: 3,
        viewport_width: 80,
        viewport_height: 24,
    };

    assert_eq!(session.cookies_snapshot().len(), 1);
    let keep_running = apply_browser_shell_command(&mut session, &mut state, 1024, "clear-cookies")
        .await
        .unwrap();

    assert!(keep_running);
    assert!(session.cookies_snapshot().is_empty());
    assert_eq!(session.current().unwrap().title, "Cookie");
    assert_eq!(state.viewport_x, 2);
    assert_eq!(state.viewport_y, 3);
}

#[tokio::test]
async fn browser_cookie_jar_file_round_trips_http_cookies() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        for request_index in 0..2 {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut buf = [0u8; 4096];
            let len = stream.read(&mut buf).await.unwrap();
            let request = String::from_utf8_lossy(&buf[..len]);
            let has_cookie = request
                .lines()
                .any(|line| line.to_ascii_lowercase().starts_with("cookie: sid=abc"));
            let body = if request_index == 0 {
                assert!(!has_cookie);
                "<html><head><title>Set</title></head><body>set</body></html>"
            } else {
                assert!(has_cookie);
                "<html><head><title>Check</title></head><body>check</body></html>"
            };
            let set_cookie = if request_index == 0 {
                "Set-Cookie: sid=abc; Path=/; HttpOnly\r\n"
            } else {
                ""
            };
            let response = format!(
                "HTTP/1.1 200 OK\r\n{set_cookie}Content-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream.write_all(response.as_bytes()).await.unwrap();
        }
    });
    let tempdir = tempfile::tempdir().unwrap();
    let cookie_path = tempdir.path().join("profile").join("cookies.json");

    let mut first_session = BrowserSession::new(BrowserRenderOptions::default());
    first_session
        .navigate(&format!("http://{addr}/set"))
        .await
        .unwrap();
    assert_eq!(first_session.cookies_snapshot()[0].name, "sid");
    save_browser_cookie_jar(&cookie_path, &first_session.cookies_snapshot()).unwrap();

    let loaded_cookie_jar = load_browser_cookie_jar(&cookie_path).unwrap();
    let mut second_session =
        BrowserSession::new_with_cookie_jar(BrowserRenderOptions::default(), loaded_cookie_jar);
    second_session
        .navigate(&format!("http://{addr}/check"))
        .await
        .unwrap();

    server.await.unwrap();
    assert_eq!(second_session.current().unwrap().title, "Check");
}

#[tokio::test]
async fn browser_local_storage_file_round_trips_origin_state() {
    let tempdir = tempfile::tempdir().unwrap();
    let set_page = tempdir.path().join("set.html");
    let read_page = tempdir.path().join("read.html");
    let storage_path = tempdir.path().join("profile").join("local-storage.json");
    std::fs::write(
        &set_page,
        r#"
        <html><body>
          <p id="out">Before</p>
          <script>
            localStorage.setItem("headline", "Saved profile state");
            document.getElementById("out").textContent = localStorage.getItem("headline");
          </script>
        </body></html>
        "#,
    )
    .unwrap();
    std::fs::write(
        &read_page,
        r#"
        <html><body>
          <p id="out">Before</p>
          <script>
            document.getElementById("out").textContent = localStorage.getItem("headline");
          </script>
        </body></html>
        "#,
    )
    .unwrap();

    let mut first_session = BrowserSession::new(BrowserRenderOptions::default());
    first_session
        .navigate(&set_page.display().to_string())
        .await
        .unwrap();
    assert_eq!(first_session.current().unwrap().text, "Saved profile state");
    save_browser_local_storage(&storage_path, &first_session.local_storage_snapshot()).unwrap();

    let loaded_local_storage = load_browser_local_storage(&storage_path).unwrap();
    let mut second_session = BrowserSession::new_with_state(
        BrowserRenderOptions::default(),
        BrowserCookieJar::default(),
        loaded_local_storage,
    );
    second_session
        .navigate(&read_page.display().to_string())
        .await
        .unwrap();

    assert_eq!(
        second_session.current().unwrap().text,
        "Saved profile state"
    );
}

#[tokio::test]
async fn browser_shell_clear_local_storage_command_preserves_page_state() {
    let tempdir = tempfile::tempdir().unwrap();
    let set_page = tempdir.path().join("set.html");
    let read_page = tempdir.path().join("read.html");
    std::fs::write(
        &set_page,
        r#"
        <html><head><title>Set Storage</title></head><body>
          <p id="out">Before</p>
          <script>
            localStorage.setItem("headline", "Saved profile state");
            document.getElementById("out").textContent = localStorage.getItem("headline");
          </script>
        </body></html>
        "#,
    )
    .unwrap();
    std::fs::write(
        &read_page,
        r#"
        <html><head><title>Read Storage</title></head><body>
          <p id="out">Before</p>
          <script>
            document.getElementById("out").textContent = localStorage.getItem("headline");
          </script>
        </body></html>
        "#,
    )
    .unwrap();

    let mut session = BrowserSession::new(BrowserRenderOptions::default());
    session
        .navigate(&set_page.display().to_string())
        .await
        .unwrap();
    assert_eq!(session.current().unwrap().text, "Saved profile state");
    let mut state = BrowserShellState {
        viewport_x: 2,
        viewport_y: 3,
        viewport_width: 80,
        viewport_height: 24,
    };

    let keep_running =
        apply_browser_shell_command(&mut session, &mut state, 1024, "clear-local-storage")
            .await
            .unwrap();

    assert!(keep_running);
    assert_eq!(session.current().unwrap().title, "Set Storage");
    assert_eq!(state.viewport_x, 2);
    assert_eq!(state.viewport_y, 3);

    apply_browser_shell_command(&mut session, &mut state, 1024, "open read.html")
        .await
        .unwrap();
    assert_eq!(session.current().unwrap().title, "Read Storage");
    assert!(
        !session
            .current()
            .unwrap()
            .text
            .contains("Saved profile state")
    );
}

#[tokio::test]
async fn browser_shell_local_storage_command_preserves_page_state() {
    let tempdir = tempfile::tempdir().unwrap();
    let page = tempdir.path().join("storage.html");
    std::fs::write(
        &page,
        r#"
        <html><head><title>Storage</title></head><body>
          <p id="out">Before</p>
          <script>
            localStorage.setItem("headline", "Saved profile state");
            localStorage.setItem("mode", "read-only");
            document.getElementById("out").textContent = localStorage.getItem("headline");
          </script>
        </body></html>
        "#,
    )
    .unwrap();

    let mut session = BrowserSession::new(BrowserRenderOptions::default());
    session.navigate(&page.display().to_string()).await.unwrap();
    assert_eq!(session.current().unwrap().text, "Saved profile state");
    let mut state = BrowserShellState {
        viewport_x: 4,
        viewport_y: 5,
        viewport_width: 80,
        viewport_height: 24,
    };

    let keep_running = apply_browser_shell_command(&mut session, &mut state, 1024, "local-storage")
        .await
        .unwrap();

    assert!(keep_running);
    assert_eq!(session.current().unwrap().title, "Storage");
    assert_eq!(state.viewport_x, 4);
    assert_eq!(state.viewport_y, 5);
    assert_eq!(
        session
            .local_storage_entries()
            .iter()
            .map(|entry| (entry.key.as_str(), entry.value.as_str()))
            .collect::<Vec<_>>(),
        vec![("headline", "Saved profile state"), ("mode", "read-only")]
    );
}

#[tokio::test]
async fn browser_shell_session_storage_command_preserves_page_state() {
    let tempdir = tempfile::tempdir().unwrap();
    let page = tempdir.path().join("session-storage.html");
    std::fs::write(
        &page,
        r#"
        <html><head><title>Session Storage</title></head><body>
          <p id="out">Before</p>
          <script>
            sessionStorage.setItem("headline", "Saved session state");
            sessionStorage.setItem("mode", "read-only");
            document.getElementById("out").textContent = sessionStorage.getItem("headline");
          </script>
        </body></html>
        "#,
    )
    .unwrap();

    let mut session = BrowserSession::new(BrowserRenderOptions::default());
    session.navigate(&page.display().to_string()).await.unwrap();
    assert_eq!(session.current().unwrap().text, "Saved session state");
    let mut state = BrowserShellState {
        viewport_x: 4,
        viewport_y: 5,
        viewport_width: 80,
        viewport_height: 24,
    };

    let keep_running =
        apply_browser_shell_command(&mut session, &mut state, 1024, "session-storage")
            .await
            .unwrap();

    assert!(keep_running);
    assert_eq!(session.current().unwrap().title, "Session Storage");
    assert_eq!(state.viewport_x, 4);
    assert_eq!(state.viewport_y, 5);
    assert_eq!(
        session
            .session_storage_entries()
            .iter()
            .map(|entry| (entry.key.as_str(), entry.value.as_str()))
            .collect::<Vec<_>>(),
        vec![("headline", "Saved session state"), ("mode", "read-only")]
    );
}

#[tokio::test]
async fn browser_shell_clear_session_storage_command_preserves_page_state() {
    let tempdir = tempfile::tempdir().unwrap();
    let set_page = tempdir.path().join("set.html");
    let read_page = tempdir.path().join("read.html");
    std::fs::write(
        &set_page,
        r#"
        <html><head><title>Set Session Storage</title></head><body>
          <p id="out">Before</p>
          <script>
            sessionStorage.setItem("headline", "Saved session state");
            document.getElementById("out").textContent = sessionStorage.getItem("headline");
          </script>
        </body></html>
        "#,
    )
    .unwrap();
    std::fs::write(
        &read_page,
        r#"
        <html><head><title>Read Session Storage</title></head><body>
          <p id="out">Before</p>
          <script>
            document.getElementById("out").textContent = sessionStorage.getItem("headline");
          </script>
        </body></html>
        "#,
    )
    .unwrap();

    let mut session = BrowserSession::new(BrowserRenderOptions::default());
    session
        .navigate(&set_page.display().to_string())
        .await
        .unwrap();
    assert_eq!(session.current().unwrap().text, "Saved session state");
    let mut state = BrowserShellState {
        viewport_x: 2,
        viewport_y: 3,
        viewport_width: 80,
        viewport_height: 24,
    };

    let keep_running =
        apply_browser_shell_command(&mut session, &mut state, 1024, "clear-session-storage")
            .await
            .unwrap();

    assert!(keep_running);
    assert_eq!(session.current().unwrap().title, "Set Session Storage");
    assert_eq!(state.viewport_x, 2);
    assert_eq!(state.viewport_y, 3);
    assert!(session.session_storage_entries().is_empty());

    apply_browser_shell_command(&mut session, &mut state, 1024, "open read.html")
        .await
        .unwrap();
    assert_eq!(session.current().unwrap().title, "Read Session Storage");
    assert!(
        !session
            .current()
            .unwrap()
            .text
            .contains("Saved session state")
    );
}

#[tokio::test]
async fn browser_shell_fill_command_updates_form_and_submit_uses_session_state() {
    let dir = tempfile::tempdir().unwrap();
    let form_page = dir.path().join("form.html");
    let results_page = dir.path().join("results.html");
    std::fs::write(
        &form_page,
        r#"
            <html><head><title>Form</title></head><body>
              <form action="results.html" method="get">
                <input name="q" value="old">
                <select id="kind" name="kind">
                  <option value="web" selected>Web</option>
                  <option value="docs">Docs</option>
                </select>
                <input type="checkbox" name="fast" checked>
              </form>
            </body></html>
            "#,
    )
    .unwrap();
    std::fs::write(
        &results_page,
        "<html><head><title>Results</title></head><body>done</body></html>",
    )
    .unwrap();

    let mut session = BrowserSession::new(BrowserRenderOptions::default());
    session
        .navigate(&form_page.display().to_string())
        .await
        .unwrap();
    let mut state = BrowserShellState {
        viewport_x: 3,
        viewport_y: 4,
        viewport_width: 80,
        viewport_height: 24,
    };

    let keep_running =
        apply_browser_shell_command(&mut session, &mut state, 1024, "fill 0 q=rust-browser")
            .await
            .unwrap();

    assert!(keep_running);
    assert_eq!(state.viewport_x, 0);
    assert_eq!(state.viewport_y, 0);
    let filled_form = &session.current().unwrap().forms[0];
    assert!(
        filled_form
            .controls
            .iter()
            .any(|control| control.name == "q" && control.value == "rust-browser")
    );

    apply_browser_shell_command(&mut session, &mut state, 1024, "select 0 1 docs")
        .await
        .unwrap();
    assert_eq!(
        session.current().unwrap().forms[0].controls[1].value,
        "docs"
    );
    assert!(session.current().unwrap().forms[0].controls[1].options[1].selected);

    apply_browser_shell_command(&mut session, &mut state, 1024, "submit 0")
        .await
        .unwrap();

    let current = session.current().unwrap();
    assert_eq!(current.title, "Results");
    assert!(
        current
            .source
            .ends_with("results.html?q=rust-browser&kind=docs&fast=on")
    );
}

#[tokio::test]
async fn browser_shell_focus_and_type_commands_update_active_form_control() {
    let dir = tempfile::tempdir().unwrap();
    let form_page = dir.path().join("form.html");
    let results_page = dir.path().join("results.html");
    std::fs::write(
        &form_page,
        r#"
            <html><head><title>Form</title></head><body>
              <form action="results.html" method="get">
                <label for="q">Query</label>
                <input id="q" name="q" value="rust ">
              </form>
            </body></html>
            "#,
    )
    .unwrap();
    std::fs::write(
        &results_page,
        "<html><head><title>Results</title></head><body>done</body></html>",
    )
    .unwrap();

    let mut session = BrowserSession::new(BrowserRenderOptions::default());
    session
        .navigate(&form_page.display().to_string())
        .await
        .unwrap();
    let mut state = BrowserShellState {
        viewport_x: 3,
        viewport_y: 4,
        viewport_width: 80,
        viewport_height: 24,
    };

    apply_browser_shell_command(&mut session, &mut state, 1024, "focus label[for=q]")
        .await
        .unwrap();
    let keep_running = apply_browser_shell_command(&mut session, &mut state, 1024, "type  browser")
        .await
        .unwrap();

    assert!(keep_running);
    assert_eq!(state.viewport_x, 0);
    assert_eq!(state.viewport_y, 0);
    assert_eq!(session.focused_control().unwrap().value, "rust browser");
    assert_eq!(
        session.current().unwrap().forms[0].controls[0].value,
        "rust browser"
    );

    apply_browser_shell_command(&mut session, &mut state, 1024, "submit 0")
        .await
        .unwrap();
    assert!(
        session
            .current()
            .unwrap()
            .source
            .ends_with("results.html?q=rust+browser")
    );
}

#[tokio::test]
async fn browser_shell_backspace_and_clear_edit_active_form_control() {
    let dir = tempfile::tempdir().unwrap();
    let form_page = dir.path().join("form.html");
    std::fs::write(
        &form_page,
        r#"
            <html><head><title>Form</title></head><body>
              <form>
                <input id="q" name="q" value="rustacean">
              </form>
            </body></html>
            "#,
    )
    .unwrap();

    let mut session = BrowserSession::new(BrowserRenderOptions::default());
    session
        .navigate(&form_page.display().to_string())
        .await
        .unwrap();
    let mut state = BrowserShellState {
        viewport_x: 3,
        viewport_y: 4,
        viewport_width: 80,
        viewport_height: 24,
    };

    apply_browser_shell_command(&mut session, &mut state, 1024, "focus #q")
        .await
        .unwrap();
    apply_browser_shell_command(&mut session, &mut state, 1024, "backspace 5")
        .await
        .unwrap();

    assert_eq!(state.viewport_x, 0);
    assert_eq!(state.viewport_y, 0);
    assert_eq!(session.focused_control().unwrap().value, "rust");
    assert_eq!(
        session.current().unwrap().forms[0].controls[0].value,
        "rust"
    );

    apply_browser_shell_command(&mut session, &mut state, 1024, "clear-input")
        .await
        .unwrap();

    assert_eq!(session.focused_control().unwrap().value, "");
    assert_eq!(session.current().unwrap().forms[0].controls[0].value, "");
}

#[tokio::test]
async fn browser_shell_tab_cycles_active_form_control() {
    let dir = tempfile::tempdir().unwrap();
    let form_page = dir.path().join("form.html");
    std::fs::write(
        &form_page,
        r#"
            <html><head><title>Form</title></head><body>
              <form>
                <input id="q" name="q" value="rust ">
                <input id="fast" type="checkbox" name="fast">
                <select id="kind" name="kind">
                  <option value="web" selected>Web</option>
                  <option value="docs">Docs</option>
                </select>
                <textarea id="notes" name="notes">note </textarea>
              </form>
            </body></html>
            "#,
    )
    .unwrap();

    let mut session = BrowserSession::new(BrowserRenderOptions::default());
    session
        .navigate(&form_page.display().to_string())
        .await
        .unwrap();
    let mut state = BrowserShellState {
        viewport_x: 5,
        viewport_y: 6,
        viewport_width: 80,
        viewport_height: 24,
    };

    apply_browser_shell_command(&mut session, &mut state, 1024, "tab")
        .await
        .unwrap();
    assert_eq!(session.focused_control().unwrap().name, "q");
    apply_browser_shell_command(&mut session, &mut state, 1024, "type browser")
        .await
        .unwrap();
    apply_browser_shell_command(&mut session, &mut state, 1024, "tab")
        .await
        .unwrap();
    assert_eq!(session.focused_control().unwrap().name, "fast");
    apply_browser_shell_command(&mut session, &mut state, 1024, "space")
        .await
        .unwrap();
    apply_browser_shell_command(&mut session, &mut state, 1024, "tab")
        .await
        .unwrap();
    assert_eq!(session.focused_control().unwrap().name, "kind");
    let error = apply_browser_shell_command(&mut session, &mut state, 1024, "type invalid")
        .await
        .unwrap_err();
    assert!(error.to_string().contains("editable text-like control"));
    apply_browser_shell_command(&mut session, &mut state, 1024, "choose docs")
        .await
        .unwrap();
    apply_browser_shell_command(&mut session, &mut state, 1024, "tab")
        .await
        .unwrap();
    assert_eq!(session.focused_control().unwrap().name, "notes");
    apply_browser_shell_command(&mut session, &mut state, 1024, "type memo")
        .await
        .unwrap();
    apply_browser_shell_command(&mut session, &mut state, 1024, "shift-tab")
        .await
        .unwrap();
    assert_eq!(session.focused_control().unwrap().name, "kind");
    apply_browser_shell_command(&mut session, &mut state, 1024, "shift-tab")
        .await
        .unwrap();
    assert_eq!(session.focused_control().unwrap().name, "fast");
    apply_browser_shell_command(&mut session, &mut state, 1024, "shift-tab")
        .await
        .unwrap();

    assert_eq!(state.viewport_x, 0);
    assert_eq!(state.viewport_y, 0);
    assert_eq!(session.focused_control().unwrap().name, "q");
    assert_eq!(
        session.current().unwrap().forms[0].controls[0].value,
        "rust browser"
    );
    assert!(session.current().unwrap().forms[0].controls[1].checked);
    assert_eq!(
        session.current().unwrap().forms[0].controls[2].value,
        "docs"
    );
    assert_eq!(
        session.current().unwrap().forms[0].controls[3].value,
        "note memo"
    );
}

#[tokio::test]
async fn browser_shell_enter_submits_focused_form() {
    let dir = tempfile::tempdir().unwrap();
    let form_page = dir.path().join("form.html");
    let results_page = dir.path().join("results.html");
    std::fs::write(
        &form_page,
        r#"
            <html><head><title>Form</title></head><body>
              <form action="results.html" method="get">
                <input id="q" name="q" value="rust ">
                <button id="go" name="commit" value="yes">Go</button>
              </form>
            </body></html>
            "#,
    )
    .unwrap();
    std::fs::write(
        &results_page,
        "<html><head><title>Results</title></head><body>done</body></html>",
    )
    .unwrap();

    let mut session = BrowserSession::new(BrowserRenderOptions::default());
    session
        .navigate(&form_page.display().to_string())
        .await
        .unwrap();
    let mut state = BrowserShellState {
        viewport_x: 4,
        viewport_y: 7,
        viewport_width: 80,
        viewport_height: 24,
    };

    apply_browser_shell_command(&mut session, &mut state, 1024, "focus #q")
        .await
        .unwrap();
    apply_browser_shell_command(&mut session, &mut state, 1024, "type browser")
        .await
        .unwrap();
    apply_browser_shell_command(&mut session, &mut state, 1024, "focus #go")
        .await
        .unwrap();
    let keep_running = apply_browser_shell_command(&mut session, &mut state, 1024, "enter")
        .await
        .unwrap();

    assert!(keep_running);
    assert_eq!(state.viewport_x, 0);
    assert_eq!(state.viewport_y, 0);
    assert_eq!(session.current().unwrap().title, "Results");
    assert!(
        session
            .current()
            .unwrap()
            .source
            .ends_with("results.html?q=rust+browser&commit=yes")
    );
}

#[tokio::test]
async fn browser_shell_find_command_scrolls_to_matching_text() {
    let dir = tempfile::tempdir().unwrap();
    let page = dir.path().join("page.html");
    std::fs::write(
        &page,
        r#"
        <html><body>
            <p>First screen</p>
            <p>Middle screen</p>
            <p>Target Needle phrase</p>
            <p>Tail screen</p>
        </body></html>
        "#,
    )
    .unwrap();

    let mut session = BrowserSession::new(BrowserRenderOptions::default());
    session.navigate(&page.display().to_string()).await.unwrap();
    let mut state = BrowserShellState {
        viewport_x: 3,
        viewport_y: 0,
        viewport_width: 80,
        viewport_height: 2,
    };

    let keep_running = apply_browser_shell_command(&mut session, &mut state, 1024, "find needle")
        .await
        .unwrap();

    assert!(keep_running);
    assert_eq!(state.viewport_x, 0);
    assert!(state.viewport_y > 0);
    let viewport = current_browser_shell_viewport(&session, state).unwrap();
    assert!(
        viewport
            .lines
            .iter()
            .any(|line| line.contains("Target Needle phrase"))
    );

    apply_browser_shell_command(&mut session, &mut state, 1024, "find-next first screen")
        .await
        .unwrap();

    assert_eq!(state.viewport_y, 0);
}

#[tokio::test]
async fn browser_shell_scroll_dispatches_wheel_and_honors_prevent_default() {
    let dir = tempfile::tempdir().unwrap();
    let page = dir.path().join("wheel.html");
    std::fs::write(
        &page,
        r#"
        <html><body>
            <p>Top</p>
            <p id="out">start</p>
            <script>
              const out = document.getElementById("out");
              document.addEventListener("wheel", event => {
                out.textContent = "wheel:";
                out.textContent += event.deltaX;
                out.textContent += ",";
                out.textContent += event.deltaY;
                event.preventDefault();
              });
            </script>
            <p>Bottom</p>
        </body></html>
        "#,
    )
    .unwrap();

    let mut session = BrowserSession::new(BrowserRenderOptions::default());
    session.navigate(&page.display().to_string()).await.unwrap();
    let mut state = BrowserShellState {
        viewport_x: 0,
        viewport_y: 0,
        viewport_width: 80,
        viewport_height: 2,
    };

    let keep_running = apply_browser_shell_command(&mut session, &mut state, 1024, "scroll 1")
        .await
        .unwrap();

    assert!(keep_running);
    assert_eq!(state.viewport_y, 0);
    assert!(session.current().unwrap().text.contains("wheel:0,1"));

    apply_browser_shell_command(&mut session, &mut state, 1024, "right 2")
        .await
        .unwrap();

    assert_eq!(state.viewport_x, 0);
    assert!(session.current().unwrap().text.contains("wheel:2,0"));
}

#[tokio::test]
async fn browser_shell_forms_command_reports_current_form_controls() {
    let dir = tempfile::tempdir().unwrap();
    let form_page = dir.path().join("form.html");
    std::fs::write(
        &form_page,
        r#"
            <html><head><title>Form</title></head><body>
              <form action="results.html" method="post">
                <input name="q" value="old">
                <select name="kind"><option value="web">Web</option><option selected>Docs</option></select>
                <input type="checkbox" name="remember" checked>
                <input type="submit" name="commit" value="go" disabled>
              </form>
            </body></html>
            "#,
    )
    .unwrap();

    let mut session = BrowserSession::new(BrowserRenderOptions::default());
    session
        .navigate(&form_page.display().to_string())
        .await
        .unwrap();
    let mut state = BrowserShellState {
        viewport_x: 5,
        viewport_y: 9,
        viewport_width: 80,
        viewport_height: 24,
    };

    let keep_running = apply_browser_shell_command(&mut session, &mut state, 1024, "forms")
        .await
        .unwrap();

    assert!(keep_running);
    assert_eq!(state.viewport_x, 5);
    assert_eq!(state.viewport_y, 9);
    let forms = browser_shell_forms(&session);
    assert_eq!(forms.len(), 1);
    assert_eq!(forms[0].index, 0);
    assert_eq!(forms[0].method, "POST");
    assert_eq!(forms[0].action, "results.html");
    assert!(forms[0].resolved_action.ends_with("results.html"));
    assert_eq!(forms[0].controls.len(), 4);
    assert_eq!(forms[0].controls[0].index, 0);
    assert_eq!(forms[0].controls[0].name, "q");
    assert_eq!(forms[0].controls[0].kind, "text");
    assert_eq!(forms[0].controls[0].value, "old");
    assert!(!forms[0].controls[0].disabled);
    assert!(!forms[0].controls[0].checked);
    assert_eq!(forms[0].controls[1].kind, "select");
    assert_eq!(forms[0].controls[1].value, "Docs");
    assert_eq!(forms[0].controls[1].options.len(), 2);
    assert!(forms[0].controls[1].options[1].selected);
    assert_eq!(forms[0].controls[2].kind, "checkbox");
    assert!(forms[0].controls[2].checked);
    assert_eq!(forms[0].controls[3].kind, "submit");
    assert!(forms[0].controls[3].disabled);
}

#[tokio::test]
async fn browser_shell_toggle_command_updates_checkable_form_control() {
    let dir = tempfile::tempdir().unwrap();
    let form_page = dir.path().join("form.html");
    let results_page = dir.path().join("results.html");
    std::fs::write(
        &form_page,
        r#"
            <html><head><title>Form</title></head><body>
              <form action="results.html" method="get">
                <label for="fast">Fast mode</label>
                <input id="fast" type="checkbox" name="fast">
                <input name="q" value="rust">
              </form>
            </body></html>
            "#,
    )
    .unwrap();
    std::fs::write(
        &results_page,
        "<html><head><title>Results</title></head><body>done</body></html>",
    )
    .unwrap();

    let mut session = BrowserSession::new(BrowserRenderOptions::default());
    session
        .navigate(&form_page.display().to_string())
        .await
        .unwrap();
    let mut state = BrowserShellState {
        viewport_x: 3,
        viewport_y: 4,
        viewport_width: 80,
        viewport_height: 24,
    };

    apply_browser_shell_command(&mut session, &mut state, 1024, "click label[for=fast]")
        .await
        .unwrap();

    assert_eq!(state.viewport_x, 0);
    assert_eq!(state.viewport_y, 0);
    assert!(session.current().unwrap().forms[0].controls[0].checked);

    apply_browser_shell_command(&mut session, &mut state, 1024, "toggle 0 0")
        .await
        .unwrap();
    assert!(!session.current().unwrap().forms[0].controls[0].checked);
    apply_browser_shell_command(&mut session, &mut state, 1024, "toggle 0 0")
        .await
        .unwrap();

    apply_browser_shell_command(&mut session, &mut state, 1024, "submit 0")
        .await
        .unwrap();

    assert!(
        session
            .current()
            .unwrap()
            .source
            .ends_with("results.html?fast=on&q=rust")
    );
}

#[tokio::test]
async fn browser_shell_submit_command_posts_method_forms() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        for request_index in 0..2 {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request_bytes = Vec::new();
            let mut buf = [0u8; 4096];
            loop {
                let n = stream.read(&mut buf).await.unwrap();
                assert!(n > 0);
                request_bytes.extend_from_slice(&buf[..n]);
                let Some(header_end) = request_bytes.windows(4).position(|w| w == b"\r\n\r\n")
                else {
                    continue;
                };
                let request_head = String::from_utf8_lossy(&request_bytes[..header_end]);
                let content_length = request_head
                    .lines()
                    .find_map(|line| {
                        let (name, value) = line.split_once(':')?;
                        name.eq_ignore_ascii_case("content-length")
                            .then(|| value.trim().parse::<usize>().ok())
                            .flatten()
                    })
                    .unwrap_or(0);
                if request_bytes.len() >= header_end + 4 + content_length {
                    break;
                }
            }
            let header_end = request_bytes
                .windows(4)
                .position(|w| w == b"\r\n\r\n")
                .unwrap();
            let request_head = String::from_utf8_lossy(&request_bytes[..header_end]);
            let request_body = String::from_utf8_lossy(&request_bytes[header_end + 4..]);
            let first_line = request_head.lines().next().unwrap_or_default();
            let body = if request_index == 0 {
                assert!(first_line.starts_with("GET /form "));
                "<html><head><title>Form</title></head><body><form action=\"/submit\" method=\"post\"><input name=\"q\" value=\"old\"></form></body></html>"
            } else {
                assert!(first_line.starts_with("POST /submit "));
                assert_eq!(request_body, "q=typed");
                "<html><head><title>Posted</title></head><body>accepted</body></html>"
            };
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream.write_all(response.as_bytes()).await.unwrap();
        }
    });

    let mut session = BrowserSession::new(BrowserRenderOptions::default());
    session
        .navigate(&format!("http://{addr}/form"))
        .await
        .unwrap();
    let mut state = BrowserShellState {
        viewport_x: 0,
        viewport_y: 0,
        viewport_width: 80,
        viewport_height: 24,
    };

    apply_browser_shell_command(&mut session, &mut state, 1024, "fill 0 q=typed")
        .await
        .unwrap();
    apply_browser_shell_command(&mut session, &mut state, 1024, "submit 0")
        .await
        .unwrap();
    server.await.unwrap();

    assert_eq!(session.current().unwrap().title, "Posted");
}

#[tokio::test]
async fn browser_shell_link_text_command_navigates_current_session() {
    let dir = tempfile::tempdir().unwrap();
    let first = dir.path().join("first.html");
    let second = dir.path().join("second.html");
    std::fs::write(
            &first,
            r#"<html><head><title>First</title></head><body><a href="second.html">Second target</a></body></html>"#,
        )
        .unwrap();
    std::fs::write(
        &second,
        r#"<html><head><title>Second</title></head><body>Arrived</body></html>"#,
    )
    .unwrap();

    let mut session = BrowserSession::new(BrowserRenderOptions::default());
    session
        .navigate(&first.display().to_string())
        .await
        .unwrap();
    let mut state = BrowserShellState {
        viewport_x: 0,
        viewport_y: 0,
        viewport_width: 80,
        viewport_height: 24,
    };

    apply_browser_shell_command(&mut session, &mut state, 1024, "link text Second target")
        .await
        .unwrap();

    assert_eq!(session.current().unwrap().title, "Second");
}

#[tokio::test]
async fn browser_shell_click_anchor_command_navigates_and_resets_viewport() {
    let dir = tempfile::tempdir().unwrap();
    let first = dir.path().join("first.html");
    let second = dir.path().join("second.html");
    std::fs::write(
            &first,
            r#"<html><head><title>First</title></head><body><p>Intro</p><a id="go" href="second.html">Second</a></body></html>"#,
        )
        .unwrap();
    std::fs::write(
        &second,
        r#"<html><head><title>Second</title></head><body>Arrived</body></html>"#,
    )
    .unwrap();

    let mut session = BrowserSession::new(BrowserRenderOptions::default());
    session
        .navigate(&first.display().to_string())
        .await
        .unwrap();
    let mut state = BrowserShellState {
        viewport_x: 5,
        viewport_y: 9,
        viewport_width: 80,
        viewport_height: 24,
    };

    let keep_running = apply_browser_shell_command(&mut session, &mut state, 1024, "click #go")
        .await
        .unwrap();

    assert!(keep_running);
    assert_eq!(session.current().unwrap().title, "Second");
    assert_eq!(state.viewport_x, 0);
    assert_eq!(state.viewport_y, 0);
    let history = session.snapshot();
    assert_eq!(history.entries.len(), 2);
    assert_eq!(history.current_index, Some(1));
}

#[tokio::test]
async fn browser_shell_fragment_link_scrolls_to_target() {
    let dir = tempfile::tempdir().unwrap();
    let page = dir.path().join("fragments.html");
    std::fs::write(
        &page,
        r##"
            <html><head><title>Fragments</title></head><body>
              <a id="jump" href="#details">Jump</a>
              <p>Intro</p>
              <p>More intro</p>
              <section id="details"><h2>Details</h2></section>
            </body></html>
        "##,
    )
    .unwrap();

    let mut session = BrowserSession::new(BrowserRenderOptions::default());
    session.navigate(&page.display().to_string()).await.unwrap();
    let mut state = BrowserShellState {
        viewport_x: 4,
        viewport_y: 0,
        viewport_width: 80,
        viewport_height: 2,
    };

    apply_browser_shell_command(&mut session, &mut state, 1024, "click #jump")
        .await
        .unwrap();

    assert_eq!(state.viewport_x, 0);
    assert!(state.viewport_y > 0);
    let viewport = current_browser_shell_viewport(&session, state).unwrap();
    assert!(viewport.lines.iter().any(|line| line.contains("Details")));
    assert!(session.current().unwrap().source.ends_with("#details"));
}

#[tokio::test]
async fn browser_shell_click_submit_button_command_submits_form_and_resets_viewport() {
    let dir = tempfile::tempdir().unwrap();
    let form_page = dir.path().join("form.html");
    let results_page = dir.path().join("results.html");
    std::fs::write(
        &form_page,
        r#"
            <html><head><title>Form</title></head><body>
              <form action="results.html" method="get">
                <input name="q" value="old">
                <button id="go" name="commit" value="yes">Go</button>
              </form>
            </body></html>
            "#,
    )
    .unwrap();
    std::fs::write(
        &results_page,
        r#"<html><head><title>Results</title></head><body>Done</body></html>"#,
    )
    .unwrap();

    let mut session = BrowserSession::new(BrowserRenderOptions::default());
    session
        .navigate(&form_page.display().to_string())
        .await
        .unwrap();
    let mut state = BrowserShellState {
        viewport_x: 5,
        viewport_y: 9,
        viewport_width: 80,
        viewport_height: 24,
    };

    apply_browser_shell_command(&mut session, &mut state, 1024, "fill 0 q=typed")
        .await
        .unwrap();
    let keep_running = apply_browser_shell_command(&mut session, &mut state, 1024, "click #go")
        .await
        .unwrap();

    assert!(keep_running);
    let render = session.current().unwrap();
    assert_eq!(render.title, "Results");
    assert!(render.source.ends_with("results.html?q=typed&commit=yes"));
    assert_eq!(state.viewport_x, 0);
    assert_eq!(state.viewport_y, 0);
}

#[tokio::test]
async fn browser_shell_click_reset_button_command_resets_form_and_viewport() {
    let dir = tempfile::tempdir().unwrap();
    let form_page = dir.path().join("form.html");
    std::fs::write(
        &form_page,
        r#"
            <html><head><title>Form</title></head><body>
              <form>
                <input name="q" value="old">
                <button id="reset" type="reset">Reset</button>
              </form>
            </body></html>
            "#,
    )
    .unwrap();

    let mut session = BrowserSession::new(BrowserRenderOptions::default());
    session
        .navigate(&form_page.display().to_string())
        .await
        .unwrap();
    let mut state = BrowserShellState {
        viewport_x: 5,
        viewport_y: 9,
        viewport_width: 80,
        viewport_height: 24,
    };

    apply_browser_shell_command(&mut session, &mut state, 1024, "fill 0 q=typed")
        .await
        .unwrap();
    let keep_running = apply_browser_shell_command(&mut session, &mut state, 1024, "click #reset")
        .await
        .unwrap();

    assert!(keep_running);
    assert_eq!(session.current().unwrap().forms[0].controls[0].value, "old");
    assert_eq!(state.viewport_x, 0);
    assert_eq!(state.viewport_y, 0);
}

#[tokio::test]
async fn browser_shell_click_at_command_navigates_and_resets_viewport() {
    let dir = tempfile::tempdir().unwrap();
    let first = dir.path().join("first.html");
    let second = dir.path().join("second.html");
    std::fs::write(
            &first,
            r#"<html><head><title>First</title></head><body><p>Intro</p><a id="go" href="second.html">Second</a></body></html>"#,
        )
        .unwrap();
    std::fs::write(
        &second,
        r#"<html><head><title>Second</title></head><body>Arrived</body></html>"#,
    )
    .unwrap();

    let mut session = BrowserSession::new(BrowserRenderOptions::default());
    session
        .navigate(&first.display().to_string())
        .await
        .unwrap();
    let mut state = BrowserShellState {
        viewport_x: 0,
        viewport_y: 1,
        viewport_width: 80,
        viewport_height: 24,
    };

    let keep_running = apply_browser_shell_command(&mut session, &mut state, 1024, "click-at 0 0")
        .await
        .unwrap();

    assert!(keep_running);
    assert_eq!(session.current().unwrap().title, "Second");
    assert_eq!(session.current().unwrap().text, "Arrived");
    assert_eq!(state.viewport_x, 0);
    assert_eq!(state.viewport_y, 0);
    let history = session.snapshot();
    assert_eq!(history.entries.len(), 2);
    assert_eq!(history.current_index, Some(1));
}

#[tokio::test]
async fn browser_shell_tap_command_mutates_and_resets_viewport() {
    let dir = tempfile::tempdir().unwrap();
    let first = dir.path().join("first.html");
    let second = dir.path().join("second.html");
    std::fs::write(
            &first,
            r#"
            <html><head><title>First</title></head><body>
              <p>Intro</p>
              <a id="go" href="second.html" onclick="document.querySelector('#out').innerText = 'Stayed'; return false">Go</a>
              <p id="out">Waiting</p>
            </body></html>
            "#,
        )
        .unwrap();
    std::fs::write(
        &second,
        r#"<html><head><title>Second</title></head><body>Arrived</body></html>"#,
    )
    .unwrap();

    let mut session = BrowserSession::new(BrowserRenderOptions::default());
    session
        .navigate(&first.display().to_string())
        .await
        .unwrap();
    let mut state = BrowserShellState {
        viewport_x: 0,
        viewport_y: 1,
        viewport_width: 80,
        viewport_height: 24,
    };

    let keep_running = apply_browser_shell_command(&mut session, &mut state, 1024, "tap 0 0")
        .await
        .unwrap();

    assert!(keep_running);
    assert_eq!(session.current().unwrap().title, "First");
    assert_eq!(session.current().unwrap().text, "Intro\nGo\nStayed");
    assert_eq!(state.viewport_x, 0);
    assert_eq!(state.viewport_y, 0);
    let history = session.snapshot();
    assert_eq!(history.entries.len(), 1);
    assert_eq!(history.current_index, Some(0));
}
