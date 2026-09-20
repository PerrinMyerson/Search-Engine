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
fn percent_sized_svg_in_inline_container_stays_icon_sized() {
    let render = render_html(
        "mem://percent-svg-icon",
        br#"
        <html><head><style>
          .icon { display: inline-flex; line-height: 1; }
          .item .icon svg { width: 100%; height: 100%; }
        </style></head>
        <body><div class="item"><span class="icon"><svg width="768" height="768" viewBox="0 0 768 768"><path d="M384 720c185 0 336-150 336-336s-150-336-336-336-336 150-336 336 150 336 336 336z"/></svg></span> or Register</div></body></html>
        "#,
        BrowserRenderOptions {
            width: 160,
            ..BrowserRenderOptions::default()
        },
    );

    let widest_dark = render
        .display_list
        .iter()
        .filter_map(|command| match command {
            DisplayCommand::ColorRect {
                width,
                red: 0,
                green: 0,
                blue: 0,
                ..
            } => Some(*width),
            _ => None,
        })
        .max()
        .unwrap_or(0);
    assert!(
        widest_dark <= 2,
        "percent-sized svg in a shrink-wrap inline container should stay icon-sized, got {widest_dark} cells"
    );
    assert!(render.text.contains("or Register"));
}

#[test]
fn css_calc_expressions_resolve_with_var_terms() {
    let render = render_html(
        "mem://css-calc-var",
        br#"
        <html><head><style>
          .h { --w: 180px; --hh: calc(var(--w) * 479 / 836); }
          .logo { max-width: var(--w); max-height: var(--hh); height: auto; }
          .plain { width: calc(100px + 60px); background: rgb(220,220,220); }
        </style></head>
        <body>
          <div class="h"><img class="logo" src="mem://logo.png" width="836" height="479" alt=""></div>
          <div class="plain">sized</div>
        </body></html>
        "#,
        BrowserRenderOptions {
            width: 80,
            ..BrowserRenderOptions::default()
        },
    );

    let (width, height) = render
        .display_list
        .iter()
        .find_map(|command| match command {
            DisplayCommand::Image { width, height, .. } => Some((*width, *height)),
            _ => None,
        })
        .expect("calc-sized image should render");
    assert!(
        (20..=23).contains(&width),
        "max-width var should clamp to ~180px; got {width} cells"
    );
    assert!(
        height <= 9,
        "calc-derived max-height (~103px) should cap the image; got {height} rows"
    );
    let plain_width = render
        .display_list
        .iter()
        .find_map(|command| match command {
            DisplayCommand::Rect {
                width, shade: 220, ..
            } => Some(*width),
            _ => None,
        })
        .expect("calc-sized div background should paint");
    assert_eq!(
        plain_width, 20,
        "calc(100px + 60px) should resolve to 160px"
    );
}

#[test]
fn scoped_custom_properties_prefer_later_declarations() {
    let render = render_html(
        "mem://scoped-var-order",
        br#"
        <html><head><style>
          .theme { --w: 80px; }
          .custom { --w: 200px; }
          .box { width: var(--w); background: rgb(220,220,220); }
        </style></head>
        <body><div class="box">Sized</div></body></html>
        "#,
        BrowserRenderOptions {
            width: 60,
            ..BrowserRenderOptions::default()
        },
    );
    let width = render
        .display_list
        .iter()
        .find_map(|command| match command {
            DisplayCommand::Rect {
                width, shade: 220, ..
            } => Some(*width),
            _ => None,
        })
        .expect("box background should paint");
    assert_eq!(
        width, 25,
        "the later scoped --w declaration (200px) should win per cascade order"
    );
}

#[test]
fn scoped_css_custom_properties_resolve_when_root_leaves_them_undefined() {
    let render = render_html(
        "mem://scoped-custom-property",
        br#"
        <html><head><style>
          .header-logo { --k-logo-width: 180px; }
          .main-logo { max-width: var(--k-logo-width); height: auto; }
        </style></head>
        <body>
          <div class="header-logo"><img class="main-logo" src="mem://logo.png" width="800" height="200" alt="logo"></div>
        </body></html>
        "#,
        BrowserRenderOptions {
            width: 80,
            ..BrowserRenderOptions::default()
        },
    );

    let (width, height) = render
        .display_list
        .iter()
        .find_map(|command| match command {
            DisplayCommand::Image { width, height, .. } => Some((*width, *height)),
            _ => None,
        })
        .expect("logo image should render");
    assert!(
        (20..=23).contains(&width),
        "var(--k-logo-width) from a scoped rule should clamp the image to ~180px; got {width} cells"
    );
    assert!(
        height <= 5,
        "height should follow the intrinsic ratio after the var-clamped width; got {height} rows"
    );
}

#[test]
fn link_pseudo_class_selector_styles_unvisited_anchors() {
    let render = render_html(
        "mem://link-pseudo",
        br#"
        <html><head><style>
          td { color: #828282; }
          a:link { color: #000000; }
          .c82 a:link { color: #828282; }
        </style></head>
        <body><table><tr><td>
          <span>faded</span> <a href="/story">Story title</a>
        </td></tr></table></body></html>
        "#,
        BrowserRenderOptions {
            width: 40,
            ..BrowserRenderOptions::default()
        },
    );

    assert!(render.text.contains("Story title"));
    let link_color = render
        .display_list
        .iter()
        .zip(render.hit_targets.iter())
        .find_map(|(command, target)| match command {
            DisplayCommand::Text { text, .. } | DisplayCommand::StyledText { text, .. }
                if text.contains("Story title") =>
            {
                Some(target.text_color)
            }
            _ => None,
        })
        .expect("link text should render");
    assert_eq!(
        link_color,
        Some((0, 0, 0)),
        "a:link should override the inherited td color for unvisited anchors"
    );
}

#[test]
fn rgba_raster_preserves_css_text_rgb_color() {
    let render = render_html(
        "mem://page",
        br#"
        <html><head><style>.teal { color: rgb(28, 123, 132); }</style></head>
        <body><p class="teal">Color</p></body></html>
        "#,
        BrowserRenderOptions {
            width: 20,
            ..BrowserRenderOptions::default()
        },
    );

    assert_eq!(render.text, "Color");
    assert!(render.display_list.iter().any(|command| {
        matches!(
            command,
            DisplayCommand::StyledText { text, .. } if text == "Color"
        )
    }));

    let rgba = rasterize_render_rgba(&render, BrowserRasterOptions::default())
        .expect("rasterize css text color");
    assert!(
        rgba.pixels
            .chunks_exact(4)
            .any(|pixel| pixel == [28, 123, 132, 255]),
        "RGBA raster should preserve CSS text RGB instead of only the grayscale shade"
    );
}

#[test]
fn rgba_raster_softens_colored_glyph_edges() {
    let render = render_html(
        "mem://page",
        br#"
        <html><head><style>.teal { color: rgb(28, 123, 132); font-size: 16px; }</style></head>
        <body><p class="teal">HH</p></body></html>
        "#,
        BrowserRenderOptions {
            width: 20,
            ..BrowserRenderOptions::default()
        },
    );
    let (text_x, text_y) = render
        .display_list
        .iter()
        .find_map(|command| match command {
            DisplayCommand::StyledText { x, y, text, .. } if text == "HH" => Some((*x, *y)),
            _ => None,
        })
        .expect("styled text command");
    let raster_options = BrowserRasterOptions {
        cell_width: 12,
        cell_height: 18,
        ..BrowserRasterOptions::default()
    };
    assert_eq!(raster_glyph_scale(raster_options), 2);
    let rgba = rasterize_render_rgba(&render, raster_options).expect("rasterize colored glyph");
    let glyph_x = raster_options
        .padding_x
        .saturating_add(text_x.saturating_mul(raster_options.cell_width))
        .saturating_add(1);
    let glyph_y = raster_options
        .padding_y
        .saturating_add(text_y.saturating_mul(raster_options.cell_height))
        .saturating_add(2);
    let pixel = |x: usize, y: usize| {
        let index = y
            .saturating_mul(rgba.width)
            .saturating_add(x)
            .saturating_mul(4);
        &rgba.pixels[index..index.saturating_add(4)]
    };

    assert_eq!(pixel(glyph_x, glyph_y), &[28, 123, 132, 255]);
    let softened_edge = pixel(glyph_x + 1, glyph_y);
    assert_ne!(softened_edge, &[28, 123, 132, 255]);
    assert_ne!(softened_edge, &[255, 255, 255, 255]);
    assert!(
        softened_edge[0] > 28 && softened_edge[1] > 123 && softened_edge[2] > 132,
        "softened colored edge should blend toward the white background, got {softened_edge:?}"
    );
}

#[test]
fn rgba_raster_uses_proportional_glyph_advance() {
    let render = render_html(
        "mem://page",
        br#"<html><body><p>im</p></body></html>"#,
        BrowserRenderOptions {
            width: 10,
            ..BrowserRenderOptions::default()
        },
    );
    let raster_options = BrowserRasterOptions {
        cell_width: 12,
        cell_height: 18,
        ..BrowserRasterOptions::default()
    };
    let rgba = rasterize_render_rgba(&render, raster_options).expect("rasterize proportional text");
    let monospace_m_start = raster_options
        .padding_x
        .saturating_add(raster_options.cell_width);
    let proportional_m_start = raster_options
        .padding_x
        .saturating_add(raster_glyph_advance('i', raster_options.cell_width));
    let pixel = |x: usize, y: usize| {
        let index = y
            .saturating_mul(rgba.width)
            .saturating_add(x)
            .saturating_mul(4);
        &rgba.pixels[index..index.saturating_add(4)]
    };
    let proportional_slot_has_ink = (raster_options.padding_y
        ..raster_options
            .padding_y
            .saturating_add(raster_options.cell_height))
        .any(|y| {
            (proportional_m_start..monospace_m_start).any(|x| pixel(x, y) != [255, 255, 255, 255])
        });

    assert!(proportional_m_start < monospace_m_start);
    assert!(
        proportional_slot_has_ink,
        "second glyph should begin before the old fixed-width cell boundary"
    );
}

#[test]
fn rgba_raster_paints_css_bold_weight_with_thicker_glyphs() {
    let ink_pixels_for_weight = |weight: &str| {
        let html = format!(
            r#"
            <html><head><style>
              .sample {{ color: rgb(28, 123, 132); font-weight: {weight}; }}
            </style></head>
            <body><p class="sample">HH</p></body></html>
            "#
        );
        let render = render_html(
            "mem://page",
            html.as_bytes(),
            BrowserRenderOptions {
                width: 20,
                ..BrowserRenderOptions::default()
            },
        );
        let rgba = rasterize_render_rgba(
            &render,
            BrowserRasterOptions {
                cell_width: 12,
                cell_height: 18,
                ..BrowserRasterOptions::default()
            },
        )
        .expect("rasterize weighted text");
        rgba.pixels
            .chunks_exact(4)
            .filter(|pixel| *pixel != [255, 255, 255, 255])
            .count()
    };

    let normal_pixels = ink_pixels_for_weight("400");
    let bold_pixels = ink_pixels_for_weight("700");

    assert!(
        bold_pixels > normal_pixels,
        "font-weight:700 should thicken glyph paint, normal={normal_pixels}, bold={bold_pixels}"
    );
}

#[test]
fn rgba_raster_inherits_parent_css_color_over_default_link_blue() {
    let render = render_html(
        "mem://page",
        br#"
        <html><head><style>.nav { color: rgb(126, 188, 199); }</style></head>
        <body><nav class="nav"><a href="/about">About</a></nav></body></html>
        "#,
        BrowserRenderOptions {
            width: 20,
            ..BrowserRenderOptions::default()
        },
    );

    assert_eq!(render.text, "About");
    let rgba = rasterize_render_rgba(&render, BrowserRasterOptions::default())
        .expect("rasterize inherited link color");
    assert!(
        rgba.pixels
            .chunks_exact(4)
            .any(|pixel| pixel == [126, 188, 199, 255]),
        "author color inherited from the parent should beat default link blue"
    );
    assert!(
        !rgba
            .pixels
            .chunks_exact(4)
            .any(|pixel| pixel == [0, 0, 255, 255]),
        "default link blue should not override inherited author color"
    );
}

#[test]
fn css_current_color_variable_background_uses_resolved_text_rgb() {
    let render = render_html(
        "mem://page",
        br#"
        <html><head><style>
          :root { --brand-fill: currentColor; }
          .badge { color: rgb(244, 201, 28); background-color: var(--brand-fill); }
        </style></head>
        <body><section class="badge">Gold</section></body></html>
        "#,
        BrowserRenderOptions {
            width: 20,
            ..BrowserRenderOptions::default()
        },
    );

    assert!(render.display_list.iter().any(|command| {
        matches!(
            command,
            DisplayCommand::ColorRect {
                red: 244,
                green: 201,
                blue: 28,
                ..
            }
        )
    }));
}

#[test]
fn css_current_color_border_preserves_rgb_paint() {
    let render = render_html(
        "mem://page",
        br#"
        <html><head><style>
          .outline { color: rgb(42, 96, 180); border: 1px solid currentColor; }
        </style></head>
        <body><section class="outline">Outlined</section></body></html>
        "#,
        BrowserRenderOptions {
            width: 24,
            ..BrowserRenderOptions::default()
        },
    );

    assert!(render.display_list.iter().any(|command| {
        matches!(
            command,
            DisplayCommand::ColorRect {
                red: 42,
                green: 96,
                blue: 180,
                ..
            }
        )
    }));
}

#[test]
fn css_text_decoration_none_suppresses_link_underline_rects() {
    let render = render_html(
        "mem://page",
        br#"
        <html><head><style>
          nav a { text-decoration: none; }
        </style></head>
        <body><nav><a href="/about">About</a></nav></body></html>
        "#,
        BrowserRenderOptions {
            width: 20,
            ..BrowserRenderOptions::default()
        },
    );

    assert_eq!(render.text, "About");
    assert!(
        !render.display_list.iter().any(|command| {
            matches!(
                command,
                DisplayCommand::Rect { shade, .. } if *shade == LINK_UNDERLINE_SHADE
            )
        }),
        "author text-decoration:none should remove the default anchor underline paint"
    );
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
fn css_important_declarations_feed_rendered_layout() {
    let render = render_html(
        "mem://important",
        br#"
        <html><head><style>
          .hide { display: none !important }
          .ghost { visibility: hidden !important }
          .muted { color: #808080!important }
        </style></head>
        <body>
          <p class="hide">Hidden CSS</p>
          <p style="display:none !important">Hidden inline</p>
          <p>Before <span class="ghost">Ghost</span> After</p>
          <p class="muted">Muted</p>
        </body></html>
        "#,
        BrowserRenderOptions {
            width: 40,
            ..BrowserRenderOptions::default()
        },
    );

    assert_eq!(render.text, "Before       After\nMuted");
    assert!(!render.text.contains("Hidden"));
    assert_eq!(
        render.display_list,
        vec![
            DisplayCommand::Text {
                x: 0,
                y: 0,
                text: "Before".to_owned(),
            },
            DisplayCommand::Text {
                x: 12,
                y: 0,
                text: " After".to_owned(),
            },
            DisplayCommand::StyledText {
                x: 0,
                y: 1,
                text: "Muted".to_owned(),
                shade: 128,
            },
        ]
    );
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
fn css_not_pseudo_selectors_filter_rendered_layout_rules() {
    let render = render_html(
        "mem://not-selectors",
        br#"
        <html><head><style>
          .item:not(.keep, [data-state="open"]) { display:none }
          .panel:not(.closed) .target { text-transform: uppercase }
          .panel.closed .target { display:none }
        </style></head>
        <body>
          <p class="item">Hidden item</p>
          <p class="item keep">Visible keep</p>
          <p class="item" data-state="open">Visible data</p>
          <section class="panel closed"><p class="target">Hidden panel</p></section>
          <section class="panel"><p class="target">visible panel</p></section>
        </body></html>
        "#,
        BrowserRenderOptions::default(),
    );

    assert_eq!(render.text, "Visible keep\nVisible data\nVISIBLE PANEL");
    assert_eq!(render.css_rule_count, 3);
    assert!(!render.text.contains("Hidden"));
    assert_eq!(
        render
            .display_list
            .iter()
            .filter_map(|command| match command {
                DisplayCommand::Text { text, .. } => Some(text.as_str()),
                DisplayCommand::StyledText { text, .. } => Some(text.as_str()),
                DisplayCommand::Rect { .. }
                | DisplayCommand::ColorRect { .. }
                | DisplayCommand::Image { .. }
                | DisplayCommand::BackgroundImage { .. } => None,
            })
            .collect::<Vec<_>>(),
        vec!["Visible keep", "Visible data", "VISIBLE PANEL"]
    );
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
        DisplayCommand::Rect { .. }
        | DisplayCommand::ColorRect { .. }
        | DisplayCommand::Image { .. }
        | DisplayCommand::BackgroundImage { .. } => "",
    }
}
