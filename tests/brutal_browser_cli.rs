use std::{
    io::Write,
    process::{Command, Stdio},
};

use tempfile::tempdir;

fn run_brutal_browser(args: &[&str]) -> String {
    let output = Command::new(env!("CARGO_BIN_EXE_brutal-browser"))
        .args(args)
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        output.status.success(),
        "status: {}\nstdout: {}\nstderr: {}",
        output.status,
        stdout,
        stderr
    );

    stdout
}

fn run_brutal_browser_with_stdin(args: &[&str], stdin_text: &str) -> String {
    let mut child = Command::new(env!("CARGO_BIN_EXE_brutal-browser"))
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(stdin_text.as_bytes())
        .unwrap();
    let output = child.wait_with_output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        output.status.success(),
        "status: {}\nstdout: {}\nstderr: {}",
        output.status,
        stdout,
        stderr
    );

    stdout
}

fn png_dimensions(bytes: &[u8]) -> (u32, u32) {
    assert!(bytes.starts_with(b"\x89PNG\r\n\x1a\n"));
    assert_eq!(&bytes[12..16], b"IHDR");
    let width = u32::from_be_bytes(bytes[16..20].try_into().unwrap());
    let height = u32::from_be_bytes(bytes[20..24].try_into().unwrap());
    (width, height)
}

#[test]
fn browse_cmd_forms_prints_none_for_formless_page() {
    let stdout = run_brutal_browser(&[
        "browse",
        "bench/browser-fixtures/static-text.html",
        "--cmd",
        "forms",
    ]);

    assert_eq!(stdout.trim(), "forms: none");
}

#[test]
fn browse_cmd_click_then_render_shows_mutated_page() {
    let stdout = run_brutal_browser(&[
        "browse",
        "bench/browser-fixtures/click-event.html",
        "--cmd",
        "click #go",
        "--cmd",
        "render",
        "--no-interactive",
    ]);

    assert!(stdout.contains("# Click Fixture"), "stdout: {stdout}");
    assert!(
        stdout.contains("Done\nClicked by handler"),
        "stdout: {stdout}"
    );
    assert!(!stdout.contains("Waiting"), "stdout: {stdout}");
}

#[test]
fn click_at_cmd_applies_viewport_offsets_to_target_visible_point() {
    let temp = tempdir().unwrap();
    let first = temp.path().join("first.html");
    let second = temp.path().join("second.html");
    std::fs::write(
        &first,
        r#"<html><head><title>First</title></head><body><p>Intro</p><a href="second.html">Second</a></body></html>"#,
    )
    .unwrap();
    std::fs::write(
        &second,
        r#"<html><head><title>Second</title></head><body>Arrived</body></html>"#,
    )
    .unwrap();
    let first_arg = first.to_string_lossy().into_owned();
    let stdout = run_brutal_browser(&[
        "click-at",
        &first_arg,
        "0",
        "0",
        "--viewport-y",
        "1",
        "--json",
    ]);

    assert!(stdout.contains("\"title\": \"Second\""), "stdout: {stdout}");
    assert!(stdout.contains("Arrived"), "stdout: {stdout}");
}

#[test]
fn layout_tree_cmd_reports_retained_document_boxes() {
    let stdout = run_brutal_browser(&[
        "layout-tree",
        "bench/browser-fixtures/max-width-layout.html",
        "--width",
        "40",
    ]);

    assert!(
        stdout.contains("layout_boxes=") && stdout.contains("retained_boxes="),
        "stdout: {stdout}"
    );
    assert!(
        stdout.contains("tag=main kind=block bounds=20x3+10+0"),
        "stdout: {stdout}"
    );
    assert!(stdout.contains("tag=p kind=block"), "stdout: {stdout}");
}

#[test]
fn viewport_cmd_reports_clamped_scroll_and_dirty_region() {
    let stdout = run_brutal_browser(&[
        "viewport",
        "bench/browser-fixtures/max-width-layout.html",
        "--width",
        "40",
        "--viewport-width",
        "40",
        "--viewport-height",
        "2",
        "--viewport-y",
        "99",
        "--previous-x",
        "0",
        "--previous-y",
        "0",
        "--previous-width",
        "40",
        "--previous-height",
        "2",
    ]);

    assert!(
        stdout.contains("current=40x2+0+1 requested=40x2+0+99 max=0+1 document=40x3"),
        "stdout: {stdout}"
    );
    assert!(
        stdout.contains("scroll_delta=0+1") && stdout.contains("invalidated_area=40"),
        "stdout: {stdout}"
    );
    assert!(stdout.contains("dirty: 40x1+0+1"), "stdout: {stdout}");
}

#[test]
fn viewport_frame_cmd_reports_dirty_pixels_and_writes_png() {
    let temp = tempdir().unwrap();
    let png_path = temp.path().join("frame.png");
    let png_arg = png_path.to_string_lossy().into_owned();
    let stdout = run_brutal_browser(&[
        "viewport-frame",
        "bench/browser-fixtures/max-width-layout.html",
        "--width",
        "40",
        "--viewport-width",
        "40",
        "--viewport-height",
        "2",
        "--viewport-y",
        "99",
        "--previous-x",
        "0",
        "--previous-y",
        "0",
        "--previous-width",
        "40",
        "--previous-height",
        "2",
        "--output",
        &png_arg,
    ]);

    assert!(
        stdout.contains("viewport_frame: frame=328x32 viewport=40x2+0+1"),
        "stdout: {stdout}"
    );
    assert!(
        stdout.contains("dirty_pixels=1") && stdout.contains("dirty_pixel_area=3840"),
        "stdout: {stdout}"
    );
    assert!(
        stdout.contains("dirty_pixel: 320x12+4+16 viewport=40x1+0+1"),
        "stdout: {stdout}"
    );

    let bytes = std::fs::read(&png_path).unwrap();
    assert!(bytes.starts_with(b"\x89PNG\r\n\x1a\n"));
    assert_eq!(&bytes[12..16], b"IHDR");
    assert!(bytes.windows(4).any(|chunk| chunk == b"IDAT"));
}

#[test]
fn app_cmd_runs_browser_app_actions_and_writes_frame() {
    let temp = tempdir().unwrap();
    let first = temp.path().join("first.html");
    let second = temp.path().join("second.html");
    let png_path = temp.path().join("app").join("frame.png");
    std::fs::write(
        &first,
        r#"<html><head><title>First App</title></head><body><a href="second.html">Second target</a></body></html>"#,
    )
    .unwrap();
    std::fs::write(
        &second,
        r#"<html><head><title>Second App</title></head><body>Arrived in app shell</body></html>"#,
    )
    .unwrap();
    let first_arg = first.to_string_lossy().into_owned();
    let png_arg = png_path.to_string_lossy().into_owned();
    let stdout = run_brutal_browser(&[
        "app",
        &first_arg,
        "--viewport-width",
        "40",
        "--viewport-height",
        "4",
        "--cmd",
        "link text Second target",
        "--output",
        &png_arg,
        "--json",
    ]);

    assert!(stdout.contains("\"active_tab\": 0"), "stdout: {stdout}");
    assert!(
        stdout.contains("\"title\": \"Second App\""),
        "stdout: {stdout}"
    );
    assert!(
        stdout.contains("\"artifact_format\": \"png-rgba8\""),
        "stdout: {stdout}"
    );
    assert!(
        stdout.contains("\"frame_width\": 328") && stdout.contains("\"frame_height\": 56"),
        "stdout: {stdout}"
    );

    let bytes = std::fs::read(&png_path).unwrap();
    assert!(bytes.starts_with(b"\x89PNG\r\n\x1a\n"));
    assert_eq!(&bytes[12..16], b"IHDR");
    assert!(bytes.windows(4).any(|chunk| chunk == b"IDAT"));
}

#[test]
fn app_cmd_writes_browser_window_frame_with_chrome() {
    let temp = tempdir().unwrap();
    let page = temp.path().join("page.html");
    let page_png = temp.path().join("app").join("page-frame.png");
    let window_png = temp.path().join("app").join("window-frame.png");
    std::fs::write(
        &page,
        r#"<html><head><title>Window App</title></head><body><p>Window frame page</p></body></html>"#,
    )
    .unwrap();
    let page_arg = page.to_string_lossy().into_owned();
    let page_png_arg = page_png.to_string_lossy().into_owned();
    let window_png_arg = window_png.to_string_lossy().into_owned();
    let stdout = run_brutal_browser(&[
        "app",
        &page_arg,
        "--viewport-width",
        "40",
        "--viewport-height",
        "4",
        "--output",
        &page_png_arg,
        "--window-output",
        &window_png_arg,
        "--json",
    ]);

    assert!(
        stdout.contains("\"title\": \"Window App\""),
        "stdout: {stdout}"
    );
    let page_bytes = std::fs::read(&page_png).unwrap();
    let window_bytes = std::fs::read(&window_png).unwrap();
    let (page_width, page_height) = png_dimensions(&page_bytes);
    let (window_width, window_height) = png_dimensions(&window_bytes);
    assert_eq!(window_width, page_width);
    assert!(window_height > page_height);
    assert!(window_bytes.windows(4).any(|chunk| chunk == b"IDAT"));
}

#[test]
fn app_cmd_window_click_routes_page_and_chrome_coordinates() {
    let temp = tempdir().unwrap();
    let first = temp.path().join("first.html");
    let second = temp.path().join("second.html");
    std::fs::write(
        &first,
        r#"<html><head><title>First Window</title></head><body><a href="second.html">Second target</a></body></html>"#,
    )
    .unwrap();
    std::fs::write(
        &second,
        r#"<html><head><title>Second Window</title></head><body>Arrived from window click</body></html>"#,
    )
    .unwrap();
    let first_arg = first.to_string_lossy().into_owned();
    let stdout = run_brutal_browser(&[
        "app",
        &first_arg,
        "--viewport-width",
        "40",
        "--viewport-height",
        "4",
        "--cmd",
        "window-click 5 50",
        "--cmd",
        "window-click 5 6",
        "--json",
    ]);

    assert!(
        stdout.contains("\"title\": \"First Window\""),
        "stdout: {stdout}"
    );
    assert!(stdout.contains("\"history_len\": 2"), "stdout: {stdout}");
}

#[test]
fn app_cmd_reads_stdin_commands_and_updates_frame() {
    let temp = tempdir().unwrap();
    let first = temp.path().join("first.html");
    let second = temp.path().join("second.html");
    let png_path = temp.path().join("app").join("interactive-frame.png");
    std::fs::write(
        &first,
        r#"<html><head><title>First App</title></head><body><a href="second.html">Second target</a></body></html>"#,
    )
    .unwrap();
    std::fs::write(
        &second,
        r#"<html><head><title>Second App</title></head><body>Arrived in app shell</body></html>"#,
    )
    .unwrap();
    let first_arg = first.to_string_lossy().into_owned();
    let png_arg = png_path.to_string_lossy().into_owned();
    let stdout = run_brutal_browser_with_stdin(
        &[
            "app",
            &first_arg,
            "--stdin",
            "--viewport-width",
            "40",
            "--viewport-height",
            "4",
            "--output",
            &png_arg,
        ],
        "links\nlink text Second target\ntabs\nquit\n",
    );

    assert!(stdout.contains("links:"), "stdout: {stdout}");
    assert!(stdout.contains("[0] Second target ->"), "stdout: {stdout}");
    assert!(stdout.contains("# Second App"), "stdout: {stdout}");
    assert!(stdout.contains("Arrived in app shell"), "stdout: {stdout}");
    assert!(stdout.contains("tabs:"), "stdout: {stdout}");
    assert!(stdout.contains("*[0] Second App"), "stdout: {stdout}");

    let bytes = std::fs::read(&png_path).unwrap();
    assert!(bytes.starts_with(b"\x89PNG\r\n\x1a\n"));
    assert_eq!(&bytes[12..16], b"IHDR");
    assert!(bytes.windows(4).any(|chunk| chunk == b"IDAT"));
}

#[test]
fn app_cmd_find_scrolls_and_reports_match_state() {
    let temp = tempdir().unwrap();
    let page = temp.path().join("find.html");
    std::fs::write(
        &page,
        r#"<html><head><title>Find App</title></head><body><p>Alpha</p><p>Beta needle</p><p>Gamma needle</p></body></html>"#,
    )
    .unwrap();
    let page_arg = page.to_string_lossy().into_owned();
    let stdout = run_brutal_browser_with_stdin(
        &[
            "app",
            &page_arg,
            "--stdin",
            "--viewport-width",
            "40",
            "--viewport-height",
            "1",
        ],
        "find needle\nfind-next needle\nquit\n",
    );

    assert!(stdout.contains("find: 1/2"), "stdout: {stdout}");
    assert!(stdout.contains("find: 2/2"), "stdout: {stdout}");
    assert!(stdout.contains("Beta needle"), "stdout: {stdout}");
    assert!(stdout.contains("Gamma needle"), "stdout: {stdout}");
}

#[test]
fn app_cmd_persists_profile_history_and_bookmarks() {
    let temp = tempdir().unwrap();
    let first = temp.path().join("first.html");
    let second = temp.path().join("second.html");
    let profile_path = temp.path().join("profile").join("app-profile.json");
    std::fs::write(
        &first,
        r#"<html><head><title>First App</title></head><body><a href="second.html">Second target</a></body></html>"#,
    )
    .unwrap();
    std::fs::write(
        &second,
        r#"<html><head><title>Second App</title></head><body>Arrived in app shell</body></html>"#,
    )
    .unwrap();
    let first_arg = first.to_string_lossy().into_owned();
    let second_arg = second.to_string_lossy().into_owned();
    let profile_arg = profile_path.to_string_lossy().into_owned();

    let stdout = run_brutal_browser_with_stdin(
        &[
            "app",
            &first_arg,
            "--profile",
            &profile_arg,
            "--stdin",
            "--viewport-width",
            "40",
            "--viewport-height",
            "4",
        ],
        "bookmark\nbookmarks\nlink text Second target\nprofile-history\nbookmark-open 0\nquit\n",
    );

    assert!(
        stdout.contains("bookmark: saved First App"),
        "stdout: {stdout}"
    );
    assert!(stdout.contains("bookmarks:"), "stdout: {stdout}");
    assert!(stdout.contains("[0] First App ->"), "stdout: {stdout}");
    assert!(stdout.contains("profile_history:"), "stdout: {stdout}");
    assert!(stdout.contains("[1] Second App ->"), "stdout: {stdout}");
    assert!(stdout.contains("# First App"), "stdout: {stdout}");

    let profile_json = std::fs::read_to_string(&profile_path).unwrap();
    assert!(profile_json.contains("\"bookmarks\""), "{profile_json}");
    assert!(profile_json.contains("First App"), "{profile_json}");
    assert!(profile_json.contains("Second App"), "{profile_json}");

    let stdout = run_brutal_browser_with_stdin(
        &[
            "app",
            &second_arg,
            "--profile",
            &profile_arg,
            "--stdin",
            "--viewport-width",
            "40",
            "--viewport-height",
            "4",
        ],
        "bookmark-open 0\nquit\n",
    );
    assert!(stdout.contains("# First App"), "stdout: {stdout}");
}

#[test]
fn app_cmd_loads_saves_and_clears_profile_storage() {
    let temp = tempdir().unwrap();
    let set_page = temp.path().join("set.html");
    let read_page = temp.path().join("read.html");
    let storage_path = temp.path().join("profile").join("local-storage.json");
    std::fs::write(
        &set_page,
        r#"<html><body><p id="out">Before</p><script>localStorage.setItem("headline", "Saved profile state"); document.getElementById("out").textContent = localStorage.getItem("headline");</script></body></html>"#,
    )
    .unwrap();
    std::fs::write(
        &read_page,
        r#"<html><body><p id="out">Before</p><script>document.getElementById("out").textContent = localStorage.getItem("headline");</script></body></html>"#,
    )
    .unwrap();
    let set_arg = set_page.to_string_lossy().into_owned();
    let read_arg = read_page.to_string_lossy().into_owned();
    let storage_arg = storage_path.to_string_lossy().into_owned();

    let stdout = run_brutal_browser(&["app", &set_arg, "--local-storage", &storage_arg, "--json"]);
    assert!(stdout.contains("Saved profile state"), "stdout: {stdout}");
    assert!(
        std::fs::read_to_string(&storage_path)
            .unwrap()
            .contains("Saved profile state")
    );

    let stdout = run_brutal_browser(&["app", &read_arg, "--local-storage", &storage_arg, "--json"]);
    assert!(
        stdout.contains("\"text\": \"Saved profile state\""),
        "stdout: {stdout}"
    );

    let stdout = run_brutal_browser(&[
        "app",
        &read_arg,
        "--local-storage",
        &storage_arg,
        "--cmd",
        "clear-local-storage",
        "--json",
    ]);
    assert!(stdout.contains("\"local_storage\": []"), "stdout: {stdout}");
    assert!(
        !std::fs::read_to_string(&storage_path)
            .unwrap()
            .contains("Saved profile state")
    );
}

#[test]
fn screenshot_file_cmd_writes_rgba_png_artifact() {
    let temp = tempdir().unwrap();
    let png_path = temp.path().join("static-text.png");
    let png_arg = png_path.to_string_lossy().into_owned();
    let stdout = run_brutal_browser(&[
        "screenshot-file",
        "bench/browser-fixtures/static-text.html",
        "--output",
        &png_arg,
        "--json",
    ]);

    assert!(
        stdout.contains("\"artifact_format\": \"png-rgba8\""),
        "stdout: {stdout}"
    );
    assert!(
        stdout.contains("\"bytes_per_pixel\": 4"),
        "stdout: {stdout}"
    );

    let bytes = std::fs::read(&png_path).unwrap();
    assert!(bytes.starts_with(b"\x89PNG\r\n\x1a\n"));
    assert_eq!(&bytes[12..16], b"IHDR");
    assert!(bytes.windows(4).any(|chunk| chunk == b"IDAT"));
}

#[test]
fn browse_cmd_json_reports_visual_frame_and_writes_screenshot() {
    let temp = tempdir().unwrap();
    let png_path = temp.path().join("profile").join("final-frame.png");
    let png_arg = png_path.to_string_lossy().into_owned();
    let stdout = run_brutal_browser(&[
        "browse",
        "bench/document-pages/document-article.html",
        "--viewport-width",
        "40",
        "--viewport-height",
        "5",
        "--cmd",
        "down 2",
        "--screenshot-output",
        &png_arg,
        "--json",
    ]);

    assert!(stdout.contains("\"frame\""), "stdout: {stdout}");
    assert!(
        stdout.contains("\"artifact_format\": \"png-rgba8\""),
        "stdout: {stdout}"
    );
    assert!(
        stdout.contains("\"raster_viewport_y\": 2"),
        "stdout: {stdout}"
    );

    let bytes = std::fs::read(&png_path).unwrap();
    assert!(bytes.starts_with(b"\x89PNG\r\n\x1a\n"));
    assert_eq!(&bytes[12..16], b"IHDR");
    assert!(bytes.windows(4).any(|chunk| chunk == b"IDAT"));
}

#[test]
fn browse_cmd_scroll_clamps_to_document_viewport_bounds() {
    let stdout = run_brutal_browser(&[
        "browse",
        "bench/browser-fixtures/max-width-layout.html",
        "--width",
        "40",
        "--viewport-width",
        "40",
        "--viewport-height",
        "2",
        "--cmd",
        "scroll 99",
        "--cmd",
        "location",
        "--no-interactive",
    ]);

    assert!(
        stdout.contains("viewport: x=0 y=1 width=40 height=2 max_x=0 max_y=1 visible_boxes=2/2"),
        "stdout: {stdout}"
    );
}

#[test]
fn browse_cmd_opens_switches_and_closes_tabs() {
    let temp = tempdir().unwrap();
    let first = temp.path().join("first.html");
    let second = temp.path().join("second.html");
    std::fs::write(
        &first,
        r#"<html><head><title>First Tab</title></head><body>first tab</body></html>"#,
    )
    .unwrap();
    std::fs::write(
        &second,
        r#"<html><head><title>Second Tab</title></head><body>second tab</body></html>"#,
    )
    .unwrap();
    let first_arg = first.to_string_lossy().into_owned();

    let stdout = run_brutal_browser(&[
        "browse",
        &first_arg,
        "--cmd",
        "new-tab second.html",
        "--cmd",
        "tabs",
        "--no-interactive",
    ]);

    assert!(stdout.contains(" [0] First Tab"), "stdout: {stdout}");
    assert!(stdout.contains("*[1] Second Tab"), "stdout: {stdout}");

    let stdout = run_brutal_browser(&[
        "browse",
        &first_arg,
        "--cmd",
        "new-tab second.html",
        "--cmd",
        "switch-tab 0",
        "--cmd",
        "render",
        "--no-interactive",
    ]);

    assert!(stdout.contains("# First Tab"), "stdout: {stdout}");
    assert!(stdout.contains("first tab"), "stdout: {stdout}");

    let stdout = run_brutal_browser(&[
        "browse",
        &first_arg,
        "--cmd",
        "new-tab second.html",
        "--cmd",
        "close-tab",
        "--cmd",
        "tabs",
        "--no-interactive",
    ]);

    assert!(stdout.contains("*[0] First Tab"), "stdout: {stdout}");
    assert!(!stdout.contains("Second Tab"), "stdout: {stdout}");
}
