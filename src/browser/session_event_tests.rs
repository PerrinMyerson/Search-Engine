use super::*;

#[tokio::test]
async fn browser_session_repeated_clicks_mutate_live_dom_without_reparsing() {
    let dir = tempfile::tempdir().unwrap();
    let page = dir.path().join("clicks.html");
    fs::write(
        &page,
        r#"
            <html><body>
              <button id="go" onclick="document.querySelector('#out').innerText += '!'">Go</button>
              <p id="out">Waiting</p>
            </body></html>
            "#,
    )
    .unwrap();

    let mut session = BrowserSession::new(BrowserRenderOptions::default());
    session.navigate(&page.display().to_string()).await.unwrap();

    assert_eq!(session.click_selector("#go").unwrap().text, "Go\nWaiting!");
    assert_eq!(session.click_selector("#go").unwrap().text, "Go\nWaiting!!");
}

#[tokio::test]
async fn browser_session_event_listeners_survive_repeated_clicks() {
    let dir = tempfile::tempdir().unwrap();
    let page = dir.path().join("listeners.html");
    fs::write(
        &page,
        r#"
            <html><body>
              <button id="go">Go</button>
              <p id="out">Clicks:</p>
              <script>
                document.getElementById("go").addEventListener("click", () => {
                  document.getElementById("out").textContent += "x";
                });
              </script>
            </body></html>
            "#,
    )
    .unwrap();

    let mut session = BrowserSession::new(BrowserRenderOptions::default());
    session.navigate(&page.display().to_string()).await.unwrap();

    assert_eq!(session.click_selector("#go").unwrap().text, "Go\nClicks:x");
    assert_eq!(session.click_selector("#go").unwrap().text, "Go\nClicks:xx");
}

#[tokio::test]
async fn browser_session_once_event_listeners_run_only_once() {
    let dir = tempfile::tempdir().unwrap();
    let page = dir.path().join("once-listeners.html");
    fs::write(
        &page,
        r#"
            <html><body>
              <button id="go">Go</button>
              <p id="out">Events:</p>
              <script>
                const out = document.getElementById("out");
                const go = document.getElementById("go");
                document.addEventListener("click", () => {
                  out.textContent += "capture-once|";
                }, { capture: true, once: true });
                go.addEventListener("click", () => {
                  out.textContent += "target-once|";
                }, { once: true });
                window.addEventListener("click", () => {
                  out.textContent += "window-once|";
                }, { once: true });
                go.addEventListener("click", () => {
                  out.textContent += "normal|";
                });
              </script>
            </body></html>
            "#,
    )
    .unwrap();

    let mut session = BrowserSession::new(BrowserRenderOptions::default());
    session.navigate(&page.display().to_string()).await.unwrap();

    assert_eq!(
        session.click_selector("#go").unwrap().text,
        "Go\nEvents:capture-once|target-once|normal|window-once|"
    );
    assert_eq!(
        session.click_selector("#go").unwrap().text,
        "Go\nEvents:capture-once|target-once|normal|window-once|normal|"
    );
}

#[tokio::test]
async fn browser_session_remove_event_listener_detaches_supported_handlers() {
    let dir = tempfile::tempdir().unwrap();
    let page = dir.path().join("remove-listeners.html");
    fs::write(
        &page,
        r#"
            <html><body>
              <button id="go">Go</button>
              <p id="out">Events:</p>
              <script>
                const out = document.getElementById("out");
                const go = document.getElementById("go");
                const removedTarget = () => {
                  out.textContent += "bad-target|";
                };
                const normalTarget = () => {
                  out.textContent += "normal|";
                };
                const removedCapture = () => {
                  out.textContent += "bad-capture|";
                };
                const removedWindow = function () {
                  out.textContent += "bad-window|";
                };
                go.addEventListener("click", removedTarget);
                go.removeEventListener("click", removedTarget);
                go.addEventListener("click", normalTarget);
                go.removeEventListener("click", normalTarget, true);
                document.addEventListener("click", removedCapture, true);
                document.removeEventListener("click", removedCapture, { capture: true });
                window.addEventListener("click", removedWindow);
                window.removeEventListener("click", removedWindow);
              </script>
            </body></html>
            "#,
    )
    .unwrap();

    let mut session = BrowserSession::new(BrowserRenderOptions::default());
    session.navigate(&page.display().to_string()).await.unwrap();

    assert_eq!(
        session.click_selector("#go").unwrap().text,
        "Go\nEvents:normal|"
    );
    assert_eq!(
        session.click_selector("#go").unwrap().text,
        "Go\nEvents:normal|normal|"
    );
}

#[tokio::test]
async fn browser_session_click_event_exposes_target_and_current_target() {
    let dir = tempfile::tempdir().unwrap();
    let page = dir.path().join("click-event-object.html");
    fs::write(
        &page,
        r#"
            <html><body>
              <section id="outer"><button id="go">Go</button></section>
              <p id="out"></p>
              <script>
                const out = document.getElementById("out");
                const go = document.getElementById("go");
                const outer = document.getElementById("outer");
                go.addEventListener("click", event => {
                  out.textContent += event.type;
                  out.textContent += event.target.id;
                  out.textContent += event.currentTarget.id;
                });
                outer.addEventListener("click", event => {
                  out.textContent += "|";
                  out.textContent += event.type;
                  out.textContent += event.target.id;
                  out.textContent += event.currentTarget.id;
                });
              </script>
            </body></html>
            "#,
    )
    .unwrap();

    let mut session = BrowserSession::new(BrowserRenderOptions::default());
    session.navigate(&page.display().to_string()).await.unwrap();

    let render = session.click_selector("#go").unwrap();
    assert!(render.text.contains("clickgogo|clickgoouter"));
}

#[tokio::test]
async fn browser_session_stop_propagation_blocks_ancestor_listeners() {
    let dir = tempfile::tempdir().unwrap();
    let page = dir.path().join("stop-propagation.html");
    fs::write(
        &page,
        r#"
            <html><body>
              <section id="outer"><button id="go">Go</button></section>
              <p id="out"></p>
              <script>
                const out = document.getElementById("out");
                const go = document.getElementById("go");
                const outer = document.getElementById("outer");
                go.addEventListener("click", event => {
                  out.textContent = "child";
                  event.stopPropagation();
                });
                go.addEventListener("click", () => {
                  out.textContent += "|same-target";
                });
                outer.addEventListener("click", () => {
                  out.textContent += "|bad-ancestor";
                });
              </script>
            </body></html>
            "#,
    )
    .unwrap();

    let mut session = BrowserSession::new(BrowserRenderOptions::default());
    session.navigate(&page.display().to_string()).await.unwrap();

    let render = session.click_selector("#go").unwrap();
    assert!(render.text.contains("child|same-target"));
    assert!(!render.text.contains("bad-ancestor"));
}

#[tokio::test]
async fn browser_session_stop_immediate_propagation_blocks_same_target_listeners() {
    let dir = tempfile::tempdir().unwrap();
    let page = dir.path().join("stop-immediate-propagation.html");
    fs::write(
        &page,
        r#"
            <html><body>
              <section id="outer"><button id="go">Go</button></section>
              <p id="out"></p>
              <script>
                const out = document.getElementById("out");
                const go = document.getElementById("go");
                const outer = document.getElementById("outer");
                go.addEventListener("click", event => {
                  out.textContent = "first";
                  event.stopImmediatePropagation();
                });
                go.addEventListener("click", () => {
                  out.textContent += "|bad-same-target";
                });
                outer.addEventListener("click", () => {
                  out.textContent += "|bad-ancestor";
                });
                window.addEventListener("click", () => {
                  out.textContent += "|bad-window";
                });
              </script>
            </body></html>
            "#,
    )
    .unwrap();

    let mut session = BrowserSession::new(BrowserRenderOptions::default());
    session.navigate(&page.display().to_string()).await.unwrap();

    let render = session.click_selector("#go").unwrap();
    assert!(render.text.contains("first"));
    assert!(!render.text.contains("bad-same-target"));
    assert!(!render.text.contains("bad-ancestor"));
    assert!(!render.text.contains("bad-window"));
}

#[tokio::test]
async fn browser_session_capture_event_listeners_precede_target_and_bubble() {
    let dir = tempfile::tempdir().unwrap();
    let page = dir.path().join("capture-events.html");
    fs::write(
        &page,
        r##"
            <html><body>
              <section id="outer"><button id="go">Go</button></section>
              <p id="out"></p>
              <script>
                const out = document.getElementById("out");
                const go = document.getElementById("go");
                const outer = document.getElementById("outer");
                document.addEventListener("click", event => {
                  out.textContent += "D";
                  out.textContent += event.eventPhase;
                  out.textContent += event.target.id;
                  out.textContent += event.currentTarget.nodeName;
                  out.textContent += "|";
                }, { capture: true });
                outer.addEventListener("click", event => {
                  out.textContent += "O";
                  out.textContent += event.eventPhase;
                  out.textContent += event.target.id;
                  out.textContent += event.currentTarget.id;
                  out.textContent += "|";
                }, true);
                go.addEventListener("click", event => {
                  out.textContent += "T";
                  out.textContent += event.eventPhase;
                  out.textContent += event.target.id;
                  out.textContent += event.currentTarget.id;
                  out.textContent += "|";
                }, { capture: true });
                go.addEventListener("click", event => {
                  out.textContent += "B";
                  out.textContent += event.eventPhase;
                  out.textContent += event.target.id;
                  out.textContent += event.currentTarget.id;
                  out.textContent += "|";
                });
                outer.addEventListener("click", event => {
                  out.textContent += "P";
                  out.textContent += event.eventPhase;
                  out.textContent += event.target.id;
                  out.textContent += event.currentTarget.id;
                  out.textContent += "|";
                });
                document.addEventListener("click", event => {
                  out.textContent += "E";
                  out.textContent += event.eventPhase;
                  out.textContent += event.target.id;
                  out.textContent += event.currentTarget.nodeName;
                  out.textContent += "|";
                });
              </script>
            </body></html>
            "##,
    )
    .unwrap();

    let mut session = BrowserSession::new(BrowserRenderOptions::default());
    session.navigate(&page.display().to_string()).await.unwrap();

    let render = session.click_selector("#go").unwrap();
    assert!(
        render
            .text
            .contains("D1go#document|O1goouter|T2gogo|B2gogo|P3goouter|E3go#document|")
    );
}

#[tokio::test]
async fn browser_session_window_event_target_wraps_document_path() {
    let dir = tempfile::tempdir().unwrap();
    let page = dir.path().join("window-event-target.html");
    fs::write(
        &page,
        r#"
            <html><body>
              <button id="go">Go</button>
              <p id="out"></p>
              <script>
                const out = document.getElementById("out");
                const go = document.getElementById("go");
                window.addEventListener("click", function (event) {
                  out.textContent += "WC";
                  out.textContent += event.eventPhase;
                  out.textContent += event.currentTarget === window;
                  out.textContent += this === window;
                  out.textContent += "|";
                }, true);
                document.addEventListener("click", event => {
                  out.textContent += "DC";
                  out.textContent += event.eventPhase;
                  out.textContent += event.currentTarget === document;
                  out.textContent += "|";
                }, true);
                go.addEventListener("click", event => {
                  out.textContent += "T";
                  out.textContent += event.eventPhase;
                  out.textContent += event.currentTarget.id;
                  out.textContent += "|";
                });
                document.addEventListener("click", event => {
                  out.textContent += "DB";
                  out.textContent += event.eventPhase;
                  out.textContent += event.currentTarget === document;
                  out.textContent += "|";
                });
                window.addEventListener("click", event => {
                  out.textContent += "WB";
                  out.textContent += event.eventPhase;
                  out.textContent += event.currentTarget === window;
                  out.textContent += "|";
                });
              </script>
            </body></html>
            "#,
    )
    .unwrap();

    let mut session = BrowserSession::new(BrowserRenderOptions::default());
    session.navigate(&page.display().to_string()).await.unwrap();

    let render = session.click_selector("#go").unwrap();
    assert!(
        render
            .text
            .contains("WC1truetrue|DC1true|T2go|DB3true|WB3true|")
    );
}

#[tokio::test]
async fn browser_session_stop_propagation_during_capture_blocks_descendants() {
    let dir = tempfile::tempdir().unwrap();
    let page = dir.path().join("capture-stop-propagation.html");
    fs::write(
        &page,
        r#"
            <html><body>
              <section id="outer"><button id="go">Go</button></section>
              <p id="out"></p>
              <script>
                const out = document.getElementById("out");
                const go = document.getElementById("go");
                const outer = document.getElementById("outer");
                outer.addEventListener("click", event => {
                  out.textContent += "outer-capture|";
                  event.stopPropagation();
                }, true);
                outer.addEventListener("click", () => {
                  out.textContent += "outer-same-node|";
                }, true);
                go.addEventListener("click", () => {
                  out.textContent += "bad-target|";
                });
                outer.addEventListener("click", () => {
                  out.textContent += "bad-bubble|";
                });
              </script>
            </body></html>
            "#,
    )
    .unwrap();

    let mut session = BrowserSession::new(BrowserRenderOptions::default());
    session.navigate(&page.display().to_string()).await.unwrap();

    let render = session.click_selector("#go").unwrap();
    assert!(render.text.contains("outer-capture|outer-same-node|"));
    assert!(!render.text.contains("bad-target"));
    assert!(!render.text.contains("bad-bubble"));
}

#[tokio::test]
async fn browser_session_document_event_listeners_receive_bubbled_events() {
    let dir = tempfile::tempdir().unwrap();
    let page = dir.path().join("document-delegated-events.html");
    fs::write(
        &page,
        r#"
            <html><body>
              <button id="go">Go</button>
              <form><input id="q" name="q" value=""></form>
              <p id="out"></p>
              <script>
                const out = document.getElementById("out");
                document.addEventListener("click", event => {
                  out.textContent += event.type;
                  out.textContent += event.target.id;
                  out.textContent += event.currentTarget.nodeName;
                });
                document.addEventListener("keydown", event => {
                  out.textContent += "|";
                  out.textContent += event.type;
                  out.textContent += event.key;
                  out.textContent += event.target.id;
                  out.textContent += event.currentTarget.nodeType;
                });
              </script>
            </body></html>
            "#,
    )
    .unwrap();

    let mut session = BrowserSession::new(BrowserRenderOptions::default());
    session.navigate(&page.display().to_string()).await.unwrap();

    let render = session.click_selector("#go").unwrap();
    assert!(render.text.contains("clickgo#document"));

    session.focus_selector("#q").unwrap();
    let render = session.type_text("r").unwrap();
    assert!(render.text.contains("clickgo#document|keydownrq9"));
}

#[tokio::test]
async fn browser_session_focus_transitions_dispatch_events_and_active_element() {
    let dir = tempfile::tempdir().unwrap();
    let page = dir.path().join("focus-events.html");
    fs::write(
        &page,
        r#"
            <html><body>
              <form id="form">
                <input id="q" name="q" value="">
                <input id="notes" name="notes" value="">
              </form>
              <p id="out"></p>
              <p id="active">none</p>
              <script>
                const form = document.getElementById("form");
                const q = document.getElementById("q");
                const notes = document.getElementById("notes");
                q.addEventListener("focus", () => {
                  document.getElementById("out").textContent += "focus-q|";
                  document.getElementById("active").textContent = document.activeElement.id;
                });
                q.addEventListener("blur", () => {
                  document.getElementById("out").textContent += "blur-q|";
                });
                form.addEventListener("focus", () => {
                  document.getElementById("out").textContent += "bad-bubble|";
                });
                form.addEventListener("focusin", () => {
                  document.getElementById("out").textContent += "focusin|";
                });
                form.addEventListener("focusout", () => {
                  document.getElementById("out").textContent += "focusout|";
                });
                notes.addEventListener("focus", () => {
                  document.getElementById("out").textContent += "focus-notes|";
                  document.getElementById("active").textContent = document.activeElement.id;
                });
              </script>
            </body></html>
            "#,
    )
    .unwrap();

    let mut session = BrowserSession::new(BrowserRenderOptions::default());
    session.navigate(&page.display().to_string()).await.unwrap();

    session.focus_selector("#q").unwrap();
    let text = &session.current().unwrap().text;
    assert!(text.contains("focus-q|focusin|"));
    assert!(text.contains("q"));
    assert!(!text.contains("bad-bubble"));

    session.focus_selector("#notes").unwrap();
    let text = &session.current().unwrap().text;
    assert!(text.contains("focus-q|focusin|blur-q|focusout|focus-notes|focusin|"));
    assert!(text.contains("notes"));
    assert!(!text.contains("bad-bubble"));
}

#[tokio::test]
async fn browser_session_click_default_action_focuses_form_control_before_change() {
    let dir = tempfile::tempdir().unwrap();
    let page = dir.path().join("click-focus-change.html");
    fs::write(
        &page,
        r#"
            <html><body>
              <form><input id="fast" type="checkbox" name="fast"></form>
              <p id="out">state</p>
              <p id="active">none</p>
              <script>
                const fast = document.getElementById("fast");
                fast.addEventListener("focus", () => {
                  document.getElementById("out").textContent = "focus";
                  document.getElementById("active").textContent = document.activeElement.id;
                });
                fast.addEventListener("change", () => {
                  document.getElementById("out").textContent += "|change";
                });
              </script>
            </body></html>
            "#,
    )
    .unwrap();

    let mut session = BrowserSession::new(BrowserRenderOptions::default());
    session.navigate(&page.display().to_string()).await.unwrap();

    let render = session
        .click_selector_with_default_action("#fast")
        .await
        .unwrap();
    assert!(render.text.contains("focus|change"));
    assert!(render.text.contains("fast"));
    assert_eq!(session.focused_control().unwrap().name, "fast");
}

#[tokio::test]
async fn browser_session_text_edits_update_live_dom_and_dispatch_input() {
    let dir = tempfile::tempdir().unwrap();
    let page = dir.path().join("input-events.html");
    fs::write(
        &page,
        r#"
            <html><body>
              <form><input id="q" name="q" value=""></form>
              <button id="read" onclick="document.getElementById('out').textContent = document.getElementById('q').value">Read</button>
              <p id="out">empty</p>
              <script>
                document.getElementById("q").addEventListener("input", () => {
                  document.getElementById("out").textContent = document.getElementById("q").value;
                });
              </script>
            </body></html>
            "#,
    )
    .unwrap();

    let mut session = BrowserSession::new(BrowserRenderOptions::default());
    session.navigate(&page.display().to_string()).await.unwrap();
    session.focus_selector("#q").unwrap();

    assert_eq!(
        session.type_text("rust").unwrap().text,
        "[rust]\nRead\nrust"
    );
    assert_eq!(
        session.click_selector("#read").unwrap().text,
        "[rust]\nRead\nrust"
    );
}

#[tokio::test]
async fn browser_session_text_edit_dispatches_keyboard_events_with_event_object() {
    let dir = tempfile::tempdir().unwrap();
    let page = dir.path().join("keyboard-events.html");
    fs::write(
        &page,
        r#"
            <html><body>
              <form><input id="q" name="q" value=""></form>
              <p id="out"></p>
              <script>
                const q = document.getElementById("q");
                const out = document.getElementById("out");
                q.addEventListener("keydown", event => {
                  out.textContent += event.type;
                  out.textContent += event.key;
                });
                q.addEventListener("input", e => {
                  out.textContent += e.type;
                  out.textContent += e.target.value;
                });
                q.addEventListener("keyup", function (evt) {
                  out.textContent += evt.type;
                  out.textContent += evt.key;
                });
              </script>
            </body></html>
            "#,
    )
    .unwrap();

    let mut session = BrowserSession::new(BrowserRenderOptions::default());
    session.navigate(&page.display().to_string()).await.unwrap();
    session.focus_selector("#q").unwrap();

    let render = session.type_text("ab").unwrap();
    assert!(render.text.contains("[ab]"));
    assert!(
        render
            .text
            .contains("keydownainputakeyupakeydownbinputabkeyupb")
    );

    let render = session.delete_text_backward(1).unwrap();
    assert!(render.text.contains("[a]"));
    assert!(render.text.contains("keydownBackspaceinputakeyupBackspace"));
}

#[tokio::test]
async fn browser_session_keydown_prevent_default_blocks_text_edit() {
    let dir = tempfile::tempdir().unwrap();
    let page = dir.path().join("keyboard-prevent-default.html");
    fs::write(
        &page,
        r#"
            <html><body>
              <form><input id="q" name="q" value=""></form>
              <p id="out"></p>
              <script>
                const q = document.getElementById("q");
                const out = document.getElementById("out");
                q.addEventListener("keydown", event => {
                  out.textContent += event.key;
                  event.preventDefault();
                });
                q.addEventListener("input", () => {
                  out.textContent += "bad-input";
                });
              </script>
            </body></html>
            "#,
    )
    .unwrap();

    let mut session = BrowserSession::new(BrowserRenderOptions::default());
    session.navigate(&page.display().to_string()).await.unwrap();
    session.focus_selector("#q").unwrap();

    let render = session.type_text("x").unwrap();
    assert!(render.text.contains("[]"));
    assert!(render.text.contains("x"));
    assert!(!render.text.contains("bad-input"));
}

#[tokio::test]
async fn browser_session_text_edits_dispatch_beforeinput_before_mutation() {
    let dir = tempfile::tempdir().unwrap();
    let page = dir.path().join("beforeinput-events.html");
    fs::write(
        &page,
        r#"
            <html><body>
              <form><input id="q" name="q" value=""></form>
              <p id="out"></p>
              <script>
                const q = document.getElementById("q");
                const out = document.getElementById("out");
                q.addEventListener("beforeinput", event => {
                  out.textContent += event.type;
                  out.textContent += event.inputType;
                  out.textContent += event.data;
                  out.textContent += event.target.value;
                  out.textContent += "|";
                });
                q.addEventListener("input", event => {
                  out.textContent += event.type;
                  out.textContent += event.target.value;
                  out.textContent += "|";
                });
              </script>
            </body></html>
            "#,
    )
    .unwrap();

    let mut session = BrowserSession::new(BrowserRenderOptions::default());
    session.navigate(&page.display().to_string()).await.unwrap();
    session.focus_selector("#q").unwrap();

    let render = session.type_text("a").unwrap();
    assert!(render.text.contains("[a]"));
    assert!(render.text.contains("beforeinputinsertTexta|inputa|"));

    let render = session.delete_text_backward(1).unwrap();
    assert!(render.text.contains("[]"));
    assert!(
        render
            .text
            .contains("beforeinputdeleteContentBackwarda|input|")
    );
}

#[tokio::test]
async fn browser_session_beforeinput_prevent_default_blocks_text_edit() {
    let dir = tempfile::tempdir().unwrap();
    let page = dir.path().join("beforeinput-prevent-default.html");
    fs::write(
        &page,
        r#"
            <html><body>
              <form><input id="q" name="q" value="a"></form>
              <p id="out"></p>
              <script>
                const q = document.getElementById("q");
                const out = document.getElementById("out");
                q.addEventListener("beforeinput", event => {
                  out.textContent += event.inputType;
                  out.textContent += ":";
                  out.textContent += event.data;
                  out.textContent += "|";
                  event.preventDefault();
                });
                q.addEventListener("input", () => {
                  out.textContent += "bad-input|";
                });
              </script>
            </body></html>
            "#,
    )
    .unwrap();

    let mut session = BrowserSession::new(BrowserRenderOptions::default());
    session.navigate(&page.display().to_string()).await.unwrap();
    session.focus_selector("#q").unwrap();

    let render = session.type_text("x").unwrap();
    assert!(render.text.contains("[a]"));
    assert!(render.text.contains("insertText:x|"));
    assert!(!render.text.contains("bad-input"));

    let render = session.delete_text_backward(1).unwrap();
    assert!(render.text.contains("[a]"));
    assert!(render.text.contains("insertText:x|deleteContentBackward:|"));
    assert!(!render.text.contains("bad-input"));
}

#[tokio::test]
async fn browser_session_coordinate_click_dispatches_pointer_events() {
    let dir = tempfile::tempdir().unwrap();
    let page = dir.path().join("pointer-events.html");
    fs::write(
        &page,
        r#"
            <html><body>
              <button id="go">Go</button>
              <p id="out"></p>
              <script>
                const go = document.getElementById("go");
                const out = document.getElementById("out");
                go.addEventListener("pointerdown", event => {
                  out.textContent += event.type;
                  out.textContent += event.clientX;
                  out.textContent += ",";
                  out.textContent += event.clientY;
                  out.textContent += ":";
                  out.textContent += event.pointerId;
                  out.textContent += ":";
                  out.textContent += event.pointerType;
                  out.textContent += ":";
                  out.textContent += event.button;
                  out.textContent += ":";
                  out.textContent += event.isPrimary;
                  out.textContent += "|";
                });
                document.addEventListener("pointerdown", event => {
                  out.textContent += "doc";
                  out.textContent += event.type;
                  out.textContent += event.target.id;
                  out.textContent += event.currentTarget.nodeName;
                  out.textContent += "|";
                });
                go.addEventListener("mousedown", event => {
                  out.textContent += event.type;
                  out.textContent += event.clientX;
                  out.textContent += ",";
                  out.textContent += event.clientY;
                  out.textContent += ":";
                  out.textContent += event.button;
                  out.textContent += ":";
                  out.textContent += event.pointerType;
                  out.textContent += "|";
                });
                go.addEventListener("pointerup", event => {
                  out.textContent += event.type;
                  out.textContent += event.clientX;
                  out.textContent += ",";
                  out.textContent += event.clientY;
                  out.textContent += "|";
                });
                go.addEventListener("mouseup", event => {
                  out.textContent += event.type;
                  out.textContent += event.clientX;
                  out.textContent += ",";
                  out.textContent += event.clientY;
                  out.textContent += ":";
                  out.textContent += event.button;
                  out.textContent += "|";
                });
                go.addEventListener("click", event => {
                  out.textContent += event.type;
                  out.textContent += event.clientX;
                  out.textContent += ",";
                  out.textContent += event.clientY;
                  out.textContent += ":";
                  out.textContent += event.button;
                  out.textContent += "|";
                });
              </script>
            </body></html>
            "#,
    )
    .unwrap();

    let mut session = BrowserSession::new(BrowserRenderOptions::default());
    session.navigate(&page.display().to_string()).await.unwrap();

    let render = session.click_at_with_default_action(0, 0).await.unwrap();
    assert!(render.text.contains(
        "pointerdown0,0:1:mouse:0:true|docpointerdowngo#document|mousedown0,0:0:|pointerup0,0|mouseup0,0:0|click0,0:0|"
    ));
}

#[tokio::test]
async fn browser_session_submit_event_can_prevent_default_navigation() {
    let dir = tempfile::tempdir().unwrap();
    let page = dir.path().join("submit-prevent.html");
    let results = dir.path().join("results.html");
    fs::write(
        &page,
        r#"
            <html><body>
              <form id="search" action="results.html" method="get">
                <input id="q" name="q" value="rust">
                <button id="go" name="commit" value="yes">Go</button>
              </form>
              <p id="out">waiting</p>
              <script>
                document.getElementById("search").addEventListener("submit", event => {
                  event.preventDefault();
                  document.getElementById("out").textContent = event.type;
                  document.getElementById("out").textContent += event.target.method;
                });
              </script>
            </body></html>
            "#,
    )
    .unwrap();
    fs::write(&results, "<html><body>results</body></html>").unwrap();

    let mut session = BrowserSession::new(BrowserRenderOptions::default());
    session.navigate(&page.display().to_string()).await.unwrap();

    let render = session
        .click_selector_with_default_action("#go")
        .await
        .unwrap();
    assert!(render.source.ends_with("submit-prevent.html"));
    assert!(render.text.contains("submitget"));
}

#[tokio::test]
async fn browser_session_submit_event_mutations_feed_submission() {
    let dir = tempfile::tempdir().unwrap();
    let page = dir.path().join("submit-mutate.html");
    let results = dir.path().join("results.html");
    fs::write(
        &page,
        r#"
            <html><body>
              <form id="search" action="results.html" method="get">
                <input id="q" name="q" value="old">
                <button id="go">Go</button>
              </form>
              <script>
                document.getElementById("search").addEventListener("submit", () => {
                  document.getElementById("q").value = "changed";
                });
              </script>
            </body></html>
            "#,
    )
    .unwrap();
    fs::write(&results, "<html><body>results</body></html>").unwrap();

    let mut session = BrowserSession::new(BrowserRenderOptions::default());
    session.navigate(&page.display().to_string()).await.unwrap();

    let render = session
        .click_selector_with_default_action("#go")
        .await
        .unwrap();
    assert!(render.source.ends_with("results.html?q=changed"));
}

#[tokio::test]
async fn browser_session_reset_event_can_prevent_default_reset() {
    let dir = tempfile::tempdir().unwrap();
    let page = dir.path().join("reset-prevent.html");
    fs::write(
        &page,
        r#"
            <html><body>
              <form id="form">
                <input id="q" name="q" value="old">
                <button id="reset" type="reset">Reset</button>
              </form>
              <p id="out">waiting</p>
              <script>
                document.getElementById("form").addEventListener("reset", event => {
                  event.preventDefault();
                  document.getElementById("out").textContent = event.type;
                });
              </script>
            </body></html>
            "#,
    )
    .unwrap();

    let mut session = BrowserSession::new(BrowserRenderOptions::default());
    session.navigate(&page.display().to_string()).await.unwrap();
    session.focus_selector("#q").unwrap();
    session.type_text(" typed").unwrap();

    let render = session
        .click_selector_with_default_action("#reset")
        .await
        .unwrap();
    assert!(render.text.contains("[old typed]"));
    assert!(render.text.contains("reset"));
}

#[tokio::test]
async fn browser_session_select_and_checkbox_dispatch_change_events() {
    let dir = tempfile::tempdir().unwrap();
    let page = dir.path().join("change-events.html");
    fs::write(
        &page,
        r#"
            <html><body>
              <form>
                <select id="kind" name="kind">
                  <option value="">Pick</option>
                  <option value="docs">Docs</option>
                </select>
                <input id="fast" type="checkbox" name="fast">
              </form>
              <p id="out">state</p>
              <script>
                document.getElementById("kind").addEventListener("change", () => {
                  document.getElementById("out").textContent = document.getElementById("kind").value;
                });
                document.getElementById("fast").addEventListener("change", () => {
                  document.getElementById("out").textContent = document.getElementById("fast").checked;
                });
              </script>
            </body></html>
            "#,
    )
    .unwrap();

    let mut session = BrowserSession::new(BrowserRenderOptions::default());
    session.navigate(&page.display().to_string()).await.unwrap();

    assert_eq!(
        session.select_form_option(0, 0, "docs").unwrap().text,
        "[Docs] [ ]\ndocs"
    );
    assert_eq!(
        session
            .click_selector_with_default_action("#fast")
            .await
            .unwrap()
            .text,
        "[Docs] [x]\ntrue"
    );
}
