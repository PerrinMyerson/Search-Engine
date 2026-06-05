use super::images::{
    test_webp_data_url_with_mime_type, tiny_test_webp_bytes, tiny_test_webp_data_url,
};
use super::*;

#[test]
fn renders_static_dom_and_skips_scripts() {
    let render = render_html(
        "mem://page",
        br#"
            <html><head><title>Hello</title><script>bad()</script></head>
            <body><h1>Fast</h1><p>Static &amp; text</p></body></html>
            "#,
        BrowserRenderOptions {
            width: 80,
            ..BrowserRenderOptions::default()
        },
    );

    assert_eq!(render.title, "Hello");
    assert!(render.text.contains("Fast"));
    assert!(render.text.contains("Static & text"));
    assert!(!render.text.contains("bad"));
    assert!(render.dom_node_count > 4);
    assert!(render.layout_box_count > 0);
    assert_eq!(render.paint_command_count, render.display_list.len());
}

#[test]
fn materializes_element_data_from_parsed_attributes() {
    let parsed = parse_html(
            br#"
            <form METHOD="post" Action="/submit" hidden>
              <input ID="hero" class="primary active" TYPE="CheckBox" Name="fast" checked disabled="">
              <option selected Value="docs">Docs</option>
            </form>
            "#,
        );

    let elements = parsed
        .dom
        .nodes
        .iter()
        .filter_map(|node| match &node.kind {
            NodeKind::Element(element) => Some(element.as_ref()),
            _ => None,
        })
        .collect::<Vec<_>>();

    let form = elements
        .iter()
        .copied()
        .find(|element| element.tag == "form")
        .unwrap();
    assert_eq!(form.method.as_deref(), Some("POST"));
    assert_eq!(form.action.as_deref(), Some("/submit"));
    assert!(form.hidden);
    assert_eq!(form.attrs.get("method").map(String::as_str), Some("post"));
    assert_eq!(
        form.attrs.get("action").map(String::as_str),
        Some("/submit")
    );

    let input = elements
        .iter()
        .copied()
        .find(|element| element.tag == "input")
        .unwrap();
    assert_eq!(input.id.as_deref(), Some("hero"));
    assert_eq!(input.classes, vec!["primary", "active"]);
    assert_eq!(input.type_hint.as_deref(), Some("CheckBox"));
    assert_eq!(input.input_type.as_deref(), Some("checkbox"));
    assert_eq!(input.name.as_deref(), Some("fast"));
    assert!(input.checked);
    assert!(input.disabled);

    let option = elements
        .iter()
        .copied()
        .find(|element| element.tag == "option")
        .unwrap();
    assert!(option.selected);
    assert_eq!(option.value.as_deref(), Some("docs"));
}

#[test]
fn parse_html_keeps_quoted_data_svg_src_with_raw_tag_markers() {
    let raw_src = "data:image/svg+xml,<svg viewBox='0 0 120 40' xmlns='http://www.w3.org/2000/svg'><rect width='120' height='40' fill='red'/></svg>";
    let parsed = parse_html(
        br##"
            <html><body>
              <img src="data:image/svg+xml,<svg viewBox='0 0 120 40' xmlns='http://www.w3.org/2000/svg'><rect width='120' height='40' fill='red'/></svg>" alt="raw svg">
              <p>After image</p>
            </body></html>
            "##,
    );

    let image = parsed
        .dom
        .nodes
        .iter()
        .find_map(|node| match &node.kind {
            NodeKind::Element(element) if element.tag == "img" => Some(element.as_ref()),
            _ => None,
        })
        .expect("quoted raw data SVG image survives parsing");
    assert_eq!(image.src.as_deref(), Some(raw_src));
    assert_eq!(image.attrs.get("src").map(String::as_str), Some(raw_src));
    assert!(parsed.dom.nodes.iter().any(|node| {
        matches!(
            &node.kind,
            NodeKind::Text(text) if text.contains("After image")
        )
    }));
}

#[test]
fn rasterizes_display_list_into_stable_grayscale_pixels() {
    let render = render_html(
        "mem://page",
        br#"<html><body><h1>ABC 123</h1></body></html>"#,
        BrowserRenderOptions {
            width: 20,
            ..BrowserRenderOptions::default()
        },
    );
    let options = BrowserRasterOptions::default();
    let raster = rasterize_render(&render, options).unwrap();
    let report = raster_report(&render, &raster, options);

    assert_eq!(raster.width, 168);
    assert_eq!(raster.height, 20);
    assert!(raster.non_background_pixels() > 0);
    assert_eq!(report.display_command_count, render.display_list.len());
    assert_eq!(report.pixel_hash.len(), 64);
    assert!(raster.encode_pgm().starts_with(b"P5\n168 20\n255\n"));
}

#[test]
fn rasterizes_display_list_into_rgba_png_artifact() {
    let render = render_html(
        "mem://page",
        br#"<html><body><h1>RGBA</h1><p>Screenshot</p></body></html>"#,
        BrowserRenderOptions {
            width: 20,
            ..BrowserRenderOptions::default()
        },
    );
    let options = BrowserRasterOptions::default();
    let raster = rasterize_render_rgba(&render, options).unwrap();
    let report = rgba_raster_report(&render, &raster, options);
    let png = raster.encode_png().unwrap();

    assert_eq!(raster.width, 168);
    assert_eq!(raster.height, 32);
    assert_eq!(raster.pixels.len(), raster.width * raster.height * 4);
    assert!(raster.non_background_pixels() > 0);
    assert_eq!(report.bytes_per_pixel, 4);
    assert_eq!(report.artifact_format, "png-rgba8");
    assert_eq!(report.pixel_hash.len(), 64);
    assert!(png.starts_with(b"\x89PNG\r\n\x1a\n"));
    assert_eq!(&png[12..16], b"IHDR");
    assert!(png.windows(4).any(|chunk| chunk == b"IDAT"));
    assert!(png.ends_with(b"IEND\xaeB`\x82"));
}

fn render_from_display_list(
    source: &str,
    viewport_width: usize,
    display_list: Vec<DisplayCommand>,
) -> BrowserRender {
    BrowserRender {
        source: source.to_owned(),
        title: String::new(),
        viewport_width,
        dom_node_count: 0,
        css_rule_count: 0,
        layout_box_count: 0,
        layout_boxes: Vec::new(),
        paint_command_count: display_list.len(),
        links: Vec::new(),
        forms: Vec::new(),
        resources: Vec::new(),
        fragment_targets: Vec::new(),
        decoded_images: Vec::new(),
        hit_targets: vec![DisplayHitTarget::default(); display_list.len()],
        text: String::new(),
        display_list,
    }
}

#[test]
fn full_document_raster_reports_all_commands_visible() {
    let render = render_from_display_list(
        "mem://full-raster",
        8,
        vec![
            DisplayCommand::Text {
                x: 0,
                y: 0,
                text: "alpha".to_owned(),
            },
            DisplayCommand::Rect {
                x: 0,
                y: 1,
                width: 8,
                height: 1,
                shade: 128,
            },
            DisplayCommand::Text {
                x: 0,
                y: 2,
                text: "omega".to_owned(),
            },
        ],
    );
    let options = BrowserRasterOptions::default();
    let raster = rasterize_render(&render, options).unwrap();
    let report = raster_report(&render, &raster, options);

    assert_eq!(report.visible_command_count, 3);
    assert_eq!(report.culled_command_count, 0);
    assert_eq!(report.raster_viewport_x, None);
    assert_eq!(report.raster_viewport_y, None);
    assert_eq!(raster.width, 72);
    assert_eq!(raster.height, 44);
}

#[test]
fn viewport_raster_culls_and_translates_vertical_scroll_window() {
    let render = render_from_display_list(
        "mem://scroll-raster",
        6,
        vec![
            DisplayCommand::Text {
                x: 0,
                y: 0,
                text: "zero".to_owned(),
            },
            DisplayCommand::Text {
                x: 0,
                y: 1,
                text: "one".to_owned(),
            },
            DisplayCommand::Text {
                x: 0,
                y: 2,
                text: "two".to_owned(),
            },
            DisplayCommand::Rect {
                x: 0,
                y: 3,
                width: 6,
                height: 1,
                shade: 128,
            },
            DisplayCommand::Text {
                x: 0,
                y: 4,
                text: "four".to_owned(),
            },
        ],
    );
    let options = BrowserRasterOptions {
        viewport_y: Some(1),
        viewport_height: Some(2),
        ..BrowserRasterOptions::default()
    };
    let raster = rasterize_render(&render, options).unwrap();
    let report = raster_report(&render, &raster, options);

    let translated = render_from_display_list(
        "mem://translated-raster",
        6,
        vec![
            DisplayCommand::Text {
                x: 0,
                y: 0,
                text: "one".to_owned(),
            },
            DisplayCommand::Text {
                x: 0,
                y: 1,
                text: "two".to_owned(),
            },
        ],
    );
    let translated_raster = rasterize_render(&translated, BrowserRasterOptions::default()).unwrap();

    assert_eq!(raster.width, 56);
    assert_eq!(raster.height, 32);
    assert_eq!(report.visible_command_count, 2);
    assert_eq!(report.culled_command_count, 3);
    assert_eq!(report.raster_viewport_y, Some(1));
    assert_eq!(report.raster_viewport_height, Some(2));
    assert_eq!(raster.pixel_hash(), translated_raster.pixel_hash());
}

#[test]
fn viewport_raster_clips_text_on_horizontal_scroll_window() {
    let render = render_from_display_list(
        "mem://horizontal-raster",
        5,
        vec![DisplayCommand::Text {
            x: 0,
            y: 0,
            text: "ABCDE".to_owned(),
        }],
    );
    let options = BrowserRasterOptions {
        viewport_x: Some(2),
        viewport_width: Some(3),
        viewport_height: Some(1),
        ..BrowserRasterOptions::default()
    };
    let raster = rasterize_render(&render, options).unwrap();
    let report = raster_report(&render, &raster, options);

    let clipped = render_from_display_list(
        "mem://horizontal-clipped-raster",
        3,
        vec![DisplayCommand::Text {
            x: 0,
            y: 0,
            text: "CDE".to_owned(),
        }],
    );
    let clipped_raster = rasterize_render(&clipped, BrowserRasterOptions::default()).unwrap();

    assert_eq!(raster.width, 32);
    assert_eq!(raster.height, 20);
    assert_eq!(report.visible_command_count, 1);
    assert_eq!(report.culled_command_count, 0);
    assert_eq!(report.raster_viewport_x, Some(2));
    assert_eq!(report.raster_viewport_width, Some(3));
    assert_eq!(raster.pixel_hash(), clipped_raster.pixel_hash());
}

#[test]
fn text_viewport_clips_display_list_for_browser_shell() {
    let render = render_from_display_list(
        "mem://viewport-shell",
        8,
        vec![
            DisplayCommand::Text {
                x: 0,
                y: 0,
                text: "zero".to_owned(),
            },
            DisplayCommand::Text {
                x: 1,
                y: 1,
                text: "one".to_owned(),
            },
            DisplayCommand::Rect {
                x: 0,
                y: 2,
                width: 8,
                height: 1,
                shade: 128,
            },
            DisplayCommand::Image {
                x: 2,
                y: 3,
                width: 3,
                height: 2,
                shade: 220,
                alt: Some("tile".to_owned()),
                url: None,
                decoded_width: None,
                decoded_height: None,
                decoded_hash: None,
            },
        ],
    );

    let viewport = browser_text_viewport(
        &render,
        BrowserTextViewportOptions {
            x: 1,
            y: 1,
            width: 2,
            height: 3,
        },
    );

    assert_eq!(viewport.document_width, 8);
    assert_eq!(viewport.document_height, 5);
    assert_eq!(viewport.max_scroll_x, 4);
    assert_eq!(viewport.max_scroll_y, 2);
    assert_eq!(viewport.visible_command_count, 3);
    assert_eq!(viewport.culled_command_count, 1);
    assert_eq!(viewport.layout_box_count, 0);
    assert_eq!(viewport.visible_layout_box_count, 0);
    assert_eq!(viewport.culled_layout_box_count, 0);
    assert_eq!(viewport.lines, vec!["one", "####", " @t@"]);
}

#[test]
fn text_viewport_overlays_image_alt_on_placeholder_cells() {
    let render = render_from_display_list(
        "mem://viewport-alt-image",
        12,
        vec![
            DisplayCommand::Image {
                x: 0,
                y: 0,
                width: 10,
                height: 2,
                shade: 220,
                alt: Some("Hero art".to_owned()),
                url: None,
                decoded_width: None,
                decoded_height: None,
                decoded_hash: None,
            },
            DisplayCommand::Text {
                x: 0,
                y: 2,
                text: "After".to_owned(),
            },
        ],
    );

    let viewport = browser_text_viewport(
        &render,
        BrowserTextViewportOptions {
            x: 0,
            y: 0,
            width: 12,
            height: 3,
        },
    );

    assert_eq!(viewport.lines, vec!["@@@@@@@@@@", "@Hero art@", "After"]);
}

#[test]
fn display_bounds_intersection_clips_to_raster_viewport() {
    let viewport = RasterViewport {
        x: 2,
        y: 1,
        width: 3,
        height: 2,
        active: true,
    };

    assert_eq!(
        intersect_display_bounds_with_viewport(
            DisplayCommandBounds {
                x: 0,
                y: 0,
                width: 4,
                height: 2,
            },
            viewport,
        ),
        Some(DisplayCommandBounds {
            x: 2,
            y: 1,
            width: 2,
            height: 1,
        })
    );
    assert_eq!(
        intersect_display_bounds_with_viewport(
            DisplayCommandBounds {
                x: 5,
                y: 1,
                width: 1,
                height: 1,
            },
            viewport,
        ),
        None
    );
}

#[test]
fn renders_horizontal_rule_as_rect_paint_command() {
    let render = render_html(
        "mem://page",
        br#"<html><body><p>Above rule</p><hr><p>Below rule</p></body></html>"#,
        BrowserRenderOptions {
            width: 40,
            ..BrowserRenderOptions::default()
        },
    );

    assert_eq!(render.text, "Above rule\nBelow rule");
    assert_eq!(
        render.display_list,
        vec![
            DisplayCommand::Text {
                x: 0,
                y: 0,
                text: "Above rule".to_owned()
            },
            DisplayCommand::Rect {
                x: 0,
                y: 1,
                width: 40,
                height: 1,
                shade: 96
            },
            DisplayCommand::Text {
                x: 0,
                y: 2,
                text: "Below rule".to_owned()
            },
        ]
    );
    let raster = rasterize_render(&render, BrowserRasterOptions::default()).unwrap();
    assert!(raster.non_background_pixels() > "Above ruleBelow rule".len());
}

#[test]
fn renders_block_background_as_underlay_rect() {
    let render = render_html(
            "mem://page",
            br#"
            <html><head><style>.panel { background-color: #d0d0d0; }</style></head>
            <body><div class="panel"><p>Background card</p><p>Still text first</p></div></body></html>
            "#,
            BrowserRenderOptions {
                width: 40,
                ..BrowserRenderOptions::default()
            },
        );

    assert_eq!(render.text, "Background card\nStill text first");
    assert_eq!(
        render.display_list,
        vec![
            DisplayCommand::Rect {
                x: 0,
                y: 0,
                width: 40,
                height: 2,
                shade: 208
            },
            DisplayCommand::Text {
                x: 0,
                y: 0,
                text: "Background card".to_owned()
            },
            DisplayCommand::Text {
                x: 0,
                y: 1,
                text: "Still text first".to_owned()
            },
        ]
    );
    let raster = rasterize_render(&render, BrowserRasterOptions::default()).unwrap();
    assert!(raster.non_background_pixels() > 40 * 2);
}

#[test]
fn hit_testing_reports_topmost_display_command_bounds() {
    let render = render_html(
            "mem://page",
            br#"
            <html><head><style>.panel { background-color: #d0d0d0; }</style></head>
            <body><div class="panel">Background card</div><img src="hero.png" alt="Hero art" width="80" height="24"></body></html>
            "#,
            BrowserRenderOptions {
                width: 40,
                ..BrowserRenderOptions::default()
            },
        );

    let text_hit = hit_test_render(&render, 1, 0).hit.unwrap();
    assert_eq!(text_hit.kind, "text");
    assert_eq!(text_hit.text.as_deref(), Some("Background card"));

    let background_hit = hit_test_render(&render, 30, 0).hit.unwrap();
    assert_eq!(background_hit.kind, "rect");
    assert_eq!(background_hit.shade, Some(208));

    let image_hit = hit_test_render(&render, 1, 1).hit.unwrap();
    assert_eq!(image_hit.kind, "image");
    assert_eq!(image_hit.alt.as_deref(), Some("Hero art"));

    assert_eq!(hit_test_render(&render, 40, 20).hit, None);
}

#[test]
fn layer_tree_promotes_image_commands_under_document_root() {
    let render = render_html(
            "mem://page",
            br#"<html><body><p>Before image</p><img src="hero.png" alt="Hero art" width="80" height="24"><p>After image</p></body></html>"#,
            BrowserRenderOptions {
                width: 40,
                ..BrowserRenderOptions::default()
            },
        );

    let report = layer_tree_render(&render);
    assert_eq!(report.layer_count, 2);
    assert_eq!(
        report.layers,
        vec![
            BrowserLayer {
                id: 0,
                parent: None,
                kind: "root".to_owned(),
                reason: "document-root".to_owned(),
                x: 0,
                y: 0,
                width: 40,
                height: 4,
                paint_order: 0,
                command_indices: vec![0, 2],
            },
            BrowserLayer {
                id: 1,
                parent: Some(0),
                kind: "image".to_owned(),
                reason: "image-replaced-element".to_owned(),
                x: 0,
                y: 1,
                width: 10,
                height: 2,
                paint_order: 1,
                command_indices: vec![1],
            },
        ]
    );

    let metrics = browser_layer_metrics(&render);
    assert_eq!(
        metrics,
        BrowserLayerMetrics {
            layer_count: 2,
            root_command_count: 2,
            image_layer_count: 1,
            root_layer_width: 40,
            root_layer_height: 4,
            max_layer_area: 160,
            total_layer_area: 180,
        }
    );
}

#[test]
fn renders_block_border_as_paint_rects() {
    let render = render_html(
            "mem://page",
            br#"
            <html><head><style>.panel { background-color: #d0d0d0; border: 1px solid #404040; }</style></head>
            <body><div class="panel"><p>Bordered card</p><p>Inside text</p></div></body></html>
            "#,
            BrowserRenderOptions {
                width: 40,
                ..BrowserRenderOptions::default()
            },
        );

    assert_eq!(render.text, "Bordered card\nInside text");
    assert_eq!(
        render.display_list,
        vec![
            DisplayCommand::Rect {
                x: 0,
                y: 0,
                width: 40,
                height: 4,
                shade: 208
            },
            DisplayCommand::Rect {
                x: 0,
                y: 0,
                width: 40,
                height: 1,
                shade: 64
            },
            DisplayCommand::Rect {
                x: 0,
                y: 1,
                width: 1,
                height: 2,
                shade: 64
            },
            DisplayCommand::Rect {
                x: 39,
                y: 1,
                width: 1,
                height: 2,
                shade: 64
            },
            DisplayCommand::Rect {
                x: 0,
                y: 3,
                width: 40,
                height: 1,
                shade: 64
            },
            DisplayCommand::Text {
                x: 1,
                y: 1,
                text: "Bordered card".to_owned()
            },
            DisplayCommand::Text {
                x: 1,
                y: 2,
                text: "Inside text".to_owned()
            },
        ]
    );
    let raster = rasterize_render(&render, BrowserRasterOptions::default()).unwrap();
    assert!(raster.non_background_pixels() > 40 * 4);
}

#[test]
fn renders_block_padding_as_box_spacing() {
    let render = render_html(
            "mem://page",
            br#"
            <html><head><style>.panel { background-color: #d0d0d0; padding: 12px 16px; }</style></head>
            <body><div class="panel"><p>Padded text</p></div></body></html>
            "#,
            BrowserRenderOptions {
                width: 40,
                ..BrowserRenderOptions::default()
            },
        );

    assert_eq!(render.text, "Padded text");
    assert_eq!(
        render.display_list,
        vec![
            DisplayCommand::Rect {
                x: 0,
                y: 0,
                width: 40,
                height: 3,
                shade: 208
            },
            DisplayCommand::Text {
                x: 2,
                y: 1,
                text: "Padded text".to_owned()
            },
        ]
    );
    let raster = rasterize_render(&render, BrowserRasterOptions::default()).unwrap();
    assert!(raster.non_background_pixels() > 40 * 3);
}

#[test]
fn renders_block_margin_as_outer_box_spacing() {
    let render = render_html(
            "mem://page",
            br#"
            <html><head><style>.panel { background-color: #d0d0d0; margin: 12px 16px; }</style></head>
            <body><p>Before</p><div class="panel">Margin text</div><p>After</p></body></html>
            "#,
            BrowserRenderOptions {
                width: 40,
                ..BrowserRenderOptions::default()
            },
        );

    assert_eq!(render.text, "Before\nMargin text\nAfter");
    assert_eq!(
        render.display_list,
        vec![
            DisplayCommand::Rect {
                x: 2,
                y: 2,
                width: 36,
                height: 1,
                shade: 208
            },
            DisplayCommand::Text {
                x: 0,
                y: 0,
                text: "Before".to_owned()
            },
            DisplayCommand::Text {
                x: 2,
                y: 2,
                text: "Margin text".to_owned()
            },
            DisplayCommand::Text {
                x: 0,
                y: 4,
                text: "After".to_owned()
            },
        ]
    );
    let raster = rasterize_render(&render, BrowserRasterOptions::default()).unwrap();
    assert!(raster.non_background_pixels() > 36);
}

#[test]
fn renders_block_size_constraints() {
    let render = render_html(
            "mem://page",
            br#"
            <html><head><style>.panel { background-color: #d0d0d0; width: 160px; min-height: 36px; }</style></head>
            <body><div class="panel">Width constrained box</div></body></html>
            "#,
            BrowserRenderOptions {
                width: 40,
                ..BrowserRenderOptions::default()
            },
        );

    assert_eq!(render.text, "Width constrained\nbox");
    assert_eq!(
        render.display_list,
        vec![
            DisplayCommand::Rect {
                x: 0,
                y: 0,
                width: 20,
                height: 3,
                shade: 208
            },
            DisplayCommand::Text {
                x: 0,
                y: 0,
                text: "Width constrained".to_owned()
            },
            DisplayCommand::Text {
                x: 0,
                y: 1,
                text: "box".to_owned()
            },
        ]
    );
    let raster = rasterize_render(&render, BrowserRasterOptions::default()).unwrap();
    assert!(raster.non_background_pixels() > 20 * 3);
}

#[test]
fn renders_image_as_replaced_element_placeholder() {
    let render = render_html(
            "https://example.com/page.html",
            br#"<html><body><p>Before image</p><img src="hero.png" alt="Hero art" width="80" height="24"><p>After image</p></body></html>"#,
            BrowserRenderOptions {
                width: 40,
                ..BrowserRenderOptions::default()
            },
        );

    assert_eq!(render.text, "Before image\nAfter image");
    assert!(render.resources.iter().any(|resource| {
        resource.kind == "image"
            && resource.alt.as_deref() == Some("Hero art")
            && resource.resolved == "https://example.com/hero.png"
    }));
    assert_eq!(
        render.display_list,
        vec![
            DisplayCommand::Text {
                x: 0,
                y: 0,
                text: "Before image".to_owned()
            },
            DisplayCommand::Image {
                x: 0,
                y: 1,
                width: 10,
                height: 2,
                shade: 220,
                alt: Some("Hero art".to_owned()),
                url: None,
                decoded_width: None,
                decoded_height: None,
                decoded_hash: None
            },
            DisplayCommand::Text {
                x: 0,
                y: 3,
                text: "After image".to_owned()
            },
        ]
    );
    let raster = rasterize_render(&render, BrowserRasterOptions::default()).unwrap();
    assert!(raster.non_background_pixels() > 10 * 2);
}

#[test]
fn decodes_local_svg_image_into_raster_pixels() {
    let dir = tempfile::tempdir().unwrap();
    let page = dir.path().join("page.html");
    let svg = dir.path().join("tile.svg");
    fs::write(
        &svg,
        r##"<svg width="80" height="24" xmlns="http://www.w3.org/2000/svg">
                <rect x="0" y="0" width="80" height="24" fill="#f0f0f0"/>
                <rect x="8" y="6" width="64" height="12" fill="#303030"/>
            </svg>"##,
    )
    .unwrap();
    let source = page.display().to_string();
    let decoded = decoded_image_entry(&source, "tile.svg").unwrap().info();
    let render = render_html(
            &source,
            br#"<html><body><p>Before svg</p><img src="tile.svg" alt="SVG tile" width="80" height="24"><p>After svg</p></body></html>"#,
            BrowserRenderOptions {
                width: 40,
                ..BrowserRenderOptions::default()
            },
        );

    assert_eq!(render.text, "Before svg\nAfter svg");
    assert_eq!(render.decoded_images.len(), 1);
    assert_eq!(decoded.width, 80);
    assert_eq!(decoded.height, 24);
    assert_eq!(render.decoded_images[0].pixel_hash, decoded.pixel_hash);
    assert_eq!(
        render.display_list,
        vec![
            DisplayCommand::Text {
                x: 0,
                y: 0,
                text: "Before svg".to_owned()
            },
            DisplayCommand::Image {
                x: 0,
                y: 1,
                width: 10,
                height: 2,
                shade: 220,
                alt: Some("SVG tile".to_owned()),
                url: Some("tile.svg".to_owned()),
                decoded_width: Some(80),
                decoded_height: Some(24),
                decoded_hash: Some(decoded.pixel_hash)
            },
            DisplayCommand::Text {
                x: 0,
                y: 3,
                text: "After svg".to_owned()
            },
        ]
    );
    let raster = rasterize_render(&render, BrowserRasterOptions::default()).unwrap();
    assert!(raster.non_background_pixels() > 10 * 2);
    let raster_hash = raster.pixel_hash();
    fs::remove_file(svg).unwrap();
    let cached_raster = rasterize_render(&render, BrowserRasterOptions::default()).unwrap();
    assert_eq!(cached_raster.pixel_hash(), raster_hash);
}

#[tokio::test]
async fn image_svg_fidelity_decodes_viewbox_shape_pixels_for_rendered_resource() {
    let dir = tempfile::tempdir().unwrap();
    let page = dir.path().join("page.html");
    let icon = dir.path().join("icon.svg");
    fs::write(
        &icon,
        r##"<svg viewBox="0 0 20 12" xmlns="http://www.w3.org/2000/svg">
                <rect width="20" height="12" fill="#f0f0f0"/>
                <circle cx="6" cy="6" r="4" style="fill:#202020"/>
                <ellipse cx="15" cy="6" rx="3" ry="5" fill="#606060"/>
            </svg>"##,
    )
    .unwrap();
    fs::write(
        &page,
        r#"<html><body><p>Before svg</p><img src="icon.svg" alt="SVG icon" width="20" height="12"><p>After svg</p></body></html>"#,
    )
    .unwrap();

    let decoded = decode_image_reference(&page.display().to_string(), "icon.svg").unwrap();
    assert_eq!(decoded.width, 20);
    assert_eq!(decoded.height, 12);
    assert!(decoded.pixels.iter().any(|&pixel| pixel <= 40));
    assert!(
        decoded
            .pixels
            .iter()
            .any(|&pixel| (80..=110).contains(&pixel))
    );
    assert!(decoded.pixels.iter().any(|&pixel| pixel >= 230));
    let expected_hash = decoded.pixel_hash();

    let mut session = BrowserSession::new(BrowserRenderOptions {
        width: 40,
        ..BrowserRenderOptions::default()
    });
    session.navigate(&page.display().to_string()).await.unwrap();

    let report = session.render_current_with_images(1024).await.unwrap();
    assert_eq!(report.image_count, 1);
    assert_eq!(report.decoded, 1);
    assert_eq!(report.failed, 0);
    assert_eq!(report.decoded_image_bytes, decoded.pixels.len());
    let fetch = report.fetches.first().unwrap();
    assert_eq!(fetch.resource.kind, "image");
    assert_eq!(fetch.resource.initiator, "img");
    assert_eq!(fetch.resource.resolved, icon.display().to_string());
    assert_eq!(fetch.status, "fetched");
    assert_eq!(fetch.content_type.as_deref(), Some("image/svg+xml"));
    assert_eq!(fetch.image_decode_status.as_deref(), Some("decoded"));
    assert_eq!(fetch.decoded_width, Some(20));
    assert_eq!(fetch.decoded_height, Some(12));
    assert_eq!(fetch.decoded_hash.as_deref(), Some(expected_hash.as_str()));

    let render = session.current().unwrap();
    assert!(render.decoded_images.iter().any(|entry| {
        entry.width == 30 && entry.height == 20 && entry.pixel_hash == expected_hash
    }));
    assert!(render.display_list.iter().any(|command| {
        matches!(
            command,
            DisplayCommand::Image {
                url: Some(url),
                decoded_width: Some(20),
                decoded_height: Some(12),
                decoded_hash: Some(hash),
                ..
            } if url == &icon.display().to_string() && *hash == expected_hash
        )
    }));
}

#[tokio::test]
async fn image_svg_paths_decodes_line_shape_pixels_for_rendered_resource() {
    let dir = tempfile::tempdir().unwrap();
    let page = dir.path().join("page.html");
    let icon = dir.path().join("icon.svg");
    fs::write(
        &icon,
        r##"<svg viewBox="0 0 24 20" xmlns="http://www.w3.org/2000/svg">
                <rect width="24" height="20" fill="#f0f0f0"/>
                <path d="M 2 2 L 10 2 H 10 V 10 L 2 10 Z" style="fill:#202020"/>
                <polygon points="14,2 21,6 14,10" fill="#606060"/>
                <polyline points="3,16 10,12 20,17" fill="none" stroke="#404040"/>
            </svg>"##,
    )
    .unwrap();
    fs::write(
        &page,
        r#"<html><body><p>Before icon</p><img src="icon.svg" alt="Path icon" width="24" height="20"><p>After icon</p></body></html>"#,
    )
    .unwrap();

    let decoded = decode_image_reference(&page.display().to_string(), "icon.svg").unwrap();
    assert_eq!(decoded.width, 24);
    assert_eq!(decoded.height, 20);
    assert!(decoded.pixels.iter().any(|&pixel| pixel <= 40));
    assert!(
        decoded
            .pixels
            .iter()
            .any(|&pixel| (58..=72).contains(&pixel))
    );
    assert!(
        decoded
            .pixels
            .iter()
            .any(|&pixel| (88..=104).contains(&pixel))
    );
    assert!(decoded.pixels.iter().any(|&pixel| pixel >= 230));
    let expected_hash = decoded.pixel_hash();

    let mut session = BrowserSession::new(BrowserRenderOptions {
        width: 40,
        ..BrowserRenderOptions::default()
    });
    session.navigate(&page.display().to_string()).await.unwrap();

    let report = session.render_current_with_images(1024).await.unwrap();
    assert_eq!(report.image_count, 1);
    assert_eq!(report.decoded, 1);
    assert_eq!(report.failed, 0);
    assert_eq!(report.decoded_image_bytes, decoded.pixels.len());
    let fetch = report.fetches.first().unwrap();
    assert_eq!(fetch.resource.kind, "image");
    assert_eq!(fetch.resource.initiator, "img");
    assert_eq!(fetch.resource.resolved, icon.display().to_string());
    assert_eq!(fetch.status, "fetched");
    assert_eq!(fetch.content_type.as_deref(), Some("image/svg+xml"));
    assert_eq!(fetch.image_decode_status.as_deref(), Some("decoded"));
    assert_eq!(fetch.decoded_width, Some(24));
    assert_eq!(fetch.decoded_height, Some(20));
    assert_eq!(fetch.decoded_hash.as_deref(), Some(expected_hash.as_str()));

    let render = session.current().unwrap();
    assert_eq!(render.decoded_images.len(), 1);
    assert_eq!(render.decoded_images[0].pixel_hash, expected_hash);
    assert!(render.display_list.iter().any(|command| {
        matches!(
            command,
            DisplayCommand::Image {
                url: Some(url),
                decoded_width: Some(24),
                decoded_height: Some(20),
                decoded_hash: Some(hash),
                ..
            } if url == &icon.display().to_string() && *hash == expected_hash
        )
    }));
}

#[tokio::test]
async fn image_svg_curves_decodes_arc_quadratic_and_cubic_path_pixels_for_rendered_resource() {
    let dir = tempfile::tempdir().unwrap();
    let page = dir.path().join("page.html");
    let icon = dir.path().join("icon.svg");
    fs::write(
        &icon,
        r##"<svg viewBox="0 0 30 20" xmlns="http://www.w3.org/2000/svg">
                <rect width="30" height="20" fill="#f0f0f0"/>
                <path d="M 2 14 Q 7 2 12 14 T 22 14 Z" fill="#202020"/>
                <path d="M 16 3 C 19 0 26 0 28 6 S 24 16 17 17 Z" fill="#606060"/>
                <path d="M 1 1 A 4 4 0 0 1 9 1 L 9 6 L 1 6 Z" fill="#000000"/>
            </svg>"##,
    )
    .unwrap();
    fs::write(
        &page,
        r#"<html><body><p>Before curves</p><img src="icon.svg" alt="Curve icon" width="30" height="20"><p>After curves</p></body></html>"#,
    )
    .unwrap();

    let decoded = decode_image_reference(&page.display().to_string(), "icon.svg").unwrap();
    assert_eq!(decoded.width, 30);
    assert_eq!(decoded.height, 20);
    let sample = |x: usize, y: usize| decoded.pixels[y * decoded.width + x];
    assert!(sample(7, 11) <= 40);
    assert!((88..=104).contains(&sample(22, 8)));
    assert!(sample(5, 5) <= 40);
    let expected_hash = decoded.pixel_hash();
    let expected_color_hash = decoded.color_pixel_hash().unwrap();

    let mut session = BrowserSession::new(BrowserRenderOptions {
        width: 40,
        ..BrowserRenderOptions::default()
    });
    session.navigate(&page.display().to_string()).await.unwrap();

    let report = session.render_current_with_images(1024).await.unwrap();
    assert_eq!(report.image_count, 1);
    assert_eq!(report.decoded, 1);
    assert_eq!(report.failed, 0);
    assert_eq!(report.decoded_image_bytes, decoded.pixels.len());
    let fetch = report.fetches.first().unwrap();
    assert_eq!(fetch.resource.kind, "image");
    assert_eq!(fetch.resource.initiator, "img");
    assert_eq!(fetch.resource.resolved, icon.display().to_string());
    assert_eq!(fetch.status, "fetched");
    assert_eq!(fetch.content_type.as_deref(), Some("image/svg+xml"));
    assert_eq!(fetch.image_decode_status.as_deref(), Some("decoded"));
    assert_eq!(fetch.decoded_width, Some(30));
    assert_eq!(fetch.decoded_height, Some(20));
    assert_eq!(fetch.decoded_hash.as_deref(), Some(expected_hash.as_str()));
    assert_eq!(
        fetch.decoded_color_hash.as_deref(),
        Some(expected_color_hash.as_str())
    );
    assert_eq!(
        fetch.decoded_color_bytes,
        Some(decoded.width * decoded.height * 3)
    );

    let render = session.current().unwrap();
    assert!(
        render
            .decoded_images
            .iter()
            .any(|decoded| decoded.pixel_hash == expected_hash)
    );
    assert!(render.display_list.iter().any(|command| {
        matches!(
            command,
            DisplayCommand::Image {
                url: Some(url),
                decoded_width: Some(30),
                decoded_height: Some(20),
                decoded_hash: Some(hash),
                ..
            } if url == &icon.display().to_string() && *hash == expected_hash
        )
    }));
}

#[tokio::test]
async fn image_svg_transforms_decodes_transformed_shapes_for_rendered_resource() {
    let dir = tempfile::tempdir().unwrap();
    let page = dir.path().join("page.html");
    let icon = dir.path().join("icon.svg");
    fs::write(
        &icon,
        r##"<svg viewBox="0 0 24 20" xmlns="http://www.w3.org/2000/svg">
                <rect width="24" height="20" fill="#f0f0f0"/>
                <g transform="translate(4,2)">
                    <rect x="0" y="0" width="4" height="4" fill="#202020"/>
                    <path d="M 6 0 L 10 0 L 10 4 L 6 4 Z" transform="scale(1.5,1)" fill="#606060"/>
                </g>
                <polyline points="1,14 8,14" fill="none" stroke="#404040" stroke-width="3" transform="matrix(1 0 0 1 10 0)"/>
                <rect x="0" y="16" width="4" height="3" fill="#000000" transform="rotate(45)"/>
            </svg>"##,
    )
    .unwrap();
    fs::write(
        &page,
        r#"<html><body><p>Before transforms</p><img src="icon.svg" alt="Transform icon" width="24" height="20"><p>After transforms</p></body></html>"#,
    )
    .unwrap();

    let decoded = decode_image_reference(&page.display().to_string(), "icon.svg").unwrap();
    assert_eq!(decoded.width, 24);
    assert_eq!(decoded.height, 20);
    let sample = |x: usize, y: usize| decoded.pixels[y * decoded.width + x];
    assert!(sample(5, 3) <= 40);
    assert!((88..=104).contains(&sample(15, 3)));
    assert!((58..=72).contains(&sample(14, 14)));
    assert!(sample(1, 17) >= 230);
    let expected_hash = decoded.pixel_hash();
    let expected_color_hash = decoded.color_pixel_hash().unwrap();

    let mut session = BrowserSession::new(BrowserRenderOptions {
        width: 40,
        ..BrowserRenderOptions::default()
    });
    session.navigate(&page.display().to_string()).await.unwrap();

    let report = session.render_current_with_images(1024).await.unwrap();
    assert_eq!(report.image_count, 1);
    assert_eq!(report.decoded, 1);
    assert_eq!(report.failed, 0);
    assert_eq!(report.decoded_image_bytes, decoded.pixels.len());
    let fetch = report.fetches.first().unwrap();
    assert_eq!(fetch.resource.kind, "image");
    assert_eq!(fetch.resource.initiator, "img");
    assert_eq!(fetch.resource.resolved, icon.display().to_string());
    assert_eq!(fetch.status, "fetched");
    assert_eq!(fetch.content_type.as_deref(), Some("image/svg+xml"));
    assert_eq!(fetch.image_decode_status.as_deref(), Some("decoded"));
    assert_eq!(fetch.decoded_width, Some(24));
    assert_eq!(fetch.decoded_height, Some(20));
    assert_eq!(fetch.decoded_hash.as_deref(), Some(expected_hash.as_str()));
    assert_eq!(
        fetch.decoded_color_hash.as_deref(),
        Some(expected_color_hash.as_str())
    );
    assert_eq!(
        fetch.decoded_color_bytes,
        Some(decoded.width * decoded.height * 3)
    );

    let render = session.current().unwrap();
    assert_eq!(render.decoded_images.len(), 1);
    assert_eq!(render.decoded_images[0].pixel_hash, expected_hash);
    assert!(render.display_list.iter().any(|command| {
        matches!(
            command,
            DisplayCommand::Image {
                url: Some(url),
                decoded_width: Some(24),
                decoded_height: Some(20),
                decoded_hash: Some(hash),
                ..
            } if url == &icon.display().to_string() && *hash == expected_hash
        )
    }));
}

#[tokio::test]
async fn image_real_color_svg_preserves_rgb_pixels_for_rendered_resource() {
    let dir = tempfile::tempdir().unwrap();
    let page = dir.path().join("page.html");
    let icon = dir.path().join("color-icon.svg");
    fs::write(
        &icon,
        r##"<svg viewBox="0 0 4 2" xmlns="http://www.w3.org/2000/svg">
                <rect width="2" height="2" fill="red"/>
                <rect x="2" width="2" height="2" style="fill:rgb(0,0,255)"/>
            </svg>"##,
    )
    .unwrap();
    fs::write(
        &page,
        r#"<html><body><p>Before color svg</p><img src="color-icon.svg" alt="Color SVG" width="16" height="8"><p>After color svg</p></body></html>"#,
    )
    .unwrap();

    let decoded = decode_image_reference(&page.display().to_string(), "color-icon.svg").unwrap();
    assert_eq!(decoded.width, 4);
    assert_eq!(decoded.height, 2);
    let rgb_pixels = decoded.rgb_pixels.as_ref().unwrap();
    assert_eq!(rgb_pixels.len(), decoded.width * decoded.height * 3);
    assert!(rgb_pixels.chunks_exact(3).any(|pixel| pixel == [255, 0, 0]));
    assert!(rgb_pixels.chunks_exact(3).any(|pixel| pixel == [0, 0, 255]));
    let expected_hash = decoded.pixel_hash();
    let expected_color_hash = decoded.color_pixel_hash().unwrap();

    let mut session = BrowserSession::new(BrowserRenderOptions {
        width: 40,
        ..BrowserRenderOptions::default()
    });
    session.navigate(&page.display().to_string()).await.unwrap();

    let report = session.render_current_with_images(1024).await.unwrap();
    assert_eq!(report.image_count, 1);
    assert_eq!(report.decoded, 1);
    assert_eq!(report.failed, 0);
    let fetch = report.fetches.first().unwrap();
    assert_eq!(fetch.resource.kind, "image");
    assert_eq!(fetch.resource.initiator, "img");
    assert_eq!(fetch.resource.resolved, icon.display().to_string());
    assert_eq!(fetch.status, "fetched");
    assert_eq!(fetch.content_type.as_deref(), Some("image/svg+xml"));
    assert_eq!(fetch.image_decode_status.as_deref(), Some("decoded"));
    assert_eq!(fetch.decoded_width, Some(4));
    assert_eq!(fetch.decoded_height, Some(2));
    assert_eq!(fetch.decoded_hash.as_deref(), Some(expected_hash.as_str()));
    assert_eq!(
        fetch.decoded_color_hash.as_deref(),
        Some(expected_color_hash.as_str())
    );
    assert_eq!(
        fetch.decoded_color_bytes,
        Some(decoded.width * decoded.height * 3)
    );

    let render = session.current().unwrap();
    let rendered_image = render
        .decoded_images
        .iter()
        .find(|image| image.pixel_hash == expected_hash)
        .unwrap();
    assert_eq!(
        rendered_image.image.rgb_pixels.as_deref(),
        decoded.rgb_pixels.as_deref()
    );
    assert!(render.display_list.iter().any(|command| {
        matches!(
            command,
            DisplayCommand::Image {
                url: Some(url),
                decoded_width: Some(4),
                decoded_height: Some(2),
                decoded_hash: Some(hash),
                ..
            } if url == &icon.display().to_string() && *hash == expected_hash
        )
    }));
}

#[tokio::test]
async fn image_visibility_fidelity_svg_current_color_decodes_and_attaches_pixels() {
    let dir = tempfile::tempdir().unwrap();
    let page = dir.path().join("page.html");
    let icon = dir.path().join("current-color.svg");
    fs::write(
        &icon,
        r##"<svg viewBox="0 0 8 8" xmlns="http://www.w3.org/2000/svg">
                <rect width="8" height="8" fill="white"/>
                <path d="M 1 1 L 7 1 L 7 7 L 1 7 Z" fill="currentColor"/>
                <polyline points="1,7 4,3 7,7" fill="none" stroke="currentColor" stroke-width="2"/>
            </svg>"##,
    )
    .unwrap();
    fs::write(
        &page,
        r#"<html><body><p>Before current color</p><img src="current-color.svg" alt="Current color SVG" width="16" height="16"><p>After current color</p></body></html>"#,
    )
    .unwrap();

    let decoded = decode_image_reference(&page.display().to_string(), "current-color.svg").unwrap();
    assert_eq!(decoded.width, 8);
    assert_eq!(decoded.height, 8);
    assert!(decoded.pixels.iter().any(|&pixel| pixel <= 10));
    assert!(decoded.pixels.iter().any(|&pixel| pixel >= 240));
    let rgb_pixels = decoded.rgb_pixels.as_ref().unwrap();
    assert!(rgb_pixels.chunks_exact(3).any(|pixel| pixel == [0, 0, 0]));
    let expected_hash = decoded.pixel_hash();
    let expected_color_hash = decoded.color_pixel_hash().unwrap();

    let mut session = BrowserSession::new(BrowserRenderOptions {
        width: 40,
        ..BrowserRenderOptions::default()
    });
    session.navigate(&page.display().to_string()).await.unwrap();

    let report = session.render_current_with_images(1024).await.unwrap();
    assert_eq!(report.image_count, 1);
    assert_eq!(report.decoded, 1);
    assert_eq!(report.failed, 0);
    let fetch = report.fetches.first().unwrap();
    assert_eq!(fetch.resource.kind, "image");
    assert_eq!(fetch.resource.initiator, "img");
    assert_eq!(fetch.resource.resolved, icon.display().to_string());
    assert_eq!(fetch.status, "fetched");
    assert_eq!(fetch.content_type.as_deref(), Some("image/svg+xml"));
    assert_eq!(fetch.image_decode_status.as_deref(), Some("decoded"));
    assert_eq!(fetch.decoded_width, Some(8));
    assert_eq!(fetch.decoded_height, Some(8));
    assert_eq!(fetch.decoded_hash.as_deref(), Some(expected_hash.as_str()));
    assert_eq!(
        fetch.decoded_color_hash.as_deref(),
        Some(expected_color_hash.as_str())
    );

    let render = session.current().unwrap();
    let rendered_image = render
        .decoded_images
        .iter()
        .find(|image| image.pixel_hash == expected_hash)
        .unwrap();
    assert_eq!(
        rendered_image.image.rgb_pixels.as_deref(),
        decoded.rgb_pixels.as_deref()
    );
    assert!(render.display_list.iter().any(|command| {
        matches!(
            command,
            DisplayCommand::Image {
                url: Some(url),
                decoded_width: Some(8),
                decoded_height: Some(8),
                decoded_hash: Some(hash),
                ..
            } if url == &icon.display().to_string() && *hash == expected_hash
        )
    }));
}

#[tokio::test]
async fn image_color_fidelity_svg_opacity_blends_rgb_pixels_for_rendered_resource() {
    let dir = tempfile::tempdir().unwrap();
    let page = dir.path().join("page.html");
    let icon = dir.path().join("opacity.svg");
    fs::write(
        &icon,
        r##"<svg viewBox="0 0 8 4" xmlns="http://www.w3.org/2000/svg">
                <rect width="8" height="4" fill="white"/>
                <rect width="4" height="4" fill="red" fill-opacity="0.5"/>
                <rect x="4" width="4" height="4" style="fill: blue; opacity: 0.5"/>
            </svg>"##,
    )
    .unwrap();
    fs::write(
        &page,
        r#"<html><body><p>Before opacity</p><img src="opacity.svg" alt="Opacity SVG" width="16" height="8"><p>After opacity</p></body></html>"#,
    )
    .unwrap();

    let decoded = decode_image_reference(&page.display().to_string(), "opacity.svg").unwrap();
    assert_eq!(decoded.width, 8);
    assert_eq!(decoded.height, 4);
    let rgb_pixels = decoded.rgb_pixels.as_ref().unwrap();
    assert!(rgb_pixels.chunks_exact(3).any(|pixel| {
        pixel[0] >= 250 && (120..=135).contains(&pixel[1]) && (120..=135).contains(&pixel[2])
    }));
    assert!(rgb_pixels.chunks_exact(3).any(|pixel| {
        (120..=135).contains(&pixel[0]) && (120..=135).contains(&pixel[1]) && pixel[2] >= 250
    }));
    let expected_hash = decoded.pixel_hash();
    let expected_color_hash = decoded.color_pixel_hash().unwrap();

    let mut session = BrowserSession::new(BrowserRenderOptions {
        width: 40,
        ..BrowserRenderOptions::default()
    });
    session.navigate(&page.display().to_string()).await.unwrap();

    let report = session.render_current_with_images(1024).await.unwrap();
    assert_eq!(report.image_count, 1);
    assert_eq!(report.decoded, 1);
    assert_eq!(report.failed, 0);
    let fetch = report.fetches.first().unwrap();
    assert_eq!(fetch.resource.kind, "image");
    assert_eq!(fetch.resource.initiator, "img");
    assert_eq!(fetch.resource.resolved, icon.display().to_string());
    assert_eq!(fetch.status, "fetched");
    assert_eq!(fetch.content_type.as_deref(), Some("image/svg+xml"));
    assert_eq!(fetch.image_decode_status.as_deref(), Some("decoded"));
    assert_eq!(fetch.decoded_width, Some(8));
    assert_eq!(fetch.decoded_height, Some(4));
    assert_eq!(fetch.decoded_hash.as_deref(), Some(expected_hash.as_str()));
    assert_eq!(
        fetch.decoded_color_hash.as_deref(),
        Some(expected_color_hash.as_str())
    );

    let render = session.current().unwrap();
    let rendered_image = render
        .decoded_images
        .iter()
        .find(|image| image.pixel_hash == expected_hash)
        .unwrap();
    assert_eq!(
        rendered_image.image.rgb_pixels.as_deref(),
        decoded.rgb_pixels.as_deref()
    );
    assert!(render.display_list.iter().any(|command| {
        matches!(
            command,
            DisplayCommand::Image {
                url: Some(url),
                decoded_width: Some(8),
                decoded_height: Some(4),
                decoded_hash: Some(hash),
                ..
            } if url == &icon.display().to_string() && *hash == expected_hash
        )
    }));
}

#[tokio::test]
async fn image_inline_color_svg_data_named_colors_decode_and_attach() {
    let dir = tempfile::tempdir().unwrap();
    let page = dir.path().join("page.html");
    let data_url = "data:image/svg+xml,%3Csvg%20viewBox%3D%220%200%204%202%22%20xmlns%3D%22http%3A%2F%2Fwww.w3.org%2F2000%2Fsvg%22%3E%3Crect%20width%3D%222%22%20height%3D%222%22%20fill%3D%22orange%22%2F%3E%3Crect%20x%3D%222%22%20width%3D%222%22%20height%3D%222%22%20fill%3D%22rebeccapurple%22%2F%3E%3C%2Fsvg%3E";
    let decoded = decode_image_reference("mem://inline-color", data_url).unwrap();
    assert_eq!(decoded.width, 4);
    assert_eq!(decoded.height, 2);
    let rgb_pixels = decoded.rgb_pixels.as_ref().unwrap();
    assert!(
        rgb_pixels
            .chunks_exact(3)
            .any(|pixel| pixel == [255, 165, 0])
    );
    assert!(
        rgb_pixels
            .chunks_exact(3)
            .any(|pixel| pixel == [102, 51, 153])
    );
    let expected_hash = decoded.pixel_hash();
    let expected_color_hash = decoded.color_pixel_hash().unwrap();

    let html = format!(
        r#"<html><body><p>Before data svg</p><img src="{data_url}" alt="Named color data SVG" width="16" height="8"><p>After data svg</p></body></html>"#
    );
    fs::write(&page, html).unwrap();
    let mut session = BrowserSession::new(BrowserRenderOptions {
        width: 40,
        ..BrowserRenderOptions::default()
    });
    session.navigate(&page.display().to_string()).await.unwrap();

    let report = session.render_current_with_images(1024).await.unwrap();
    assert_eq!(report.image_count, 1);
    assert_eq!(report.decoded, 1);
    assert_eq!(report.failed, 0);
    let fetch = report.fetches.first().unwrap();
    assert_eq!(fetch.resource.kind, "image");
    assert_eq!(fetch.resource.initiator, "img");
    assert_eq!(fetch.resource.url, data_url);
    assert_eq!(fetch.status, "cached");
    assert_eq!(fetch.content_type.as_deref(), Some("image/svg+xml"));
    assert_eq!(fetch.image_decode_status.as_deref(), Some("decoded"));
    assert_eq!(fetch.decoded_width, Some(4));
    assert_eq!(fetch.decoded_height, Some(2));
    assert_eq!(fetch.decoded_hash.as_deref(), Some(expected_hash.as_str()));
    assert_eq!(
        fetch.decoded_color_hash.as_deref(),
        Some(expected_color_hash.as_str())
    );

    let render = session.current().unwrap();
    let rendered_image = render
        .decoded_images
        .iter()
        .find(|image| image.pixel_hash == expected_hash)
        .unwrap();
    assert_eq!(
        rendered_image.image.rgb_pixels.as_deref(),
        decoded.rgb_pixels.as_deref()
    );
    assert!(render.display_list.iter().any(|command| {
        matches!(
            command,
            DisplayCommand::Image {
                url: Some(url),
                decoded_width: Some(4),
                decoded_height: Some(2),
                decoded_hash: Some(hash),
                ..
            } if url == data_url && *hash == expected_hash
        )
    }));
}

#[tokio::test]
async fn image_real_visibility_svg_data_hsl_colors_decode_and_attach() {
    let dir = tempfile::tempdir().unwrap();
    let page = dir.path().join("page.html");
    let data_url = "data:image/svg+xml,%3Csvg%20viewBox%3D%220%200%206%202%22%20xmlns%3D%22http%3A%2F%2Fwww.w3.org%2F2000%2Fsvg%22%3E%3Crect%20width%3D%222%22%20height%3D%222%22%20fill%3D%22hsl(120%20100%25%2050%25)%22%2F%3E%3Crect%20x%3D%222%22%20width%3D%222%22%20height%3D%222%22%20fill%3D%22hsla(240%2C100%25%2C50%25%2C0.5)%22%2F%3E%3Crect%20x%3D%224%22%20width%3D%222%22%20height%3D%222%22%20fill%3D%22hsl(0.833333turn%20100%25%2050%25)%22%2F%3E%3C%2Fsvg%3E";
    let decoded = decode_image_reference("mem://hsl-color", data_url).unwrap();
    assert_eq!(decoded.width, 6);
    assert_eq!(decoded.height, 2);
    let rgb_pixels = decoded.rgb_pixels.as_ref().unwrap();
    assert!(rgb_pixels.chunks_exact(3).any(|pixel| pixel == [0, 255, 0]));
    assert!(rgb_pixels.chunks_exact(3).any(|pixel| pixel == [0, 0, 255]));
    assert!(
        rgb_pixels
            .chunks_exact(3)
            .any(|pixel| pixel[0] >= 250 && pixel[1] <= 5 && pixel[2] >= 250)
    );
    let expected_hash = decoded.pixel_hash();
    let expected_color_hash = decoded.color_pixel_hash().unwrap();

    let html = format!(
        r#"<html><body><p>Before hsl svg</p><img src="{data_url}" alt="HSL data SVG" width="18" height="6"><p>After hsl svg</p></body></html>"#
    );
    fs::write(&page, html).unwrap();
    let mut session = BrowserSession::new(BrowserRenderOptions {
        width: 40,
        ..BrowserRenderOptions::default()
    });
    session.navigate(&page.display().to_string()).await.unwrap();

    let report = session.render_current_with_images(1024).await.unwrap();
    assert_eq!(report.image_count, 1);
    assert_eq!(report.decoded, 1);
    assert_eq!(report.failed, 0);
    let fetch = report.fetches.first().unwrap();
    assert_eq!(fetch.resource.kind, "image");
    assert_eq!(fetch.resource.initiator, "img");
    assert_eq!(fetch.resource.url, data_url);
    assert_eq!(fetch.status, "cached");
    assert_eq!(fetch.content_type.as_deref(), Some("image/svg+xml"));
    assert_eq!(fetch.image_decode_status.as_deref(), Some("decoded"));
    assert_eq!(fetch.decoded_width, Some(6));
    assert_eq!(fetch.decoded_height, Some(2));
    assert_eq!(fetch.decoded_hash.as_deref(), Some(expected_hash.as_str()));
    assert_eq!(
        fetch.decoded_color_hash.as_deref(),
        Some(expected_color_hash.as_str())
    );

    let render = session.current().unwrap();
    let rendered_image = render
        .decoded_images
        .iter()
        .find(|image| image.pixel_hash == expected_hash)
        .unwrap();
    assert_eq!(
        rendered_image.image.rgb_pixels.as_deref(),
        decoded.rgb_pixels.as_deref()
    );
    assert!(render.display_list.iter().any(|command| {
        matches!(
            command,
            DisplayCommand::Image {
                url: Some(url),
                decoded_width: Some(6),
                decoded_height: Some(2),
                decoded_hash: Some(hash),
                ..
            } if url == data_url && *hash == expected_hash
        )
    }));
}

#[tokio::test]
async fn image_real_parity_sloppy_data_svg_percent_colors_decode_and_attach() {
    let dir = tempfile::tempdir().unwrap();
    let page = dir.path().join("page.html");
    let data_url = "data:image/svg+xml,%3Csvg%20viewBox%3D%220%200%204%202%22%20xmlns%3D%22http%3A%2F%2Fwww.w3.org%2F2000%2Fsvg%22%3E%3Crect%20width%3D%222%22%20height%3D%222%22%20fill%3D%22hsl(120%20100%%2050%)%22%2F%3E%3Crect%20x%3D%222%22%20width%3D%222%22%20height%3D%222%22%20fill%3D%22hsl(240%20100%%2050%)%22%2F%3E%3C%2Fsvg%3E";
    let decoded = decode_image_reference("mem://sloppy-hsl-data-svg", data_url).unwrap();
    assert_eq!(decoded.width, 4);
    assert_eq!(decoded.height, 2);
    let rgb_pixels = decoded.rgb_pixels.as_ref().unwrap();
    assert!(rgb_pixels.chunks_exact(3).any(|pixel| pixel == [0, 255, 0]));
    assert!(rgb_pixels.chunks_exact(3).any(|pixel| pixel == [0, 0, 255]));
    let expected_hash = decoded.pixel_hash();
    let expected_color_hash = decoded.color_pixel_hash().unwrap();

    let html = format!(
        r#"<html><body><p>Before sloppy svg</p><img src="{data_url}" alt="Sloppy HSL data SVG" width="16" height="8"><p>After sloppy svg</p></body></html>"#
    );
    fs::write(&page, html).unwrap();
    let mut session = BrowserSession::new(BrowserRenderOptions {
        width: 40,
        ..BrowserRenderOptions::default()
    });
    session.navigate(&page.display().to_string()).await.unwrap();

    let report = session.render_current_with_images(1024).await.unwrap();
    assert_eq!(report.image_count, 1);
    assert_eq!(report.decoded, 1);
    assert_eq!(report.failed, 0);
    let fetch = report.fetches.first().unwrap();
    assert_eq!(fetch.resource.kind, "image");
    assert_eq!(fetch.resource.initiator, "img");
    assert_eq!(fetch.resource.url, data_url);
    assert_eq!(fetch.status, "cached");
    assert_eq!(fetch.content_type.as_deref(), Some("image/svg+xml"));
    assert_eq!(fetch.image_decode_status.as_deref(), Some("decoded"));
    assert_eq!(fetch.decoded_width, Some(4));
    assert_eq!(fetch.decoded_height, Some(2));
    assert_eq!(fetch.decoded_hash.as_deref(), Some(expected_hash.as_str()));
    assert_eq!(
        fetch.decoded_color_hash.as_deref(),
        Some(expected_color_hash.as_str())
    );

    let render = session.current().unwrap();
    assert!(render.text.contains("Before sloppy svg"));
    assert!(render.text.contains("After sloppy svg"));
    let rendered_image = render
        .decoded_images
        .iter()
        .find(|image| image.pixel_hash == expected_hash)
        .unwrap();
    assert_eq!(
        rendered_image.image.rgb_pixels.as_deref(),
        decoded.rgb_pixels.as_deref()
    );
    assert!(render.display_list.iter().any(|command| {
        matches!(
            command,
            DisplayCommand::Image {
                url: Some(url),
                decoded_width: Some(4),
                decoded_height: Some(2),
                decoded_hash: Some(hash),
                ..
            } if url == data_url && *hash == expected_hash
        )
    }));
}

#[tokio::test]
async fn image_real_page_srcset_data_svg_commas_decode_color_and_attach() {
    let dir = tempfile::tempdir().unwrap();
    let page = dir.path().join("page.html");
    let data_url = "data:image/svg+xml,%3Csvg%20viewBox%3D%220%200%204%202%22%20xmlns%3D%22http%3A%2F%2Fwww.w3.org%2F2000%2Fsvg%22%3E%3Crect%20width%3D%222%22%20height%3D%222%22%20fill%3D%22rgb(255,0,0)%22%2F%3E%3Crect%20x%3D%222%22%20width%3D%222%22%20height%3D%222%22%20fill%3D%22rgb(0,0,255)%22%2F%3E%3C%2Fsvg%3E";
    let decoded = decode_image_reference("mem://srcset-data-svg", data_url).unwrap();
    assert_eq!(decoded.width, 4);
    assert_eq!(decoded.height, 2);
    let rgb_pixels = decoded.rgb_pixels.as_ref().unwrap();
    assert!(rgb_pixels.chunks_exact(3).any(|pixel| pixel == [255, 0, 0]));
    assert!(rgb_pixels.chunks_exact(3).any(|pixel| pixel == [0, 0, 255]));
    let expected_hash = decoded.pixel_hash();
    let expected_color_hash = decoded.color_pixel_hash().unwrap();

    let html = format!(
        r#"<html><body><p>Before srcset svg</p><img src="/assets/loading.gif" srcset="{data_url}, fallback.webp 2x" alt="Srcset data SVG" width="16" height="8"><p>After srcset svg</p></body></html>"#
    );
    fs::write(&page, html).unwrap();
    let mut session = BrowserSession::new(BrowserRenderOptions {
        width: 40,
        ..BrowserRenderOptions::default()
    });
    session.navigate(&page.display().to_string()).await.unwrap();

    let report = session.render_current_with_images(1024).await.unwrap();
    assert_eq!(report.image_count, 1);
    assert_eq!(report.decoded, 1);
    assert!(
        !report
            .fetches
            .iter()
            .any(|fetch| fetch.resource.url == "/assets/loading.gif")
    );
    assert!(
        !report
            .fetches
            .iter()
            .any(|fetch| fetch.resource.url == "fallback.webp")
    );
    let fetch = report
        .fetches
        .iter()
        .find(|fetch| fetch.resource.url == data_url)
        .unwrap();
    assert_eq!(fetch.resource.kind, "image");
    assert_eq!(fetch.resource.initiator, "img");
    assert_eq!(fetch.status, "cached");
    assert_eq!(fetch.content_type.as_deref(), Some("image/svg+xml"));
    assert_eq!(fetch.image_decode_status.as_deref(), Some("decoded"));
    assert_eq!(fetch.decoded_width, Some(4));
    assert_eq!(fetch.decoded_height, Some(2));
    assert_eq!(fetch.decoded_hash.as_deref(), Some(expected_hash.as_str()));
    assert_eq!(
        fetch.decoded_color_hash.as_deref(),
        Some(expected_color_hash.as_str())
    );

    let render = session.current().unwrap();
    assert!(render.text.contains("Before srcset svg"));
    assert!(render.text.contains("After srcset svg"));
    let rendered_image = render
        .decoded_images
        .iter()
        .find(|image| image.pixel_hash == expected_hash)
        .unwrap();
    assert_eq!(
        rendered_image.image.rgb_pixels.as_deref(),
        decoded.rgb_pixels.as_deref()
    );
    assert!(render.display_list.iter().any(|command| {
        matches!(
            command,
            DisplayCommand::Image {
                url: Some(url),
                decoded_width: Some(4),
                decoded_height: Some(2),
                decoded_hash: Some(hash),
                ..
            } if url == data_url && *hash == expected_hash
        )
    }));
}

#[tokio::test]
async fn image_resource_discovery_svg_inherits_group_paint_and_current_color() {
    let dir = tempfile::tempdir().unwrap();
    let page = dir.path().join("page.html");
    let data_url = "data:image/svg+xml,%3Csvg%20viewBox%3D%220%200%206%202%22%20xmlns%3D%22http%3A%2F%2Fwww.w3.org%2F2000%2Fsvg%22%20style%3D%22color%3Ahsl(30%20100%25%2050%25)%22%20fill%3D%22currentColor%22%3E%3Cg%20opacity%3D%220.5%22%3E%3Cpath%20d%3D%22M0%200%20H2%20V2%20H0%20Z%22%2F%3E%3C%2Fg%3E%3Cg%20style%3D%22fill%3Argb(0%20128%20255)%22%3E%3Crect%20x%3D%222%22%20width%3D%222%22%20height%3D%222%22%2F%3E%3C%2Fg%3E%3Cg%20fill%3D%22lime%22%3E%3Cpath%20d%3D%22M4%200%20H6%20V2%20H4%20Z%22%2F%3E%3C%2Fg%3E%3C%2Fsvg%3E";
    let decoded = decode_image_reference("mem://inherited-svg-paint", data_url).unwrap();
    assert_eq!(decoded.width, 6);
    assert_eq!(decoded.height, 2);
    let rgb_pixels = decoded.rgb_pixels.as_ref().unwrap();
    assert!(
        rgb_pixels
            .chunks_exact(3)
            .any(|pixel| pixel[0] >= 250 && (185..=195).contains(&pixel[1]) && pixel[2] <= 130)
    );
    assert!(
        rgb_pixels
            .chunks_exact(3)
            .any(|pixel| pixel == [0, 128, 255])
    );
    assert!(rgb_pixels.chunks_exact(3).any(|pixel| pixel == [0, 255, 0]));
    let expected_hash = decoded.pixel_hash();
    let expected_color_hash = decoded.color_pixel_hash().unwrap();

    let html = format!(
        r#"<html><body><p>Before inherited svg</p><img src="{data_url}" alt="Inherited SVG paint" width="18" height="6"><p>After inherited svg</p></body></html>"#
    );
    fs::write(&page, html).unwrap();
    let mut session = BrowserSession::new(BrowserRenderOptions {
        width: 40,
        ..BrowserRenderOptions::default()
    });
    session.navigate(&page.display().to_string()).await.unwrap();

    let report = session.render_current_with_images(1024).await.unwrap();
    assert_eq!(report.image_count, 1);
    assert_eq!(report.decoded, 1);
    assert_eq!(report.failed, 0);
    let fetch = report.fetches.first().unwrap();
    assert_eq!(fetch.resource.kind, "image");
    assert_eq!(fetch.resource.initiator, "img");
    assert_eq!(fetch.resource.url, data_url);
    assert_eq!(fetch.status, "cached");
    assert_eq!(fetch.content_type.as_deref(), Some("image/svg+xml"));
    assert_eq!(fetch.image_decode_status.as_deref(), Some("decoded"));
    assert_eq!(fetch.decoded_width, Some(6));
    assert_eq!(fetch.decoded_height, Some(2));
    assert_eq!(fetch.decoded_hash.as_deref(), Some(expected_hash.as_str()));
    assert_eq!(
        fetch.decoded_color_hash.as_deref(),
        Some(expected_color_hash.as_str())
    );

    let render = session.current().unwrap();
    let rendered_image = render
        .decoded_images
        .iter()
        .find(|image| image.pixel_hash == expected_hash)
        .unwrap();
    assert_eq!(
        rendered_image.image.rgb_pixels.as_deref(),
        decoded.rgb_pixels.as_deref()
    );
    assert!(render.display_list.iter().any(|command| {
        matches!(
            command,
            DisplayCommand::Image {
                url: Some(url),
                decoded_width: Some(6),
                decoded_height: Some(2),
                decoded_hash: Some(hash),
                ..
            } if url == data_url && *hash == expected_hash
        )
    }));
}

#[test]
fn image_load_buttons_attach_product_gallery_lazy_sources() {
    let dir = tempfile::tempdir().unwrap();
    let page = dir.path().join("page.html");
    let hero = dir.path().join("hero.webp");
    let zoom = dir.path().join("zoom.webp");
    let product = dir.path().join("product.webp");
    fs::write(&hero, tiny_test_webp_bytes()).unwrap();
    fs::write(&zoom, tiny_test_webp_bytes()).unwrap();
    fs::write(&product, tiny_test_webp_bytes()).unwrap();

    let source = page.display().to_string();
    let hero_hash = decoded_image_entry(&source, "hero.webp")
        .unwrap()
        .pixel_hash;
    let zoom_hash = decoded_image_entry(&source, "zoom.webp")
        .unwrap()
        .pixel_hash;
    let product_hash = decoded_image_entry(&source, "product.webp")
        .unwrap()
        .pixel_hash;
    let render = render_html(
        &source,
        br#"<html><body>
            <img src="/assets/loading.svg" data-large-image="hero.webp" alt="Large hero" width="80" height="24">
            <img src="/assets/loader.png" data-zoom-src="zoom.webp" alt="Zoom hero" width="80" height="24">
            <picture>
                <source type="image/webp" data-product-srcset="product.avif 320w, product.webp 640w">
                <img src="/assets/blank.gif" alt="Product picture" width="80" height="24">
            </picture>
        </body></html>"#,
        BrowserRenderOptions {
            width: 40,
            ..BrowserRenderOptions::default()
        },
    );

    assert_eq!(render.decoded_images.len(), 3);
    assert!(render.display_list.iter().any(|command| {
        matches!(
            command,
            DisplayCommand::Image {
                url: Some(url),
                decoded_hash: Some(hash),
                ..
            } if url == "hero.webp" && *hash == hero_hash
        )
    }));
    assert!(render.display_list.iter().any(|command| {
        matches!(
            command,
            DisplayCommand::Image {
                url: Some(url),
                decoded_hash: Some(hash),
                ..
            } if url == "zoom.webp" && *hash == zoom_hash
        )
    }));
    assert!(render.display_list.iter().any(|command| {
        matches!(
            command,
            DisplayCommand::Image {
                url: Some(url),
                decoded_hash: Some(hash),
                ..
            } if url == "product.webp" && *hash == product_hash
        )
    }));
    assert!(!render.display_list.iter().any(|command| {
        matches!(
            command,
            DisplayCommand::Image {
                url: Some(url),
                ..
            } if url.ends_with(".avif") || url.contains("/assets/")
        )
    }));
}

#[test]
fn decodes_local_png_image_into_cached_raster_pixels() {
    let dir = tempfile::tempdir().unwrap();
    let page = dir.path().join("page.html");
    let png = dir.path().join("tile.png");
    let png_bytes = tiny_test_png_rgb_with_sub_filter();
    fs::write(&png, &png_bytes).unwrap();

    let decoded = decode_simple_png(&png_bytes).unwrap();
    assert_eq!(decoded.width, 2);
    assert_eq!(decoded.height, 2);
    assert_eq!(decoded.pixels, vec![0, 255, 77, 29]);

    let source = page.display().to_string();
    let decoded_info = decoded_image_entry(&source, "tile.png").unwrap().info();
    let render = render_html(
            &source,
            br#"<html><body><p>Before png</p><img src="tile.png" alt="PNG tile" width="16" height="24"><p>After png</p></body></html>"#,
            BrowserRenderOptions {
                width: 40,
                ..BrowserRenderOptions::default()
            },
        );

    assert_eq!(render.text, "Before png\nAfter png");
    assert_eq!(render.decoded_images.len(), 1);
    assert_eq!(decoded_info.width, 2);
    assert_eq!(decoded_info.height, 2);
    assert_eq!(render.decoded_images[0].pixel_hash, decoded_info.pixel_hash);
    assert_eq!(
        render.display_list,
        vec![
            DisplayCommand::Text {
                x: 0,
                y: 0,
                text: "Before png".to_owned()
            },
            DisplayCommand::Image {
                x: 0,
                y: 1,
                width: 2,
                height: 2,
                shade: 220,
                alt: Some("PNG tile".to_owned()),
                url: Some("tile.png".to_owned()),
                decoded_width: Some(2),
                decoded_height: Some(2),
                decoded_hash: Some(decoded_info.pixel_hash)
            },
            DisplayCommand::Text {
                x: 0,
                y: 3,
                text: "After png".to_owned()
            },
        ]
    );

    let raster = rasterize_render(&render, BrowserRasterOptions::default()).unwrap();
    assert!(raster.non_background_pixels() > 2 * 2);
    let raster_hash = raster.pixel_hash();
    fs::remove_file(png).unwrap();
    let cached_raster = rasterize_render(&render, BrowserRasterOptions::default()).unwrap();
    assert_eq!(cached_raster.pixel_hash(), raster_hash);
}

#[tokio::test]
async fn image_color_pipeline_preserves_rgb_pixels_in_decoded_resource_report() {
    let png_bytes = tiny_test_png_rgb_with_sub_filter();
    let decoded = decode_simple_png(&png_bytes).unwrap();
    let expected_hash = decoded.pixel_hash();
    let expected_color_hash = decoded.color_pixel_hash().unwrap();
    assert_eq!(decoded.width, 2);
    assert_eq!(decoded.height, 2);
    assert_eq!(decoded.pixels, vec![0, 255, 77, 29]);
    assert_eq!(
        decoded.rgb_pixels.as_deref(),
        Some(&[0, 0, 0, 255, 255, 255, 255, 0, 0, 0, 0, 255][..])
    );

    let dir = tempfile::tempdir().unwrap();
    let page = dir.path().join("page.html");
    let png = dir.path().join("color.png");
    fs::write(&png, png_bytes).unwrap();
    fs::write(
        &page,
        r#"<html><body><p>Before color</p><img src="color.png" alt="Color PNG" width="16" height="24"><p>After color</p></body></html>"#,
    )
    .unwrap();

    let mut session = BrowserSession::new(BrowserRenderOptions {
        width: 40,
        ..BrowserRenderOptions::default()
    });
    session.navigate(&page.display().to_string()).await.unwrap();

    let report = session.render_current_with_images(1024).await.unwrap();
    assert_eq!(report.image_count, 1);
    assert_eq!(report.decoded, 1);
    assert_eq!(report.failed, 0);
    assert_eq!(report.decoded_image_bytes, decoded.pixels.len());
    let fetch = report.fetches.first().unwrap();
    assert_eq!(fetch.resource.kind, "image");
    assert_eq!(fetch.resource.initiator, "img");
    assert_eq!(fetch.resource.resolved, png.display().to_string());
    assert_eq!(fetch.status, "fetched");
    assert_eq!(fetch.content_type.as_deref(), Some("image/png"));
    assert_eq!(fetch.image_decode_status.as_deref(), Some("decoded"));
    assert_eq!(fetch.decoded_width, Some(2));
    assert_eq!(fetch.decoded_height, Some(2));
    assert_eq!(fetch.decoded_hash.as_deref(), Some(expected_hash.as_str()));
    assert_eq!(
        fetch.decoded_color_hash.as_deref(),
        Some(expected_color_hash.as_str())
    );
    assert_eq!(fetch.decoded_color_bytes, Some(12));

    let render = session.current().unwrap();
    assert_eq!(render.decoded_images.len(), 1);
    assert_eq!(render.decoded_images[0].pixel_hash, expected_hash);
    assert_eq!(
        render.decoded_images[0].image.rgb_pixels.as_deref(),
        decoded.rgb_pixels.as_deref()
    );
}

#[test]
fn decodes_local_webp_image_into_rendered_image_command() {
    let dir = tempfile::tempdir().unwrap();
    let page = dir.path().join("page.html");
    let webp = dir.path().join("tile.webp");
    fs::write(&webp, tiny_test_webp_bytes()).unwrap();

    let source = page.display().to_string();
    let decoded_info = decoded_image_entry(&source, "tile.webp").unwrap().info();
    let render = render_html(
        &source,
        br#"<html><body><p>Before webp</p><img src="tile.webp" alt="WebP tile" width="16" height="24"><p>After webp</p></body></html>"#,
        BrowserRenderOptions {
            width: 40,
            ..BrowserRenderOptions::default()
        },
    );

    assert_eq!(render.text, "Before webp\nAfter webp");
    assert_eq!(render.decoded_images.len(), 1);
    assert_eq!(render.decoded_images[0].pixel_hash, decoded_info.pixel_hash);
    assert_eq!(
        render.display_list,
        vec![
            DisplayCommand::Text {
                x: 0,
                y: 0,
                text: "Before webp".to_owned()
            },
            DisplayCommand::Image {
                x: 0,
                y: 1,
                width: 2,
                height: 2,
                shade: 220,
                alt: Some("WebP tile".to_owned()),
                url: Some("tile.webp".to_owned()),
                decoded_width: Some(1),
                decoded_height: Some(1),
                decoded_hash: Some(decoded_info.pixel_hash)
            },
            DisplayCommand::Text {
                x: 0,
                y: 3,
                text: "After webp".to_owned()
            },
        ]
    );
}

#[test]
fn decodes_data_url_webp_image_into_rendered_image_command() {
    let data_url = tiny_test_webp_data_url();
    let decoded = decode_image_reference("mem://webp", &data_url).unwrap();
    let expected_hash = decoded.pixel_hash();
    let render = render_html(
        "mem://webp",
        format!(
            r#"<html><body><p>Before webp</p><img src="{data_url}" alt="Inline WebP" width="16" height="24"><p>After webp</p></body></html>"#
        )
        .as_bytes(),
        BrowserRenderOptions {
            width: 40,
            ..BrowserRenderOptions::default()
        },
    );

    assert_eq!(render.text, "Before webp\nAfter webp");
    assert_eq!(render.decoded_images.len(), 1);
    assert_eq!(render.decoded_images[0].pixel_hash, expected_hash);
    assert_eq!(
        render.display_list,
        vec![
            DisplayCommand::Text {
                x: 0,
                y: 0,
                text: "Before webp".to_owned()
            },
            DisplayCommand::Image {
                x: 0,
                y: 1,
                width: 2,
                height: 2,
                shade: 220,
                alt: Some("Inline WebP".to_owned()),
                url: Some(data_url),
                decoded_width: Some(1),
                decoded_height: Some(1),
                decoded_hash: Some(expected_hash)
            },
            DisplayCommand::Text {
                x: 0,
                y: 3,
                text: "After webp".to_owned()
            },
        ]
    );
}

#[test]
fn reuses_duplicate_image_decodes_within_single_render() {
    let dir = tempfile::tempdir().unwrap();
    let page = dir.path().join("page.html");
    let png = dir.path().join("tile.png");
    let png_bytes = tiny_test_png_rgb_with_sub_filter();
    fs::write(&png, &png_bytes).unwrap();

    let source = page.display().to_string();
    let decoded_info = decoded_image_entry(&source, "tile.png").unwrap().info();
    let render = render_html(
            &source,
            br#"<html><body><img src="tile.png" alt="First tile" width="16" height="24"><img src="tile.png" alt="Second tile" width="16" height="24"></body></html>"#,
            BrowserRenderOptions {
                width: 40,
                ..BrowserRenderOptions::default()
            },
        );

    assert_eq!(render.decoded_images.len(), 1);
    assert_eq!(render.decoded_images[0].pixel_hash, decoded_info.pixel_hash);
    assert_eq!(
        render.display_list,
        vec![
            DisplayCommand::Image {
                x: 0,
                y: 0,
                width: 2,
                height: 2,
                shade: 220,
                alt: Some("First tile".to_owned()),
                url: Some("tile.png".to_owned()),
                decoded_width: Some(2),
                decoded_height: Some(2),
                decoded_hash: Some(decoded_info.pixel_hash.clone())
            },
            DisplayCommand::Image {
                x: 0,
                y: 2,
                width: 2,
                height: 2,
                shade: 220,
                alt: Some("Second tile".to_owned()),
                url: Some("tile.png".to_owned()),
                decoded_width: Some(2),
                decoded_height: Some(2),
                decoded_hash: Some(decoded_info.pixel_hash.clone())
            },
        ]
    );

    let raster = rasterize_render(&render, BrowserRasterOptions::default()).unwrap();
    let raster_hash = raster.pixel_hash();
    fs::remove_file(png).unwrap();
    let cached_raster = rasterize_render(&render, BrowserRasterOptions::default()).unwrap();
    assert_eq!(cached_raster.pixel_hash(), raster_hash);
}

#[test]
fn selects_srcset_image_candidate_for_static_render() {
    let dir = tempfile::tempdir().unwrap();
    let page = dir.path().join("page.html");
    let png = dir.path().join("large.png");
    let png_bytes = tiny_test_png_rgb_with_sub_filter();
    fs::write(&png, &png_bytes).unwrap();

    let source = page.display().to_string();
    let decoded_info = decoded_image_entry(&source, "large.png").unwrap().info();
    let render = render_html(
            &source,
            br#"<html><body><img src="fallback.png" srcset="small.png 16w, large.png 80w" alt="Chosen" width="80" height="24"></body></html>"#,
            BrowserRenderOptions {
                width: 40,
                ..BrowserRenderOptions::default()
            },
        );

    assert_eq!(render.decoded_images.len(), 1);
    assert_eq!(
        render.display_list,
        vec![DisplayCommand::Image {
            x: 0,
            y: 0,
            width: 10,
            height: 2,
            shade: 220,
            alt: Some("Chosen".to_owned()),
            url: Some("large.png".to_owned()),
            decoded_width: Some(2),
            decoded_height: Some(2),
            decoded_hash: Some(decoded_info.pixel_hash)
        }]
    );
}

#[test]
fn selects_data_url_jpeg_srcset_candidate_for_static_render() {
    let data_url = tiny_test_jpeg_data_url();
    let expected_hash = decode_image_reference("mem://srcset-jpeg", &data_url)
        .unwrap()
        .pixel_hash();
    let html = format!(
        r#"<html><body><img src="fallback.jpg" srcset="{data_url} 80w" alt="Data JPEG" width="80" height="24"></body></html>"#
    );
    let render = render_html(
        "mem://srcset-jpeg",
        html.as_bytes(),
        BrowserRenderOptions {
            width: 40,
            ..BrowserRenderOptions::default()
        },
    );

    assert_eq!(render.decoded_images.len(), 1);
    assert_eq!(
        render.display_list,
        vec![DisplayCommand::Image {
            x: 0,
            y: 0,
            width: 10,
            height: 2,
            shade: 220,
            alt: Some("Data JPEG".to_owned()),
            url: Some(data_url),
            decoded_width: Some(2),
            decoded_height: Some(2),
            decoded_hash: Some(expected_hash),
        }]
    );
}

#[test]
fn image_decode_visibility_skips_unsupported_width_srcset_candidate() {
    let dir = tempfile::tempdir().unwrap();
    let page = dir.path().join("page.html");
    let large_jpeg = dir.path().join("large.jpg");
    fs::write(&large_jpeg, tiny_test_jpeg_bytes()).unwrap();

    let source = page.display().to_string();
    let decoded_info = decoded_image_entry(&source, "large.jpg").unwrap().info();
    let render = render_html(
        &source,
        br#"<html><body><img src="fallback.jpg" srcset="hero.avif 320w, large.jpg 640w" alt="Supported width JPEG" height="24"></body></html>"#,
        BrowserRenderOptions {
            width: 40,
            ..BrowserRenderOptions::default()
        },
    );

    assert_eq!(render.decoded_images.len(), 1);
    assert_eq!(
        render.display_list,
        vec![DisplayCommand::Image {
            x: 0,
            y: 0,
            width: 2,
            height: 2,
            shade: 220,
            alt: Some("Supported width JPEG".to_owned()),
            url: Some("large.jpg".to_owned()),
            decoded_width: Some(2),
            decoded_height: Some(2),
            decoded_hash: Some(decoded_info.pixel_hash),
        }]
    );
}

#[test]
fn image_decode_visibility_skips_unsupported_density_srcset_candidate() {
    let dir = tempfile::tempdir().unwrap();
    let page = dir.path().join("page.html");
    let retina_webp = dir.path().join("retina.webp");
    fs::write(&retina_webp, tiny_test_webp_bytes()).unwrap();

    let source = page.display().to_string();
    let decoded_info = decoded_image_entry(&source, "retina.webp").unwrap().info();
    let render = render_html(
        &source,
        br#"<html><body><img src="fallback.jpg" srcset="placeholder.gif 1x, retina.webp 2x" alt="Supported density WebP" width="80" height="24"></body></html>"#,
        BrowserRenderOptions {
            width: 40,
            ..BrowserRenderOptions::default()
        },
    );

    assert_eq!(render.decoded_images.len(), 1);
    assert_eq!(
        render.display_list,
        vec![DisplayCommand::Image {
            x: 0,
            y: 0,
            width: 10,
            height: 2,
            shade: 220,
            alt: Some("Supported density WebP".to_owned()),
            url: Some("retina.webp".to_owned()),
            decoded_width: Some(1),
            decoded_height: Some(1),
            decoded_hash: Some(decoded_info.pixel_hash),
        }]
    );
}

#[test]
fn selects_descriptorless_jpeg_srcset_candidate_as_default_density() {
    let dir = tempfile::tempdir().unwrap();
    let page = dir.path().join("page.html");
    let small_jpeg = dir.path().join("small.jpg");
    let retina_jpeg = dir.path().join("retina.jpg");
    fs::write(&small_jpeg, tiny_test_jpeg_bytes()).unwrap();
    fs::write(&retina_jpeg, tiny_test_jpeg_bytes()).unwrap();

    let source = page.display().to_string();
    let decoded_info = decoded_image_entry(&source, "large.jpg").unwrap().info();
    let render = render_html(
        &source,
        br#"<html><body><img src="fallback.jpg" srcset="small.jpg, retina.jpg 2x" alt="Default Density JPEG" width="80" height="24"></body></html>"#,
        BrowserRenderOptions {
            width: 40,
            ..BrowserRenderOptions::default()
        },
    );

    assert_eq!(render.decoded_images.len(), 1);
    assert_eq!(
        render.display_list,
        vec![DisplayCommand::Image {
            x: 0,
            y: 0,
            width: 10,
            height: 2,
            shade: 220,
            alt: Some("Default Density JPEG".to_owned()),
            url: Some("small.jpg".to_owned()),
            decoded_width: Some(2),
            decoded_height: Some(2),
            decoded_hash: Some(decoded_info.pixel_hash),
        }]
    );
}

#[test]
fn ignores_jpeg_srcset_candidate_with_invalid_descriptor() {
    let dir = tempfile::tempdir().unwrap();
    let page = dir.path().join("page.html");
    let bad_jpeg = dir.path().join("bad.jpg");
    let valid_jpeg = dir.path().join("valid.jpg");
    fs::write(&bad_jpeg, tiny_test_jpeg_bytes()).unwrap();
    fs::write(&valid_jpeg, tiny_test_jpeg_bytes()).unwrap();

    let source = page.display().to_string();
    let decoded_info = decoded_image_entry(&source, "valid.jpg").unwrap().info();
    let render = render_html(
        &source,
        br#"<html><body><img src="fallback.jpg" srcset="bad.jpg 1x bogus, valid.jpg" alt="Invalid Descriptor JPEG" width="80" height="24"></body></html>"#,
        BrowserRenderOptions {
            width: 40,
            ..BrowserRenderOptions::default()
        },
    );

    assert_eq!(render.decoded_images.len(), 1);
    assert_eq!(
        render.display_list,
        vec![DisplayCommand::Image {
            x: 0,
            y: 0,
            width: 10,
            height: 2,
            shade: 220,
            alt: Some("Invalid Descriptor JPEG".to_owned()),
            url: Some("valid.jpg".to_owned()),
            decoded_width: Some(2),
            decoded_height: Some(2),
            decoded_hash: Some(decoded_info.pixel_hash),
        }]
    );
}

#[test]
fn ignores_jpeg_srcset_candidate_with_mixed_descriptors() {
    let dir = tempfile::tempdir().unwrap();
    let page = dir.path().join("page.html");
    let bad_jpeg = dir.path().join("bad.jpg");
    let small_jpeg = dir.path().join("small.jpg");
    let large_jpeg = dir.path().join("large.jpg");
    fs::write(&bad_jpeg, tiny_test_jpeg_bytes()).unwrap();
    fs::write(&small_jpeg, tiny_test_jpeg_bytes()).unwrap();
    fs::write(&large_jpeg, tiny_test_jpeg_bytes()).unwrap();

    let source = page.display().to_string();
    let decoded_info = decoded_image_entry(&source, "large.jpg").unwrap().info();
    let render = render_html(
        &source,
        br#"<html><body><img src="fallback.jpg" sizes="160px" srcset="bad.jpg 160w 1x, small.jpg 160w, large.jpg 320w" alt="Mixed Descriptor JPEG" height="24"></body></html>"#,
        BrowserRenderOptions {
            width: 40,
            ..BrowserRenderOptions::default()
        },
    );

    assert_eq!(render.decoded_images.len(), 1);
    assert_eq!(
        render.display_list,
        vec![DisplayCommand::Image {
            x: 0,
            y: 0,
            width: 2,
            height: 2,
            shade: 220,
            alt: Some("Mixed Descriptor JPEG".to_owned()),
            url: Some("small.jpg".to_owned()),
            decoded_width: Some(2),
            decoded_height: Some(2),
            decoded_hash: Some(decoded_info.pixel_hash),
        }]
    );
}

#[test]
fn selects_viewport_width_jpeg_srcset_candidate_without_width_attr() {
    let dir = tempfile::tempdir().unwrap();
    let page = dir.path().join("page.html");
    let small_jpeg = dir.path().join("small.jpg");
    let large_jpeg = dir.path().join("large.jpg");
    fs::write(&small_jpeg, tiny_test_jpeg_bytes()).unwrap();
    fs::write(&large_jpeg, tiny_test_jpeg_bytes()).unwrap();

    let source = page.display().to_string();
    let decoded_info = decoded_image_entry(&source, "small.jpg").unwrap().info();
    let render = render_html(
        &source,
        br#"<html><body><img src="fallback.jpg" srcset="small.jpg 320w, large.jpg 1200w" alt="Viewport JPEG" height="24"></body></html>"#,
        BrowserRenderOptions {
            width: 40,
            ..BrowserRenderOptions::default()
        },
    );

    assert_eq!(render.decoded_images.len(), 1);
    assert_eq!(
        render.display_list,
        vec![DisplayCommand::Image {
            x: 0,
            y: 0,
            width: 2,
            height: 2,
            shade: 220,
            alt: Some("Viewport JPEG".to_owned()),
            url: Some("small.jpg".to_owned()),
            decoded_width: Some(2),
            decoded_height: Some(2),
            decoded_hash: Some(decoded_info.pixel_hash),
        }]
    );
}

#[test]
fn selects_jpeg_srcset_candidate_from_sizes_vw() {
    let dir = tempfile::tempdir().unwrap();
    let page = dir.path().join("page.html");
    let small_jpeg = dir.path().join("small.jpg");
    let medium_jpeg = dir.path().join("medium.jpg");
    let large_jpeg = dir.path().join("large.jpg");
    fs::write(&small_jpeg, tiny_test_jpeg_bytes()).unwrap();
    fs::write(&medium_jpeg, tiny_test_jpeg_bytes()).unwrap();
    fs::write(&large_jpeg, tiny_test_jpeg_bytes()).unwrap();

    let source = page.display().to_string();
    let decoded_info = decoded_image_entry(&source, "small.jpg").unwrap().info();
    let render = render_html(
        &source,
        br#"<html><body><img src="fallback.jpg" sizes="50vw" srcset="small.jpg 160w, medium.jpg 320w, large.jpg 1200w" alt="Sized JPEG" height="24"></body></html>"#,
        BrowserRenderOptions {
            width: 40,
            ..BrowserRenderOptions::default()
        },
    );

    assert_eq!(render.decoded_images.len(), 1);
    assert_eq!(
        render.display_list,
        vec![DisplayCommand::Image {
            x: 0,
            y: 0,
            width: 2,
            height: 2,
            shade: 220,
            alt: Some("Sized JPEG".to_owned()),
            url: Some("small.jpg".to_owned()),
            decoded_width: Some(2),
            decoded_height: Some(2),
            decoded_hash: Some(decoded_info.pixel_hash),
        }]
    );
}

#[test]
fn ignores_unitless_jpeg_srcset_sizes_length() {
    let dir = tempfile::tempdir().unwrap();
    let page = dir.path().join("page.html");
    let small_jpeg = dir.path().join("small.jpg");
    let medium_jpeg = dir.path().join("medium.jpg");
    fs::write(&small_jpeg, tiny_test_jpeg_bytes()).unwrap();
    fs::write(&medium_jpeg, tiny_test_jpeg_bytes()).unwrap();

    let source = page.display().to_string();
    let decoded_info = decoded_image_entry(&source, "medium.jpg").unwrap().info();
    let render = render_html(
        &source,
        br#"<html><body><img src="fallback.jpg" sizes="160" srcset="small.jpg 160w, medium.jpg 320w" alt="Unitless Sizes JPEG" height="24"></body></html>"#,
        BrowserRenderOptions {
            width: 40,
            ..BrowserRenderOptions::default()
        },
    );

    assert_eq!(render.decoded_images.len(), 1);
    assert_eq!(
        render.display_list,
        vec![DisplayCommand::Image {
            x: 0,
            y: 0,
            width: 2,
            height: 2,
            shade: 220,
            alt: Some("Unitless Sizes JPEG".to_owned()),
            url: Some("medium.jpg".to_owned()),
            decoded_width: Some(2),
            decoded_height: Some(2),
            decoded_hash: Some(decoded_info.pixel_hash),
        }]
    );
}

#[test]
fn ignores_unitless_jpeg_sizes_media_width() {
    let dir = tempfile::tempdir().unwrap();
    let page = dir.path().join("page.html");
    let small_jpeg = dir.path().join("small.jpg");
    let medium_jpeg = dir.path().join("medium.jpg");
    fs::write(&small_jpeg, tiny_test_jpeg_bytes()).unwrap();
    fs::write(&medium_jpeg, tiny_test_jpeg_bytes()).unwrap();

    let source = page.display().to_string();
    let decoded_info = decoded_image_entry(&source, "medium.jpg").unwrap().info();
    let render = render_html(
        &source,
        br#"<html><body><img src="fallback.jpg" sizes="(max-width: 100) 160px, 320px" srcset="small.jpg 160w, medium.jpg 320w" alt="Unitless Media Sizes JPEG" height="24"></body></html>"#,
        BrowserRenderOptions {
            width: 40,
            ..BrowserRenderOptions::default()
        },
    );

    assert_eq!(render.decoded_images.len(), 1);
    assert_eq!(
        render.display_list,
        vec![DisplayCommand::Image {
            x: 0,
            y: 0,
            width: 2,
            height: 2,
            shade: 220,
            alt: Some("Unitless Media Sizes JPEG".to_owned()),
            url: Some("medium.jpg".to_owned()),
            decoded_width: Some(2),
            decoded_height: Some(2),
            decoded_hash: Some(decoded_info.pixel_hash),
        }]
    );
}

#[test]
fn selects_picture_jpeg_srcset_candidate_from_source_sizes() {
    let dir = tempfile::tempdir().unwrap();
    let page = dir.path().join("page.html");
    let small_jpeg = dir.path().join("small.jpg");
    let medium_jpeg = dir.path().join("medium.jpg");
    let fallback_jpeg = dir.path().join("fallback.jpg");
    fs::write(&small_jpeg, tiny_test_jpeg_bytes()).unwrap();
    fs::write(&medium_jpeg, tiny_test_jpeg_bytes()).unwrap();
    fs::write(&fallback_jpeg, tiny_test_jpeg_bytes()).unwrap();

    let source = page.display().to_string();
    let decoded_info = decoded_image_entry(&source, "small.jpg").unwrap().info();
    let render = render_html(
        &source,
        br#"<html><body><picture><source type="image/jpeg" sizes="(max-width: 400px) 160px, 320px" srcset="small.jpg 160w, medium.jpg 320w"><img src="fallback.jpg" alt="Source Sizes JPEG" height="24"></picture></body></html>"#,
        BrowserRenderOptions {
            width: 40,
            ..BrowserRenderOptions::default()
        },
    );

    assert_eq!(render.decoded_images.len(), 1);
    assert_eq!(
        render.display_list,
        vec![DisplayCommand::Image {
            x: 0,
            y: 0,
            width: 2,
            height: 2,
            shade: 220,
            alt: Some("Source Sizes JPEG".to_owned()),
            url: Some("small.jpg".to_owned()),
            decoded_width: Some(2),
            decoded_height: Some(2),
            decoded_hash: Some(decoded_info.pixel_hash),
        }]
    );
}

#[test]
fn selects_jpeg_srcset_candidate_from_calc_sizes_media_condition() {
    let dir = tempfile::tempdir().unwrap();
    let page = dir.path().join("page.html");
    let small_jpeg = dir.path().join("small.jpg");
    let medium_jpeg = dir.path().join("medium.jpg");
    fs::write(&small_jpeg, tiny_test_jpeg_bytes()).unwrap();
    fs::write(&medium_jpeg, tiny_test_jpeg_bytes()).unwrap();

    let source = page.display().to_string();
    let decoded_info = decoded_image_entry(&source, "small.jpg").unwrap().info();
    let render = render_html(
        &source,
        br#"<html><body><img src="fallback.jpg" sizes="(max-width: 400px) calc(50vw - 1px), 320px" srcset="small.jpg 160w, medium.jpg 320w" alt="Calc Sizes JPEG" height="24"></body></html>"#,
        BrowserRenderOptions {
            width: 40,
            ..BrowserRenderOptions::default()
        },
    );

    assert_eq!(render.decoded_images.len(), 1);
    assert_eq!(
        render.display_list,
        vec![DisplayCommand::Image {
            x: 0,
            y: 0,
            width: 2,
            height: 2,
            shade: 220,
            alt: Some("Calc Sizes JPEG".to_owned()),
            url: Some("small.jpg".to_owned()),
            decoded_width: Some(2),
            decoded_height: Some(2),
            decoded_hash: Some(decoded_info.pixel_hash),
        }]
    );
}

#[test]
fn selects_jpeg_srcset_candidate_from_min_sizes_function() {
    let dir = tempfile::tempdir().unwrap();
    let page = dir.path().join("page.html");
    let small_jpeg = dir.path().join("small.jpg");
    let medium_jpeg = dir.path().join("medium.jpg");
    fs::write(&small_jpeg, tiny_test_jpeg_bytes()).unwrap();
    fs::write(&medium_jpeg, tiny_test_jpeg_bytes()).unwrap();

    let source = page.display().to_string();
    let decoded_info = decoded_image_entry(&source, "small.jpg").unwrap().info();
    let render = render_html(
        &source,
        br#"<html><body><img src="fallback.jpg" sizes="min(50vw, 320px)" srcset="small.jpg 160w, medium.jpg 320w" alt="Min Sizes JPEG" height="24"></body></html>"#,
        BrowserRenderOptions {
            width: 40,
            ..BrowserRenderOptions::default()
        },
    );

    assert_eq!(render.decoded_images.len(), 1);
    assert_eq!(
        render.display_list,
        vec![DisplayCommand::Image {
            x: 0,
            y: 0,
            width: 2,
            height: 2,
            shade: 220,
            alt: Some("Min Sizes JPEG".to_owned()),
            url: Some("small.jpg".to_owned()),
            decoded_width: Some(2),
            decoded_height: Some(2),
            decoded_hash: Some(decoded_info.pixel_hash),
        }]
    );
}

#[test]
fn selects_jpeg_srcset_candidate_from_clamp_sizes_function() {
    let dir = tempfile::tempdir().unwrap();
    let page = dir.path().join("page.html");
    let small_jpeg = dir.path().join("small.jpg");
    let medium_jpeg = dir.path().join("medium.jpg");
    fs::write(&small_jpeg, tiny_test_jpeg_bytes()).unwrap();
    fs::write(&medium_jpeg, tiny_test_jpeg_bytes()).unwrap();

    let source = page.display().to_string();
    let decoded_info = decoded_image_entry(&source, "small.jpg").unwrap().info();
    let render = render_html(
        &source,
        br#"<html><body><img src="fallback.jpg" sizes="clamp(120px, max(50vw, 1px), 180px)" srcset="small.jpg 160w, medium.jpg 320w" alt="Clamp Sizes JPEG" height="24"></body></html>"#,
        BrowserRenderOptions {
            width: 40,
            ..BrowserRenderOptions::default()
        },
    );

    assert_eq!(render.decoded_images.len(), 1);
    assert_eq!(
        render.display_list,
        vec![DisplayCommand::Image {
            x: 0,
            y: 0,
            width: 2,
            height: 2,
            shade: 220,
            alt: Some("Clamp Sizes JPEG".to_owned()),
            url: Some("small.jpg".to_owned()),
            decoded_width: Some(2),
            decoded_height: Some(2),
            decoded_hash: Some(decoded_info.pixel_hash),
        }]
    );
}

#[test]
fn selects_picture_source_srcset_before_img_src() {
    let dir = tempfile::tempdir().unwrap();
    let page = dir.path().join("page.html");
    let png = dir.path().join("art.png");
    let png_bytes = tiny_test_png_rgb_with_sub_filter();
    fs::write(&png, &png_bytes).unwrap();

    let source = page.display().to_string();
    let decoded_info = decoded_image_entry(&source, "art.png").unwrap().info();
    let render = render_html(
            &source,
            br#"<html><body><picture><source srcset="art.png 80w"><img src="fallback.png" alt="Picture" width="80" height="24"></picture></body></html>"#,
            BrowserRenderOptions {
                width: 40,
                ..BrowserRenderOptions::default()
            },
        );

    assert_eq!(render.decoded_images.len(), 1);
    assert_eq!(
        render.display_list,
        vec![DisplayCommand::Image {
            x: 0,
            y: 0,
            width: 10,
            height: 2,
            shade: 220,
            alt: Some("Picture".to_owned()),
            url: Some("art.png".to_owned()),
            decoded_width: Some(2),
            decoded_height: Some(2),
            decoded_hash: Some(decoded_info.pixel_hash)
        }]
    );
}

#[test]
fn skips_data_empty_picture_placeholder_source_for_img_fallback() {
    let dir = tempfile::tempdir().unwrap();
    let page = dir.path().join("page.html");
    let fallback_jpeg = dir.path().join("fallback.jpg");
    fs::write(&fallback_jpeg, tiny_test_jpeg_bytes()).unwrap();

    let placeholder = "data:image/gif;base64,R0lGODlhAQABAHAAACH5BAEAAAAALAAAAAABAAEAAAICRAEAOw==";
    let source = page.display().to_string();
    let decoded_info = decoded_image_entry(&source, "fallback.jpg").unwrap().info();
    let html = format!(
        r#"<html><body><picture><source data-empty="" srcset="{placeholder}" media="(min-width:0px)"><img src="fallback.jpg" alt="Apple fallback JPEG" width="80" height="24"></picture></body></html>"#
    );
    let render = render_html(
        &source,
        html.as_bytes(),
        BrowserRenderOptions {
            width: 40,
            ..BrowserRenderOptions::default()
        },
    );

    assert_eq!(render.decoded_images.len(), 1);
    assert_eq!(
        render.display_list,
        vec![DisplayCommand::Image {
            x: 0,
            y: 0,
            width: 10,
            height: 2,
            shade: 220,
            alt: Some("Apple fallback JPEG".to_owned()),
            url: Some("fallback.jpg".to_owned()),
            decoded_width: Some(2),
            decoded_height: Some(2),
            decoded_hash: Some(decoded_info.pixel_hash)
        }]
    );
}

#[test]
fn selects_picture_webp_source_srcset_before_img_src() {
    let dir = tempfile::tempdir().unwrap();
    let page = dir.path().join("page.html");
    let webp = dir.path().join("hero.webp");
    let fallback_jpeg = dir.path().join("fallback.jpg");
    fs::write(&webp, tiny_test_webp_bytes()).unwrap();
    fs::write(&fallback_jpeg, tiny_test_jpeg_bytes()).unwrap();

    let source = page.display().to_string();
    let decoded_info = decoded_image_entry(&source, "hero.webp").unwrap().info();
    let render = render_html(
        &source,
        br#"<html><body><picture><source type="image/webp; charset=binary" srcset="hero.webp 80w"><img src="fallback.jpg" alt="WebP picture" width="80" height="24"></picture></body></html>"#,
        BrowserRenderOptions {
            width: 40,
            ..BrowserRenderOptions::default()
        },
    );

    assert_eq!(render.decoded_images.len(), 1);
    assert_eq!(
        render.display_list,
        vec![DisplayCommand::Image {
            x: 0,
            y: 0,
            width: 10,
            height: 2,
            shade: 220,
            alt: Some("WebP picture".to_owned()),
            url: Some("hero.webp".to_owned()),
            decoded_width: Some(1),
            decoded_height: Some(1),
            decoded_hash: Some(decoded_info.pixel_hash),
        }]
    );
}

#[test]
fn skips_unsupported_picture_avif_source_type_for_img_jpeg_fallback() {
    let dir = tempfile::tempdir().unwrap();
    let page = dir.path().join("page.html");
    let jpeg = dir.path().join("fallback.jpg");
    fs::write(&jpeg, tiny_test_jpeg_bytes()).unwrap();

    let source = page.display().to_string();
    let decoded_info = decoded_image_entry(&source, "fallback.jpg").unwrap().info();
    let render = render_html(
            &source,
            br#"<html><body><picture><source type="image/avif" srcset="hero.avif 80w"><img src="fallback.jpg" alt="JPEG fallback" width="80" height="24"></picture></body></html>"#,
            BrowserRenderOptions {
                width: 40,
                ..BrowserRenderOptions::default()
            },
        );

    assert_eq!(render.decoded_images.len(), 1);
    assert_eq!(
        render.display_list,
        vec![DisplayCommand::Image {
            x: 0,
            y: 0,
            width: 10,
            height: 2,
            shade: 220,
            alt: Some("JPEG fallback".to_owned()),
            url: Some("fallback.jpg".to_owned()),
            decoded_width: Some(2),
            decoded_height: Some(2),
            decoded_hash: Some(decoded_info.pixel_hash)
        }]
    );
}

#[test]
fn skips_picture_jpeg_source_with_unitless_media_width() {
    let dir = tempfile::tempdir().unwrap();
    let page = dir.path().join("page.html");
    let source_jpeg = dir.path().join("source.jpg");
    let fallback_jpeg = dir.path().join("fallback.jpg");
    fs::write(&source_jpeg, tiny_test_jpeg_bytes()).unwrap();
    fs::write(&fallback_jpeg, tiny_test_jpeg_bytes()).unwrap();

    let source = page.display().to_string();
    let decoded_info = decoded_image_entry(&source, "fallback.jpg").unwrap().info();
    let render = render_html(
            &source,
            br#"<html><body><picture><source media="(max-width: 100)" type="image/jpeg" srcset="source.jpg 80w"><img src="fallback.jpg" alt="Unitless Media JPEG" width="80" height="24"></picture></body></html>"#,
            BrowserRenderOptions {
                width: 40,
                ..BrowserRenderOptions::default()
            },
        );

    assert_eq!(render.decoded_images.len(), 1);
    assert_eq!(
        render.display_list,
        vec![DisplayCommand::Image {
            x: 0,
            y: 0,
            width: 10,
            height: 2,
            shade: 220,
            alt: Some("Unitless Media JPEG".to_owned()),
            url: Some("fallback.jpg".to_owned()),
            decoded_width: Some(2),
            decoded_height: Some(2),
            decoded_hash: Some(decoded_info.pixel_hash)
        }]
    );
}

#[test]
fn selects_picture_jpeg_source_with_screen_media() {
    let dir = tempfile::tempdir().unwrap();
    let page = dir.path().join("page.html");
    let screen_jpeg = dir.path().join("screen.jpg");
    let fallback_jpeg = dir.path().join("fallback.jpg");
    fs::write(&screen_jpeg, tiny_test_jpeg_bytes()).unwrap();
    fs::write(&fallback_jpeg, tiny_test_jpeg_bytes()).unwrap();

    let source = page.display().to_string();
    let decoded_info = decoded_image_entry(&source, "screen.jpg").unwrap().info();
    let render = render_html(
            &source,
            br#"<html><body><picture><source media="print" type="image/jpeg" srcset="print.jpg 80w"><source media="screen" type="image/jpeg" srcset="screen.jpg 80w"><img src="fallback.jpg" alt="Screen JPEG" width="80" height="24"></picture></body></html>"#,
            BrowserRenderOptions {
                width: 40,
                ..BrowserRenderOptions::default()
            },
        );

    assert_eq!(render.decoded_images.len(), 1);
    assert_eq!(
        render.display_list,
        vec![DisplayCommand::Image {
            x: 0,
            y: 0,
            width: 10,
            height: 2,
            shade: 220,
            alt: Some("Screen JPEG".to_owned()),
            url: Some("screen.jpg".to_owned()),
            decoded_width: Some(2),
            decoded_height: Some(2),
            decoded_hash: Some(decoded_info.pixel_hash)
        }]
    );
}

#[test]
fn selects_picture_jpeg_source_with_min_width_media() {
    let dir = tempfile::tempdir().unwrap();
    let page = dir.path().join("page.html");
    let wide_jpeg = dir.path().join("wide.jpg");
    let narrow_jpeg = dir.path().join("narrow.jpg");
    let fallback_jpeg = dir.path().join("fallback.jpg");
    fs::write(&wide_jpeg, tiny_test_jpeg_bytes()).unwrap();
    fs::write(&narrow_jpeg, tiny_test_jpeg_bytes()).unwrap();
    fs::write(&fallback_jpeg, tiny_test_jpeg_bytes()).unwrap();

    let source = page.display().to_string();
    let decoded_info = decoded_image_entry(&source, "wide.jpg").unwrap().info();
    let render = render_html(
            &source,
            br#"<html><body><picture><source media="(max-width: 639px)" type="image/jpeg" srcset="narrow.jpg 80w"><source media="(min-width: 640px)" type="image/jpeg" srcset="wide.jpg 80w"><img src="fallback.jpg" alt="Wide JPEG" width="80" height="24"></picture></body></html>"#,
            BrowserRenderOptions {
                width: 80,
                ..BrowserRenderOptions::default()
            },
        );

    assert_eq!(render.decoded_images.len(), 1);
    assert_eq!(
        render.display_list,
        vec![DisplayCommand::Image {
            x: 0,
            y: 0,
            width: 10,
            height: 2,
            shade: 220,
            alt: Some("Wide JPEG".to_owned()),
            url: Some("wide.jpg".to_owned()),
            decoded_width: Some(2),
            decoded_height: Some(2),
            decoded_hash: Some(decoded_info.pixel_hash)
        }]
    );
}

#[test]
fn lazy_svg_placeholder_img_uses_real_data_source_for_rendering() {
    let data_url = concat!(
        "data:image/png;base64,",
        "iVBORw0KGgoAAAANSUhEUgAAAAIAAAACCAIAAAAAAAAAAAAAFklEQVR4AWNgYGD4//8/438GBkaG/wAh9gT+AAAAAAAAAABJRU5EAAAAAA=="
    );
    let decoded = decode_image_reference("mem://lazy-image", data_url).unwrap();
    let expected_hash = decoded.pixel_hash();
    let html = format!(
        r#"<html><body><img src="data:image/svg+xml,%3Csvg%20xmlns='http://www.w3.org/2000/svg'%20viewBox='0%200%20640%20480'%3E%3C/svg%3E" data-lazy-src="{data_url}" alt="Cat" width="80" height="48"><p>After</p></body></html>"#
    );
    let render = render_html(
        "mem://lazy-image",
        html.as_bytes(),
        BrowserRenderOptions {
            width: 80,
            ..BrowserRenderOptions::default()
        },
    );

    assert_eq!(render.text, "After");
    assert_eq!(render.decoded_images.len(), 1);
    assert_eq!(
        render.display_list,
        vec![
            DisplayCommand::Image {
                x: 0,
                y: 0,
                width: 10,
                height: 4,
                shade: 220,
                alt: Some("Cat".to_owned()),
                url: Some(data_url.to_owned()),
                decoded_width: Some(2),
                decoded_height: Some(2),
                decoded_hash: Some(expected_hash),
            },
            DisplayCommand::Text {
                x: 0,
                y: 4,
                text: "After".to_owned(),
            },
        ]
    );
}

#[test]
fn lazy_png_placeholder_img_uses_real_jpeg_data_source_for_rendering() {
    let placeholder = concat!(
        "data:image/png;base64,",
        "iVBORw0KGgoAAAANSUhEUgAAAAIAAAACCAIAAAAAAAAAAAAAFklEQVR4AWNgYGD4//8/438GBkaG/wAh9gT+AAAAAAAAAABJRU5EAAAAAA=="
    );
    let data_url = tiny_test_jpeg_data_url();
    let expected_hash = decode_image_reference("mem://lazy-png-image", &data_url)
        .unwrap()
        .pixel_hash();
    let html = format!(
        r#"<html><body><img src="{placeholder}" data-src="{data_url}" alt="Lazy JPEG" width="80" height="48"><p>After</p></body></html>"#
    );
    let render = render_html(
        "mem://lazy-png-image",
        html.as_bytes(),
        BrowserRenderOptions {
            width: 80,
            ..BrowserRenderOptions::default()
        },
    );

    assert_eq!(render.text, "After");
    assert_eq!(render.decoded_images.len(), 1);
    assert_eq!(
        render.display_list,
        vec![
            DisplayCommand::Image {
                x: 0,
                y: 0,
                width: 10,
                height: 4,
                shade: 220,
                alt: Some("Lazy JPEG".to_owned()),
                url: Some(data_url),
                decoded_width: Some(2),
                decoded_height: Some(2),
                decoded_hash: Some(expected_hash),
            },
            DisplayCommand::Text {
                x: 0,
                y: 4,
                text: "After".to_owned(),
            },
        ]
    );
}

#[test]
fn lazy_gif_placeholder_img_uses_data_image_source_for_rendering() {
    let dir = tempfile::tempdir().unwrap();
    let page = dir.path().join("page.html");
    let hero_jpeg = dir.path().join("hero.jpg");
    fs::write(&hero_jpeg, tiny_test_jpeg_bytes()).unwrap();

    let placeholder = "data:image/gif;base64,R0lGODlhAQABAHAAACH5BAEAAAAALAAAAAABAAEAAAICRAEAOw==";
    let source = page.display().to_string();
    let decoded_info = decoded_image_entry(&source, "hero.jpg").unwrap().info();
    let html = format!(
        r#"<html><body><img src="{placeholder}" data-image="hero.jpg" alt="Data image JPEG" width="80" height="48"><p>After</p></body></html>"#
    );
    let render = render_html(
        &source,
        html.as_bytes(),
        BrowserRenderOptions {
            width: 80,
            ..BrowserRenderOptions::default()
        },
    );

    assert_eq!(render.text, "After");
    assert_eq!(render.decoded_images.len(), 1);
    assert_eq!(
        render.display_list,
        vec![
            DisplayCommand::Image {
                x: 0,
                y: 0,
                width: 10,
                height: 4,
                shade: 220,
                alt: Some("Data image JPEG".to_owned()),
                url: Some("hero.jpg".to_owned()),
                decoded_width: Some(2),
                decoded_height: Some(2),
                decoded_hash: Some(decoded_info.pixel_hash),
            },
            DisplayCommand::Text {
                x: 0,
                y: 4,
                text: "After".to_owned(),
            },
        ]
    );
}

#[test]
fn image_render_coverage_decodes_x_webp_after_x_png_placeholder() {
    let data_url = test_webp_data_url_with_mime_type("image/x-webp");
    let decoded = decode_image_reference("mem://x-webp", &data_url).unwrap();
    let placeholder = "data:image/x-png;base64,not-a-real-placeholder";
    let render = render_html(
        "mem://x-webp",
        format!(
            r#"<html><body><img src="{placeholder}" data-src="{data_url}" alt="X WebP" width="80" height="48"><p>After</p></body></html>"#
        )
        .as_bytes(),
        BrowserRenderOptions {
            width: 80,
            ..BrowserRenderOptions::default()
        },
    );

    assert_eq!(render.text, "After");
    assert_eq!(render.decoded_images.len(), 1);
    assert_eq!(
        render.display_list,
        vec![
            DisplayCommand::Image {
                x: 0,
                y: 0,
                width: 10,
                height: 4,
                shade: 220,
                alt: Some("X WebP".to_owned()),
                url: Some(data_url),
                decoded_width: Some(decoded.width),
                decoded_height: Some(decoded.height),
                decoded_hash: Some(decoded.pixel_hash()),
            },
            DisplayCommand::Text {
                x: 0,
                y: 4,
                text: "After".to_owned()
            },
        ]
    );
}

#[test]
fn lazy_gif_placeholder_img_uses_data_image_srcset_for_rendering() {
    let dir = tempfile::tempdir().unwrap();
    let page = dir.path().join("page.html");
    let small_jpeg = dir.path().join("small.jpg");
    let large_jpeg = dir.path().join("large.jpg");
    fs::write(&small_jpeg, tiny_test_jpeg_bytes()).unwrap();
    fs::write(&large_jpeg, tiny_test_jpeg_bytes()).unwrap();

    let placeholder = "data:image/gif;base64,R0lGODlhAQABAHAAACH5BAEAAAAALAAAAAABAAEAAAICRAEAOw==";
    let source = page.display().to_string();
    let decoded_info = decoded_image_entry(&source, "small.jpg").unwrap().info();
    let html = format!(
        r#"<html><body><img src="{placeholder}" sizes="160px" data-image-srcset="small.jpg 160w, large.jpg 320w" alt="Data image srcset JPEG" height="48"><p>After</p></body></html>"#
    );
    let render = render_html(
        &source,
        html.as_bytes(),
        BrowserRenderOptions {
            width: 80,
            ..BrowserRenderOptions::default()
        },
    );

    assert_eq!(render.text, "After");
    assert_eq!(render.decoded_images.len(), 1);
    assert_eq!(
        render.display_list,
        vec![
            DisplayCommand::Image {
                x: 0,
                y: 0,
                width: 4,
                height: 4,
                shade: 220,
                alt: Some("Data image srcset JPEG".to_owned()),
                url: Some("large.jpg".to_owned()),
                decoded_width: Some(2),
                decoded_height: Some(2),
                decoded_hash: Some(decoded_info.pixel_hash),
            },
            DisplayCommand::Text {
                x: 0,
                y: 4,
                text: "After".to_owned(),
            },
        ]
    );
}

#[test]
fn image_visible_resources_uses_lazy_source_for_file_placeholder() {
    let dir = tempfile::tempdir().unwrap();
    let page = dir.path().join("page.html");
    let hero_jpeg = dir.path().join("hero.jpg");
    fs::write(&hero_jpeg, tiny_test_jpeg_bytes()).unwrap();

    let source = page.display().to_string();
    let decoded_info = decoded_image_entry(&source, "hero.jpg").unwrap().info();
    let render = render_html(
        &source,
        br#"<html><body><img src="/assets/blank.gif?cache=1" data-original-url="hero.jpg" alt="Original URL JPEG" width="80" height="48"><p>After</p></body></html>"#,
        BrowserRenderOptions {
            width: 80,
            ..BrowserRenderOptions::default()
        },
    );

    assert_eq!(render.text, "After");
    assert_eq!(render.decoded_images.len(), 1);
    assert_eq!(
        render.display_list,
        vec![
            DisplayCommand::Image {
                x: 0,
                y: 0,
                width: 10,
                height: 4,
                shade: 220,
                alt: Some("Original URL JPEG".to_owned()),
                url: Some("hero.jpg".to_owned()),
                decoded_width: Some(2),
                decoded_height: Some(2),
                decoded_hash: Some(decoded_info.pixel_hash),
            },
            DisplayCommand::Text {
                x: 0,
                y: 4,
                text: "After".to_owned(),
            },
        ]
    );
}

#[test]
fn image_visible_resources_uses_picture_original_srcset_for_file_placeholder() {
    let dir = tempfile::tempdir().unwrap();
    let page = dir.path().join("page.html");
    let small_jpeg = dir.path().join("small.jpg");
    let large_jpeg = dir.path().join("large.jpg");
    fs::write(&small_jpeg, tiny_test_jpeg_bytes()).unwrap();
    fs::write(&large_jpeg, tiny_test_jpeg_bytes()).unwrap();

    let source = page.display().to_string();
    let decoded_info = decoded_image_entry(&source, "small.jpg").unwrap().info();
    let render = render_html(
        &source,
        br#"<html><body><picture><source type="image/jpeg" data-original-srcset="small.jpg 160w, large.jpg 320w"><img src="/assets/placeholder.png" alt="Original srcset JPEG" height="48"></picture><p>After</p></body></html>"#,
        BrowserRenderOptions {
            width: 80,
            ..BrowserRenderOptions::default()
        },
    );

    assert_eq!(render.text, "After");
    assert_eq!(render.decoded_images.len(), 1);
    assert_eq!(
        render.display_list,
        vec![
            DisplayCommand::Image {
                x: 0,
                y: 0,
                width: 4,
                height: 4,
                shade: 220,
                alt: Some("Original srcset JPEG".to_owned()),
                url: Some("large.jpg".to_owned()),
                decoded_width: Some(2),
                decoded_height: Some(2),
                decoded_hash: Some(decoded_info.pixel_hash),
            },
            DisplayCommand::Text {
                x: 0,
                y: 4,
                text: "After".to_owned(),
            },
        ]
    );
}

#[test]
fn decodes_data_url_png_image_into_raster_pixels() {
    let data_url = concat!(
        "data:image/png;base64,",
        "iVBORw0KGgoAAAANSUhEUgAAAAIAAAACCAIAAAAAAAAAAAAAFklEQVR4AWNgYGD4//8/438GBkaG/wAh9gT+AAAAAAAAAABJRU5EAAAAAA=="
    );
    let decoded = decode_image_reference("mem://page", data_url).unwrap();
    assert_eq!(decoded.width, 2);
    assert_eq!(decoded.height, 2);
    assert_eq!(decoded.pixels, vec![0, 255, 77, 29]);

    let html = format!(
        r#"<html><body><p>Before data</p><img src="{data_url}" alt="Data PNG" width="16" height="24"><p>After data</p></body></html>"#
    );
    let render = render_html(
        "mem://page",
        html.as_bytes(),
        BrowserRenderOptions {
            width: 40,
            ..BrowserRenderOptions::default()
        },
    );

    assert_eq!(render.text, "Before data\nAfter data");
    assert_eq!(render.decoded_images.len(), 1);
    assert_eq!(render.decoded_images[0].width, 2);
    assert_eq!(render.decoded_images[0].height, 2);
    let raster = rasterize_render(&render, BrowserRasterOptions::default()).unwrap();
    assert!(raster.non_background_pixels() > 2 * 2);
}

#[test]
fn decodes_data_url_jpeg_image_into_rendered_image_command() {
    let data_url = tiny_test_jpeg_data_url();
    let decoded = decode_image_reference("mem://page", &data_url).unwrap();
    let expected_hash = decoded.pixel_hash();
    assert_eq!(decoded.width, 2);
    assert_eq!(decoded.height, 2);

    let html = format!(
        r#"<html><body><p>Before jpeg</p><img src="{data_url}" alt="Data JPEG" width="16" height="24"><p>After jpeg</p></body></html>"#
    );
    let render = render_html(
        "mem://page",
        html.as_bytes(),
        BrowserRenderOptions {
            width: 40,
            ..BrowserRenderOptions::default()
        },
    );

    assert_eq!(render.text, "Before jpeg\nAfter jpeg");
    assert_eq!(render.decoded_images.len(), 1);
    assert_eq!(render.decoded_images[0].width, 2);
    assert_eq!(render.decoded_images[0].height, 2);
    assert_eq!(render.decoded_images[0].pixel_hash, expected_hash);
    assert_eq!(
        render.display_list,
        vec![
            DisplayCommand::Text {
                x: 0,
                y: 0,
                text: "Before jpeg".to_owned()
            },
            DisplayCommand::Image {
                x: 0,
                y: 1,
                width: 2,
                height: 2,
                shade: 220,
                alt: Some("Data JPEG".to_owned()),
                url: Some(data_url),
                decoded_width: Some(2),
                decoded_height: Some(2),
                decoded_hash: Some(expected_hash)
            },
            DisplayCommand::Text {
                x: 0,
                y: 3,
                text: "After jpeg".to_owned()
            },
        ]
    );
    let raster = rasterize_render(&render, BrowserRasterOptions::default()).unwrap();
    assert!(raster.non_background_pixels() > 2 * 2);
}

#[tokio::test]
async fn session_render_images_decodes_data_url_image_resource() {
    let data_url = concat!(
        "data:image/png;base64,",
        "iVBORw0KGgoAAAANSUhEUgAAAAIAAAACCAIAAAAAAAAAAAAAFklEQVR4AWNgYGD4//8/438GBkaG/wAh9gT+AAAAAAAAAABJRU5EAAAAAA=="
    );
    let decoded = decode_image_reference("mem://page", data_url).unwrap();
    let expected_hash = decoded.pixel_hash();
    let dir = tempfile::tempdir().unwrap();
    let page = dir.path().join("page.html");
    fs::write(
        &page,
        format!(
            r#"<html><body><p>Before inline</p><img src="{data_url}" alt="Inline PNG" width="16" height="24"><p>After inline</p></body></html>"#
        ),
    )
    .unwrap();

    let mut session = BrowserSession::new(BrowserRenderOptions {
        width: 40,
        ..BrowserRenderOptions::default()
    });
    session.navigate(&page.display().to_string()).await.unwrap();

    let report = session.render_current_with_images(1024).await.unwrap();
    assert_eq!(report.image_count, 1);
    assert_eq!(report.decoded, 1);
    assert_eq!(report.failed, 1);
    assert_eq!(report.fetches.len(), 1);
    assert_eq!(report.fetches[0].status, "cached");
    assert_eq!(report.fetches[0].content_type.as_deref(), Some("image/png"));
    assert_eq!(report.cached_resource_count, 1);
    assert_eq!(report.cached_resource_bytes, report.fetches[0].bytes);
    assert_eq!(report.decoded_image_bytes, decoded.pixels.len());

    let render = session.current().unwrap();
    assert_eq!(render.decoded_images.len(), 1);
    assert_eq!(render.decoded_images[0].pixel_hash, expected_hash);
    assert_eq!(
        render.display_list,
        vec![
            DisplayCommand::Text {
                x: 0,
                y: 0,
                text: "Before inline".to_owned()
            },
            DisplayCommand::Image {
                x: 0,
                y: 1,
                width: 2,
                height: 2,
                shade: 220,
                alt: Some("Inline PNG".to_owned()),
                url: Some(data_url.to_owned()),
                decoded_width: Some(2),
                decoded_height: Some(2),
                decoded_hash: Some(expected_hash)
            },
            DisplayCommand::Text {
                x: 0,
                y: 3,
                text: "After inline".to_owned()
            },
        ]
    );
}

#[tokio::test]
async fn session_render_images_decodes_css_background_image_resource() {
    let png_bytes = tiny_test_png_rgb_with_sub_filter();
    let decoded = decode_simple_png(&png_bytes).unwrap();
    let expected_hash = decoded.pixel_hash();
    let dir = tempfile::tempdir().unwrap();
    let page = dir.path().join("page.html");
    let tile = dir.path().join("tile.png");
    fs::write(&tile, png_bytes).unwrap();
    fs::write(
        &page,
        r#"<html><head><style>
            .hero {
                background: linear-gradient(#fff, #eee), url('tile.png');
                min-height: 24px;
            }
        </style></head><body><p>Before bg</p><section class="hero">Background</section><p>After bg</p></body></html>"#,
    )
    .unwrap();

    let mut session = BrowserSession::new(BrowserRenderOptions {
        width: 40,
        ..BrowserRenderOptions::default()
    });
    session.navigate(&page.display().to_string()).await.unwrap();

    let report = session.render_current_with_images(1024).await.unwrap();
    assert_eq!(report.image_count, 1);
    assert_eq!(report.decoded, 1);
    assert_eq!(report.failed, 0);
    assert_eq!(report.fetches.len(), 1);
    assert_eq!(report.fetches[0].resource.kind, "background_image");
    assert_eq!(report.fetches[0].resource.initiator, "css");
    assert_eq!(report.fetches[0].resource.url, "tile.png");
    assert_eq!(
        report.fetches[0].resource.resolved,
        tile.display().to_string()
    );
    assert_eq!(report.fetches[0].status, "fetched");
    assert_eq!(report.fetches[0].content_type.as_deref(), Some("image/png"));
    assert_eq!(report.cached_resource_count, 1);
    assert_eq!(report.cached_resource_bytes, report.fetches[0].bytes);
    assert_eq!(report.decoded_image_bytes, decoded.pixels.len());

    let render = session.current().unwrap();
    assert!(render.resources.iter().any(|resource| {
        resource.kind == "background_image"
            && resource.initiator == "css"
            && resource.resolved == tile.display().to_string()
    }));
    assert_eq!(render.decoded_images.len(), 1);
    assert_eq!(render.decoded_images[0].pixel_hash, expected_hash);
    assert!(render.display_list.iter().any(|command| {
        matches!(
            command,
            DisplayCommand::BackgroundImage {
                url: Some(url),
                decoded_hash: Some(hash),
                ..
            } if url == &tile.display().to_string() && *hash == expected_hash
        )
    }));
}

#[tokio::test]
async fn image_raster_fidelity_decodes_indexed_png_resource_pixels() {
    let png_bytes = tiny_test_indexed_png_with_transparency();
    let decoded = decode_simple_png(&png_bytes).unwrap();
    assert_eq!(decoded.width, 2);
    assert_eq!(decoded.height, 2);
    assert_eq!(decoded.pixels, vec![255, 255, 77, 29]);
    let expected_hash = decoded.pixel_hash();

    let dir = tempfile::tempdir().unwrap();
    let page = dir.path().join("page.html");
    let icon = dir.path().join("icon.png");
    fs::write(&icon, png_bytes).unwrap();
    fs::write(
        &page,
        r#"<html><body><img src="icon.png" alt="Indexed PNG" width="16" height="16"><p>After</p></body></html>"#,
    )
    .unwrap();

    let mut session = BrowserSession::new(BrowserRenderOptions {
        width: 40,
        ..BrowserRenderOptions::default()
    });
    session.navigate(&page.display().to_string()).await.unwrap();

    let report = session.render_current_with_images(1024).await.unwrap();
    assert_eq!(report.image_count, 1);
    assert_eq!(report.decoded, 1);
    assert_eq!(report.failed, 0);
    assert_eq!(report.decoded_image_bytes, decoded.pixels.len());

    let fetch = report.fetches.first().unwrap();
    assert_eq!(fetch.resource.kind, "image");
    assert_eq!(fetch.resource.initiator, "img");
    assert_eq!(fetch.resource.resolved, icon.display().to_string());
    assert_eq!(fetch.status, "fetched");
    assert_eq!(fetch.content_type.as_deref(), Some("image/png"));
    assert_eq!(fetch.image_decode_status.as_deref(), Some("decoded"));
    assert_eq!(fetch.decoded_width, Some(2));
    assert_eq!(fetch.decoded_height, Some(2));
    assert_eq!(fetch.decoded_hash.as_deref(), Some(expected_hash.as_str()));

    let render = session.current().unwrap();
    assert_eq!(render.decoded_images.len(), 1);
    assert_eq!(render.decoded_images[0].pixel_hash, expected_hash);
    assert!(render.display_list.iter().any(|command| {
        matches!(
            command,
            DisplayCommand::Image {
                url: Some(url),
                decoded_hash: Some(hash),
                ..
            } if url == &icon.display().to_string() && *hash == expected_hash
        )
    }));
}

#[tokio::test]
async fn image_resource_bundle_decodes_replaced_media_image_resources() {
    let png_bytes = tiny_test_png_rgb_with_sub_filter();
    let decoded = decode_simple_png(&png_bytes).unwrap();
    let expected_hash = decoded.pixel_hash();
    let dir = tempfile::tempdir().unwrap();
    let page = dir.path().join("page.html");
    let poster = dir.path().join("poster.png");
    let object = dir.path().join("object.png");
    let embed = dir.path().join("embed.webp");
    fs::write(&poster, &png_bytes).unwrap();
    fs::write(&object, &png_bytes).unwrap();
    fs::write(&embed, tiny_test_webp_bytes()).unwrap();
    fs::write(
        &page,
        r#"<html><body>
            <p>Before media</p>
            <video poster="poster.png" width="16" height="24"></video>
            <object data="object.png" width="16" height="24"></object>
            <embed src="embed.webp" width="16" height="24">
            <p>After media</p>
        </body></html>"#,
    )
    .unwrap();

    let mut session = BrowserSession::new(BrowserRenderOptions {
        width: 40,
        ..BrowserRenderOptions::default()
    });
    session.navigate(&page.display().to_string()).await.unwrap();

    let report = session.render_current_with_images(1024).await.unwrap();
    assert_eq!(report.image_count, 3);
    assert_eq!(report.decoded, 3);
    assert_eq!(report.failed, 0);
    assert!(report.fetches.iter().any(|fetch| {
        fetch.resource.kind == "poster"
            && fetch.resource.initiator == "video"
            && fetch.resource.resolved == poster.display().to_string()
            && fetch.status == "fetched"
            && fetch.content_type.as_deref() == Some("image/png")
    }));
    assert!(report.fetches.iter().any(|fetch| {
        fetch.resource.kind == "image"
            && fetch.resource.initiator == "object"
            && fetch.resource.resolved == object.display().to_string()
            && fetch.status == "fetched"
    }));
    assert!(report.fetches.iter().any(|fetch| {
        fetch.resource.kind == "image"
            && fetch.resource.initiator == "embed"
            && fetch.resource.resolved == embed.display().to_string()
            && fetch.status == "fetched"
            && fetch.content_type.as_deref() == Some("image/webp")
    }));

    let render = session.current().unwrap();
    assert_eq!(render.decoded_images.len(), 3);
    assert!(render.display_list.iter().any(|command| {
        matches!(
            command,
            DisplayCommand::Image {
                url: Some(url),
                decoded_hash: Some(hash),
                ..
            } if url == &poster.display().to_string() && *hash == expected_hash
        )
    }));
    assert!(render.display_list.iter().any(|command| {
        matches!(
            command,
            DisplayCommand::Image {
                url: Some(url),
                decoded_hash: Some(hash),
                ..
            } if url == &object.display().to_string() && *hash == expected_hash
        )
    }));
    assert!(render.display_list.iter().any(|command| {
        matches!(
            command,
            DisplayCommand::Image {
                url: Some(url),
                decoded_hash: Some(_),
                ..
            } if url == &embed.display().to_string()
        )
    }));
}

#[tokio::test]
async fn image_real_page_resources_dedupes_selected_image_fetches() {
    let png_bytes = tiny_test_png_rgb_with_sub_filter();
    let decoded = decode_simple_png(&png_bytes).unwrap();
    let expected_hash = decoded.pixel_hash();
    let dir = tempfile::tempdir().unwrap();
    let page = dir.path().join("page.html");
    let tile = dir.path().join("tile.png");
    fs::write(&tile, png_bytes).unwrap();
    fs::write(
        &page,
        r#"<html><head>
            <link rel="preload" as="image" href="tile.png">
        </head><body>
            <p>Before duplicate</p>
            <img src="tile.png" alt="First" width="16" height="24">
            <img src="tile.png" alt="Second" width="16" height="24">
            <p>After duplicate</p>
        </body></html>"#,
    )
    .unwrap();

    let mut session = BrowserSession::new(BrowserRenderOptions {
        width: 40,
        ..BrowserRenderOptions::default()
    });
    session.navigate(&page.display().to_string()).await.unwrap();

    let report = session.render_current_with_images(1024).await.unwrap();
    assert_eq!(report.image_count, 1);
    assert_eq!(report.decoded, 1);
    assert_eq!(report.failed, 0);
    assert_eq!(report.fetches.len(), 1);
    assert_eq!(
        report.fetches[0].resource.resolved,
        tile.display().to_string()
    );
    assert_eq!(report.fetches[0].status, "fetched");

    let render = session.current().unwrap();
    assert_eq!(render.decoded_images.len(), 1);
    assert_eq!(render.decoded_images[0].pixel_hash, expected_hash);
    let decoded_image_commands = render
        .display_list
        .iter()
        .filter(|command| {
            matches!(
                command,
                DisplayCommand::Image {
                    url: Some(url),
                    decoded_hash: Some(hash),
                    ..
                } if url == &tile.display().to_string() && *hash == expected_hash
            )
        })
        .count();
    assert_eq!(decoded_image_commands, 2);
}

#[tokio::test]
async fn session_render_images_decodes_http_resource_cache_pixels() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    let png_bytes = tiny_test_png_rgb_with_sub_filter();
    let decoded = decode_simple_png(&png_bytes).unwrap();
    let expected_hash = decoded.pixel_hash();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        for _ in 0..2 {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut buf = [0u8; 4096];
            let n = stream.read(&mut buf).await.unwrap();
            let request = String::from_utf8_lossy(&buf[..n]);
            let first_line = request.lines().next().unwrap_or_default();
            let (content_type, body) = if first_line.contains(" /tile.png ") {
                ("image/png", png_bytes.clone())
            } else {
                (
                        "text/html",
                        br#"<html><body><p>Before network</p><img src="/tile.png" alt="Network PNG" width="16" height="24"><p>After network</p></body></html>"#.to_vec(),
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

    let mut session = BrowserSession::new(BrowserRenderOptions {
        width: 40,
        ..BrowserRenderOptions::default()
    });
    session
        .navigate(&format!("http://{addr}/page.html"))
        .await
        .unwrap();
    assert_eq!(session.current().unwrap().decoded_images.len(), 0);

    let report = session.render_current_with_images(1024).await.unwrap();
    assert_eq!(report.image_count, 1);
    assert_eq!(report.decoded, 1);
    assert_eq!(report.failed, 0);
    assert_eq!(report.cached_resource_count, 1);
    assert_eq!(report.cached_resource_bytes, report.fetches[0].bytes);
    assert_eq!(report.decoded_image_bytes, decoded.pixels.len());
    let render = session.current().unwrap();
    assert_eq!(render.decoded_images.len(), 1);
    assert_eq!(render.decoded_images[0].pixel_hash, expected_hash);
    assert_eq!(
        render.display_list,
        vec![
            DisplayCommand::Text {
                x: 0,
                y: 0,
                text: "Before network".to_owned()
            },
            DisplayCommand::Image {
                x: 0,
                y: 1,
                width: 2,
                height: 2,
                shade: 220,
                alt: Some("Network PNG".to_owned()),
                url: Some(format!("http://{addr}/tile.png")),
                decoded_width: Some(2),
                decoded_height: Some(2),
                decoded_hash: Some(expected_hash)
            },
            DisplayCommand::Text {
                x: 0,
                y: 3,
                text: "After network".to_owned()
            },
        ]
    );
    let raster = rasterize_render(render, BrowserRasterOptions::default()).unwrap();
    assert!(raster.non_background_pixels() > 2 * 2);
    server.await.unwrap();
}

#[tokio::test]
async fn session_render_images_sniffs_http_jpeg_resource_pixels() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    let jpeg_bytes = tiny_test_jpeg_bytes();
    let expected_hash = decode_image_reference("mem://jpeg", &tiny_test_jpeg_data_url())
        .unwrap()
        .pixel_hash();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        for _ in 0..2 {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut buf = [0u8; 4096];
            let n = stream.read(&mut buf).await.unwrap();
            let request = String::from_utf8_lossy(&buf[..n]);
            let first_line = request.lines().next().unwrap_or_default();
            let (content_type, body) = if first_line.contains(" /opaque-resource ") {
                ("application/octet-stream", jpeg_bytes.clone())
            } else {
                (
                    "text/html",
                    br#"<html><body><p>Before sniff</p><img src="/opaque-resource" alt="Sniffed JPEG" width="16" height="24"><p>After sniff</p></body></html>"#.to_vec(),
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

    let mut session = BrowserSession::new(BrowserRenderOptions {
        width: 40,
        ..BrowserRenderOptions::default()
    });
    session
        .navigate(&format!("http://{addr}/page.html"))
        .await
        .unwrap();
    assert_eq!(session.current().unwrap().decoded_images.len(), 0);

    let report = session.render_current_with_images(1024).await.unwrap();
    assert_eq!(report.image_count, 1);
    assert_eq!(report.decoded, 1);
    assert_eq!(report.failed, 0);
    let render = session.current().unwrap();
    assert_eq!(render.decoded_images.len(), 1);
    assert_eq!(render.decoded_images[0].pixel_hash, expected_hash);
    assert_eq!(
        render.display_list,
        vec![
            DisplayCommand::Text {
                x: 0,
                y: 0,
                text: "Before sniff".to_owned()
            },
            DisplayCommand::Image {
                x: 0,
                y: 1,
                width: 2,
                height: 2,
                shade: 220,
                alt: Some("Sniffed JPEG".to_owned()),
                url: Some(format!("http://{addr}/opaque-resource")),
                decoded_width: Some(2),
                decoded_height: Some(2),
                decoded_hash: Some(expected_hash)
            },
            DisplayCommand::Text {
                x: 0,
                y: 3,
                text: "After sniff".to_owned()
            },
        ]
    );
    let raster = rasterize_render(render, BrowserRasterOptions::default()).unwrap();
    assert!(raster.non_background_pixels() > 2 * 2);
    server.await.unwrap();
}

#[tokio::test]
async fn session_render_images_uses_decoded_intrinsic_size_without_attrs() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    let png_bytes = tiny_test_png_rgb_with_sub_filter();
    let decoded = decode_simple_png(&png_bytes).unwrap();
    let expected_hash = decoded.pixel_hash();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        for _ in 0..2 {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut buf = [0u8; 4096];
            let n = stream.read(&mut buf).await.unwrap();
            let request = String::from_utf8_lossy(&buf[..n]);
            let first_line = request.lines().next().unwrap_or_default();
            let (content_type, body) = if first_line.contains(" /tile.png ") {
                ("image/png", png_bytes.clone())
            } else {
                (
                    "text/html",
                    br#"<html><body><p>Before network</p><img src="/tile.png" alt="Network PNG"><p>After network</p></body></html>"#.to_vec(),
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

    let mut session = BrowserSession::new(BrowserRenderOptions {
        width: 40,
        ..BrowserRenderOptions::default()
    });
    session
        .navigate(&format!("http://{addr}/page.html"))
        .await
        .unwrap();

    let initial_render = session.current().unwrap();
    assert_eq!(
        initial_render.display_list,
        vec![
            DisplayCommand::Text {
                x: 0,
                y: 0,
                text: "Before network".to_owned()
            },
            DisplayCommand::Image {
                x: 0,
                y: 1,
                width: 10,
                height: 4,
                shade: 220,
                alt: Some("Network PNG".to_owned()),
                url: None,
                decoded_width: None,
                decoded_height: None,
                decoded_hash: None
            },
            DisplayCommand::Text {
                x: 0,
                y: 5,
                text: "After network".to_owned()
            },
        ]
    );

    let report = session.render_current_with_images(1024).await.unwrap();
    assert_eq!(report.image_count, 1);
    assert_eq!(report.decoded, 1);
    assert_eq!(report.failed, 0);
    let render = session.current().unwrap();
    assert_eq!(render.decoded_images.len(), 1);
    assert_eq!(render.decoded_images[0].pixel_hash, expected_hash);
    assert_eq!(
        render.display_list,
        vec![
            DisplayCommand::Text {
                x: 0,
                y: 0,
                text: "Before network".to_owned()
            },
            DisplayCommand::Image {
                x: 0,
                y: 1,
                width: 1,
                height: 1,
                shade: 220,
                alt: Some("Network PNG".to_owned()),
                url: Some(format!("http://{addr}/tile.png")),
                decoded_width: Some(2),
                decoded_height: Some(2),
                decoded_hash: Some(expected_hash)
            },
            DisplayCommand::Text {
                x: 0,
                y: 2,
                text: "After network".to_owned()
            },
        ]
    );
    server.await.unwrap();
}

#[tokio::test]
async fn image_resource_reporting_distinguishes_decode_outcomes() {
    let png_bytes = tiny_test_png_rgb_with_sub_filter();
    let decoded = decode_simple_png(&png_bytes).unwrap();
    let expected_hash = decoded.pixel_hash();
    let gif_bytes = b"GIF89a\x01\0\x01\0\x80\0\0\0\0\0\xff\xff\xff!\xf9\x04\0\0\0\0\0,\0\0\0\0\x01\0\x01\0\0\x02\x02D\x01\0;";
    let dir = tempfile::tempdir().unwrap();
    let page = dir.path().join("page.html");
    let decoded_path = dir.path().join("decoded.png");
    let unsupported_path = dir.path().join("spinner.gif");
    let undecoded_path = dir.path().join("broken.png");
    let missing_path = dir.path().join("missing.jpg");
    fs::write(&decoded_path, png_bytes).unwrap();
    fs::write(&unsupported_path, gif_bytes).unwrap();
    fs::write(&undecoded_path, b"not actually a png").unwrap();
    fs::write(
        &page,
        r#"<html><body>
            <img src="decoded.png" alt="Decoded" width="16" height="24">
            <img src="spinner.gif" alt="Unsupported" width="16" height="24">
            <img src="broken.png" alt="Undecoded" width="16" height="24">
            <img src="missing.jpg" alt="Missing" width="16" height="24">
        </body></html>"#,
    )
    .unwrap();

    let mut session = BrowserSession::new(BrowserRenderOptions {
        width: 40,
        ..BrowserRenderOptions::default()
    });
    session.navigate(&page.display().to_string()).await.unwrap();

    let report = session.render_current_with_images(1024).await.unwrap();
    assert_eq!(report.image_count, 4);
    assert_eq!(report.decoded, 1);
    assert_eq!(report.failed, 1);

    let decoded_fetch = report
        .fetches
        .iter()
        .find(|fetch| fetch.resource.resolved == decoded_path.display().to_string())
        .unwrap();
    assert_eq!(decoded_fetch.status, "fetched");
    assert_eq!(
        decoded_fetch.image_decode_status.as_deref(),
        Some("decoded")
    );
    assert_eq!(decoded_fetch.decoded_width, Some(2));
    assert_eq!(decoded_fetch.decoded_height, Some(2));
    assert_eq!(
        decoded_fetch.decoded_hash.as_deref(),
        Some(expected_hash.as_str())
    );
    assert_eq!(decoded_fetch.image_decode_error, None);

    let unsupported_fetch = report
        .fetches
        .iter()
        .find(|fetch| fetch.resource.resolved == unsupported_path.display().to_string())
        .unwrap();
    assert_eq!(unsupported_fetch.status, "fetched");
    assert_eq!(
        unsupported_fetch.image_decode_status.as_deref(),
        Some("unsupported_format")
    );
    assert_eq!(
        unsupported_fetch.image_decode_error.as_deref(),
        Some("unsupported image content type: image/gif")
    );
    assert_eq!(unsupported_fetch.decoded_hash, None);

    let undecoded_fetch = report
        .fetches
        .iter()
        .find(|fetch| fetch.resource.resolved == undecoded_path.display().to_string())
        .unwrap();
    assert_eq!(undecoded_fetch.status, "fetched");
    assert_eq!(
        undecoded_fetch.image_decode_status.as_deref(),
        Some("undecoded")
    );
    assert_eq!(
        undecoded_fetch.image_decode_error.as_deref(),
        Some("image bytes did not match a supported decoder")
    );

    let missing_fetch = report
        .fetches
        .iter()
        .find(|fetch| fetch.resource.resolved == missing_path.display().to_string())
        .unwrap();
    assert_eq!(missing_fetch.status, "failed");
    assert_eq!(
        missing_fetch.image_decode_status.as_deref(),
        Some("not_fetched")
    );
    assert_eq!(
        missing_fetch.image_decode_error.as_deref(),
        Some("resource failed before decode")
    );
}

#[tokio::test]
async fn image_render_coverage_skips_unsupported_picture_source_for_renderable_candidate() {
    let dir = tempfile::tempdir().unwrap();
    let page = dir.path().join("page.html");
    let hero = dir.path().join("hero.webp");
    fs::write(&hero, tiny_test_webp_bytes()).unwrap();
    fs::write(
        &page,
        r#"<html><body>
            <picture>
                <source srcset="hero.avif 1x">
                <source srcset="hero.webp 1x">
                <img src="fallback.jpg" alt="Renderable picture" width="80" height="24">
            </picture>
        </body></html>"#,
    )
    .unwrap();

    let mut session = BrowserSession::new(BrowserRenderOptions {
        width: 40,
        ..BrowserRenderOptions::default()
    });
    session.navigate(&page.display().to_string()).await.unwrap();

    let report = session.render_current_with_images(1024).await.unwrap();
    assert_eq!(report.image_count, 1);
    assert_eq!(report.decoded, 1);
    assert_eq!(report.failed, 0);

    let hero_url = hero.display().to_string();
    let fetch = report
        .fetches
        .iter()
        .find(|fetch| fetch.resource.resolved == hero_url)
        .unwrap();
    assert_eq!(fetch.status, "fetched");
    assert_eq!(fetch.resource.url, "hero.webp");
    assert_eq!(fetch.content_type.as_deref(), Some("image/webp"));
    assert_eq!(fetch.image_decode_status.as_deref(), Some("decoded"));
    let decoded_hash = fetch.decoded_hash.clone().unwrap();

    let render = session.current().unwrap();
    assert_eq!(render.decoded_images.len(), 1);
    assert!(render.display_list.iter().any(|command| {
        matches!(
            command,
            DisplayCommand::Image {
                url: Some(url),
                decoded_hash: Some(hash),
                ..
            } if url == &hero_url && hash == &decoded_hash
        )
    }));
}

#[tokio::test]
async fn image_picture_srcset_skips_unsupported_typed_source_resources() {
    let dir = tempfile::tempdir().unwrap();
    let page = dir.path().join("page.html");
    let hero = dir.path().join("hero.webp");
    fs::write(&hero, tiny_test_webp_bytes()).unwrap();
    fs::write(
        &page,
        r#"<html><body>
            <picture>
                <source type="image/avif" srcset="dead-resource 1x">
                <source type="image/webp" srcset="hero.webp 1x">
                <img src="fallback.jpg" alt="Renderable typed picture" width="80" height="24">
            </picture>
        </body></html>"#,
    )
    .unwrap();

    let mut render_session = BrowserSession::new(BrowserRenderOptions {
        width: 40,
        ..BrowserRenderOptions::default()
    });
    render_session
        .navigate(&page.display().to_string())
        .await
        .unwrap();

    let report = render_session
        .render_current_with_images(1024)
        .await
        .unwrap();
    assert_eq!(report.image_count, 1);
    assert_eq!(report.decoded, 1);
    assert_eq!(report.failed, 0);
    assert!(
        !report
            .fetches
            .iter()
            .any(|fetch| fetch.resource.url == "dead-resource")
    );

    let hero_url = hero.display().to_string();
    let fetch = report
        .fetches
        .iter()
        .find(|fetch| fetch.resource.resolved == hero_url)
        .unwrap();
    assert_eq!(fetch.resource.kind, "image");
    assert_eq!(fetch.resource.initiator, "img");
    assert_eq!(fetch.resource.url, "hero.webp");
    assert_eq!(fetch.status, "fetched");
    assert_eq!(fetch.content_type.as_deref(), Some("image/webp"));
    assert_eq!(fetch.image_decode_status.as_deref(), Some("decoded"));
    let decoded_hash = fetch.decoded_hash.clone().unwrap();

    let render = render_session.current().unwrap();
    assert!(render.display_list.iter().any(|command| {
        matches!(
            command,
            DisplayCommand::Image {
                url: Some(url),
                decoded_hash: Some(hash),
                ..
            } if url == &hero_url && hash == &decoded_hash
        )
    }));

    let mut resource_session = BrowserSession::new(BrowserRenderOptions::default());
    resource_session
        .navigate(&page.display().to_string())
        .await
        .unwrap();
    let resource_report = resource_session
        .fetch_current_resources(1024)
        .await
        .unwrap();
    assert_eq!(resource_report.failed, 0);
    assert!(
        !resource_report
            .resources
            .iter()
            .any(|fetch| fetch.resource.url == "dead-resource")
    );
    let hero_fetch = resource_report
        .resources
        .iter()
        .find(|fetch| fetch.resource.resolved == hero_url)
        .unwrap();
    assert_eq!(hero_fetch.resource.kind, "image_candidate");
    assert_eq!(hero_fetch.resource.initiator, "source");
    assert_eq!(hero_fetch.resource.url, "hero.webp");
    assert_eq!(hero_fetch.status, "fetched");
    assert_eq!(hero_fetch.content_type.as_deref(), Some("image/webp"));
    assert_eq!(hero_fetch.image_decode_status.as_deref(), Some("decoded"));
    assert!(hero_fetch.decoded_hash.is_some());
}

#[tokio::test]
async fn image_css_backgrounds_selects_renderable_preload_imagesrcset_candidate() {
    let dir = tempfile::tempdir().unwrap();
    let page = dir.path().join("page.html");
    let hero = dir.path().join("hero.webp");
    fs::write(&hero, tiny_test_webp_bytes()).unwrap();
    fs::write(
        &page,
        r#"<html><head>
            <link rel="preload" as="image" href="fallback.jpg" imagesrcset="hero.avif 320w, hero.webp 640w" imagesizes="80px">
        </head><body>
            <img src="hero.webp" alt="Preloaded WebP" width="80" height="24">
        </body></html>"#,
    )
    .unwrap();

    let mut session = BrowserSession::new(BrowserRenderOptions {
        width: 40,
        ..BrowserRenderOptions::default()
    });
    session.navigate(&page.display().to_string()).await.unwrap();

    let report = session.render_current_with_images(1024).await.unwrap();
    assert_eq!(report.image_count, 1);
    assert_eq!(report.decoded, 1);
    assert_eq!(report.failed, 0);

    let hero_url = hero.display().to_string();
    let fetch = report.fetches.first().unwrap();
    assert_eq!(fetch.resource.initiator, "link");
    assert_eq!(fetch.resource.url, "hero.webp");
    assert_eq!(fetch.resource.resolved, hero_url);
    assert_eq!(fetch.status, "fetched");
    assert_eq!(fetch.content_type.as_deref(), Some("image/webp"));
    assert_eq!(fetch.image_decode_status.as_deref(), Some("decoded"));
    let decoded_hash = fetch.decoded_hash.clone().unwrap();

    let render = session.current().unwrap();
    assert_eq!(render.decoded_images.len(), 1);
    assert!(render.display_list.iter().any(|command| {
        matches!(
            command,
            DisplayCommand::Image {
                url: Some(url),
                decoded_hash: Some(hash),
                ..
            } if url == &hero_url && hash == &decoded_hash
        )
    }));
}

#[tokio::test]
async fn image_responsive_preload_uses_href_when_imagesrcset_candidates_unsupported() {
    let dir = tempfile::tempdir().unwrap();
    let page = dir.path().join("page.html");
    let hero = dir.path().join("hero.webp");
    fs::write(&hero, tiny_test_webp_bytes()).unwrap();
    fs::write(
        &page,
        r#"<html><head>
            <link rel="preload" as="image" href="hero.webp" imagesrcset="hero.avif 640w" imagesizes="80px">
        </head><body>
            <p>Preload fallback</p>
        </body></html>"#,
    )
    .unwrap();

    let mut session = BrowserSession::new(BrowserRenderOptions {
        width: 40,
        ..BrowserRenderOptions::default()
    });
    session.navigate(&page.display().to_string()).await.unwrap();

    let report = session.render_current_with_images(1024).await.unwrap();
    assert_eq!(report.image_count, 2);
    assert_eq!(report.decoded, 1);
    assert_eq!(report.failed, 1);

    let hero_url = hero.display().to_string();
    let fetch = report
        .fetches
        .iter()
        .find(|fetch| fetch.resource.url == "hero.webp")
        .expect("supported href fallback fetch");
    assert_eq!(fetch.resource.initiator, "link");
    assert_eq!(fetch.resource.resolved, hero_url);
    assert_eq!(fetch.status, "fetched");
    assert_eq!(fetch.content_type.as_deref(), Some("image/webp"));
    assert_eq!(fetch.image_decode_status.as_deref(), Some("decoded"));
    assert!(fetch.decoded_hash.is_some());
    assert!(
        !report
            .fetches
            .iter()
            .any(|fetch| fetch.resource.url == "hero.avif" && fetch.status == "fetched")
    );
}

#[tokio::test]
async fn image_lazy_source_uses_current_srcset_for_placeholder_rendering() {
    let dir = tempfile::tempdir().unwrap();
    let page = dir.path().join("page.html");
    let hero = dir.path().join("hero.webp");
    fs::write(&hero, tiny_test_webp_bytes()).unwrap();
    fs::write(
        &page,
        r#"<html><body>
            <img src="/assets/placeholder.gif" data-currentSrcset="hero.avif 320w, hero.webp 640w" sizes="80px" alt="Current source WebP" width="80" height="24">
        </body></html>"#,
    )
    .unwrap();

    let mut session = BrowserSession::new(BrowserRenderOptions {
        width: 40,
        ..BrowserRenderOptions::default()
    });
    session.navigate(&page.display().to_string()).await.unwrap();

    let report = session.render_current_with_images(1024).await.unwrap();
    assert_eq!(report.image_count, 1);
    assert_eq!(report.decoded, 1);
    assert_eq!(report.failed, 0);

    let hero_url = hero.display().to_string();
    let fetch = report.fetches.first().unwrap();
    assert_eq!(fetch.resource.initiator, "img");
    assert_eq!(fetch.resource.url, "hero.webp");
    assert_eq!(fetch.resource.resolved, hero_url);
    assert_eq!(fetch.status, "fetched");
    assert_eq!(fetch.content_type.as_deref(), Some("image/webp"));
    assert_eq!(fetch.image_decode_status.as_deref(), Some("decoded"));
    let decoded_hash = fetch.decoded_hash.clone().unwrap();

    let render = session.current().unwrap();
    assert_eq!(render.decoded_images.len(), 1);
    assert!(render.display_list.iter().any(|command| {
        matches!(
            command,
            DisplayCommand::Image {
                url: Some(url),
                decoded_hash: Some(hash),
                ..
            } if url == &hero_url && hash == &decoded_hash
        )
    }));
}

#[tokio::test]
async fn image_visible_render_uses_lazy_sizes_for_selected_sources() {
    let dir = tempfile::tempdir().unwrap();
    let page = dir.path().join("page.html");
    let small_img = dir.path().join("small-img.webp");
    let small_bg = dir.path().join("small-bg.webp");
    fs::write(&small_img, tiny_test_webp_bytes()).unwrap();
    fs::write(&small_bg, tiny_test_webp_bytes()).unwrap();
    fs::write(
        &page,
        r#"<html><body>
            <img src="/assets/placeholder.gif" data-currentSrcset="small-img.webp 160w, missing-img.webp 640w" data-sizes="160px" alt="Sized lazy WebP" width="80" height="24">
            <section data-bgset="small-bg.webp 160w, missing-bg.webp 640w" data-lazy-sizes="160px">Background</section>
        </body></html>"#,
    )
    .unwrap();

    let mut session = BrowserSession::new(BrowserRenderOptions {
        width: 40,
        ..BrowserRenderOptions::default()
    });
    session.navigate(&page.display().to_string()).await.unwrap();

    let report = session.render_current_with_images(1024).await.unwrap();
    assert_eq!(report.image_count, 2);
    assert_eq!(report.decoded, 2);
    assert_eq!(report.failed, 0);
    assert_eq!(report.fetches.len(), 2);
    assert!(
        !report
            .fetches
            .iter()
            .any(|fetch| fetch.resource.url.starts_with("missing-"))
    );

    let small_img_url = small_img.display().to_string();
    let img_fetch = report
        .fetches
        .iter()
        .find(|fetch| fetch.resource.resolved == small_img_url)
        .unwrap();
    assert_eq!(img_fetch.resource.kind, "image");
    assert_eq!(img_fetch.resource.initiator, "img");
    assert_eq!(img_fetch.resource.url, "small-img.webp");
    assert_eq!(img_fetch.status, "fetched");
    assert_eq!(img_fetch.content_type.as_deref(), Some("image/webp"));
    assert_eq!(img_fetch.image_decode_status.as_deref(), Some("decoded"));
    let img_hash = img_fetch.decoded_hash.clone().unwrap();

    let bg_fetch = report
        .fetches
        .iter()
        .find(|fetch| fetch.resource.resolved == small_bg.display().to_string())
        .unwrap();
    assert_eq!(bg_fetch.resource.kind, "background_image");
    assert_eq!(bg_fetch.resource.initiator, "section");
    assert_eq!(bg_fetch.resource.url, "small-bg.webp");
    assert_eq!(bg_fetch.status, "fetched");
    assert_eq!(bg_fetch.content_type.as_deref(), Some("image/webp"));
    assert_eq!(bg_fetch.image_decode_status.as_deref(), Some("decoded"));
    let bg_hash = bg_fetch.decoded_hash.clone().unwrap();

    let render = session.current().unwrap();
    assert!(render.display_list.iter().any(|command| {
        matches!(
            command,
            DisplayCommand::Image {
                url: Some(url),
                decoded_hash: Some(hash),
                ..
            } if url == &small_img_url && hash == &img_hash
        )
    }));
    assert!(render.display_list.iter().any(|command| {
        matches!(
            command,
            DisplayCommand::BackgroundImage {
                url: Some(url),
                decoded_hash: Some(hash),
                ..
            } if url == &small_bg.display().to_string() && hash == &bg_hash
        )
    }));
}

#[tokio::test]
async fn image_picture_current_source_uses_source_current_src_for_placeholder_rendering() {
    let dir = tempfile::tempdir().unwrap();
    let page = dir.path().join("page.html");
    let hero = dir.path().join("hero.webp");
    fs::write(&hero, tiny_test_webp_bytes()).unwrap();
    fs::write(
        &page,
        r#"<html><body>
            <picture>
                <source data-current-src="hero.webp" type="image/webp">
                <img src="/assets/placeholder.gif" alt="Picture current WebP" width="80" height="24">
            </picture>
        </body></html>"#,
    )
    .unwrap();

    let mut session = BrowserSession::new(BrowserRenderOptions {
        width: 40,
        ..BrowserRenderOptions::default()
    });
    session.navigate(&page.display().to_string()).await.unwrap();

    let report = session.render_current_with_images(1024).await.unwrap();
    assert_eq!(report.image_count, 1);
    assert_eq!(report.decoded, 1);
    assert_eq!(report.failed, 0);

    let hero_url = hero.display().to_string();
    let fetch = report.fetches.first().unwrap();
    assert_eq!(fetch.resource.initiator, "img");
    assert_eq!(fetch.resource.url, "hero.webp");
    assert_eq!(fetch.resource.resolved, hero_url);
    assert_eq!(fetch.status, "fetched");
    assert_eq!(fetch.content_type.as_deref(), Some("image/webp"));
    assert_eq!(fetch.image_decode_status.as_deref(), Some("decoded"));
    let decoded_hash = fetch.decoded_hash.clone().unwrap();

    let render = session.current().unwrap();
    assert_eq!(render.decoded_images.len(), 1);
    assert!(render.display_list.iter().any(|command| {
        matches!(
            command,
            DisplayCommand::Image {
                url: Some(url),
                decoded_hash: Some(hash),
                ..
            } if url == &hero_url && hash == &decoded_hash
        )
    }));
}

#[tokio::test]
async fn image_source_coverage_uses_current_source_when_src_is_unsupported() {
    let dir = tempfile::tempdir().unwrap();
    let page = dir.path().join("page.html");
    let hero = dir.path().join("hero.webp");
    fs::write(&hero, tiny_test_webp_bytes()).unwrap();
    fs::write(
        &page,
        r#"<html><body>
            <img src="hero.avif" data-current-src="hero.webp" alt="Current fallback WebP" width="80" height="24">
        </body></html>"#,
    )
    .unwrap();

    let mut session = BrowserSession::new(BrowserRenderOptions {
        width: 40,
        ..BrowserRenderOptions::default()
    });
    session.navigate(&page.display().to_string()).await.unwrap();

    let report = session.render_current_with_images(1024).await.unwrap();
    assert_eq!(report.image_count, 1);
    assert_eq!(report.decoded, 1);
    assert_eq!(report.failed, 0);

    let hero_url = hero.display().to_string();
    let fetch = report.fetches.first().unwrap();
    assert_eq!(fetch.resource.initiator, "img");
    assert_eq!(fetch.resource.url, "hero.webp");
    assert_eq!(fetch.resource.resolved, hero_url);
    assert_eq!(fetch.status, "fetched");
    assert_eq!(fetch.content_type.as_deref(), Some("image/webp"));
    assert_eq!(fetch.image_decode_status.as_deref(), Some("decoded"));
    let decoded_hash = fetch.decoded_hash.clone().unwrap();

    let render = session.current().unwrap();
    assert_eq!(render.decoded_images.len(), 1);
    assert!(render.display_list.iter().any(|command| {
        matches!(
            command,
            DisplayCommand::Image {
                url: Some(url),
                decoded_hash: Some(hash),
                ..
            } if url == &hero_url && hash == &decoded_hash
        )
    }));
}

fn tiny_test_png_rgb_with_sub_filter() -> Vec<u8> {
    let filtered_scanlines = [0, 0, 0, 0, 255, 255, 255, 1, 255, 0, 0, 1, 0, 255];
    encode_test_png(2, 2, 2, &filtered_scanlines)
}

fn tiny_test_indexed_png_with_transparency() -> Vec<u8> {
    let filtered_scanlines = [0, 0, 1, 0, 2, 3];
    let palette = [0, 0, 0, 255, 255, 255, 255, 0, 0, 0, 0, 255];
    let transparency = [0];
    encode_test_indexed_png(2, 2, &filtered_scanlines, &palette, &transparency)
}

fn encode_test_png(width: u32, height: u32, color_type: u8, filtered_scanlines: &[u8]) -> Vec<u8> {
    use std::io::Write as _;

    let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::fast());
    encoder.write_all(filtered_scanlines).unwrap();
    let idat = encoder.finish().unwrap();

    let mut ihdr = Vec::with_capacity(13);
    ihdr.extend_from_slice(&width.to_be_bytes());
    ihdr.extend_from_slice(&height.to_be_bytes());
    ihdr.push(8);
    ihdr.push(color_type);
    ihdr.push(0);
    ihdr.push(0);
    ihdr.push(0);

    let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
    push_test_png_chunk(&mut png, b"IHDR", &ihdr);
    push_test_png_chunk(&mut png, b"IDAT", &idat);
    push_test_png_chunk(&mut png, b"IEND", &[]);
    png
}

fn encode_test_indexed_png(
    width: u32,
    height: u32,
    filtered_scanlines: &[u8],
    palette: &[u8],
    transparency: &[u8],
) -> Vec<u8> {
    use std::io::Write as _;

    let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::fast());
    encoder.write_all(filtered_scanlines).unwrap();
    let idat = encoder.finish().unwrap();

    let mut ihdr = Vec::with_capacity(13);
    ihdr.extend_from_slice(&width.to_be_bytes());
    ihdr.extend_from_slice(&height.to_be_bytes());
    ihdr.push(8);
    ihdr.push(3);
    ihdr.push(0);
    ihdr.push(0);
    ihdr.push(0);

    let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
    push_test_png_chunk(&mut png, b"IHDR", &ihdr);
    push_test_png_chunk(&mut png, b"PLTE", palette);
    push_test_png_chunk(&mut png, b"tRNS", transparency);
    push_test_png_chunk(&mut png, b"IDAT", &idat);
    push_test_png_chunk(&mut png, b"IEND", &[]);
    png
}

fn push_test_png_chunk(png: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    png.extend_from_slice(&(data.len() as u32).to_be_bytes());
    png.extend_from_slice(kind);
    png.extend_from_slice(data);
    png.extend_from_slice(&0u32.to_be_bytes());
}

#[test]
fn visual_baseline_runner_checks_hashes_and_writes_artifacts() {
    let dir = tempfile::tempdir().unwrap();
    let page = dir.path().join("page.html");
    fs::write(
        &page,
        r#"<html><head><title>Visual</title></head><body><h1>Visual Fixture</h1></body></html>"#,
    )
    .unwrap();
    let render = render_html(
        &page.display().to_string(),
        &fs::read(&page).unwrap(),
        BrowserRenderOptions::default(),
    );
    let raster = rasterize_render(&render, BrowserRasterOptions::default()).unwrap();
    let manifest = dir.path().join("manifest.json");
    fs::write(
        &manifest,
        format!(
            r#"{{
                    "fixtures": [{{
                        "name": "visual fixture",
                        "path": "page.html",
                        "expected_title": "Visual",
                        "expected_text": "Visual Fixture",
                        "expected_raster_hash": "{}"
                    }}]
                }}"#,
            raster.pixel_hash()
        ),
    )
    .unwrap();
    let artifact_dir = dir.path().join("artifacts");

    let report =
        verify_browser_visuals(&manifest, Some(&artifact_dir), None, true, None, None).unwrap();

    assert_eq!(report.fixture_count, 1);
    assert_eq!(report.checked, 1);
    assert_eq!(report.passed, 1);
    assert_eq!(report.failed, 0);
    assert_eq!(report.missing_baseline, 0);
    assert_eq!(report.comparisons[0].matched, Some(true));
    assert!(
        Path::new(report.comparisons[0].artifact.as_ref().unwrap())
            .extension()
            .is_some_and(|extension| extension == "pgm")
    );
    assert!(Path::new(report.comparisons[0].artifact.as_ref().unwrap()).exists());

    let diff_dir = dir.path().join("diffs");
    let diff_report = verify_browser_visuals(
        &manifest,
        Some(&diff_dir),
        Some(&artifact_dir),
        true,
        Some(0),
        Some(0.0),
    )
    .unwrap();
    assert_eq!(diff_report.diff_checked, 1);
    assert_eq!(diff_report.diff_passed, 1);
    assert_eq!(diff_report.diff_failed, 0);
    assert_eq!(diff_report.comparisons[0].diff_pixels, Some(0));
    assert_eq!(diff_report.comparisons[0].diff_ratio, Some(0.0));
    assert!(Path::new(diff_report.comparisons[0].diff_artifact.as_ref().unwrap()).exists());

    let bad_baseline_dir = dir.path().join("bad-baseline");
    fs::create_dir_all(&bad_baseline_dir).unwrap();
    let baseline_name = Path::new(report.comparisons[0].artifact.as_ref().unwrap())
        .file_name()
        .unwrap();
    let mut baseline = fs::read(report.comparisons[0].artifact.as_ref().unwrap()).unwrap();
    let header_end = baseline
        .windows(b"\n255\n".len())
        .position(|window| window == b"\n255\n")
        .unwrap()
        + b"\n255\n".len();
    baseline[header_end] = if baseline[header_end] == 0 { 255 } else { 0 };
    fs::write(bad_baseline_dir.join(baseline_name), baseline).unwrap();

    let failed_diff_report = verify_browser_visuals(
        &manifest,
        Some(&dir.path().join("bad-diffs")),
        Some(&bad_baseline_dir),
        true,
        Some(0),
        Some(0.0),
    )
    .unwrap();
    assert_eq!(failed_diff_report.diff_checked, 1);
    assert_eq!(failed_diff_report.diff_failed, 1);
    assert_eq!(failed_diff_report.failed, 1);
    assert_eq!(failed_diff_report.comparisons[0].diff_pixels, Some(1));
}

#[test]
fn visual_baseline_runner_can_require_all_fixture_hashes() {
    let dir = tempfile::tempdir().unwrap();
    let page = dir.path().join("page.html");
    fs::write(&page, r#"<html><body><p>No baseline</p></body></html>"#).unwrap();
    let manifest = dir.path().join("manifest.json");
    fs::write(
            &manifest,
            r#"{"fixtures":[{"name":"missing visual","path":"page.html","expected_text":"No baseline"}]}"#,
        )
        .unwrap();

    let report = verify_browser_visuals(&manifest, None, None, true, None, None).unwrap();

    assert_eq!(report.checked, 0);
    assert_eq!(report.comparisons.len(), 1);
    assert_eq!(report.comparisons[0].matched, None);
    assert_eq!(report.missing_baseline, 1);
    assert_eq!(report.failed, 1);
    assert!(
        report.failures[0]
            .reason
            .contains("missing expected_raster_hash")
    );
}

#[tokio::test]
async fn browser_session_click_selector_updates_current_render() {
    let dir = tempfile::tempdir().unwrap();
    let page = dir.path().join("click.html");
    fs::write(
            &page,
            r#"
            <html><body>
              <button id="go" onclick="document.querySelector('#out').innerText = 'Clicked'">Go</button>
              <p id="out">Waiting</p>
            </body></html>
            "#,
        )
        .unwrap();

    let mut session = BrowserSession::new(BrowserRenderOptions::default());
    session.navigate(&page.display().to_string()).await.unwrap();
    assert_eq!(session.current().unwrap().text, "Go\nWaiting");
    let render = session.click_selector("#go").unwrap();
    assert_eq!(render.text, "Go\nClicked");
    let history = session.snapshot();
    assert_eq!(history.entries.len(), 1);
    assert_eq!(history.current_index, Some(0));
}

#[tokio::test]
async fn browser_session_click_selector_default_action_navigates_anchor() {
    let dir = tempfile::tempdir().unwrap();
    let first = dir.path().join("first.html");
    let second = dir.path().join("second.html");
    fs::write(
            &first,
            r#"<html><head><title>First</title></head><body><a id="go" href="second.html">Go</a></body></html>"#,
        )
        .unwrap();
    fs::write(
        &second,
        r#"<html><head><title>Second</title></head><body>Arrived</body></html>"#,
    )
    .unwrap();

    let mut session = BrowserSession::new(BrowserRenderOptions::default());
    session
        .navigate(&first.display().to_string())
        .await
        .unwrap();
    let render = session
        .click_selector_with_default_action("#go")
        .await
        .unwrap();
    assert_eq!(render.title, "Second");
    assert_eq!(render.text, "Arrived");
    let history = session.snapshot();
    assert_eq!(history.entries.len(), 2);
    assert_eq!(history.current_index, Some(1));
}

#[test]
fn render_records_fragment_targets_for_ids_and_legacy_anchor_names() {
    let render = render_html(
        "mem://page#details",
        br#"
            <html><body>
              <p>Intro</p>
              <section id="details"><h2>Details</h2></section>
              <a name="legacy">Legacy</a>
            </body></html>
            "#,
        BrowserRenderOptions::default(),
    );

    let details_y = render.fragment_scroll_y("details").unwrap();
    let legacy_y = render.fragment_scroll_y("legacy").unwrap();
    assert!(details_y < legacy_y);
    assert_eq!(render.source_fragment_scroll_y(), Some(details_y));
}

#[tokio::test]
async fn browser_session_fragment_link_replaces_existing_fragment() {
    let dir = tempfile::tempdir().unwrap();
    let page = dir.path().join("page.html");
    fs::write(
        &page,
        r##"
            <html><head><title>Fragments</title></head><body>
              <a id="jump" href="#details">Jump</a>
              <p id="intro">Intro</p>
              <section id="details"><h2>Details</h2></section>
            </body></html>
            "##,
    )
    .unwrap();

    let mut session = BrowserSession::new(BrowserRenderOptions::default());
    session
        .navigate(&format!("{}#intro", page.display()))
        .await
        .unwrap();
    let render = session
        .click_selector_with_default_action("#jump")
        .await
        .unwrap();

    assert!(render.source.ends_with("page.html#details"));
    assert_eq!(
        render.source_fragment_scroll_y(),
        render.fragment_scroll_y("details")
    );
}

#[tokio::test]
async fn browser_session_click_selector_uses_href_after_onclick_mutation() {
    let dir = tempfile::tempdir().unwrap();
    let first = dir.path().join("first.html");
    let wrong = dir.path().join("wrong.html");
    let second = dir.path().join("second.html");
    fs::write(
            &first,
            r#"<html><head><title>First</title></head><body><a id="go" href="wrong.html" onclick="this.href = 'second.html'">Go</a></body></html>"#,
        )
        .unwrap();
    fs::write(
        &wrong,
        r#"<html><head><title>Wrong</title></head><body>Wrong target</body></html>"#,
    )
    .unwrap();
    fs::write(
        &second,
        r#"<html><head><title>Second</title></head><body>Arrived</body></html>"#,
    )
    .unwrap();

    let mut session = BrowserSession::new(BrowserRenderOptions::default());
    session
        .navigate(&first.display().to_string())
        .await
        .unwrap();
    let render = session
        .click_selector_with_default_action("#go")
        .await
        .unwrap();
    assert_eq!(render.title, "Second");
    assert_eq!(render.text, "Arrived");
}

#[tokio::test]
async fn browser_session_click_anchor_default_ignores_later_timer_href_mutation() {
    let dir = tempfile::tempdir().unwrap();
    let first = dir.path().join("first.html");
    let wrong = dir.path().join("wrong.html");
    let second = dir.path().join("second.html");
    fs::write(
            &first,
            r#"<html><head><title>First</title></head><body><a id="go" href="second.html" onclick="setTimeout(() => { document.getElementById('go').href = 'wrong.html'; }, 0)">Go</a></body></html>"#,
        )
        .unwrap();
    fs::write(
        &wrong,
        r#"<html><head><title>Wrong</title></head><body>Wrong target</body></html>"#,
    )
    .unwrap();
    fs::write(
        &second,
        r#"<html><head><title>Second</title></head><body>Arrived</body></html>"#,
    )
    .unwrap();

    let mut session = BrowserSession::new(BrowserRenderOptions::default());
    session
        .navigate(&first.display().to_string())
        .await
        .unwrap();
    let render = session
        .click_selector_with_default_action("#go")
        .await
        .unwrap();

    assert_eq!(render.title, "Second");
    assert_eq!(render.text, "Arrived");
}

#[tokio::test]
async fn browser_session_click_selector_uses_parent_anchor_for_child_target() {
    let dir = tempfile::tempdir().unwrap();
    let first = dir.path().join("first.html");
    let second = dir.path().join("second.html");
    fs::write(
            &first,
            r#"<html><head><title>First</title></head><body><a href="second.html"><span class="label">Child target</span></a></body></html>"#,
        )
        .unwrap();
    fs::write(
        &second,
        r#"<html><head><title>Second</title></head><body>Arrived</body></html>"#,
    )
    .unwrap();

    let mut session = BrowserSession::new(BrowserRenderOptions::default());
    session
        .navigate(&first.display().to_string())
        .await
        .unwrap();
    let render = session
        .click_selector_with_default_action(".label")
        .await
        .unwrap();
    assert_eq!(render.title, "Second");
}

#[tokio::test]
async fn browser_session_click_submit_button_submits_get_form_with_state() {
    let dir = tempfile::tempdir().unwrap();
    let form_page = dir.path().join("form.html");
    let results_page = dir.path().join("results.html");
    fs::write(
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
    fs::write(
        &results_page,
        "<html><head><title>Results</title></head><body>done</body></html>",
    )
    .unwrap();

    let mut session = BrowserSession::new(BrowserRenderOptions::default());
    session
        .navigate(&form_page.display().to_string())
        .await
        .unwrap();
    session.set_form_field(0, "q", "typed value").unwrap();
    let render = session
        .click_selector_with_default_action("#go")
        .await
        .unwrap();

    assert_eq!(render.title, "Results");
    assert!(
        render
            .source
            .ends_with("results.html?q=typed+value&commit=yes")
    );
}

#[tokio::test]
async fn browser_session_click_input_submit_submits_get_form() {
    let dir = tempfile::tempdir().unwrap();
    let form_page = dir.path().join("form.html");
    let results_page = dir.path().join("results.html");
    fs::write(
        &form_page,
        r#"
            <html><head><title>Form</title></head><body>
              <form action="results.html" method="get">
                <input name="q" value="old">
                <input id="go" type="submit" name="commit" value="search">
              </form>
            </body></html>
            "#,
    )
    .unwrap();
    fs::write(
        &results_page,
        "<html><head><title>Results</title></head><body>done</body></html>",
    )
    .unwrap();

    let mut session = BrowserSession::new(BrowserRenderOptions::default());
    session
        .navigate(&form_page.display().to_string())
        .await
        .unwrap();
    let render = session
        .click_selector_with_default_action("#go")
        .await
        .unwrap();

    assert_eq!(render.title, "Results");
    assert!(render.source.ends_with("results.html?q=old&commit=search"));
}

#[tokio::test]
async fn browser_session_click_button_without_value_submits_empty_submitter_value() {
    let dir = tempfile::tempdir().unwrap();
    let form_page = dir.path().join("form.html");
    let results_page = dir.path().join("results.html");
    fs::write(
        &form_page,
        r#"
            <html><head><title>Form</title></head><body>
              <form action="results.html" method="get">
                <input name="q" value="old">
                <button id="go" name="commit">Go</button>
              </form>
            </body></html>
            "#,
    )
    .unwrap();
    fs::write(
        &results_page,
        "<html><head><title>Results</title></head><body>done</body></html>",
    )
    .unwrap();

    let mut session = BrowserSession::new(BrowserRenderOptions::default());
    session
        .navigate(&form_page.display().to_string())
        .await
        .unwrap();
    let render = session
        .click_selector_with_default_action("#go")
        .await
        .unwrap();

    assert_eq!(render.title, "Results");
    assert!(render.source.ends_with("results.html?q=old&commit="));
}

#[tokio::test]
async fn browser_session_click_child_inside_submit_button_submits_form() {
    let dir = tempfile::tempdir().unwrap();
    let form_page = dir.path().join("form.html");
    let results_page = dir.path().join("results.html");
    fs::write(
        &form_page,
        r#"
            <html><head><title>Form</title></head><body>
              <form action="results.html" method="get">
                <input name="q" value="nested">
                <button id="go" name="commit" value="yes"><span id="label">Go</span></button>
              </form>
            </body></html>
            "#,
    )
    .unwrap();
    fs::write(
        &results_page,
        "<html><head><title>Results</title></head><body>done</body></html>",
    )
    .unwrap();

    let mut session = BrowserSession::new(BrowserRenderOptions::default());
    session
        .navigate(&form_page.display().to_string())
        .await
        .unwrap();
    let render = session
        .click_selector_with_default_action("#label")
        .await
        .unwrap();

    assert_eq!(render.title, "Results");
    assert!(render.source.ends_with("results.html?q=nested&commit=yes"));
}

#[tokio::test]
async fn browser_session_click_non_submit_controls_do_not_submit() {
    let dir = tempfile::tempdir().unwrap();
    let form_page = dir.path().join("form.html");
    let results_page = dir.path().join("results.html");
    fs::write(
        &form_page,
        r#"
            <html><head><title>Form</title></head><body>
              <form action="results.html" method="get">
                <input name="q" value="old">
                <button id="plain" type="button">Plain</button>
                <button id="reset" type="reset">Reset</button>
                <input id="disabled" type="submit" disabled name="commit" value="blocked">
              </form>
            </body></html>
            "#,
    )
    .unwrap();
    fs::write(
        &results_page,
        "<html><head><title>Results</title></head><body>done</body></html>",
    )
    .unwrap();

    let mut session = BrowserSession::new(BrowserRenderOptions::default());
    session
        .navigate(&form_page.display().to_string())
        .await
        .unwrap();

    for selector in ["#plain", "#reset", "#disabled"] {
        let render = session
            .click_selector_with_default_action(selector)
            .await
            .unwrap();
        assert_eq!(render.title, "Form");
        assert_eq!(session.snapshot().entries.len(), 1);
    }
}

#[tokio::test]
async fn browser_session_click_reset_button_clears_form_fill_state() {
    let dir = tempfile::tempdir().unwrap();
    let form_page = dir.path().join("form.html");
    fs::write(
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
    session.set_form_field(0, "q", "typed").unwrap();
    let render = session
        .click_selector_with_default_action("#reset")
        .await
        .unwrap();

    assert_eq!(render.title, "Form");
    assert_eq!(render.forms[0].controls[0].value, "old");
    assert_eq!(session.snapshot().entries.len(), 1);
}

#[tokio::test]
async fn browser_session_click_input_reset_clears_form_fill_state() {
    let dir = tempfile::tempdir().unwrap();
    let form_page = dir.path().join("form.html");
    fs::write(
        &form_page,
        r#"
            <html><head><title>Form</title></head><body>
              <form>
                <input name="q" value="old">
                <input id="reset" type="reset" value="Reset">
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
    session.set_form_field(0, "q", "typed").unwrap();
    let render = session
        .click_selector_with_default_action("#reset")
        .await
        .unwrap();

    assert_eq!(render.forms[0].controls[0].value, "old");
    assert_eq!(session.snapshot().entries.len(), 1);
}

#[tokio::test]
async fn browser_session_click_child_inside_reset_button_resets_form() {
    let dir = tempfile::tempdir().unwrap();
    let form_page = dir.path().join("form.html");
    fs::write(
        &form_page,
        r#"
            <html><head><title>Form</title></head><body>
              <form>
                <input name="q" value="old">
                <button id="reset" type="reset"><span id="label">Reset</span></button>
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
    session.set_form_field(0, "q", "typed").unwrap();
    let render = session
        .click_selector_with_default_action("#label")
        .await
        .unwrap();

    assert_eq!(render.forms[0].controls[0].value, "old");
}

#[tokio::test]
async fn browser_session_click_reset_button_honors_prevent_default() {
    let dir = tempfile::tempdir().unwrap();
    let form_page = dir.path().join("form.html");
    fs::write(
        &form_page,
        r#"
            <html><head><title>Form</title></head><body>
              <form>
                <input name="q" value="old">
                <button id="reset" type="reset" onclick="return false">Reset</button>
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
    session.set_form_field(0, "q", "typed").unwrap();
    let render = session
        .click_selector_with_default_action("#reset")
        .await
        .unwrap();

    assert_eq!(render.forms[0].controls[0].value, "typed");
}

#[tokio::test]
async fn browser_session_click_reset_button_only_clears_target_form() {
    let dir = tempfile::tempdir().unwrap();
    let form_page = dir.path().join("form.html");
    fs::write(
        &form_page,
        r#"
            <html><head><title>Form</title></head><body>
              <form id="first">
                <input name="q" value="one">
                <button id="reset-first" type="reset">Reset</button>
              </form>
              <form id="second">
                <input name="q" value="two">
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
    session.set_form_field(0, "q", "typed one").unwrap();
    session.set_form_field(1, "q", "typed two").unwrap();
    let render = session
        .click_selector_with_default_action("#reset-first")
        .await
        .unwrap();

    assert_eq!(render.forms[0].controls[0].value, "one");
    assert_eq!(render.forms[1].controls[0].value, "typed two");
}

#[tokio::test]
async fn browser_session_click_reset_button_drains_post_click_timers() {
    let dir = tempfile::tempdir().unwrap();
    let form_page = dir.path().join("form.html");
    fs::write(
            &form_page,
            r#"
            <html><head><title>Form</title></head><body>
              <form>
                <input name="q" value="old">
                <button id="reset" type="reset" onclick="setTimeout(() => { document.getElementById('status').textContent = 'timer ran'; }, 0)">Reset</button>
              </form>
              <p id="status">waiting</p>
            </body></html>
            "#,
        )
        .unwrap();

    let mut session = BrowserSession::new(BrowserRenderOptions::default());
    session
        .navigate(&form_page.display().to_string())
        .await
        .unwrap();
    session.set_form_field(0, "q", "typed").unwrap();
    let render = session
        .click_selector_with_default_action("#reset")
        .await
        .unwrap();

    assert_eq!(render.forms[0].controls[0].value, "old");
    assert!(render.text.contains("timer ran"));
}

#[tokio::test]
async fn browser_session_click_submit_button_honors_prevent_default() {
    let dir = tempfile::tempdir().unwrap();
    let form_page = dir.path().join("form.html");
    let results_page = dir.path().join("results.html");
    fs::write(
        &form_page,
        r#"
            <html><head><title>Form</title></head><body>
              <form action="results.html" method="get">
                <input name="q" value="old">
                <button id="go" onclick="return false">Go</button>
              </form>
            </body></html>
            "#,
    )
    .unwrap();
    fs::write(
        &results_page,
        "<html><head><title>Results</title></head><body>done</body></html>",
    )
    .unwrap();

    let mut session = BrowserSession::new(BrowserRenderOptions::default());
    session
        .navigate(&form_page.display().to_string())
        .await
        .unwrap();
    let render = session
        .click_selector_with_default_action("#go")
        .await
        .unwrap();

    assert_eq!(render.title, "Form");
    assert_eq!(session.snapshot().entries.len(), 1);
}

#[tokio::test]
async fn browser_session_click_submit_button_posts_urlencoded_form() {
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
                "<html><head><title>Form</title></head><body><form action=\"/submit\" method=\"post\"><input name=\"q\" value=\"old\"><button id=\"go\" name=\"commit\" value=\"yes\">Go</button></form></body></html>"
            } else {
                assert!(first_line.starts_with("POST /submit "));
                assert_eq!(request_body, "q=typed&commit=yes");
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
    session.set_form_field(0, "q", "typed").unwrap();
    let render = session
        .click_selector_with_default_action("#go")
        .await
        .unwrap();
    server.await.unwrap();

    assert_eq!(render.title, "Posted");
    assert_eq!(render.text, "accepted");
}

#[tokio::test]
async fn browser_session_click_at_default_action_navigates_anchor_text() {
    let dir = tempfile::tempdir().unwrap();
    let first = dir.path().join("first.html");
    let second = dir.path().join("second.html");
    fs::write(
            &first,
            r#"<html><head><title>First</title></head><body><a href="second.html">Go</a></body></html>"#,
        )
        .unwrap();
    fs::write(
        &second,
        r#"<html><head><title>Second</title></head><body>Arrived</body></html>"#,
    )
    .unwrap();

    let mut session = BrowserSession::new(BrowserRenderOptions::default());
    session
        .navigate(&first.display().to_string())
        .await
        .unwrap();
    let render = session.click_at_with_default_action(0, 0).await.unwrap();
    assert_eq!(render.title, "Second");
    assert_eq!(render.text, "Arrived");
    assert_eq!(session.snapshot().current_index, Some(1));
}

#[tokio::test]
async fn browser_session_click_at_prefers_visible_anchor_text_over_later_image_fill() {
    let dir = tempfile::tempdir().unwrap();
    let first = dir.path().join("first.html");
    let second = dir.path().join("second.html");
    fs::write(
        &first,
        r#"<html><head><title>First</title></head><body><a href="second.html">Go</a><img src="missing.png" width="64" height="12" style="position:absolute; top:0; left:0" alt=""></body></html>"#,
    )
    .unwrap();
    fs::write(
        &second,
        r#"<html><head><title>Second</title></head><body>Arrived</body></html>"#,
    )
    .unwrap();

    let mut session = BrowserSession::new(BrowserRenderOptions::default());
    session
        .navigate(&first.display().to_string())
        .await
        .unwrap();
    let expected_target = resolve_browser_href(&first.display().to_string(), "second.html");
    assert_eq!(
        session.link_target_at(0, 0).as_deref(),
        Some(expected_target.as_str())
    );
    let render = session.click_at_with_default_action(0, 0).await.unwrap();
    assert_eq!(render.title, "Second");
    assert_eq!(render.text, "Arrived");
    assert_eq!(session.snapshot().current_index, Some(1));
}

#[tokio::test]
async fn browser_session_click_viewport_at_hits_fixed_link_after_scroll_over_image() {
    let dir = tempfile::tempdir().unwrap();
    let first = dir.path().join("first.html");
    let second = dir.path().join("second.html");
    fs::write(
        &first,
        r#"<html><head><title>First</title></head><body><a href="second.html" style="position:fixed; top:0; left:0">Fixed Go</a><img src="missing.png" width="32" height="96" alt=""><p>Scrollable body</p></body></html>"#,
    )
    .unwrap();
    fs::write(
        &second,
        r#"<html><head><title>Second</title></head><body>Arrived</body></html>"#,
    )
    .unwrap();

    let mut session = BrowserSession::new(BrowserRenderOptions {
        width: 40,
        ..BrowserRenderOptions::default()
    });
    session
        .navigate(&first.display().to_string())
        .await
        .unwrap();
    let viewport = BrowserViewportState {
        x: 0,
        y: 5,
        width: 20,
        height: 4,
    };
    let document_viewport = browser_document_viewport(session.current().unwrap(), viewport, None);
    assert_eq!(document_viewport.viewport.y, 5);
    let expected_target = resolve_browser_href(&first.display().to_string(), "second.html");
    assert_eq!(
        session.link_target_at_viewport(viewport, 0, 0).as_deref(),
        Some(expected_target.as_str())
    );

    let render = session
        .click_viewport_at_with_default_action(viewport, 0, 0)
        .await
        .unwrap();
    assert_eq!(render.title, "Second");
    assert_eq!(render.text, "Arrived");
    assert_eq!(session.snapshot().current_index, Some(1));
}

#[test]
fn coordinate_hit_targets_track_multiline_anchor_text() {
    let render = render_html(
        "mem://page",
        br#"<html><body><p>Intro</p><a href="next.html">Second</a></body></html>"#,
        BrowserRenderOptions::default(),
    );

    assert_eq!(render.text, "Intro\nSecond");
    assert_eq!(render.display_list.len(), render.hit_targets.len());
    assert!(hit_test_target_node(&render, 0, 1).is_some());
}

#[test]
fn coordinate_hit_targets_stay_out_of_serialized_render_json() {
    let render = render_html(
        "mem://page",
        br#"<html><body><button>Go</button></body></html>"#,
        BrowserRenderOptions::default(),
    );

    let json = serde_json::to_string(&render).unwrap();
    assert!(!json.contains("hit_targets"));
}

#[tokio::test]
async fn browser_session_click_at_handles_multiline_anchor_text() {
    let dir = tempfile::tempdir().unwrap();
    let first = dir.path().join("first.html");
    let second = dir.path().join("second.html");
    fs::write(
            &first,
            r#"<html><head><title>First</title></head><body><p>Intro</p><a href="second.html">Second</a></body></html>"#,
        )
        .unwrap();
    fs::write(
        &second,
        r#"<html><head><title>Second</title></head><body>Arrived</body></html>"#,
    )
    .unwrap();

    let mut session = BrowserSession::new(BrowserRenderOptions::default());
    session
        .navigate(&first.display().to_string())
        .await
        .unwrap();
    assert_eq!(session.current().unwrap().text, "Intro\nSecond");
    let render = session.click_at_with_default_action(0, 1).await.unwrap();
    assert_eq!(render.title, "Second");
}

#[tokio::test]
async fn browser_session_click_at_mutates_button_without_navigation() {
    let dir = tempfile::tempdir().unwrap();
    let page = dir.path().join("click.html");
    fs::write(
        &page,
        r#"
            <html><head><title>Click</title></head><body>
              <button onclick="document.querySelector('#out').innerText = 'Clicked'">Go</button>
              <p id="out">Waiting</p>
            </body></html>
            "#,
    )
    .unwrap();

    let mut session = BrowserSession::new(BrowserRenderOptions::default());
    session.navigate(&page.display().to_string()).await.unwrap();
    let render = session.click_at_with_default_action(0, 0).await.unwrap();
    assert_eq!(render.title, "Click");
    assert_eq!(render.text, "Go\nClicked");
    assert_eq!(session.snapshot().entries.len(), 1);
}

#[tokio::test]
async fn browser_session_click_at_bubbles_and_can_prevent_anchor_default() {
    let dir = tempfile::tempdir().unwrap();
    let first = dir.path().join("first.html");
    let second = dir.path().join("second.html");
    fs::write(
        &first,
        r#"
            <html><head><title>First</title></head><body>
              <a id="go" href="second.html"><span>Go</span></a>
              <p id="out">Waiting</p>
              <script>
                const go = document.getElementById("go");
                const out = document.getElementById("out");
                go.addEventListener("click", (event) => {
                  event.preventDefault();
                  out.textContent = "Stayed";
                });
              </script>
            </body></html>
            "#,
    )
    .unwrap();
    fs::write(
        &second,
        r#"<html><head><title>Second</title></head><body>Arrived</body></html>"#,
    )
    .unwrap();

    let mut session = BrowserSession::new(BrowserRenderOptions::default());
    session
        .navigate(&first.display().to_string())
        .await
        .unwrap();
    let render = session.click_at_with_default_action(0, 0).await.unwrap();
    assert_eq!(render.title, "First");
    assert_eq!(render.text, "Go\nStayed");
    assert_eq!(session.snapshot().entries.len(), 1);
}

#[tokio::test]
async fn browser_session_click_selector_return_false_prevents_default_navigation() {
    let dir = tempfile::tempdir().unwrap();
    let first = dir.path().join("first.html");
    let second = dir.path().join("second.html");
    fs::write(
            &first,
            r#"
            <html><head><title>First</title></head><body>
              <a id="go" href="second.html" onclick="document.querySelector('#out').innerText = 'Stayed'; return false">Go</a>
              <p id="out">Waiting</p>
            </body></html>
            "#,
        )
        .unwrap();
    fs::write(
        &second,
        r#"<html><head><title>Second</title></head><body>Arrived</body></html>"#,
    )
    .unwrap();

    let mut session = BrowserSession::new(BrowserRenderOptions::default());
    session
        .navigate(&first.display().to_string())
        .await
        .unwrap();
    let render = session
        .click_selector_with_default_action("#go")
        .await
        .unwrap();
    assert_eq!(render.title, "First");
    assert_eq!(render.text, "Go\nStayed");
    let history = session.snapshot();
    assert_eq!(history.entries.len(), 1);
    assert_eq!(history.current_index, Some(0));
}

#[tokio::test]
async fn browser_session_click_selector_prevent_default_listener_cancels_navigation() {
    let dir = tempfile::tempdir().unwrap();
    let first = dir.path().join("first.html");
    let second = dir.path().join("second.html");
    fs::write(
        &first,
        r#"
            <html><head><title>First</title></head><body>
              <a id="go" href="second.html">Go</a>
              <p id="out">Waiting</p>
              <script>
                const go = document.getElementById("go");
                const out = document.getElementById("out");
                go.addEventListener("click", (event) => {
                  event.preventDefault();
                  out.textContent = "Stayed";
                });
              </script>
            </body></html>
            "#,
    )
    .unwrap();
    fs::write(
        &second,
        r#"<html><head><title>Second</title></head><body>Arrived</body></html>"#,
    )
    .unwrap();

    let mut session = BrowserSession::new(BrowserRenderOptions::default());
    session
        .navigate(&first.display().to_string())
        .await
        .unwrap();
    let render = session
        .click_selector_with_default_action("#go")
        .await
        .unwrap();
    assert_eq!(render.title, "First");
    assert_eq!(render.text, "Go\nStayed");
    assert_eq!(session.snapshot().entries.len(), 1);
}

#[tokio::test]
async fn browser_session_activates_link_by_index() {
    let dir = tempfile::tempdir().unwrap();
    let first = dir.path().join("first.html");
    let second = dir.path().join("second.html");
    fs::write(
            &first,
            r#"<html><head><title>First</title></head><body><a href="second.html">Second page</a></body></html>"#,
        )
        .unwrap();
    fs::write(
        &second,
        r#"<html><head><title>Second</title></head><body>Arrived</body></html>"#,
    )
    .unwrap();

    let mut session = BrowserSession::new(BrowserRenderOptions::default());
    session
        .navigate(&first.display().to_string())
        .await
        .unwrap();
    assert_eq!(session.current_links()[0].text, "Second page");

    let render = session.activate_link(0).await.unwrap();
    assert_eq!(render.title, "Second");
    assert_eq!(render.text, "Arrived");

    let history = session.snapshot();
    assert_eq!(history.entries.len(), 2);
    assert_eq!(history.current_index, Some(1));
}

#[tokio::test]
async fn browser_session_exposes_current_forms() {
    let dir = tempfile::tempdir().unwrap();
    let page = dir.path().join("form.html");
    fs::write(
        &page,
        r#"
            <html><head><title>Form</title></head><body>
              <form action="results.html" method="post">
                <input name="q" value="old">
              </form>
            </body></html>
            "#,
    )
    .unwrap();

    let mut session = BrowserSession::new(BrowserRenderOptions::default());
    assert!(session.current_forms().is_empty());
    session.navigate(&page.display().to_string()).await.unwrap();

    let forms = session.current_forms();
    assert_eq!(forms.len(), 1);
    assert_eq!(forms[0].index, 0);
    assert_eq!(forms[0].method, "POST");
    assert_eq!(forms[0].action, "results.html");
    assert_eq!(forms[0].controls[0].name, "q");
}

#[tokio::test]
async fn browser_session_activates_links_by_text_and_selector() {
    let dir = tempfile::tempdir().unwrap();
    let first = dir.path().join("first.html");
    let second = dir.path().join("second.html");
    let third = dir.path().join("third.html");
    fs::write(
        &first,
        r#"
            <html><head><title>First</title></head><body>
              <a id="to-second" href="second.html">Second page</a>
              <a id="to-third" href="third.html"><span class="label">Third page</span></a>
            </body></html>
            "#,
    )
    .unwrap();
    fs::write(
        &second,
        r#"<html><head><title>Second</title></head><body>Second target</body></html>"#,
    )
    .unwrap();
    fs::write(
        &third,
        r#"<html><head><title>Third</title></head><body>Third target</body></html>"#,
    )
    .unwrap();

    let mut session = BrowserSession::new(BrowserRenderOptions::default());
    session
        .navigate(&first.display().to_string())
        .await
        .unwrap();

    let render = session.activate_link_text("Second page").await.unwrap();
    assert_eq!(render.title, "Second");
    session.back().unwrap();

    let render = session.activate_link_selector(".label").await.unwrap();
    assert_eq!(render.title, "Third");
    assert_eq!(render.text, "Third target");
}

#[tokio::test]
async fn browser_session_link_activation_does_not_dispatch_onclick() {
    let dir = tempfile::tempdir().unwrap();
    let first = dir.path().join("first.html");
    let second = dir.path().join("second.html");
    fs::write(
            &first,
            r#"
            <html><head><title>First</title></head><body>
              <a id="go" href="second.html" onclick="document.querySelector('#out').innerText = 'Clicked'">Go</a>
              <p id="out">Waiting</p>
            </body></html>
            "#,
        )
        .unwrap();
    fs::write(
        &second,
        r#"<html><head><title>Second</title></head><body>Arrived</body></html>"#,
    )
    .unwrap();

    let mut session = BrowserSession::new(BrowserRenderOptions::default());
    session
        .navigate(&first.display().to_string())
        .await
        .unwrap();
    session.activate_link_selector("#go").await.unwrap();
    assert_eq!(session.current().unwrap().title, "Second");

    let render = session.back().unwrap();
    assert_eq!(render.text, "Go\nWaiting");
}

#[test]
fn wraps_text_to_viewport_width() {
    let render = render_html(
        "mem://page",
        b"<body><p>one two three four five six seven eight</p></body>",
        BrowserRenderOptions {
            width: 20,
            ..BrowserRenderOptions::default()
        },
    );

    assert!(render.text.lines().count() > 1);
    assert!(render.text.lines().all(|line| line.len() <= 20));
    assert_eq!(render.display_list.len(), render.text.lines().count());
    assert_eq!(
        render.display_list.first(),
        Some(&DisplayCommand::Text {
            x: 0,
            y: 0,
            text: "one two three four".to_owned(),
        })
    );
}

#[test]
fn extracts_and_resolves_links() {
    let render = render_html(
        "/tmp/site/page.html",
        br#"
            <html><body>
            <a href="next.html"> Next &amp; page </a>
            <a href="https://example.com/out">Out</a>
            </body></html>
            "#,
        BrowserRenderOptions::default(),
    );

    assert_eq!(
        render.links,
        vec![
            BrowserLink {
                text: "Next & page".to_owned(),
                href: "next.html".to_owned(),
                resolved: "/tmp/site/next.html".to_owned(),
            },
            BrowserLink {
                text: "Out".to_owned(),
                href: "https://example.com/out".to_owned(),
                resolved: "https://example.com/out".to_owned(),
            },
        ]
    );
}

#[test]
fn discovers_static_subresources() {
    let render = render_html(
            "https://example.com/app/page.html",
            br#"
            <html><head>
              <link rel="stylesheet" href="/app.css" media="screen">
              <link rel="shortcut icon" href="favicon.ico">
              <script src="app.js" type="module"></script>
            </head><body>
              <img src="img/a.png" srcset="img/a@2.png 2x, /img/a-large.png 1000w" alt="Hero">
              <video src="movie.mp4" poster="poster.jpg"><source src="movie.webm" type="video/webm"></video>
              <iframe src="/frame.html"></iframe>
              <object data="thing.swf"></object>
            </body></html>
            "#,
            BrowserRenderOptions::default(),
        );

    let kinds = render
        .resources
        .iter()
        .map(|resource| resource.kind.as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        kinds,
        vec![
            "stylesheet",
            "icon",
            "script",
            "image",
            "image_candidate",
            "image_candidate",
            "media",
            "poster",
            "media_source",
            "frame",
            "object"
        ]
    );
    assert!(render.resources.iter().any(|resource| {
        resource.kind == "stylesheet"
            && resource.resolved == "https://example.com/app.css"
            && resource.media.as_deref() == Some("screen")
    }));
    assert!(render.resources.iter().any(|resource| {
        resource.kind == "script"
            && resource.resolved == "https://example.com/app/app.js"
            && resource.type_hint.as_deref() == Some("module")
    }));
    assert!(render.resources.iter().any(|resource| {
        resource.kind == "image"
            && resource.alt.as_deref() == Some("Hero")
            && resource.resolved == "https://example.com/app/img/a.png"
    }));
    assert!(render.resources.iter().any(|resource| {
        resource.kind == "object" && resource.resolved == "https://example.com/app/thing.swf"
    }));
}

#[test]
fn image_resource_bundle_discovers_image_preload_subresources() {
    let render = render_html(
        "https://example.com/app/page.html",
        br#"
            <html><head>
              <link rel="preload" as="image" href="/img/hero.jpg" imagesrcset="/img/hero.webp 640w, /img/hero@2x.jpg 1280w" imagesizes="100vw" type="image/webp">
              <link rel="preload" as="font" href="/fonts/app.woff2">
            </head><body></body></html>
            "#,
        BrowserRenderOptions::default(),
    );

    assert!(render.resources.iter().any(|resource| {
        resource.kind == "image"
            && resource.initiator == "link"
            && resource.url == "/img/hero.jpg"
            && resource.resolved == "https://example.com/img/hero.jpg"
            && resource.rel.as_deref() == Some("preload")
            && resource.type_hint.as_deref() == Some("image/webp")
    }));
    assert!(render.resources.iter().any(|resource| {
        resource.kind == "image_candidate"
            && resource.initiator == "link"
            && resource.url == "/img/hero.webp"
            && resource.resolved == "https://example.com/img/hero.webp"
    }));
    assert!(render.resources.iter().any(|resource| {
        resource.kind == "image_candidate"
            && resource.initiator == "link"
            && resource.url == "/img/hero@2x.jpg"
            && resource.resolved == "https://example.com/img/hero@2x.jpg"
    }));
    assert!(render.resources.iter().any(|resource| {
        resource.kind == "preload" && resource.resolved == "https://example.com/fonts/app.woff2"
    }));
}

#[test]
fn discovers_css_background_image_subresources() {
    let render = render_html(
        "https://example.com/app/page.html",
        br#"
            <html><head><style>
              .hero { background-image: linear-gradient(#fff, #eee), url('/img/hero-bg.png'); }
              .inline { display: block; }
              .hidden { display: none; background-image: url('/img/hidden.png'); }
            </style></head><body>
              <section class="hero">Hero</section>
              <section class="inline" style="background-image: url('inline-bg.png')">Inline</section>
              <section class="hidden">Hidden</section>
            </body></html>
            "#,
        BrowserRenderOptions::default(),
    );

    let background_resources = render
        .resources
        .iter()
        .filter(|resource| resource.kind == "background_image")
        .collect::<Vec<_>>();
    assert_eq!(background_resources.len(), 2);
    assert!(background_resources.iter().any(|resource| {
        resource.initiator == "css"
            && resource.url == "/img/hero-bg.png"
            && resource.resolved == "https://example.com/img/hero-bg.png"
    }));
    assert!(background_resources.iter().any(|resource| {
        resource.initiator == "css"
            && resource.url == "inline-bg.png"
            && resource.resolved == "https://example.com/app/inline-bg.png"
    }));
    assert!(
        !background_resources
            .iter()
            .any(|resource| resource.resolved.ends_with("/img/hidden.png"))
    );
}

#[test]
fn discovers_data_url_srcset_resource_without_truncating_payload() {
    let data_url = tiny_test_jpeg_data_url();
    let html = format!(
        r#"<html><body><img src="fallback.jpg" srcset="{data_url} 2x, photo.jpg 1x" alt="Hero"></body></html>"#
    );
    let render = render_html(
        "https://example.com/app/page.html",
        html.as_bytes(),
        BrowserRenderOptions::default(),
    );

    assert!(render.resources.iter().any(|resource| {
        resource.kind == "image_candidate"
            && resource.url == data_url
            && resource.resolved == data_url
    }));
    assert!(render.resources.iter().any(|resource| {
        resource.kind == "image_candidate"
            && resource.resolved == "https://example.com/app/photo.jpg"
    }));
    assert!(!render.resources.iter().any(|resource| {
        resource.kind == "image_candidate" && resource.url == "data:image/jpeg;base64"
    }));
}

#[test]
fn quoted_raw_svg_data_urls_survive_tag_scan_and_resource_discovery() {
    let src_data_url = "data:image/svg+xml,<svg><rect/></svg>";
    let srcset_data_url = "data:image/svg+xml,<svg><circle/></svg>";
    let html = format!(
        r#"<html><body><img src="{src_data_url}" alt="Raw SVG src"><img srcset="{srcset_data_url} 1x, fallback.webp 2x" alt="Raw SVG srcset"></body></html>"#
    );
    let render = render_html(
        "https://example.com/app/page.html",
        html.as_bytes(),
        BrowserRenderOptions {
            width: 40,
            ..BrowserRenderOptions::default()
        },
    );

    assert!(render.resources.iter().any(|resource| {
        resource.kind == "image"
            && resource.url == src_data_url
            && resource.resolved == src_data_url
    }));
    assert!(render.resources.iter().any(|resource| {
        resource.kind == "image_candidate"
            && resource.url == srcset_data_url
            && resource.resolved == srcset_data_url
    }));
    assert!(!render.resources.iter().any(|resource| {
        resource.url.starts_with("data:image/svg+xml,<svg") && !resource.url.ends_with("</svg>")
    }));

    assert!(render.dom_node_count > 0);
}

#[test]
fn skips_invalid_jpeg_srcset_candidate_resources() {
    let render = render_html(
        "https://example.com/app/page.html",
        br#"<html><body><img src="fallback.jpg" srcset="bad.jpg 1x bogus, mixed.jpg 160w 1x, valid.jpg" alt="Hero"></body></html>"#,
        BrowserRenderOptions::default(),
    );

    assert!(render.resources.iter().any(|resource| {
        resource.kind == "image_candidate"
            && resource.resolved == "https://example.com/app/valid.jpg"
    }));
    assert!(!render.resources.iter().any(|resource| {
        resource.kind == "image_candidate" && resource.resolved == "https://example.com/app/bad.jpg"
    }));
    assert!(!render.resources.iter().any(|resource| {
        resource.kind == "image_candidate"
            && resource.resolved == "https://example.com/app/mixed.jpg"
    }));
}

#[tokio::test]
async fn image_source_coverage_reports_supported_responsive_candidates() {
    let dir = tempfile::tempdir().unwrap();
    let page = dir.path().join("page.html");
    let hero = dir.path().join("hero.webp");
    let wide = dir.path().join("wide.webp");
    let preload = dir.path().join("preload.webp");
    let background = dir.path().join("background.webp");
    fs::write(&hero, tiny_test_webp_bytes()).unwrap();
    fs::write(&wide, tiny_test_webp_bytes()).unwrap();
    fs::write(&preload, tiny_test_webp_bytes()).unwrap();
    fs::write(&background, tiny_test_webp_bytes()).unwrap();
    fs::write(
        &page,
        r#"<html><head>
            <link rel="preload" as="image" imagesrcset="preload.avif 320w, preload.webp 640w" imagesizes="80px">
        </head><body>
            <img srcset="hero.avif 320w, hero.webp 640w" alt="Hero">
            <picture>
                <source type="image/webp" data-currentSrcset="wide.avif 320w, wide.webp 640w">
                <img alt="Wide">
            </picture>
            <section data-bgset="background.avif 320w, background.webp 640w">Background</section>
        </body></html>"#,
    )
    .unwrap();

    let mut session = BrowserSession::new(BrowserRenderOptions::default());
    session.navigate(&page.display().to_string()).await.unwrap();
    let report = session.fetch_current_resources(1024).await.unwrap();

    assert_eq!(report.total, 4);
    assert_eq!(report.fetched, 4);
    assert_eq!(report.failed, 0);
    assert_eq!(report.cached_resource_count, 4);
    assert!(
        !report
            .resources
            .iter()
            .any(|fetch| fetch.resource.url.ends_with(".avif"))
    );

    for expected in [&hero, &wide, &preload, &background] {
        let fetch = report
            .resources
            .iter()
            .find(|fetch| fetch.resource.resolved == expected.display().to_string())
            .unwrap();
        assert_eq!(fetch.status, "fetched");
        assert_eq!(fetch.content_type.as_deref(), Some("image/webp"));
        assert_eq!(fetch.image_decode_status.as_deref(), Some("decoded"));
        assert!(fetch.decoded_hash.is_some());
    }
}

#[tokio::test]
async fn image_real_pages_decode_normalized_lazy_alias_sources() {
    let dir = tempfile::tempdir().unwrap();
    let page = dir.path().join("page.html");
    let hero = dir.path().join("hero.webp");
    let picture = dir.path().join("picture.webp");
    let background = dir.path().join("background.webp");
    let discovery = dir.path().join("discovery.webp");
    fs::write(&hero, tiny_test_webp_bytes()).unwrap();
    fs::write(&picture, tiny_test_webp_bytes()).unwrap();
    fs::write(&background, tiny_test_webp_bytes()).unwrap();
    fs::write(&discovery, tiny_test_webp_bytes()).unwrap();
    fs::write(
        &page,
        r#"<html><body>
            <img src="/assets/placeholder.gif" data-lazySrcset="hero.avif 320w, hero.webp 640w" data-lazySizes="80px" alt="Lazy hero" width="80" height="24">
            <picture>
                <source type="image/webp" data-originalSrcset="picture.avif 320w, picture.webp 640w">
                <img src="/assets/blank.gif" alt="Picture hero" width="80" height="24">
            </picture>
            <section data-lazyBackgroundSrcset="background.avif 320w, background.webp 640w">Background</section>
            <img data-imageSrc="discovery.webp" alt="Discovery-only image">
        </body></html>"#,
    )
    .unwrap();

    let mut render_session = BrowserSession::new(BrowserRenderOptions {
        width: 40,
        ..BrowserRenderOptions::default()
    });
    render_session
        .navigate(&page.display().to_string())
        .await
        .unwrap();

    let report = render_session
        .render_current_with_images(1024)
        .await
        .unwrap();
    assert_eq!(report.image_count, 4);
    assert_eq!(report.decoded, 4);
    assert_eq!(report.failed, 0);
    assert!(
        !report
            .fetches
            .iter()
            .any(|fetch| fetch.resource.url.ends_with(".avif"))
    );

    let expected = [
        (hero.display().to_string(), "image", "img", "hero.webp"),
        (
            picture.display().to_string(),
            "image",
            "img",
            "picture.webp",
        ),
        (
            background.display().to_string(),
            "background_image",
            "section",
            "background.webp",
        ),
        (
            discovery.display().to_string(),
            "image",
            "img",
            "discovery.webp",
        ),
    ];
    let mut decoded_hashes = Vec::new();
    for (resolved, kind, initiator, url) in expected {
        let fetch = report
            .fetches
            .iter()
            .find(|fetch| fetch.resource.resolved == resolved)
            .unwrap();
        assert_eq!(fetch.resource.kind, kind);
        assert_eq!(fetch.resource.initiator, initiator);
        assert_eq!(fetch.resource.url, url);
        assert_eq!(fetch.status, "fetched");
        assert_eq!(fetch.content_type.as_deref(), Some("image/webp"));
        assert_eq!(fetch.image_decode_status.as_deref(), Some("decoded"));
        decoded_hashes.push((resolved, kind, fetch.decoded_hash.clone().unwrap()));
    }

    let render = render_session.current().unwrap();
    for (resolved, kind, decoded_hash) in decoded_hashes {
        let attached = render.display_list.iter().any(|command| match kind {
            "background_image" => matches!(
                command,
                DisplayCommand::BackgroundImage {
                    url: Some(url),
                    decoded_hash: Some(hash),
                    ..
                } if url == &resolved && hash == &decoded_hash
            ),
            _ => matches!(
                command,
                DisplayCommand::Image {
                    url: Some(url),
                    decoded_hash: Some(hash),
                    ..
                } if url == &resolved && hash == &decoded_hash
            ),
        });
        assert!(attached, "expected decoded {kind} command for {resolved}");
    }

    let mut resource_session = BrowserSession::new(BrowserRenderOptions::default());
    resource_session
        .navigate(&page.display().to_string())
        .await
        .unwrap();
    let resource_report = resource_session
        .fetch_current_resources(1024)
        .await
        .unwrap();
    assert_eq!(resource_report.failed, 0);
    assert!(
        !resource_report
            .resources
            .iter()
            .any(|fetch| fetch.resource.url.ends_with(".avif"))
    );
    for expected in [&hero, &picture, &background, &discovery] {
        let fetch = resource_report
            .resources
            .iter()
            .find(|fetch| fetch.resource.resolved == expected.display().to_string())
            .unwrap();
        assert_eq!(fetch.status, "fetched");
        assert_eq!(fetch.content_type.as_deref(), Some("image/webp"));
        assert_eq!(fetch.image_decode_status.as_deref(), Some("decoded"));
        assert!(fetch.decoded_hash.is_some());
    }
}

#[tokio::test]
async fn image_inline_real_pages_decode_lazyload_alias_sources() {
    let dir = tempfile::tempdir().unwrap();
    let page = dir.path().join("page.html");
    let hero = dir.path().join("hero.webp");
    let picture = dir.path().join("picture.webp");
    let gallery = dir.path().join("gallery.webp");
    fs::write(&hero, tiny_test_webp_bytes()).unwrap();
    fs::write(&picture, tiny_test_webp_bytes()).unwrap();
    fs::write(&gallery, tiny_test_webp_bytes()).unwrap();
    fs::write(
        &page,
        r#"<html><body>
            <img src="/assets/loading.gif" data-lazyload-srcset="hero.avif 320w, hero.webp 640w" data-lazyload-sizes="80px" alt="Lazyload hero" width="80" height="24">
            <picture>
                <source type="image/webp" data-flickity-lazyload-srcset="picture.avif 320w, picture.webp 640w">
                <img src="/assets/blank.gif" alt="Flickity picture" width="80" height="24">
            </picture>
            <img data-flickity-lazyload="gallery.webp" alt="Flickity image" width="80" height="24">
        </body></html>"#,
    )
    .unwrap();

    let mut render_session = BrowserSession::new(BrowserRenderOptions {
        width: 40,
        ..BrowserRenderOptions::default()
    });
    render_session
        .navigate(&page.display().to_string())
        .await
        .unwrap();
    let report = render_session
        .render_current_with_images(1024)
        .await
        .unwrap();
    assert_eq!(report.image_count, 3);
    assert_eq!(report.decoded, 3);
    assert_eq!(report.failed, 0);
    assert!(
        !report
            .fetches
            .iter()
            .any(|fetch| fetch.resource.url.ends_with(".avif"))
    );

    let expected = [
        (hero.display().to_string(), "hero.webp"),
        (picture.display().to_string(), "picture.webp"),
        (gallery.display().to_string(), "gallery.webp"),
    ];
    let mut decoded_hashes = Vec::new();
    for (resolved, url) in expected {
        let fetch = report
            .fetches
            .iter()
            .find(|fetch| fetch.resource.resolved == resolved)
            .unwrap();
        assert_eq!(fetch.resource.kind, "image");
        assert_eq!(fetch.resource.initiator, "img");
        assert_eq!(fetch.resource.url, url);
        assert_eq!(fetch.status, "fetched");
        assert_eq!(fetch.content_type.as_deref(), Some("image/webp"));
        assert_eq!(fetch.image_decode_status.as_deref(), Some("decoded"));
        decoded_hashes.push((resolved, fetch.decoded_hash.clone().unwrap()));
    }

    let render = render_session.current().unwrap();
    for (resolved, decoded_hash) in decoded_hashes {
        assert!(render.display_list.iter().any(|command| {
            matches!(
                command,
                DisplayCommand::Image {
                    url: Some(url),
                    decoded_hash: Some(hash),
                    ..
                } if url == &resolved && hash == &decoded_hash
            )
        }));
    }

    let mut resource_session = BrowserSession::new(BrowserRenderOptions::default());
    resource_session
        .navigate(&page.display().to_string())
        .await
        .unwrap();
    let resource_report = resource_session
        .fetch_current_resources(1024)
        .await
        .unwrap();
    assert_eq!(resource_report.failed, 0);
    assert!(
        !resource_report
            .resources
            .iter()
            .any(|fetch| fetch.resource.url.ends_with(".avif"))
    );
    for expected in [&hero, &picture, &gallery] {
        let fetch = resource_report
            .resources
            .iter()
            .find(|fetch| fetch.resource.resolved == expected.display().to_string())
            .unwrap();
        assert_eq!(fetch.status, "fetched");
        assert_eq!(fetch.content_type.as_deref(), Some("image/webp"));
        assert_eq!(fetch.image_decode_status.as_deref(), Some("decoded"));
        assert!(fetch.decoded_hash.is_some());
    }
}

#[tokio::test]
async fn fetches_current_resources_and_uses_session_cache() {
    let dir = tempfile::tempdir().unwrap();
    let page = dir.path().join("page.html");
    let stylesheet = dir.path().join("style.css");
    let script = dir.path().join("app.js");
    let stylesheet_text = "body { display:block }";
    let script_text = "console.log('ok')";
    fs::write(&stylesheet, stylesheet_text).unwrap();
    fs::write(&script, script_text).unwrap();
    fs::write(
        &page,
        r#"
            <html><head>
              <link rel="stylesheet" href="style.css">
              <script src="app.js"></script>
              <script src="app.js"></script>
            </head><body><img src="missing.png"></body></html>
            "#,
    )
    .unwrap();

    let mut session = BrowserSession::new(BrowserRenderOptions::default());
    session.navigate(&page.display().to_string()).await.unwrap();
    let report = session.fetch_current_resources(1024).await.unwrap();

    assert_eq!(report.total, 4);
    assert_eq!(report.fetched, 2);
    assert_eq!(report.cached, 1);
    assert_eq!(report.failed, 1);
    assert_eq!(report.skipped, 0);
    assert_eq!(report.cached_resource_count, 2);
    assert_eq!(
        report.cached_resource_bytes,
        stylesheet_text.len() + script_text.len()
    );
    assert!(report.resources.iter().any(|resource| {
        resource.status == "fetched"
            && resource.resource.kind == "stylesheet"
            && resource.content_type.as_deref() == Some("text/css")
    }));
    assert!(report.resources.iter().any(|resource| {
        resource.status == "cached" && resource.resource.resolved.ends_with("app.js")
    }));
    assert!(report.resources.iter().any(|resource| {
        resource.status == "failed" && resource.resource.resolved.ends_with("missing.png")
    }));
}

#[tokio::test]
async fn image_realpage_attachments_skip_supported_loading_placeholder_for_lazy_srcset() {
    let dir = tempfile::tempdir().unwrap();
    let page = dir.path().join("page.html");
    let hero = dir.path().join("hero.webp");
    fs::write(&hero, tiny_test_webp_bytes()).unwrap();
    fs::write(
        &page,
        r#"<html><body>
            <img src="loading.svg" data-lazyload-srcset="hero.avif 320w, hero.webp 640w" data-lazyload-sizes="80px" alt="Lazy hero" width="80" height="24">
        </body></html>"#,
    )
    .unwrap();

    let mut session = BrowserSession::new(BrowserRenderOptions {
        width: 40,
        ..BrowserRenderOptions::default()
    });
    session.navigate(&page.display().to_string()).await.unwrap();

    let report = session.render_current_with_images(1024).await.unwrap();
    assert_eq!(report.image_count, 1);
    assert_eq!(report.decoded, 1);
    assert_eq!(report.failed, 0);
    assert!(
        !report
            .fetches
            .iter()
            .any(|fetch| fetch.resource.url == "loading.svg")
    );
    assert!(
        !report
            .fetches
            .iter()
            .any(|fetch| fetch.resource.url.ends_with(".avif"))
    );

    let hero_url = hero.display().to_string();
    let fetch = report
        .fetches
        .iter()
        .find(|fetch| fetch.resource.resolved == hero_url)
        .unwrap();
    assert_eq!(fetch.resource.kind, "image");
    assert_eq!(fetch.resource.initiator, "img");
    assert_eq!(fetch.resource.url, "hero.webp");
    assert_eq!(fetch.status, "fetched");
    assert_eq!(fetch.content_type.as_deref(), Some("image/webp"));
    assert_eq!(fetch.image_decode_status.as_deref(), Some("decoded"));
    let decoded_hash = fetch.decoded_hash.clone().unwrap();

    let render = session.current().unwrap();
    assert!(render.display_list.iter().any(|command| {
        matches!(
            command,
            DisplayCommand::Image {
                url: Some(url),
                decoded_hash: Some(hash),
                ..
            } if url == &hero_url && hash == &decoded_hash
        )
    }));
}

#[tokio::test]
async fn image_css_background_coverage_fetches_lazy_alias_resources() {
    let dir = tempfile::tempdir().unwrap();
    let page = dir.path().join("page.html");
    let hero = dir.path().join("hero.webp");
    let wide = dir.path().join("wide.webp");
    fs::write(&hero, tiny_test_webp_bytes()).unwrap();
    fs::write(&wide, tiny_test_webp_bytes()).unwrap();
    fs::write(
        &page,
        r#"<html><body>
            <img data-currentSrc="hero.webp" alt="Current image">
            <picture>
                <source type="image/webp" data-currentSrcset="wide.webp 640w">
                <img alt="Picture current image">
            </picture>
        </body></html>"#,
    )
    .unwrap();

    let mut session = BrowserSession::new(BrowserRenderOptions::default());
    session.navigate(&page.display().to_string()).await.unwrap();

    let report = session.fetch_current_resources(1024).await.unwrap();
    assert_eq!(report.total, 2);
    assert_eq!(report.fetched, 2);
    assert_eq!(report.failed, 0);
    assert_eq!(report.cached_resource_count, 2);

    let hero_fetch = report
        .resources
        .iter()
        .find(|fetch| fetch.resource.resolved == hero.display().to_string())
        .unwrap();
    assert_eq!(hero_fetch.resource.kind, "image");
    assert_eq!(hero_fetch.resource.initiator, "img");
    assert_eq!(hero_fetch.resource.url, "hero.webp");
    assert_eq!(hero_fetch.status, "fetched");
    assert_eq!(hero_fetch.content_type.as_deref(), Some("image/webp"));
    assert_eq!(hero_fetch.image_decode_status.as_deref(), Some("decoded"));
    assert!(hero_fetch.decoded_hash.is_some());

    let wide_fetch = report
        .resources
        .iter()
        .find(|fetch| fetch.resource.resolved == wide.display().to_string())
        .unwrap();
    assert_eq!(wide_fetch.resource.kind, "image_candidate");
    assert_eq!(wide_fetch.resource.initiator, "source");
    assert_eq!(wide_fetch.resource.url, "wide.webp");
    assert_eq!(wide_fetch.status, "fetched");
    assert_eq!(wide_fetch.content_type.as_deref(), Some("image/webp"));
    assert_eq!(wide_fetch.image_decode_status.as_deref(), Some("decoded"));
    assert!(wide_fetch.decoded_hash.is_some());
}

#[tokio::test]
async fn image_style_background_fetches_lazy_background_alias_resources() {
    let dir = tempfile::tempdir().unwrap();
    let page = dir.path().join("page.html");
    let hero = dir.path().join("hero.webp");
    let wide = dir.path().join("wide.webp");
    let set = dir.path().join("set.webp");
    fs::write(&hero, tiny_test_webp_bytes()).unwrap();
    fs::write(&wide, tiny_test_webp_bytes()).unwrap();
    fs::write(&set, tiny_test_webp_bytes()).unwrap();
    fs::write(
        &page,
        r#"<html><body>
            <section data-background-image="url('hero.webp')">Hero</section>
            <section data-bgset="wide.avif 320w, wide.webp 640w">Wide</section>
            <section data-lazy-background-image="-webkit-image-set(url('set.avif') type('image/avif') 1x, url('set.webp') type('image/webp') 2x)">Set</section>
        </body></html>"#,
    )
    .unwrap();

    let mut session = BrowserSession::new(BrowserRenderOptions {
        width: 40,
        ..BrowserRenderOptions::default()
    });
    session.navigate(&page.display().to_string()).await.unwrap();

    let report = session.render_current_with_images(1024).await.unwrap();
    assert_eq!(report.image_count, 3);
    assert_eq!(report.decoded, 3);
    assert_eq!(report.failed, 0);
    assert_eq!(report.fetches.len(), 3);

    let hero_fetch = report
        .fetches
        .iter()
        .find(|fetch| fetch.resource.resolved == hero.display().to_string())
        .unwrap();
    assert_eq!(hero_fetch.resource.kind, "background_image");
    assert_eq!(hero_fetch.resource.initiator, "section");
    assert_eq!(hero_fetch.resource.url, "hero.webp");
    assert_eq!(hero_fetch.status, "fetched");
    assert_eq!(hero_fetch.content_type.as_deref(), Some("image/webp"));
    assert_eq!(hero_fetch.image_decode_status.as_deref(), Some("decoded"));
    assert!(hero_fetch.decoded_hash.is_some());

    let wide_fetch = report
        .fetches
        .iter()
        .find(|fetch| fetch.resource.resolved == wide.display().to_string())
        .unwrap();
    assert_eq!(wide_fetch.resource.kind, "background_image");
    assert_eq!(wide_fetch.resource.initiator, "section");
    assert_eq!(wide_fetch.resource.url, "wide.webp");
    assert_eq!(wide_fetch.status, "fetched");
    assert_eq!(wide_fetch.content_type.as_deref(), Some("image/webp"));
    assert_eq!(wide_fetch.image_decode_status.as_deref(), Some("decoded"));
    assert!(wide_fetch.decoded_hash.is_some());

    let set_fetch = report
        .fetches
        .iter()
        .find(|fetch| fetch.resource.resolved == set.display().to_string())
        .unwrap();
    assert_eq!(set_fetch.resource.kind, "background_image");
    assert_eq!(set_fetch.resource.initiator, "section");
    assert_eq!(set_fetch.resource.url, "set.webp");
    assert_eq!(set_fetch.status, "fetched");
    assert_eq!(set_fetch.content_type.as_deref(), Some("image/webp"));
    assert_eq!(set_fetch.image_decode_status.as_deref(), Some("decoded"));
    let set_hash = set_fetch.decoded_hash.clone().unwrap();
    let render = session.current().unwrap();
    assert!(render.display_list.iter().any(|command| {
        matches!(
            command,
            DisplayCommand::BackgroundImage {
                url: Some(url),
                decoded_hash: Some(hash),
                ..
            } if url == &set.display().to_string() && hash == &set_hash
        )
    }));
    assert!(
        !report
            .fetches
            .iter()
            .any(|fetch| fetch.resource.url == "set.avif")
    );
}

#[tokio::test]
async fn image_background_fidelity_skips_unsupported_typed_imageset_candidate_for_rendering() {
    let dir = tempfile::tempdir().unwrap();
    let page = dir.path().join("page.html");
    let hero = dir.path().join("hero.webp");
    fs::write(&hero, tiny_test_webp_bytes()).unwrap();
    fs::write(
        &page,
        r#"<html><body>
            <section data-lazy-background-image="image-set(url('dead-resource') type('image/avif') 1x, url('hero.webp') type('image/webp') 2x)">Hero</section>
        </body></html>"#,
    )
    .unwrap();

    let mut session = BrowserSession::new(BrowserRenderOptions {
        width: 40,
        ..BrowserRenderOptions::default()
    });
    session.navigate(&page.display().to_string()).await.unwrap();

    let report = session.render_current_with_images(1024).await.unwrap();
    assert_eq!(report.image_count, 1);
    assert_eq!(report.decoded, 1);
    assert_eq!(report.failed, 0);
    assert!(
        !report
            .fetches
            .iter()
            .any(|fetch| fetch.resource.url == "dead-resource")
    );
    let hero_fetch = report
        .fetches
        .iter()
        .find(|fetch| fetch.resource.resolved == hero.display().to_string())
        .unwrap();
    assert_eq!(hero_fetch.resource.kind, "background_image");
    assert_eq!(hero_fetch.resource.initiator, "section");
    assert_eq!(hero_fetch.resource.url, "hero.webp");
    assert_eq!(hero_fetch.status, "fetched");
    assert_eq!(hero_fetch.content_type.as_deref(), Some("image/webp"));
    assert_eq!(hero_fetch.image_decode_status.as_deref(), Some("decoded"));
    let hero_hash = hero_fetch.decoded_hash.clone().unwrap();

    let render = session.current().unwrap();
    assert!(render.display_list.iter().any(|command| {
        matches!(
            command,
            DisplayCommand::BackgroundImage {
                url: Some(url),
                decoded_hash: Some(hash),
                ..
            } if url == &hero.display().to_string() && hash == &hero_hash
        )
    }));
}

#[tokio::test]
async fn external_stylesheets_can_rerender_current_page() {
    let dir = tempfile::tempdir().unwrap();
    let page = dir.path().join("page.html");
    let stylesheet = dir.path().join("style.css");
    let stylesheet_text = ".hide { display:none }";
    fs::write(&stylesheet, stylesheet_text).unwrap();
    fs::write(
        &page,
        r#"
            <html><head><link rel="stylesheet" href="style.css"></head>
            <body><p>Visible</p><p class="hide">Hidden</p></body></html>
            "#,
    )
    .unwrap();

    let mut session = BrowserSession::new(BrowserRenderOptions::default());
    session.navigate(&page.display().to_string()).await.unwrap();
    assert!(session.current().unwrap().text.contains("Hidden"));

    let report = session.render_current_with_stylesheets(1024).await.unwrap();
    assert_eq!(report.stylesheet_count, 1);
    assert_eq!(report.applied, 1);
    assert_eq!(report.failed, 0);
    assert_eq!(report.cached_resource_count, 1);
    assert_eq!(report.cached_resource_bytes, stylesheet_text.len());
    let render = session.current().unwrap();
    assert!(render.text.contains("Visible"));
    assert!(!render.text.contains("Hidden"));
    assert_eq!(render.css_rule_count, 1);
}

#[tokio::test]
async fn external_scripts_can_rerender_current_page() {
    let dir = tempfile::tempdir().unwrap();
    let page = dir.path().join("page.html");
    let script = dir.path().join("app.js");
    let script_text = r#"
            document.title = "External Script";
            const heading = document.createElement("h1");
            heading.textContent = "Loaded from script";
            document.body.appendChild(heading);
            "#;
    fs::write(&script, script_text).unwrap();
    fs::write(
        &page,
        r#"
            <html><head><title>Before</title><script src="app.js"></script></head>
            <body><p>Static</p></body></html>
            "#,
    )
    .unwrap();

    let mut session = BrowserSession::new(BrowserRenderOptions::default());
    session.navigate(&page.display().to_string()).await.unwrap();
    assert_eq!(session.current().unwrap().title, "Before");
    assert_eq!(session.current().unwrap().text, "Static");

    let report = session.render_current_with_scripts(1024).await.unwrap();
    assert_eq!(report.script_count, 1);
    assert_eq!(report.applied, 1);
    assert_eq!(report.failed, 0);
    assert_eq!(report.cached_resource_count, 1);
    assert_eq!(report.cached_resource_bytes, script_text.len());
    let render = session.current().unwrap();
    assert_eq!(render.title, "External Script");
    assert_eq!(render.text, "Static\nLoaded from script");
}

#[test]
fn extracts_forms_and_builds_get_submission_urls() {
    let render = render_html(
            "https://example.com/docs/page.html",
            br#"
            <html><body>
            <form action="/search" method="get">
                <input name="q" value="rust search">
                <input type="checkbox" name="fast" checked>
                <input type="checkbox" name="slow">
                <textarea name="note">hello there</textarea>
                <select name="kind"><option value="web">Web</option><option selected>Docs</option></select>
                <input name="disabled" value="nope" disabled>
                <button name="go" value="1">Go</button>
            </form>
            </body></html>
            "#,
            BrowserRenderOptions::default(),
        );

    assert_eq!(render.forms.len(), 1);
    let form = &render.forms[0];
    assert_eq!(form.method, "GET");
    assert_eq!(form.action, "/search");
    assert_eq!(form.resolved_action, "https://example.com/search");
    assert!(form.controls.iter().any(|control| control.name == "q"));
    assert!(
        form.controls
            .iter()
            .any(|control| control.name == "fast" && control.value == "on")
    );

    let url = build_get_form_url(form, &[("q".to_owned(), "browser forms".to_owned())]).unwrap();
    assert_eq!(
        url,
        "https://example.com/search?q=browser+forms&fast=on&note=hello+there&kind=Docs"
    );
    assert!(!url.contains("slow"));
    assert!(!url.contains("disabled"));
    assert!(!url.contains("go="));
}

#[test]
fn chromium_parity_helpers_extract_and_normalize_text() {
    let dump = r#"<html><head><script id="brutal-chromium-result" type="application/json">{"title":"Fixture","text":"one   two\nthree"}</script></head></html>"#;
    let json = extract_chromium_result_json(dump).unwrap();
    let parsed: ChromiumStaticRender = serde_json::from_str(json).unwrap();
    assert_eq!(parsed.title, "Fixture");
    assert_eq!(normalize_browser_parity_text(&parsed.text), "one two three");

    let reason = parity_failure_reason(&BrowserChromiumParityComparison {
        name: "fixture".to_owned(),
        path: "fixture.html".to_owned(),
        title_match: true,
        text_match: false,
        brutal_title: "Fixture".to_owned(),
        chromium_title: "Fixture".to_owned(),
        brutal_text: "one two".to_owned(),
        chromium_text: "one two three".to_owned(),
    });
    assert_eq!(reason, "text mismatch");
}

#[test]
fn rejects_static_post_form_url_builds() {
    let render = render_html(
            "https://example.com/page.html",
            br#"<form action="/login" method="post"><input name="user" value="a"><input type="checkbox" name="remember" checked></form>"#,
            BrowserRenderOptions::default(),
        );

    let error = build_get_form_url(&render.forms[0], &[]).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("cannot build GET form URL for POST form")
    );
    let body =
        build_post_form_body(&render.forms[0], &[("user".to_owned(), "b".to_owned())]).unwrap();
    assert_eq!(body, "user=b&remember=on");
}

#[tokio::test]
async fn browser_session_sends_cookies_between_http_navigations() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        for _ in 0..2 {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut buf = [0u8; 4096];
            let n = stream.read(&mut buf).await.unwrap();
            let request = String::from_utf8_lossy(&buf[..n]);
            let first_line = request.lines().next().unwrap_or_default();
            let cookie_line = request
                .lines()
                .find(|line| line.to_ascii_lowercase().starts_with("cookie:"))
                .unwrap_or("Cookie:");
            let (body, set_cookie) = if first_line.contains(" /set ") {
                (
                    "<html><head><title>Set</title></head><body>set</body></html>".to_owned(),
                    "Set-Cookie: sid=abc; Path=/; HttpOnly\r\n",
                )
            } else {
                (
                    format!(
                        "<html><head><title>Check</title></head><body>{cookie_line}</body></html>"
                    ),
                    "",
                )
            };
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\n{}Content-Length: {}\r\nConnection: close\r\n\r\n{}",
                set_cookie,
                body.len(),
                body
            );
            stream.write_all(response.as_bytes()).await.unwrap();
        }
    });

    let mut session = BrowserSession::new(BrowserRenderOptions::default());
    session
        .navigate(&format!("http://{addr}/set"))
        .await
        .unwrap();
    session
        .navigate(&format!("http://{addr}/check"))
        .await
        .unwrap();
    server.await.unwrap();

    assert!(session.current().unwrap().text.contains("sid=abc"));
    assert_eq!(session.cookies_snapshot()[0].name, "sid");
}

#[tokio::test]
async fn browser_session_redirect_updates_current_source_and_history_target() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut buf = [0u8; 4096];
        let n = stream.read(&mut buf).await.unwrap();
        let request = String::from_utf8_lossy(&buf[..n]);
        assert!(request.starts_with("GET /start "));
        stream
                .write_all(
                    b"HTTP/1.1 302 Found\r\nLocation: /final\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                )
                .await
                .unwrap();

        let (mut stream, _) = listener.accept().await.unwrap();
        let n = stream.read(&mut buf).await.unwrap();
        let request = String::from_utf8_lossy(&buf[..n]);
        assert!(request.starts_with("GET /final "));
        let body = "<html><head><title>Final</title></head><body>redirected</body></html>";
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        stream.write_all(response.as_bytes()).await.unwrap();
    });

    let mut session = BrowserSession::new(BrowserRenderOptions::default());
    session
        .navigate(&format!("http://{addr}/start"))
        .await
        .unwrap();
    server.await.unwrap();

    let final_url = format!("http://{addr}/final");
    assert_eq!(session.current().unwrap().source, final_url);
    assert_eq!(session.snapshot().entries[0].target, final_url);
}

#[tokio::test]
async fn browser_session_submits_post_form_with_state_and_cookies() {
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
                "<html><head><title>Form</title></head><body><form action=\"/submit\" method=\"post\"><input name=\"q\" value=\"old\"><input type=\"checkbox\" name=\"remember\" checked></form></body></html>"
            } else {
                assert!(first_line.starts_with("POST /submit "));
                assert!(
                    request_head
                        .to_ascii_lowercase()
                        .contains("content-type: application/x-www-form-urlencoded")
                );
                assert!(
                    request_head
                        .to_ascii_lowercase()
                        .contains("cookie: sid=abc")
                );
                assert_eq!(request_body, "q=rust+browser&remember=on");
                "<html><head><title>Posted</title></head><body>accepted</body></html>"
            };
            let set_cookie = if request_index == 0 {
                "Set-Cookie: sid=abc; Path=/\r\n"
            } else {
                "Set-Cookie: posted=1; Path=/\r\n"
            };
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\n{}Content-Length: {}\r\nConnection: close\r\n\r\n{}",
                set_cookie,
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
    session.set_form_field(0, "q", "rust browser").unwrap();
    session.submit_form(0, &[]).await.unwrap();
    server.await.unwrap();

    let current = session.current().unwrap();
    assert_eq!(current.title, "Posted");
    assert_eq!(current.source, format!("http://{addr}/submit"));
    assert!(
        session
            .cookies_snapshot()
            .iter()
            .any(|cookie| cookie.name == "posted" && cookie.value == "1")
    );
}

#[tokio::test]
async fn browser_session_post_submit_override_wins_over_filled_form_field() {
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
            let request_body = String::from_utf8_lossy(&request_bytes[header_end + 4..]);
            let body = if request_index == 0 {
                "<html><head><title>Form</title></head><body><form action=\"/submit\" method=\"post\"><input name=\"q\" value=\"old\"></form></body></html>"
            } else {
                assert_eq!(request_body, "q=override");
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
    session.set_form_field(0, "q", "stored").unwrap();
    session
        .submit_form(0, &[("q".to_owned(), "override".to_owned())])
        .await
        .unwrap();
    server.await.unwrap();

    assert_eq!(session.current().unwrap().title, "Posted");
}

#[tokio::test]
async fn browser_session_submits_get_form_and_loads_local_query_target() {
    let dir = tempfile::tempdir().unwrap();
    let form_page = dir.path().join("form.html");
    let results_page = dir.path().join("results.html");
    fs::write(
        &form_page,
        r#"
            <html><head><title>Form</title></head><body>
            <form action="results.html" method="get">
              <input name="q" value="old">
              <input type="checkbox" name="fast" checked>
            </form>
            </body></html>
            "#,
    )
    .unwrap();
    fs::write(
        &results_page,
        "<html><head><title>Results</title></head><body>done</body></html>",
    )
    .unwrap();

    let mut session = BrowserSession::new(BrowserRenderOptions::default());
    session
        .navigate(&form_page.display().to_string())
        .await
        .unwrap();
    session
        .submit_get_form(0, &[("q".to_owned(), "rust browser".to_owned())])
        .await
        .unwrap();

    let current = session.current().unwrap();
    assert_eq!(current.title, "Results");
    assert!(
        current
            .source
            .ends_with("results.html?q=rust+browser&fast=on")
    );
    let snapshot = session.snapshot();
    assert_eq!(snapshot.current_index, Some(1));
    assert_eq!(snapshot.entries.len(), 2);
}

#[tokio::test]
async fn browser_session_rejects_local_file_post_form_submit() {
    let dir = tempfile::tempdir().unwrap();
    let form_page = dir.path().join("form.html");
    fs::write(
        &form_page,
        r#"
            <html><head><title>Form</title></head><body>
            <form action="results.html" method="post">
              <input name="q" value="old">
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
    let error = session.submit_form(0, &[]).await.unwrap_err();
    assert!(
        error
            .to_string()
            .contains("POST form submission currently requires an HTTP(S) action target")
    );
}

#[tokio::test]
async fn browser_session_remembers_filled_form_field_for_submit() {
    let dir = tempfile::tempdir().unwrap();
    let form_page = dir.path().join("form.html");
    let results_page = dir.path().join("results.html");
    fs::write(
        &form_page,
        r#"
            <html><head><title>Form</title></head><body>
            <form action="results.html" method="get">
              <input name="q" value="old">
              <textarea name="notes">before</textarea>
            </form>
            </body></html>
            "#,
    )
    .unwrap();
    fs::write(
        &results_page,
        "<html><head><title>Results</title></head><body>done</body></html>",
    )
    .unwrap();

    let mut session = BrowserSession::new(BrowserRenderOptions::default());
    session
        .navigate(&form_page.display().to_string())
        .await
        .unwrap();
    let render = session.set_form_field(0, "q", "rust browser").unwrap();
    assert_eq!(render.forms[0].controls[0].value, "rust browser");
    session.set_form_field(0, "notes", "typed memo").unwrap();
    session.submit_get_form(0, &[]).await.unwrap();

    let current = session.current().unwrap();
    assert_eq!(current.title, "Results");
    assert!(
        current
            .source
            .ends_with("results.html?q=rust+browser&notes=typed+memo")
    );
}

#[tokio::test]
async fn browser_session_focus_selector_and_type_text_updates_form_state() {
    let dir = tempfile::tempdir().unwrap();
    let form_page = dir.path().join("form.html");
    let results_page = dir.path().join("results.html");
    fs::write(
        &form_page,
        r#"
            <html><head><title>Form</title></head><body>
            <form action="results.html" method="get">
              <input id="q" name="q" value="rust">
            </form>
            </body></html>
            "#,
    )
    .unwrap();
    fs::write(
        &results_page,
        "<html><head><title>Results</title></head><body>done</body></html>",
    )
    .unwrap();

    let mut session = BrowserSession::new(BrowserRenderOptions::default());
    session
        .navigate(&form_page.display().to_string())
        .await
        .unwrap();
    let focused = session.focus_selector("#q").unwrap();
    assert_eq!(focused.name, "q");
    assert_eq!(focused.value, "rust");
    let render = session.type_text(" browser").unwrap();
    assert_eq!(render.forms[0].controls[0].value, "rust browser");
    assert_eq!(render.text, "[rust browser]");
    assert_eq!(session.focused_control().unwrap().value, "rust browser");

    session.submit_get_form(0, &[]).await.unwrap();
    assert!(
        session
            .current()
            .unwrap()
            .source
            .ends_with("results.html?q=rust+browser")
    );
}

#[tokio::test]
async fn browser_session_edits_focused_text_control() {
    let dir = tempfile::tempdir().unwrap();
    let form_page = dir.path().join("form.html");
    fs::write(
        &form_page,
        r#"
            <html><head><title>Form</title></head><body>
            <form>
              <input id="q" name="q" value="rust🔥">
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
    session.focus_selector("#q").unwrap();

    let render = session.delete_text_backward(1).unwrap();
    assert_eq!(render.forms[0].controls[0].value, "rust");
    assert_eq!(render.text, "[rust]");
    let render = session.type_text(" browser").unwrap();
    assert_eq!(render.forms[0].controls[0].value, "rust browser");
    assert_eq!(render.text, "[rust browser]");
    let render = session.delete_text_backward(8).unwrap();
    assert_eq!(render.forms[0].controls[0].value, "rust");
    assert_eq!(render.text, "[rust]");
    let render = session.clear_focused_text().unwrap();
    assert_eq!(render.forms[0].controls[0].value, "");
    assert_eq!(render.text, "[]");
    assert_eq!(session.focused_control().unwrap().value, "");
}

#[tokio::test]
async fn browser_session_cycles_focus_through_fillable_controls() {
    let dir = tempfile::tempdir().unwrap();
    let form_page = dir.path().join("form.html");
    fs::write(
        &form_page,
        r#"
            <html><head><title>Form</title></head><body>
              <form>
                <input id="q" name="q" value="first">
              <input type="checkbox" name="fast">
              <textarea id="notes" name="notes">memo</textarea>
              <button id="go" name="commit" value="yes">Go</button>
              <input id="disabled" name="disabled" value="nope" disabled>
            </form>
            <form>
              <input id="email" type="email" name="email" value="a@example.com">
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

    assert_eq!(session.focus_next_control().unwrap().name, "q");
    assert_eq!(session.focus_next_control().unwrap().name, "fast");
    assert_eq!(session.focus_next_control().unwrap().name, "notes");
    assert_eq!(session.focus_next_control().unwrap().name, "commit");
    assert_eq!(session.focus_next_control().unwrap().name, "email");
    assert_eq!(session.focus_next_control().unwrap().name, "q");
    assert_eq!(session.focus_previous_control().unwrap().name, "email");
    assert_eq!(session.focus_previous_control().unwrap().name, "commit");
    assert_eq!(session.focus_previous_control().unwrap().name, "notes");
}

#[tokio::test]
async fn browser_session_submits_focused_form_with_current_state() {
    let dir = tempfile::tempdir().unwrap();
    let form_page = dir.path().join("form.html");
    let results_page = dir.path().join("results.html");
    fs::write(
        &form_page,
        r#"
            <html><head><title>Form</title></head><body>
            <form action="results.html" method="get">
              <input id="q" name="q" value="rust">
              <button id="go" name="commit" value="yes">Go</button>
            </form>
            </body></html>
            "#,
    )
    .unwrap();
    fs::write(
        &results_page,
        "<html><head><title>Results</title></head><body>done</body></html>",
    )
    .unwrap();

    let mut session = BrowserSession::new(BrowserRenderOptions::default());
    session
        .navigate(&form_page.display().to_string())
        .await
        .unwrap();
    session.focus_selector("#q").unwrap();
    session.type_text(" browser").unwrap();
    session.focus_selector("#go").unwrap();

    let render = session.submit_focused_form().await.unwrap();

    assert_eq!(render.title, "Results");
    assert!(
        render
            .source
            .ends_with("results.html?q=rust+browser&commit=yes")
    );
}

#[tokio::test]
async fn browser_session_clicking_text_input_or_label_focuses_for_typing() {
    let dir = tempfile::tempdir().unwrap();
    let form_page = dir.path().join("form.html");
    fs::write(
        &form_page,
        r#"
            <html><head><title>Form</title></head><body>
            <form>
              <input id="q" name="q" value="">
              <label for="notes"><span>Notes</span></label>
              <textarea id="notes" name="notes"></textarea>
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
    session
        .click_selector_with_default_action("#q")
        .await
        .unwrap();
    session.type_text("typed").unwrap();

    assert_eq!(session.focused_control().unwrap().name, "q");
    assert_eq!(
        session.current().unwrap().forms[0].controls[0].value,
        "typed"
    );

    session
        .click_selector_with_default_action("label[for=notes] span")
        .await
        .unwrap();
    let render = session.type_text("memo").unwrap();

    assert_eq!(render.forms[0].controls[1].value, "memo");
    assert_eq!(session.focused_control().unwrap().name, "notes");
}

#[tokio::test]
async fn browser_session_submit_override_wins_over_filled_form_field() {
    let dir = tempfile::tempdir().unwrap();
    let form_page = dir.path().join("form.html");
    let results_page = dir.path().join("results.html");
    fs::write(
        &form_page,
        r#"
            <html><head><title>Form</title></head><body>
            <form action="results.html" method="get">
              <input name="q" value="old">
            </form>
            </body></html>
            "#,
    )
    .unwrap();
    fs::write(
        &results_page,
        "<html><head><title>Results</title></head><body>done</body></html>",
    )
    .unwrap();

    let mut session = BrowserSession::new(BrowserRenderOptions::default());
    session
        .navigate(&form_page.display().to_string())
        .await
        .unwrap();
    session.set_form_field(0, "q", "stored").unwrap();
    session
        .submit_get_form(0, &[("q".to_owned(), "override".to_owned())])
        .await
        .unwrap();

    assert!(
        session
            .current()
            .unwrap()
            .source
            .ends_with("results.html?q=override")
    );
}

#[tokio::test]
async fn browser_session_form_state_survives_history_and_rerender() {
    let dir = tempfile::tempdir().unwrap();
    let form_page = dir.path().join("form.html");
    let results_page = dir.path().join("results.html");
    fs::write(
            &form_page,
            r#"
            <html><head><title>Form</title></head><body>
            <button id="go" onclick="document.querySelector('#out').innerText = 'Clicked'">Go</button>
            <form action="results.html" method="get">
              <input name="q" value="old">
            </form>
            <p id="out">Waiting</p>
            </body></html>
            "#,
        )
        .unwrap();
    fs::write(
        &results_page,
        "<html><head><title>Results</title></head><body>done</body></html>",
    )
    .unwrap();

    let mut session = BrowserSession::new(BrowserRenderOptions::default());
    session
        .navigate(&form_page.display().to_string())
        .await
        .unwrap();
    session.set_form_field(0, "q", "stored").unwrap();
    let render = session.click_selector("#go").unwrap();
    assert_eq!(render.forms[0].controls[0].value, "stored");
    session.submit_get_form(0, &[]).await.unwrap();
    session.back().unwrap();
    assert_eq!(
        session.current().unwrap().forms[0].controls[0].value,
        "stored"
    );
    session.forward().unwrap();
    assert_eq!(session.current().unwrap().title, "Results");
}

#[tokio::test]
async fn browser_session_does_not_submit_stale_filled_field_after_rerender() {
    let dir = tempfile::tempdir().unwrap();
    let form_page = dir.path().join("form.html");
    let results_page = dir.path().join("results.html");
    fs::write(
            &form_page,
            r#"
            <html><head><title>Form</title></head><body>
            <button id="rename" onclick="document.getElementById('q').setAttribute('name', 'other')">Rename</button>
            <form action="results.html" method="get">
              <input id="q" name="q" value="old">
            </form>
            </body></html>
            "#,
        )
        .unwrap();
    fs::write(
        &results_page,
        "<html><head><title>Results</title></head><body>done</body></html>",
    )
    .unwrap();

    let mut session = BrowserSession::new(BrowserRenderOptions::default());
    session
        .navigate(&form_page.display().to_string())
        .await
        .unwrap();
    session.set_form_field(0, "q", "stored").unwrap();
    session.click_selector("#rename").unwrap();
    assert_eq!(
        session.current().unwrap().forms[0].controls[0].name,
        "other"
    );
    assert_eq!(
        session.current().unwrap().forms[0].controls[0].value,
        "stored"
    );
    session.submit_get_form(0, &[]).await.unwrap();

    assert!(
        session
            .current()
            .unwrap()
            .source
            .ends_with("results.html?other=stored")
    );
}

#[tokio::test]
async fn browser_session_form_fill_uses_exact_field_name() {
    let dir = tempfile::tempdir().unwrap();
    let form_page = dir.path().join("form.html");
    fs::write(
        &form_page,
        r#"
            <html><head><title>Form</title></head><body>
            <form>
              <input name=" q " value="old">
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
    assert!(session.set_form_field(0, "q", "wrong").is_err());
    session.set_form_field(0, " q ", "exact").unwrap();
    assert_eq!(
        session.current().unwrap().forms[0].controls[0].value,
        "exact"
    );
}

#[tokio::test]
async fn browser_session_rejects_checkbox_fill_state() {
    let dir = tempfile::tempdir().unwrap();
    let form_page = dir.path().join("form.html");
    fs::write(
        &form_page,
        r#"
            <html><head><title>Form</title></head><body>
            <form>
              <input type="checkbox" name="fast" checked>
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
    let error = session.set_form_field(0, "fast", "on").unwrap_err();
    assert!(error.to_string().contains("not a fillable form control"));
}

#[tokio::test]
async fn browser_session_toggles_checkbox_state_for_submission() {
    let dir = tempfile::tempdir().unwrap();
    let form_page = dir.path().join("form.html");
    let results_page = dir.path().join("results.html");
    fs::write(
        &form_page,
        r#"
            <html><head><title>Form</title></head><body>
            <form action="results.html" method="get">
              <input id="fast" type="checkbox" name="fast">
              <input name="q" value="rust">
            </form>
            </body></html>
            "#,
    )
    .unwrap();
    fs::write(
        &results_page,
        "<html><head><title>Results</title></head><body>done</body></html>",
    )
    .unwrap();

    let mut session = BrowserSession::new(BrowserRenderOptions::default());
    session
        .navigate(&form_page.display().to_string())
        .await
        .unwrap();
    session
        .click_selector_with_default_action("#fast")
        .await
        .unwrap();
    assert!(session.current().unwrap().forms[0].controls[0].checked);
    assert!(session.current().unwrap().text.contains("[x]"));

    session.submit_get_form(0, &[]).await.unwrap();

    assert!(
        session
            .current()
            .unwrap()
            .source
            .ends_with("results.html?fast=on&q=rust")
    );
}

#[tokio::test]
async fn browser_session_label_click_toggles_associated_checkbox() {
    let dir = tempfile::tempdir().unwrap();
    let form_page = dir.path().join("form.html");
    fs::write(
        &form_page,
        r#"
            <html><head><title>Form</title></head><body>
            <form>
              <label for="fast">Fast mode</label>
              <input id="fast" type="checkbox" name="fast">
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

    session
        .click_selector_with_default_action("label[for=fast]")
        .await
        .unwrap();

    assert!(session.current().unwrap().forms[0].controls[0].checked);
}

#[tokio::test]
async fn browser_session_radio_toggle_unchecks_same_group() {
    let dir = tempfile::tempdir().unwrap();
    let form_page = dir.path().join("form.html");
    fs::write(
        &form_page,
        r#"
            <html><head><title>Form</title></head><body>
            <form>
              <input id="docs" type="radio" name="kind" value="docs" checked>
              <input id="web" type="radio" name="kind" value="web">
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
    session.toggle_form_control(0, 1).unwrap();

    let controls = &session.current().unwrap().forms[0].controls;
    assert!(!controls[0].checked);
    assert!(controls[1].checked);
    assert_eq!(session.current().unwrap().text, "( ) (x)");
}

#[tokio::test]
async fn browser_session_reset_restores_default_checkbox_state() {
    let dir = tempfile::tempdir().unwrap();
    let form_page = dir.path().join("form.html");
    fs::write(
        &form_page,
        r#"
            <html><head><title>Form</title></head><body>
            <form>
              <input id="fast" type="checkbox" name="fast" checked>
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
    session.toggle_form_control(0, 0).unwrap();
    assert!(!session.current().unwrap().forms[0].controls[0].checked);
    assert!(session.current().unwrap().text.contains("[ ]"));

    session
        .click_selector_with_default_action("#reset")
        .await
        .unwrap();

    assert!(session.current().unwrap().forms[0].controls[0].checked);
    assert!(session.current().unwrap().text.contains("[x]"));

    session.toggle_form_control(0, 0).unwrap();
    session.focus_selector("#reset").unwrap();
    session.submit_focused_form().await.unwrap();
    assert!(session.current().unwrap().forms[0].controls[0].checked);
}

#[tokio::test]
async fn browser_session_tracks_back_forward_and_truncates_forward_history() {
    let dir = tempfile::tempdir().unwrap();
    let first = dir.path().join("first.html");
    let second = dir.path().join("second.html");
    let third = dir.path().join("third.html");
    fs::write(&first, "<title>First</title><body>one</body>").unwrap();
    fs::write(&second, "<title>Second</title><body>two</body>").unwrap();
    fs::write(&third, "<title>Third</title><body>three</body>").unwrap();

    let mut session = BrowserSession::new(BrowserRenderOptions::default());
    session
        .navigate(&first.display().to_string())
        .await
        .unwrap();
    session
        .navigate(&second.display().to_string())
        .await
        .unwrap();
    assert_eq!(session.current().unwrap().title, "Second");

    session.back().unwrap();
    assert_eq!(session.current().unwrap().title, "First");
    session.forward().unwrap();
    assert_eq!(session.current().unwrap().title, "Second");

    session.back().unwrap();
    session
        .navigate(&third.display().to_string())
        .await
        .unwrap();
    let snapshot = session.snapshot();
    assert_eq!(snapshot.current_index, Some(1));
    assert_eq!(snapshot.entries.len(), 2);
    assert_eq!(session.current().unwrap().title, "Third");
    assert!(session.forward().is_err());
}

#[tokio::test]
async fn browser_session_history_retains_recent_entries_with_visible_limit() {
    let dir = tempfile::tempdir().unwrap();
    let mut session = BrowserSession::new(BrowserRenderOptions::default());

    for index in 0..(BROWSER_SESSION_HISTORY_MAX_ENTRIES + 3) {
        let page = dir.path().join(format!("page-{index}.html"));
        fs::write(
            &page,
            format!("<title>Page {index}</title><body>{index}</body>"),
        )
        .unwrap();
        session.navigate(&page.display().to_string()).await.unwrap();
    }

    let snapshot = session.snapshot();
    assert_eq!(
        snapshot.retained_entry_limit,
        BROWSER_SESSION_HISTORY_MAX_ENTRIES
    );
    assert_eq!(
        snapshot.retained_entry_count,
        BROWSER_SESSION_HISTORY_MAX_ENTRIES
    );
    assert_eq!(snapshot.entries.len(), BROWSER_SESSION_HISTORY_MAX_ENTRIES);
    assert_eq!(
        snapshot.current_index,
        Some(BROWSER_SESSION_HISTORY_MAX_ENTRIES - 1)
    );
    assert_eq!(snapshot.entries[0].title, "Page 3");
    assert_eq!(
        snapshot.entries[BROWSER_SESSION_HISTORY_MAX_ENTRIES - 1].title,
        format!("Page {}", BROWSER_SESSION_HISTORY_MAX_ENTRIES + 2)
    );
    assert!(session.back().is_ok());
}

#[tokio::test]
async fn browser_session_reload_replaces_current_history_entry() {
    let dir = tempfile::tempdir().unwrap();
    let first = dir.path().join("first.html");
    let second = dir.path().join("second.html");
    fs::write(&first, "<title>First</title><body>one</body>").unwrap();
    fs::write(&second, "<title>Second</title><body>two</body>").unwrap();

    let mut session = BrowserSession::new(BrowserRenderOptions::default());
    session
        .navigate(&first.display().to_string())
        .await
        .unwrap();
    session
        .navigate(&second.display().to_string())
        .await
        .unwrap();
    fs::write(
        &second,
        "<html><head><title>Second Reloaded</title></head><body>updated</body></html>",
    )
    .unwrap();

    let render = session.reload().await.unwrap();
    assert_eq!(render.title, "Second Reloaded");
    assert_eq!(render.text, "updated");
    let snapshot = session.snapshot();
    assert_eq!(snapshot.current_index, Some(1));
    assert_eq!(snapshot.entries.len(), 2);
    assert_eq!(snapshot.entries[0].title, "First");
    assert_eq!(snapshot.entries[1].title, "Second Reloaded");
}

#[tokio::test]
async fn browser_session_reload_clears_focused_transient_form_state() {
    let dir = tempfile::tempdir().unwrap();
    let form_page = dir.path().join("form.html");
    fs::write(
        &form_page,
        r#"
            <html><head><title>Form</title></head><body>
              <form><input id="q" name="q" value="old"></form>
            </body></html>
            "#,
    )
    .unwrap();

    let mut session = BrowserSession::new(BrowserRenderOptions::default());
    session
        .navigate(&form_page.display().to_string())
        .await
        .unwrap();
    session.focus_selector("#q").unwrap();
    session.type_text(" typed").unwrap();
    assert_eq!(
        session.current().unwrap().forms[0].controls[0].value,
        "old typed"
    );

    let render = session.reload().await.unwrap();
    assert_eq!(render.forms[0].controls[0].value, "old");
    assert_eq!(session.focused_control(), None);
}

#[test]
fn verifies_browser_fixture_manifest() {
    let dir = tempfile::tempdir().unwrap();
    let html_path = dir.path().join("page.html");
    let manifest_path = dir.path().join("fixtures.json");
    fs::write(
            &html_path,
            "<html><head><title>Fixture</title></head><body><p>one two three four five</p></body></html>",
        )
        .unwrap();
    fs::write(
        &manifest_path,
        serde_json::json!({
            "fixtures": [{
                "name": "wrap fixture",
                "path": "page.html",
                "width": 20,
                "expected_title": "Fixture",
                "expected_text": "one two three four\nfive",
                "expected_display_list": [
                    {"command":"text","x":0,"y":0,"text":"one two three four"},
                    {"command":"text","x":0,"y":1,"text":"five"}
                ]
            }]
        })
        .to_string(),
    )
    .unwrap();

    let report = verify_browser_fixtures(&manifest_path).unwrap();
    assert_eq!(report.fixture_count, 1);
    assert_eq!(report.passed, 1);
    assert_eq!(report.failed, 0);
}

#[test]
fn verifies_browser_fixture_manifest_raster_viewport_expectations() {
    let dir = tempfile::tempdir().unwrap();
    let html_path = dir.path().join("page.html");
    let manifest_path = dir.path().join("fixtures.json");
    fs::write(
        &html_path,
        "<body><p>zero</p><p>one</p><p>two</p><p>three</p></body>",
    )
    .unwrap();
    let render = render_html(
        &html_path.display().to_string(),
        &fs::read(&html_path).unwrap(),
        BrowserRenderOptions {
            width: 10,
            ..BrowserRenderOptions::default()
        },
    );
    let options = BrowserRasterOptions {
        viewport_y: Some(1),
        viewport_height: Some(2),
        ..BrowserRasterOptions::default()
    };
    let raster = rasterize_render(&render, options).unwrap();
    fs::write(
        &manifest_path,
        serde_json::json!({
            "fixtures": [{
                "name": "viewport raster fixture",
                "path": "page.html",
                "width": 10,
                "raster_viewport_y": 1,
                "raster_viewport_height": 2,
                "expected_visible_command_count": 2,
                "expected_culled_command_count": 2,
                "expected_raster_hash": raster.pixel_hash()
            }]
        })
        .to_string(),
    )
    .unwrap();

    let report = verify_browser_fixtures(&manifest_path).unwrap();
    assert_eq!(report.fixture_count, 1);
    assert_eq!(report.passed, 1);
    assert_eq!(report.failed, 0);
}

#[test]
fn verifies_browser_fixture_manifest_screenshot_hash_expectations() {
    let dir = tempfile::tempdir().unwrap();
    let html_path = dir.path().join("page.html");
    let manifest_path = dir.path().join("fixtures.json");
    fs::write(
        &html_path,
        "<html><head><title>Shot</title></head><body><h1>PNG</h1></body></html>",
    )
    .unwrap();
    let render = render_html(
        &html_path.display().to_string(),
        &fs::read(&html_path).unwrap(),
        BrowserRenderOptions::default(),
    );
    let screenshot_hash = rasterize_render_rgba(&render, BrowserRasterOptions::default())
        .unwrap()
        .pixel_hash();

    fs::write(
        &manifest_path,
        serde_json::json!({
            "fixtures": [{
                "name": "screenshot hash fixture",
                "path": "page.html",
                "expected_text": "PNG",
                "expected_screenshot_hash": screenshot_hash
            }]
        })
        .to_string(),
    )
    .unwrap();

    let report = verify_browser_fixtures(&manifest_path).unwrap();
    assert_eq!(report.fixture_count, 1);
    assert_eq!(report.failed, 0);

    fs::write(
        &manifest_path,
        serde_json::json!({
            "fixtures": [{
                "name": "bad screenshot hash fixture",
                "path": "page.html",
                "expected_text": "PNG",
                "expected_screenshot_hash": "0000000000000000000000000000000000000000000000000000000000000000"
            }]
        })
        .to_string(),
    )
    .unwrap();

    let failed = verify_browser_fixtures(&manifest_path).unwrap();
    assert_eq!(failed.failed, 1);
    assert!(
        failed.failures[0]
            .reason
            .contains("screenshot hash mismatch"),
        "{:?}",
        failed.failures
    );
}

#[test]
fn reports_browser_fixture_failures() {
    let dir = tempfile::tempdir().unwrap();
    let html_path = dir.path().join("page.html");
    let manifest_path = dir.path().join("fixtures.json");
    fs::write(&html_path, "<body>actual</body>").unwrap();
    fs::write(
        &manifest_path,
        serde_json::json!({
            "fixtures": [{
                "name": "bad fixture",
                "path": "page.html",
                "expected_text": "expected"
            }]
        })
        .to_string(),
    )
    .unwrap();

    let report = verify_browser_fixtures(&manifest_path).unwrap();
    assert_eq!(report.passed, 0);
    assert_eq!(report.failed, 1);
    assert!(report.failures[0].reason.contains("text mismatch"));
}
