use super::*;

#[test]
fn renders_css_text_color_as_styled_text() {
    let render = render_html(
        "mem://page",
        br#"
        <html><head><style>.muted { color: #808080; }</style></head>
        <body><div class="muted"><p>Muted text</p></div></body></html>
        "#,
        BrowserRenderOptions {
            width: 40,
            ..BrowserRenderOptions::default()
        },
    );

    assert_eq!(render.text, "Muted text");
    assert_eq!(
        render.display_list,
        vec![DisplayCommand::StyledText {
            x: 0,
            y: 0,
            text: "Muted text".to_owned(),
            shade: 128
        }]
    );
    let raster = rasterize_render(&render, BrowserRasterOptions::default()).unwrap();
    assert!(raster.non_background_pixels() > "Muted text".len());
}

#[test]
fn renders_inline_css_text_color_runs_on_one_line() {
    let render = render_html(
        "mem://page",
        br#"
        <html><head><style>.muted { color: #808080; }</style></head>
        <body><p>Fast <span class="muted">muted</span> text</p></body></html>
        "#,
        BrowserRenderOptions {
            width: 40,
            ..BrowserRenderOptions::default()
        },
    );

    assert_eq!(render.text, "Fast muted text");
    assert_eq!(
        render.display_list,
        vec![
            DisplayCommand::Text {
                x: 0,
                y: 0,
                text: "Fast".to_owned()
            },
            DisplayCommand::StyledText {
                x: 4,
                y: 0,
                text: " muted".to_owned(),
                shade: 128
            },
            DisplayCommand::Text {
                x: 10,
                y: 0,
                text: " text".to_owned()
            },
        ]
    );
}

#[test]
fn css_text_align_offsets_block_text() {
    let render = render_html(
        "mem://page",
        br#"
        <html><head><style>
          .center { text-align: center }
          .right { text-align: right }
          section { text-align: center }
        </style></head>
        <body>
          <p class="center">Centered</p>
          <p class="right">Right</p>
          <p style="text-align: end">End</p>
          <section><p>Child text</p></section>
        </body></html>
        "#,
        BrowserRenderOptions {
            width: 20,
            ..BrowserRenderOptions::default()
        },
    );

    assert_eq!(render.text, "Centered\nRight\nEnd\nChild text");
    assert_eq!(
        render.display_list,
        vec![
            DisplayCommand::Text {
                x: 6,
                y: 0,
                text: "Centered".to_owned(),
            },
            DisplayCommand::Text {
                x: 15,
                y: 1,
                text: "Right".to_owned(),
            },
            DisplayCommand::Text {
                x: 17,
                y: 2,
                text: "End".to_owned(),
            },
            DisplayCommand::Text {
                x: 5,
                y: 3,
                text: "Child text".to_owned(),
            },
        ]
    );
}

#[test]
fn css_max_width_and_auto_margins_constrain_document_blocks() {
    let render = render_html(
        "mem://page",
        br#"
        <html><head><style>
          main { max-width: 160px; margin: 0 auto; background-color: #d0d0d0; }
        </style></head>
        <body>
          <main><p>Readable document column wraps words cleanly</p></main>
        </body></html>
        "#,
        BrowserRenderOptions {
            width: 40,
            ..BrowserRenderOptions::default()
        },
    );

    assert_eq!(
        render.text,
        "Readable document\ncolumn wraps words\ncleanly"
    );
    assert_eq!(
        render.display_list,
        vec![
            DisplayCommand::Rect {
                x: 10,
                y: 0,
                width: 20,
                height: 3,
                shade: 208
            },
            DisplayCommand::Text {
                x: 10,
                y: 0,
                text: "Readable document".to_owned(),
            },
            DisplayCommand::Text {
                x: 10,
                y: 1,
                text: "column wraps words".to_owned(),
            },
            DisplayCommand::Text {
                x: 10,
                y: 2,
                text: "cleanly".to_owned(),
            },
        ]
    );
    let main_box = render
        .layout_boxes
        .iter()
        .find(|layout_box| layout_box.tag == "main")
        .expect("main layout box");
    assert_eq!(main_box.kind, "block");
    assert_eq!(
        (main_box.x, main_box.y, main_box.width, main_box.height),
        (10, 0, 20, 3)
    );

    let paragraph_box = render
        .layout_boxes
        .iter()
        .find(|layout_box| layout_box.tag == "p")
        .expect("paragraph layout box");
    assert_eq!(paragraph_box.parent, Some(main_box.id));
    assert_eq!(main_box.children, vec![paragraph_box.id]);
    assert_eq!(paragraph_box.kind, "block");
    assert_eq!(
        (paragraph_box.x, paragraph_box.y, paragraph_box.height),
        (10, 0, 3)
    );

    let viewport = browser_text_viewport(
        &render,
        BrowserTextViewportOptions {
            x: 0,
            y: 999,
            width: 40,
            height: 2,
        },
    );
    assert_eq!(viewport.y, viewport.max_scroll_y);
    assert_eq!(viewport.max_scroll_x, 0);
    assert_eq!(viewport.max_scroll_y, 1);
    assert_eq!(viewport.layout_box_count, 2);
    assert_eq!(viewport.visible_layout_box_count, 2);
    assert_eq!(viewport.culled_layout_box_count, 0);
    assert_eq!(
        viewport
            .visible_layout_boxes
            .iter()
            .map(|layout_box| layout_box.tag.as_str())
            .collect::<Vec<_>>(),
        vec!["main", "p"]
    );
    assert!(
        viewport
            .visible_layout_boxes
            .iter()
            .all(|layout_box| layout_box.visible_y == 0 && layout_box.visible_height == 2)
    );
}

#[test]
fn style_property_mutations_feed_cascade_and_readback() {
    let render = render_html(
        "mem://page",
        br##"
        <html><body>
          <p id="accent">Accent</p>
          <p id="hidden">Hidden after style</p>
          <p id="readout">Before</p>
          <script>
            const accent = document.getElementById("accent");
            accent.style.color = "#808080";
            accent.style.setProperty("background-color", "#d0d0d0");
            document.getElementById("hidden").style.display = "none";
            document.getElementById("readout").textContent = accent.style.getPropertyValue("color");
          </script>
        </body></html>
        "##,
        BrowserRenderOptions {
            width: 40,
            ..BrowserRenderOptions::default()
        },
    );

    assert_eq!(render.text, "Accent\nrgb(128, 128, 128)");
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
                text: "Accent".to_owned(),
                shade: 128
            },
            DisplayCommand::Text {
                x: 0,
                y: 1,
                text: "rgb(128, 128, 128)".to_owned()
            },
        ]
    );
}

#[test]
fn style_remove_property_updates_inline_cascade() {
    let render = render_html(
        "mem://page",
        br#"
        <html><body>
          <p id="restored" style="display: none">Visible after remove</p>
          <script>
            document.getElementById("restored").style.removeProperty("display");
          </script>
        </body></html>
        "#,
        BrowserRenderOptions::default(),
    );

    assert_eq!(render.text, "Visible after remove");
}

#[test]
fn css_and_inline_display_none_hide_content() {
    let render = render_html(
        "mem://page",
        br#"
        <html><head><style>.hide { display: none } #gone { display:none } p { display:block }</style></head>
        <body><p>Visible</p><p class="hide">Hidden class</p>
        <p id="gone">Hidden id</p><p style="display:none">Hidden inline</p></body></html>
        "#,
        BrowserRenderOptions::default(),
    );

    assert!(render.text.contains("Visible"));
    assert!(!render.text.contains("Hidden class"));
    assert!(!render.text.contains("Hidden id"));
    assert!(!render.text.contains("Hidden inline"));
    assert!(
        render
            .display_list
            .iter()
            .all(|command| !style_display_command_text(command).contains("Hidden"))
    );
    assert_eq!(render.css_rule_count, 3);
}

#[test]
fn css_comments_do_not_suppress_layout_rules() {
    let render = render_html(
        "mem://css-comments",
        br#"
        <html><head><style>
          /* reset comments should not become part of the selector */
          .hide { display: none; /* comments should not become part of the value */ }
          /* comments between rules should be ignored */
          .shown { display: block; }
        </style></head>
        <body>
          <p class="hide">Hidden by stylesheet comment handling</p>
          <p class="shown">Visible comment rule</p>
          <p style="/* inline comment */ display:none">Hidden by inline comment handling</p>
        </body></html>
        "#,
        BrowserRenderOptions::default(),
    );

    assert_eq!(render.text, "Visible comment rule");
    assert_eq!(render.css_rule_count, 2);
}

#[test]
fn compound_child_and_descendant_css_selectors_hide_content() {
    let render = render_html(
        "mem://page",
        br#"
        <html><head><style>
          .note.ghost { display:none }
          nav > a.hidden { display:none }
          article.card span.hidden { display:none }
        </style></head>
        <body>
          <h1>Compound CSS Visible</h1>
          <p class="note ghost">Hidden compound</p>
          <nav><a class="hidden">Hidden child</a><a>Visible nav link</a></nav>
          <article class="card"><p><span class="hidden">Hidden descendant</span><span>Visible descendant sibling</span></p></article>
        </body></html>
        "#,
        BrowserRenderOptions::default(),
    );

    assert_eq!(
        render.text,
        "Compound CSS Visible\nVisible nav link\nVisible descendant sibling"
    );
    assert_eq!(render.css_rule_count, 3);
    assert!(!render.text.contains("Hidden"));
}

#[test]
fn indexed_css_cascade_preserves_specificity_and_source_order() {
    let render = render_html(
        "mem://page",
        br#"
        <html><head><style>
          .later { display:block }
          .earlier { display:none }
          p { display:block }
          #strong { display:none }
        </style></head>
        <body>
          <p class="earlier later">Hidden by later same-specificity rule</p>
          <p id="strong" class="later">Hidden by id specificity</p>
          <p>Visible paragraph</p>
        </body></html>
        "#,
        BrowserRenderOptions::default(),
    );

    assert_eq!(render.text, "Visible paragraph");
}

#[test]
fn hidden_attribute_and_attribute_css_selectors_hide_content() {
    let render = render_html(
        "mem://page",
        br#"
        <html><head><style>
          [aria-hidden="true"] { display:none }
          [data-state="closed"] { display:none }
          section[data-kind="promo"] .target { display:none }
        </style></head>
        <body>
          <h1>Attribute CSS Visible</h1>
          <p hidden>Hidden by hidden attribute</p>
          <p aria-hidden="true">Hidden by aria</p>
          <p data-state="closed">Hidden by data state</p>
          <section data-kind="promo"><p class="target">Hidden descendant</p></section>
          <p data-state="open">Visible open state</p>
        </body></html>
        "#,
        BrowserRenderOptions::default(),
    );

    assert_eq!(render.text, "Attribute CSS Visible\nVisible open state");
    assert_eq!(render.css_rule_count, 3);
    assert!(!render.text.contains("Hidden"));
}

fn style_display_command_text(command: &DisplayCommand) -> &str {
    match command {
        DisplayCommand::Text { text, .. } | DisplayCommand::StyledText { text, .. } => text,
        DisplayCommand::Rect { .. } | DisplayCommand::Image { .. } => "",
    }
}
