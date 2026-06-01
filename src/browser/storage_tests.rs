use std::fs;

use super::*;

#[test]
fn local_storage_api_feeds_dom_text_and_mutations() {
    let render = render_html(
        "https://example.test/app/page.html",
        br#"
        <html><head><title>Storage</title></head>
        <body>
          <h1 id="out">Before</h1>
          <p id="count">0</p>
          <p id="after">before clear</p>
          <p id="missing">missing</p>
          <script>
            localStorage.setItem("headline", "Stored locally");
            const headline = localStorage.getItem("headline");
            document.getElementById("out").textContent = headline;
            localStorage.setItem("side", "value");
            document.getElementById("count").textContent = localStorage.length;
            localStorage.removeItem("headline");
            document.getElementById("missing").textContent = localStorage.getItem("headline");
            localStorage.clear();
            document.getElementById("after").textContent = localStorage.length;
          </script>
        </body></html>
        "#,
        BrowserRenderOptions::default(),
    );

    assert_eq!(render.text, "Stored locally\n2\n0");
}

#[test]
fn local_storage_is_scoped_by_origin() {
    let mut storage = BrowserLocalStorage::default();
    let options = BrowserRenderOptions::default();

    let set = render_html_prepared(
        "https://a.example.test/app/set.html",
        br#"
        <html><body>
          <p id="out">Before</p>
          <script>
            localStorage.setItem("token", "Origin A");
            document.getElementById("out").textContent = localStorage.getItem("token");
          </script>
        </body></html>
        "#,
        options,
        &[],
        &[],
        None,
        Some(&mut storage),
    )
    .unwrap();
    assert_eq!(set.text, "Origin A");

    let same_origin = render_html_prepared(
        "https://a.example.test/app/read.html",
        br#"
        <html><body>
          <p id="out">Before</p>
          <script>
            document.getElementById("out").textContent = localStorage.getItem("token");
          </script>
        </body></html>
        "#,
        options,
        &[],
        &[],
        None,
        Some(&mut storage),
    )
    .unwrap();
    assert_eq!(same_origin.text, "Origin A");

    let other_origin = render_html_prepared(
        "https://b.example.test/app/read.html",
        br#"
        <html><body>
          <p id="out">Before</p>
          <script>
            document.getElementById("out").textContent = localStorage.getItem("token");
          </script>
        </body></html>
        "#,
        options,
        &[],
        &[],
        None,
        Some(&mut storage),
    )
    .unwrap();
    assert_eq!(other_origin.text, "");
}

#[test]
fn session_storage_api_feeds_dom_text_and_mutations() {
    let render = render_html(
        "https://example.test/app/page.html",
        br#"
        <html><head><title>Session Storage</title></head>
        <body>
          <h1 id="out">Before</h1>
          <p id="count">0</p>
          <p id="after">before clear</p>
          <p id="missing">missing</p>
          <script>
            sessionStorage.setItem("headline", "Stored for session");
            const headline = sessionStorage.getItem("headline");
            document.getElementById("out").textContent = headline;
            sessionStorage.setItem("side", "value");
            document.getElementById("count").textContent = sessionStorage.length;
            sessionStorage.removeItem("headline");
            document.getElementById("missing").textContent = sessionStorage.getItem("headline");
            sessionStorage.clear();
            document.getElementById("after").textContent = sessionStorage.length;
          </script>
        </body></html>
        "#,
        BrowserRenderOptions::default(),
    );

    assert_eq!(render.text, "Stored for session\n2\n0");
}

#[tokio::test]
async fn browser_session_persists_local_storage_across_file_navigations() {
    let dir = tempfile::tempdir().unwrap();
    let set_page = dir.path().join("set.html");
    let read_page = dir.path().join("read.html");
    fs::write(
        &set_page,
        r#"
        <html><body>
          <p id="out">Before</p>
          <script>
            localStorage.setItem("headline", "Persisted headline");
            document.getElementById("out").textContent = localStorage.getItem("headline");
          </script>
        </body></html>
        "#,
    )
    .unwrap();
    fs::write(
        &read_page,
        r#"
        <html><body>
          <p id="out">Before</p>
          <script>
            const headline = localStorage.getItem("headline");
            document.getElementById("out").textContent = headline;
          </script>
        </body></html>
        "#,
    )
    .unwrap();

    let mut session = BrowserSession::new(BrowserRenderOptions::default());
    let first = session
        .navigate(&set_page.display().to_string())
        .await
        .unwrap();
    assert_eq!(first.text, "Persisted headline");
    let second = session
        .navigate(&read_page.display().to_string())
        .await
        .unwrap();
    assert_eq!(second.text, "Persisted headline");
}

#[tokio::test]
async fn browser_session_persists_session_storage_across_file_navigations() {
    let dir = tempfile::tempdir().unwrap();
    let set_page = dir.path().join("set.html");
    let read_page = dir.path().join("read.html");
    fs::write(
        &set_page,
        r#"
        <html><body>
          <p id="out">Before</p>
          <script>
            sessionStorage.setItem("headline", "Session headline");
            document.getElementById("out").textContent = sessionStorage.getItem("headline");
          </script>
        </body></html>
        "#,
    )
    .unwrap();
    fs::write(
        &read_page,
        r#"
        <html><body>
          <p id="out">Before</p>
          <script>
            const headline = sessionStorage.getItem("headline");
            document.getElementById("out").textContent = headline;
          </script>
        </body></html>
        "#,
    )
    .unwrap();

    let mut session = BrowserSession::new(BrowserRenderOptions::default());
    let first = session
        .navigate(&set_page.display().to_string())
        .await
        .unwrap();
    assert_eq!(first.text, "Session headline");
    let second = session
        .navigate(&read_page.display().to_string())
        .await
        .unwrap();
    assert_eq!(second.text, "Session headline");
    assert_eq!(session.session_storage_entries().len(), 1);
}
