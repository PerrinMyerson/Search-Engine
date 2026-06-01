use super::*;

#[test]
fn executes_inline_script_dom_text_assignments() {
    let render = render_html(
        "mem://page",
        br#"
        <html><head><title>Before</title></head>
        <body>
          <h1 id="app">Before</h1>
          <p>Static</p>
          <script>
            document.title = "After";
            document.getElementById("app").textContent = "Hydrated";
            document.querySelector("p").innerText += " plus";
          </script>
        </body></html>
        "#,
        BrowserRenderOptions::default(),
    );

    assert_eq!(render.title, "After");
    assert_eq!(render.text, "Hydrated\nStatic plus");
    assert!(!render.text.contains("Before"));
}

#[test]
fn location_readback_properties_feed_dom_text() {
    let render = render_html(
        "https://example.com:8443/app/page.html?q=rust#details",
        br#"
        <html><head><title>Location</title></head>
        <body>
          <p id="state">Before</p>
          <script>
            const state = document.getElementById("state");
            state.textContent = location.href;
            state.textContent += " ";
            state.textContent += window.location.protocol;
            state.textContent += " ";
            state.textContent += document.location.host;
            state.textContent += " ";
            state.textContent += location.hostname;
            state.textContent += " ";
            state.textContent += location.port;
            state.textContent += " ";
            state.textContent += location.pathname;
            state.textContent += " ";
            state.textContent += location.search;
            state.textContent += " ";
            state.textContent += location.hash;
            state.textContent += " ";
            state.textContent += location.origin;
            state.textContent += " ";
            state.textContent += document.URL;
            state.textContent += " ";
            state.textContent += window.location.toString();
          </script>
        </body></html>
        "#,
        BrowserRenderOptions {
            width: 320,
            ..BrowserRenderOptions::default()
        },
    );

    assert_eq!(
        render.text,
        "https://example.com:8443/app/page.html?q=rust#details https: example.com:8443 example.com 8443 /app/page.html ?q=rust #details https://example.com:8443 https://example.com:8443/app/page.html?q=rust#details https://example.com:8443/app/page.html?q=rust#details"
    );
}

#[test]
fn executes_inline_script_create_and_append_child() {
    let render = render_html(
        "mem://page",
        br#"
        <html><head><title>Create</title></head>
        <body>
          <main id="app"></main>
          <script>
            const card = document.createElement("article");
            card.id = "created";
            card.className = "result featured";
            card.textContent = "Created by script";
            document.getElementById("app").appendChild(card);
            const tail = document.createTextNode("Tail text");
            document.body.appendChild(tail);
            const link = document.createElement("a");
            link.href = "next.html";
            link.textContent = "Runtime Link";
            document.body.appendChild(link);
          </script>
        </body></html>
        "#,
        BrowserRenderOptions::default(),
    );

    assert_eq!(render.text, "Created by script\nTail text Runtime Link");
    assert_eq!(render.links.len(), 1);
    assert_eq!(render.links[0].text, "Runtime Link");
    assert_eq!(render.links[0].href, "next.html");
}

#[test]
fn tree_mutation_methods_update_rendered_dom_order() {
    let render = render_html(
        "mem://page",
        br#"
        <html><body>
          <ul id="list">
            <li id="stale">Stale</li>
            <li id="anchor">Anchor</li>
            <li id="remove-me">Remove me</li>
          </ul>
          <p id="state">Before</p>
          <script>
            const list = document.getElementById("list");
            const first = document.createElement("li");
            first.textContent = "First";
            list.insertBefore(first, document.getElementById("anchor"));
            const replacement = document.createElement("li");
            replacement.textContent = "Replaced";
            list.replaceChild(replacement, document.getElementById("stale"));
            document.getElementById("remove-me").remove();
            const anchor = document.getElementById("anchor");
            anchor.parentNode.removeChild(anchor);
            const tail = document.createElement("li");
            tail.textContent = "Tail";
            list.insertBefore(tail, null);
            document.getElementById("state").textContent = list.querySelectorAll("li").length;
          </script>
        </body></html>
        "#,
        BrowserRenderOptions::default(),
    );

    assert_eq!(render.text, "- Replaced\n- First\n- Tail\n3");
}

#[test]
fn child_and_sibling_traversal_reads_live_dom_order() {
    let render = render_html(
        "mem://page",
        br#"
        <html><body>
          <main id="root"><section id="first">Alpha</section><section id="second">Beta</section><aside id="third">Gamma</aside></main>
          <p id="state">Before</p>
          <script>
            const root = document.getElementById("root");
            root.firstElementChild.textContent = "Alpha one";
            root.firstElementChild.nextElementSibling.textContent += " two";
            root.lastElementChild.textContent = "Gamma three";
            const state = document.getElementById("state");
            state.textContent = root.children.length;
            state.textContent += " ";
            state.textContent += root.childElementCount;
            state.textContent += " ";
            state.textContent += root.children.item(1).textContent;
            state.textContent += " ";
            state.textContent += root.childNodes[0].textContent;
            state.textContent += " ";
            state.textContent += root.lastChild.previousElementSibling.id;
            state.textContent += " ";
            state.textContent += root.firstChild.nodeType;
          </script>
        </body></html>
        "#,
        BrowserRenderOptions::default(),
    );

    assert_eq!(
        render.text,
        "Alpha one\nBeta two\nGamma three\n3 3 Beta two Alpha one second 1"
    );
}

#[test]
fn dom_insertion_convenience_methods_update_tree_and_text_nodes() {
    let render = render_html(
        "mem://page",
        br#"
        <html><body>
          <main id="root">
            <p id="anchor">Anchor</p>
            <p id="old">Old</p>
          </main>
          <p id="state">Before</p>
          <script>
            const root = document.getElementById("root");
            const anchor = document.getElementById("anchor");
            const before = document.createElement("p");
            before.textContent = "Before";
            anchor.before(before);
            anchor.prepend("Lead ");
            anchor.append(" Tail");
            const after = document.createElement("p");
            after.textContent = "After";
            anchor.after(after);
            const replacement = document.createElement("p");
            replacement.textContent = "Replacement";
            document.getElementById("old").replaceWith(replacement);
            const extra = document.createElement("p");
            extra.textContent = "Extra";
            root.append(extra);
            root.prepend(document.createTextNode("Start"));
            document.getElementById("state").replaceChildren("Children ", document.createTextNode("reset"));
          </script>
        </body></html>
        "#,
        BrowserRenderOptions::default(),
    );

    assert_eq!(
        render.text,
        "Start\nBefore\nLead Anchor Tail\nAfter\nReplacement\nExtra\nChildren reset"
    );
}

#[test]
fn document_fragment_children_splice_into_insert_targets() {
    let render = render_html(
        "mem://page",
        br#"
        <html><body>
          <main id="root"><p id="stale">Stale</p></main>
          <p id="state">Before</p>
          <script>
            const fragment = document.createDocumentFragment();
            const first = document.createElement("p");
            first.textContent = "First";
            const second = document.createElement("p");
            second.textContent = "Second";
            fragment.appendChild(first);
            fragment.append(second);
            document.getElementById("root").replaceChildren(fragment);
            const state = document.getElementById("state");
            state.textContent = fragment.childNodes.length;
            state.textContent += " ";
            state.textContent += fragment.nodeType;
            state.textContent += " ";
            state.textContent += document.getElementById("root").children.length;
            state.textContent += " ";
            state.textContent += document.getElementById("root").lastElementChild.nodeName;
          </script>
        </body></html>
        "#,
        BrowserRenderOptions::default(),
    );

    assert_eq!(render.text, "First\nSecond\n0 11 2 P");
}

#[test]
fn set_attribute_updates_dom_extraction_and_layout() {
    let render = render_html(
        "https://example.test/docs/page.html",
        br#"
        <html><head>
          <title>Attributes</title>
          <style>.gone { display: none; }</style>
        </head>
        <body>
          <a id="runtime">Before</a>
          <p id="hidden">Hidden after script</p>
          <script>
            const link = document.getElementById("runtime");
            link.setAttribute("href", "runtime.html");
            link.setAttribute("class", "primary result");
            link.textContent = "Runtime Link";
            document.getElementById("hidden").setAttribute("class", "gone");
            const image = document.createElement("img");
            image.setAttribute("src", "hero.png");
            image.setAttribute("alt", "Hero");
            document.body.appendChild(image);
          </script>
        </body></html>
        "#,
        BrowserRenderOptions::default(),
    );

    assert_eq!(render.text, "Runtime Link");
    assert_eq!(render.links.len(), 1);
    assert_eq!(render.links[0].href, "runtime.html");
    assert_eq!(
        render.links[0].resolved,
        "https://example.test/docs/runtime.html"
    );
    assert_eq!(render.resources.len(), 1);
    assert_eq!(render.resources[0].kind, "image");
    assert_eq!(render.resources[0].url, "hero.png");
    assert_eq!(
        render.resources[0].resolved,
        "https://example.test/docs/hero.png"
    );
    assert_eq!(render.resources[0].alt.as_deref(), Some("Hero"));
}

#[test]
fn get_attribute_and_string_bindings_feed_text_and_attributes() {
    let render = render_html(
        "https://example.test/docs/page.html",
        br#"
        <html><head><title>Read Attributes</title></head>
        <body>
          <a id="runtime" href="initial.html" data-role="primary">Before</a>
          <script>
            const link = document.getElementById("runtime");
            const hrefName = "href";
            const roleName = "data-role";
            const href = link.getAttribute(hrefName);
            const role = link.getAttribute(roleName);
            link.textContent = href;
            link.textContent += " ";
            link.textContent += role;
            const clone = document.createElement("a");
            clone.setAttribute(hrefName, href);
            clone.textContent = role;
            document.body.appendChild(clone);
          </script>
        </body></html>
        "#,
        BrowserRenderOptions::default(),
    );

    assert_eq!(render.text, "initial.html primary primary");
    assert_eq!(render.links.len(), 2);
    assert_eq!(render.links[0].text, "initial.html primary");
    assert_eq!(render.links[1].text, "primary");
    assert!(render.links.iter().all(|link| link.href == "initial.html"));
}

#[test]
fn class_list_mutations_feed_css_selectors_and_readback() {
    let render = render_html(
        "mem://page",
        br#"
        <html><head><style>
          .hidden { display:none }
          .accent { color:#808080 }
          .panel { background-color:#d0d0d0 }
        </style></head>
        <body>
          <p id="target" class="hidden">Class list visible</p>
          <p id="state">Before</p>
          <script>
            const target = document.getElementById("target");
            target.classList.remove("hidden");
            target.classList.add("accent", "panel");
            target.classList.toggle("active");
            const state = document.getElementById("state");
            state.textContent = target.classList.contains("accent");
            state.textContent += " ";
            state.textContent += target.classList.length;
          </script>
        </body></html>
        "#,
        BrowserRenderOptions {
            width: 40,
            ..BrowserRenderOptions::default()
        },
    );

    assert_eq!(render.text, "Class list visible\ntrue 3");
    assert_eq!(
        render.display_list,
        vec![
            DisplayCommand::Rect {
                x: 0,
                y: 0,
                width: 40,
                height: 1,
                shade: 208
            },
            DisplayCommand::StyledText {
                x: 0,
                y: 0,
                text: "Class list visible".to_owned(),
                shade: 128
            },
            DisplayCommand::Text {
                x: 0,
                y: 1,
                text: "true 3".to_owned()
            },
        ]
    );
}

#[test]
fn query_collection_bindings_support_indexing_and_scoped_queries() {
    let render = render_html(
        "mem://page",
        br#"
        <html><head><style>.active { color:#808080 }</style></head>
        <body>
          <main id="root">
            <section class="card featured">First</section>
            <section class="card">Second</section>
            <p class="item">Alpha</p>
            <p class="item">Beta</p>
          </main>
          <p id="state">Before</p>
          <script>
            const root = document.getElementById("root");
            const cards = root.querySelectorAll("section.card");
            cards[1].classList.add("active");
            cards.item(0).textContent = "Cards";
            const featured = document.getElementsByClassName("featured");
            featured[0].textContent += " ";
            featured[0].textContent += cards.length;
            const items = root.querySelectorAll(".item");
            const state = document.getElementById("state");
            state.textContent = items.length;
            state.textContent += " ";
            state.textContent += document.getElementsByTagName("section").length;
            state.textContent += " ";
            state.textContent += cards[1].textContent;
          </script>
        </body></html>
        "#,
        BrowserRenderOptions {
            width: 80,
            ..BrowserRenderOptions::default()
        },
    );

    assert_eq!(render.text, "Cards 2\nSecond\nAlpha\nBeta\n2 2 Second");
    assert_eq!(
        render.display_list,
        vec![
            DisplayCommand::Text {
                x: 0,
                y: 0,
                text: "Cards 2".to_owned()
            },
            DisplayCommand::StyledText {
                x: 0,
                y: 1,
                text: "Second".to_owned(),
                shade: 128
            },
            DisplayCommand::Text {
                x: 0,
                y: 2,
                text: "Alpha".to_owned()
            },
            DisplayCommand::Text {
                x: 0,
                y: 3,
                text: "Beta".to_owned()
            },
            DisplayCommand::Text {
                x: 0,
                y: 4,
                text: "2 2 Second".to_owned()
            },
        ]
    );
}

#[test]
fn set_timeout_runs_after_current_script_task() {
    let render = render_html(
        "mem://page",
        br#"
        <html><body>
          <h1 id="out">Before</h1>
          <p id="order">start</p>
          <script>
            const out = document.getElementById("out");
            setTimeout(() => {
              out.textContent = "Timer fired";
              document.getElementById("order").textContent += " timer";
            }, 0);
            document.getElementById("order").textContent += " sync";
          </script>
        </body></html>
        "#,
        BrowserRenderOptions::default(),
    );

    assert_eq!(render.text, "Timer fired\nstart sync timer");
}

#[test]
fn clear_timeout_cancels_pending_timer_and_assignment_ids() {
    let render = render_html(
        "mem://page",
        br#"
        <html><body>
          <h1 id="out">Before</h1>
          <script>
            const blocked = setTimeout(() => {
              document.getElementById("out").textContent = "Blocked";
            }, 0);
            clearTimeout(blocked);
            nextTimer = setTimeout(() => {
              document.getElementById("out").textContent = "Allowed";
            }, 80);
          </script>
        </body></html>
        "#,
        BrowserRenderOptions::default(),
    );

    assert_eq!(render.text, "Allowed");
}

#[test]
fn click_handlers_can_schedule_timer_callbacks() {
    let render = render_html_with_click(
        "mem://page",
        br#"
        <html><body>
          <button id="go">Go</button>
          <p id="out">Waiting</p>
          <script>
            const button = document.getElementById("go");
            button.addEventListener("click", () => {
              setTimeout(() => {
                document.getElementById("out").textContent = "Clicked later";
              }, 0);
            });
          </script>
        </body></html>
        "#,
        BrowserRenderOptions::default(),
        "#go",
    )
    .unwrap();

    assert_eq!(render.text, "Go\nClicked later");
}

#[test]
fn document_and_window_lifecycle_listeners_run_after_initial_scripts() {
    let render = render_html(
        "mem://page",
        br#"
        <html><body>
          <h1 id="out">Before</h1>
          <p id="order">start</p>
          <script>
            document.addEventListener("DOMContentLoaded", () => {
              document.getElementById("out").textContent = "DOM ready";
              document.getElementById("order").textContent += " dom";
            });
            window.addEventListener("load", function () {
              document.getElementById("order").textContent += " load";
              setTimeout(() => {
                document.getElementById("order").textContent += " timer";
              }, 0);
            });
            document.getElementById("order").textContent += " script";
          </script>
        </body></html>
        "#,
        BrowserRenderOptions::default(),
    );

    assert_eq!(render.text, "DOM ready\nstart script dom load timer");
}

#[test]
fn inline_onclick_dispatch_mutates_dom_before_render() {
    let render = render_html_with_click(
        "mem://page",
        br#"
        <html><head><title>Click</title></head>
        <body>
          <button id="go" onclick="document.getElementById('out').textContent = 'Clicked'; this.textContent = 'Done'">Go</button>
          <p id="out">Waiting</p>
        </body></html>
        "#,
        BrowserRenderOptions::default(),
        "#go",
    )
    .unwrap();

    assert_eq!(render.text, "Done\nClicked");
    assert!(!render.text.contains("Waiting"));
}

#[test]
fn complex_query_selector_and_click_selector_share_css_matching() {
    let render = render_html_with_click(
        "mem://page",
        br#"
        <html><head><title>Complex Selector</title></head>
        <body>
          <section data-kind="result">
            <button class="primary action" onclick="document.querySelector('main[data-view=&quot;results&quot;] .status').textContent = 'Clicked'; this.textContent = 'Done'">Go</button>
            <p class="title primary">Before</p>
          </section>
          <main data-view="results"><p class="status">Waiting</p></main>
          <script>
            document.querySelector('section[data-kind="result"] .title.primary').textContent = 'Found by query selector';
            document.querySelector('main[data-view="results"] .status').textContent = 'Ready';
          </script>
        </body></html>
        "#,
        BrowserRenderOptions::default(),
        r#"section[data-kind="result"] button.primary"#,
    )
    .unwrap();

    assert_eq!(render.text, "Done\nFound by query selector\nClicked");
}

#[test]
fn selector_element_methods_support_matches_and_closest() {
    let render = render_html(
        "mem://page",
        br#"
        <html><head><title>Selector Methods</title></head>
        <body>
          <main data-view="results">
            <section id="result" data-kind="result">
              <p id="target" class="title primary">Before</p>
            </section>
          </main>
          <p id="state">Waiting</p>
          <script>
            const target = document.getElementById("target");
            const state = document.getElementById("state");
            state.textContent = target.matches("p.title.primary");
            state.textContent += " ";
            state.textContent += target.matches("section .missing");
            state.textContent += " ";
            state.textContent += target.closest('main[data-view="results"]').nodeName;
            state.textContent += " ";
            state.textContent += target.closest("section").id;
          </script>
        </body></html>
        "#,
        BrowserRenderOptions::default(),
    );

    assert_eq!(render.text, "Before\ntrue false MAIN result");
}

#[test]
fn inner_html_assignment_parses_fragment_children() {
    let render = render_html(
        "mem://page",
        br##"
        <html><head><title>innerHTML</title></head>
        <body>
          <div id="root"><p>Old</p></div>
          <p id="state">Waiting</p>
          <script>
            const root = document.getElementById("root");
            root.innerHTML = `<section data-kind="card"><p class="title">Hello</p></section>`;
            root.innerHTML += `<p class="tail">Tail</p>`;
            const state = document.getElementById("state");
            state.textContent = root.children.length;
            state.textContent += " ";
            state.textContent += root.querySelector(".title").matches("p.title");
            state.textContent += " ";
            state.textContent += root.querySelector(".tail").closest("#root").id;
            state.textContent += " ";
            state.textContent += document.head.nodeName;
            state.textContent += " ";
            state.textContent += root.innerHTML;
          </script>
        </body></html>
        "##,
        BrowserRenderOptions {
            width: 120,
            ..BrowserRenderOptions::default()
        },
    );

    assert_eq!(
        render.text,
        "Hello\nTail\n2 true root HEAD <section data-kind=\"card\"><p class=\"title\">Hello</p></section><p class=\"tail\">Tail</p>"
    );
}

#[test]
fn form_control_properties_feed_dom_and_form_state() {
    let render = render_html(
        "https://example.com/start",
        br#"
        <html><head><title>Form Properties</title></head>
        <body>
          <form id="search" action="/old" method="get">
            <input id="q" name="q" value="old">
            <input id="fast" type="checkbox" name="fast">
            <button id="go" name="commit" value="old">Go</button>
          </form>
          <p id="state">Waiting</p>
          <script>
            const form = document.getElementById("search");
            const q = document.getElementById("q");
            const fast = document.getElementById("fast");
            const go = document.getElementById("go");
            form.action = "/find";
            form.method = "post";
            q.name = "query";
            q.value = "rust";
            q.value += " browser";
            q.type = "search";
            fast.checked = true;
            fast.disabled = false;
            go.value = "submit";
            const state = document.getElementById("state");
            state.textContent = form.method;
            state.textContent += " ";
            state.textContent += form.action;
            state.textContent += " ";
            state.textContent += q.name;
            state.textContent += " ";
            state.textContent += q.value;
            state.textContent += " ";
            state.textContent += q.type;
            state.textContent += " ";
            state.textContent += fast.checked;
            state.textContent += " ";
            state.textContent += fast.disabled;
            state.textContent += " ";
            state.textContent += go.value;
          </script>
        </body></html>
        "#,
        BrowserRenderOptions::default(),
    );

    assert_eq!(
        render.text,
        "[rust browser] [x] Go\npost /find query rust browser search true false submit"
    );
    assert_eq!(render.forms.len(), 1);
    let form = &render.forms[0];
    assert_eq!(form.method, "POST");
    assert_eq!(form.action, "/find");
    assert_eq!(form.resolved_action, "https://example.com/find");
    assert!(form.controls.iter().any(|control| {
        control.name == "query" && control.kind == "search" && control.value == "rust browser"
    }));
    assert!(form.controls.iter().any(|control| {
        control.name == "fast" && control.kind == "checkbox" && control.checked && !control.disabled
    }));
    assert!(
        form.controls
            .iter()
            .any(|control| control.name == "commit" && control.value == "submit")
    );
}

#[test]
fn add_event_listener_click_dispatch_mutates_dom() {
    let render = render_html_with_click(
        "mem://page",
        br#"
        <html><body>
          <button id="go">Go</button>
          <p id="out">Waiting</p>
          <script>
            const button = document.getElementById("go");
            const out = document.getElementById("out");
            button.addEventListener("click", () => {
              out.textContent = "Clicked listener";
              button.textContent = "Listened";
            });
          </script>
        </body></html>
        "#,
        BrowserRenderOptions::default(),
        "#go",
    )
    .unwrap();

    assert_eq!(render.text, "Listened\nClicked listener");
}
