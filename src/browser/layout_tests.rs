use super::*;

#[test]
fn renders_ordered_and_unordered_list_markers() {
    let render = render_html(
        "mem://lists",
        br#"
            <html><body>
              <ol start="3">
                <li>Install Rust</li>
                <li value="7">Run tests</li>
                <li>Ship browser</li>
              </ol>
              <ol reversed>
                <li>Third</li>
                <li>Second</li>
              </ol>
              <ul><li>Plain bullet</li></ul>
            </body></html>
            "#,
        BrowserRenderOptions {
            width: 80,
            ..BrowserRenderOptions::default()
        },
    );

    assert_eq!(
        render.text,
        "3. Install Rust\n7. Run tests\n8. Ship browser\n2. Third\n1. Second\n- Plain bullet"
    );
    assert_eq!(
        render.display_list,
        vec![
            DisplayCommand::Text {
                x: 0,
                y: 0,
                text: "3. Install Rust".to_owned(),
            },
            DisplayCommand::Text {
                x: 0,
                y: 1,
                text: "7. Run tests".to_owned(),
            },
            DisplayCommand::Text {
                x: 0,
                y: 2,
                text: "8. Ship browser".to_owned(),
            },
            DisplayCommand::Text {
                x: 0,
                y: 3,
                text: "2. Third".to_owned(),
            },
            DisplayCommand::Text {
                x: 0,
                y: 4,
                text: "1. Second".to_owned(),
            },
            DisplayCommand::Text {
                x: 0,
                y: 5,
                text: "- Plain bullet".to_owned(),
            },
        ]
    );
}

#[test]
fn ordered_list_counters_skip_hidden_items() {
    let render = render_html(
        "mem://hidden-list-items",
        br#"
            <html><body>
              <ol start="4">
                <li hidden value="20">Hidden reset</li>
                <li>Visible start</li>
                <li>Visible next</li>
              </ol>
              <ol reversed>
                <li>Two</li>
                <li hidden>Hidden middle</li>
                <li>One</li>
              </ol>
            </body></html>
            "#,
        BrowserRenderOptions {
            width: 80,
            ..BrowserRenderOptions::default()
        },
    );

    assert_eq!(
        render.text,
        "4. Visible start\n5. Visible next\n2. Two\n1. One"
    );
    assert_eq!(
        render.display_list,
        vec![
            DisplayCommand::Text {
                x: 0,
                y: 0,
                text: "4. Visible start".to_owned(),
            },
            DisplayCommand::Text {
                x: 0,
                y: 1,
                text: "5. Visible next".to_owned(),
            },
            DisplayCommand::Text {
                x: 0,
                y: 2,
                text: "2. Two".to_owned(),
            },
            DisplayCommand::Text {
                x: 0,
                y: 3,
                text: "1. One".to_owned(),
            },
        ]
    );
}

#[test]
fn renders_ordered_list_type_markers() {
    let render = render_html(
        "mem://typed-lists",
        br#"
            <html><body>
              <ol type="a" start="26">
                <li>Twenty six</li>
                <li>Twenty seven</li>
                <li value="28">Twenty eight</li>
              </ol>
              <ol type="I" start="4">
                <li>Four</li>
                <li>Nine</li>
              </ol>
              <ol type="i" reversed start="3">
                <li>Three</li>
                <li>Two</li>
              </ol>
              <ol type="A">
                <li>Upper alpha</li>
              </ol>
            </body></html>
            "#,
        BrowserRenderOptions {
            width: 80,
            ..BrowserRenderOptions::default()
        },
    );

    assert_eq!(
        render.text,
        "z. Twenty six\naa. Twenty seven\nab. Twenty eight\nIV. Four\nV. Nine\niii. Three\nii. Two\nA. Upper alpha"
    );
    assert_eq!(
        render
            .display_list
            .iter()
            .filter_map(|command| match command {
                DisplayCommand::Text { text, .. } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>(),
        vec![
            "z. Twenty six",
            "aa. Twenty seven",
            "ab. Twenty eight",
            "IV. Four",
            "V. Nine",
            "iii. Three",
            "ii. Two",
            "A. Upper alpha",
        ]
    );
}

#[test]
fn ordered_list_item_type_overrides_parent_marker_style() {
    let render = render_html(
        "mem://list-item-type",
        br#"
            <html><body>
              <ol type="a" start="2">
                <li>Lower alpha</li>
                <li type="I">Upper roman</li>
                <li type="1" value="9">Decimal override</li>
                <li type="A">Upper alpha</li>
                <li>Back to lower alpha</li>
              </ol>
            </body></html>
            "#,
        BrowserRenderOptions {
            width: 80,
            ..BrowserRenderOptions::default()
        },
    );

    assert_eq!(
        render.text,
        "b. Lower alpha\nIII. Upper roman\n9. Decimal override\nJ. Upper alpha\nk. Back to lower alpha"
    );
    assert_eq!(
        render
            .display_list
            .iter()
            .filter_map(|command| match command {
                DisplayCommand::Text { text, .. } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>(),
        vec![
            "b. Lower alpha",
            "III. Upper roman",
            "9. Decimal override",
            "J. Upper alpha",
            "k. Back to lower alpha",
        ]
    );
}

#[test]
fn list_marker_attributes_ignore_surrounding_whitespace() {
    let render = render_html(
        "mem://list-marker-attribute-normalization",
        br#"
            <html><body>
              <ol type=" A " start=" +26 ">
                <li>Upper alpha</li>
                <li type=" i " value=" 9 ">Lower roman</li>
              </ol>
              <ul type=" CIRCLE ">
                <li>Uppercase circle</li>
                <li type=" SQUARE ">Uppercase square</li>
              </ul>
            </body></html>
            "#,
        BrowserRenderOptions {
            width: 80,
            ..BrowserRenderOptions::default()
        },
    );

    assert_eq!(
        render.text,
        "Z. Upper alpha\nix. Lower roman\no Uppercase circle\n* Uppercase square"
    );
    assert_eq!(
        render
            .display_list
            .iter()
            .filter_map(|command| match command {
                DisplayCommand::Text { text, .. } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>(),
        vec![
            "Z. Upper alpha",
            "ix. Lower roman",
            "o Uppercase circle",
            "* Uppercase square",
        ]
    );
}

#[test]
fn css_list_style_type_controls_markers() {
    let render = render_html(
        "mem://css-list-style-type",
        br#"
            <html><head><style>
              ol.alpha { list-style-type: upper-alpha }
              ul.square { list-style: square inside }
              ul.none { list-style-type: none }
              li.roman { list-style-type: lower-roman }
              li.hidden { display: none }
            </style></head><body>
              <ol class="alpha" start="2">
                <li>Beta</li>
                <li class="roman" value="4">Roman item</li>
                <li>Delta</li>
              </ol>
              <ul class="square">
                <li>Square item</li>
                <li style="list-style-type: circle">Circle item</li>
              </ul>
              <ul class="none">
                <li>No marker</li>
              </ul>
              <ol>
                <li>One</li>
                <li class="hidden" value="9">Hidden reset</li>
                <li>Two</li>
              </ol>
            </body></html>
            "#,
        BrowserRenderOptions {
            width: 80,
            ..BrowserRenderOptions::default()
        },
    );

    assert_eq!(
        render.text,
        "B. Beta\niv. Roman item\nE. Delta\n* Square item\no Circle item\nNo marker\n1. One\n2. Two"
    );
    assert_eq!(
        render
            .display_list
            .iter()
            .filter_map(|command| match command {
                DisplayCommand::Text { text, .. } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>(),
        vec![
            "B. Beta",
            "iv. Roman item",
            "E. Delta",
            "* Square item",
            "o Circle item",
            "No marker",
            "1. One",
            "2. Two",
        ]
    );
}

#[test]
fn css_display_list_item_controls_marker_generation() {
    let render = render_html(
        "mem://css-display-list-item",
        br#"
            <html><body>
              <ul>
                <li style="display: inline">Inline item</li>
                <li>Default item</li>
              </ul>
              <ol>
                <li>One</li>
                <li style="display: block" value="9">Block item</li>
                <li>Two</li>
              </ol>
              <div style="display: list-item; list-style-type: square">Generated marker</div>
            </body></html>
            "#,
        BrowserRenderOptions {
            width: 80,
            ..BrowserRenderOptions::default()
        },
    );

    assert_eq!(
        render.text,
        "Inline item\n- Default item\n1. One\nBlock item\n2. Two\n* Generated marker"
    );
    assert_eq!(
        render
            .display_list
            .iter()
            .filter_map(|command| match command {
                DisplayCommand::Text { text, .. } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>(),
        vec![
            "Inline item",
            "- Default item",
            "1. One",
            "Block item",
            "2. Two",
            "* Generated marker",
        ]
    );
    assert_eq!(
        render
            .layout_boxes
            .iter()
            .filter(|layout_box| matches!(layout_box.tag.as_str(), "li" | "div"))
            .map(|layout_box| (layout_box.tag.as_str(), layout_box.kind.as_str()))
            .collect::<Vec<_>>(),
        vec![
            ("li", "inline"),
            ("li", "list-item"),
            ("li", "list-item"),
            ("li", "block"),
            ("li", "list-item"),
            ("div", "list-item"),
        ]
    );
}

#[test]
fn css_modern_display_values_map_to_flow_and_suppress_markers() {
    let render = render_html(
        "mem://css-modern-display-values",
        br#"
            <html><body>
              <ul>
                <li style="display:flex">Flex item</li>
                <li style="display:grid">Grid item</li>
                <li style="display:inline-flex">Inline flex item</li>
                <li>Default marker</li>
              </ul>
              <div style="display:flow-root">Flow root block</div>
              <span style="display:inline-grid">Inline grid</span><span>after</span>
            </body></html>
            "#,
        BrowserRenderOptions {
            width: 80,
            ..BrowserRenderOptions::default()
        },
    );

    assert_eq!(
        render.text,
        "Flex item\nGrid item\nInline flex item\n- Default marker\nFlow root block\nInline gridafter"
    );
    assert_eq!(
        render
            .layout_boxes
            .iter()
            .filter(|layout_box| matches!(layout_box.tag.as_str(), "li" | "div" | "span"))
            .map(|layout_box| (layout_box.tag.as_str(), layout_box.kind.as_str()))
            .collect::<Vec<_>>(),
        vec![
            ("li", "flex"),
            ("li", "grid"),
            ("li", "inline-flex"),
            ("li", "list-item"),
            ("div", "flow-root"),
            ("span", "inline-grid"),
            ("span", "inline"),
        ]
    );
}

#[test]
fn css_first_child_selector_restores_first_hidden_slide() {
    let render = render_html(
        "mem://css-first-child-slider",
        br#"
            <html><head><style>
              .slider .slide { display: none; background-color: black; }
              .slider .slide:first-child { display: block; }
            </style></head><body>
              <div class="slider">
                <div class="slide">Visible slide</div>
                <div class="slide">Hidden slide</div>
              </div>
              <p>After</p>
            </body></html>
            "#,
        BrowserRenderOptions {
            width: 80,
            ..BrowserRenderOptions::default()
        },
    );

    assert_eq!(render.text, "Visible slide\nAfter");
    assert!(!render.text.contains("Hidden slide"));
}

#[test]
fn css_media_rules_do_not_leak_hidden_hero_text() {
    let render = render_html(
        "mem://css-media-hidden-hero",
        br#"
            <html><head><style>
              .hero-title { display: block; }
              @media (max-width: 600px) {
                .unused { color: red; }
                .hero-title { display: none; }
              }
              .hero-subtitle { display: block; }
            </style></head><body>
              <section>
                <h1 class="hero-title">Saving Lives with Data</h1>
                <p class="hero-subtitle">Real-time intelligence</p>
              </section>
            </body></html>
            "#,
        BrowserRenderOptions {
            width: 80,
            ..BrowserRenderOptions::default()
        },
    );

    assert_eq!(
        render.text,
        "Saving Lives with Data\nReal-time intelligence"
    );
}

#[test]
fn css_flex_container_lays_out_block_children_in_row() {
    let render = render_html(
        "mem://css-flex-row-children",
        br#"
            <html><body>
              <nav style="display:flex"><div>Data</div><div>Intelligence</div><div style="display:contents"><div>Evidence</div></div><div>Customers</div></nav>
              <p>After</p>
            </body></html>
            "#,
        BrowserRenderOptions {
            width: 80,
            ..BrowserRenderOptions::default()
        },
    );

    assert_eq!(render.text, "Data Intelligence Evidence Customers\nAfter");
    assert_eq!(
        render.display_list,
        vec![
            DisplayCommand::Text {
                x: 0,
                y: 0,
                text: "Data Intelligence Evidence Customers".to_owned(),
            },
            DisplayCommand::Text {
                x: 0,
                y: 1,
                text: "After".to_owned(),
            },
        ]
    );
}

#[test]
fn css_flex_row_images_reserve_inline_space_before_text() {
    let render = render_html(
        "mem://css-flex-row-image",
        br#"
            <html><body>
              <section style="display:flex"><img alt="chart" width="24" height="24"><div>Hero copy</div></section>
              <p>After</p>
            </body></html>
            "#,
        BrowserRenderOptions {
            width: 80,
            ..BrowserRenderOptions::default()
        },
    );

    assert_eq!(
        render.display_list,
        vec![
            DisplayCommand::Image {
                x: 0,
                y: 0,
                width: 3,
                height: 2,
                shade: 220,
                alt: Some("chart".to_owned()),
                url: None,
                decoded_width: None,
                decoded_height: None,
                decoded_hash: None,
            },
            DisplayCommand::Text {
                x: 3,
                y: 0,
                text: " Hero copy".to_owned(),
            },
            DisplayCommand::Text {
                x: 0,
                y: 2,
                text: "After".to_owned(),
            },
        ]
    );
}

#[test]
fn css_flex_row_images_keep_following_text_on_row() {
    let render = render_html(
        "mem://css-flex-row-image",
        br#"
            <html><body>
              <section style="display:flex">
                <img alt="chart" width="16" height="24">
                <div>Hero copy</div>
              </section>
              <p>After</p>
            </body></html>
            "#,
        BrowserRenderOptions {
            width: 40,
            ..BrowserRenderOptions::default()
        },
    );

    assert_eq!(
        render.display_list,
        vec![
            DisplayCommand::Image {
                x: 0,
                y: 0,
                width: 2,
                height: 2,
                shade: 220,
                alt: Some("chart".to_owned()),
                url: None,
                decoded_width: None,
                decoded_height: None,
                decoded_hash: None,
            },
            DisplayCommand::Text {
                x: 2,
                y: 0,
                text: " Hero copy".to_owned(),
            },
            DisplayCommand::Text {
                x: 0,
                y: 2,
                text: "After".to_owned(),
            },
        ]
    );
}

#[test]
fn css_gap_row_flow_preserves_image_text_in_scrolled_viewport() {
    let image_url = "mem://gap-row-flow-image".to_owned();
    let decoded = DecodedImage {
        width: 1,
        height: 1,
        pixels: vec![96],
        rgb_pixels: Some(vec![20, 120, 220]),
    };
    let decoded_entry = DecodedImageEntry {
        url: image_url.clone(),
        width: decoded.width,
        height: decoded.height,
        pixel_hash: decoded.pixel_hash(),
        image: decoded,
    };
    let html = format!(
        r#"
            <html><body>
              <div style="height:12px"></div>
              <section style="display:flex; gap:12px 16px; column-gap:24px">
                <img src="{image_url}" alt="" width="16" height="24">
                <div>Hero copy</div>
              </section>
              <p>After</p>
            </body></html>
            "#
    );
    let render = render_html_prepared_with_inputs(
        "mem://gap-row-flow",
        html.as_bytes(),
        BrowserRenderOptions {
            width: 32,
            ..BrowserRenderOptions::default()
        },
        RenderPreparation {
            external_css: &[],
            external_scripts: &[],
            click_target: None,
            local_storage: None,
            session_storage: None,
            cached_images: &[decoded_entry],
        },
    )
    .expect("render flex row gap with decoded image");

    let image_bounds = render
        .display_list
        .iter()
        .find_map(|command| match command {
            DisplayCommand::Image {
                x,
                y,
                width,
                height,
                decoded_width: Some(1),
                decoded_height: Some(1),
                ..
            } => Some((*x, *y, *width, *height)),
            _ => None,
        })
        .expect("decoded image command");
    assert_eq!(image_bounds, (0, 1, 2, 2));
    let (hero_text_run_x, hero_text_y, hero_text) = render
        .display_list
        .iter()
        .find_map(|command| match command {
            DisplayCommand::Text { x, y, text } | DisplayCommand::StyledText { x, y, text, .. }
                if text.contains("Hero copy") =>
            {
                Some((*x, *y, text.as_str()))
            }
            _ => None,
        })
        .expect("hero text command");
    let hero_text_x = hero_text_run_x.saturating_add(
        hero_text
            .chars()
            .take_while(|ch| ch.is_whitespace())
            .count(),
    );
    assert_eq!(hero_text_y, image_bounds.1);
    assert!(
        hero_text_x
            >= image_bounds
                .0
                .saturating_add(image_bounds.2)
                .saturating_add(3),
        "expected explicit column gap between image and text, got image={image_bounds:?} text_x={hero_text_x}"
    );

    let raster_options = BrowserRasterOptions {
        viewport_y: Some(1),
        viewport_width: Some(32),
        viewport_height: Some(2),
        ..BrowserRasterOptions::default()
    };
    let rgba =
        rasterize_render_rgba(&render, raster_options).expect("rasterize scrolled flex gap row");
    let pixel = |x: usize, y: usize| {
        let index = y
            .saturating_mul(rgba.width)
            .saturating_add(x)
            .saturating_mul(4);
        &rgba.pixels[index..index.saturating_add(4)]
    };
    assert_eq!(
        pixel(raster_options.padding_x, raster_options.padding_y),
        &[20, 120, 220, 255]
    );

    let text_row_y = raster_options.padding_y.saturating_add(2);
    let text_col_start = raster_options
        .padding_x
        .saturating_add(hero_text_x.saturating_mul(raster_options.cell_width));
    let text_col_end = text_col_start.saturating_add("Hero copy".len() * raster_options.cell_width);
    let mut glyph_pixels = 0usize;
    for y in text_row_y..text_row_y.saturating_add(7).min(rgba.height) {
        for x in text_col_start..text_col_end.min(rgba.width) {
            if pixel(x, y) == &[0, 0, 0, 255] {
                glyph_pixels = glyph_pixels.saturating_add(1);
            }
        }
    }
    assert!(glyph_pixels >= 8);
}

#[test]
fn css_inline_block_menu_items_keep_block_links_on_row() {
    let render = render_html(
        "mem://css-inline-block-menu-links",
        br#"
            <html><body>
              <ul>
                <li class="item"><a class="link">Data</a></li>
                <li class="item"><a class="link">Intelligence</a></li>
                <li class="item"><a class="link">Customers</a></li>
              </ul>
              <p>After</p>
              <style>
                ul > li.item { display: inline-block; margin: 0 8px 0 0; }
                ul > li.item > a.link { display: block; line-height: 40px; padding: 0 10px; }
              </style>
            </body></html>
            "#,
        BrowserRenderOptions {
            width: 80,
            ..BrowserRenderOptions::default()
        },
    );

    assert_eq!(render.text, "Data Intelligence Customers\nAfter");
    assert_eq!(
        render.display_list,
        vec![
            DisplayCommand::Text {
                x: 0,
                y: 0,
                text: "Data Intelligence Customers".to_owned(),
            },
            DisplayCommand::Text {
                x: 0,
                y: 1,
                text: "After".to_owned(),
            },
        ]
    );
}

#[test]
fn css_display_contents_flattens_wrapper_without_painting_box() {
    let render = render_html(
        "mem://css-display-contents",
        br#"
            <html><body>
              Before <div style="display: contents; text-transform: uppercase; padding-left: 10px; border: 2px solid black; background: black">Middle <span>Inner</span></div> After
            </body></html>
            "#,
        BrowserRenderOptions {
            width: 80,
            ..BrowserRenderOptions::default()
        },
    );

    assert_eq!(render.text, "Before MIDDLE INNER After");
    assert_eq!(
        render.display_list,
        vec![DisplayCommand::Text {
            x: 0,
            y: 0,
            text: "Before MIDDLE INNER After".to_owned(),
        }]
    );
    assert!(
        !render
            .layout_boxes
            .iter()
            .any(|layout_box| layout_box.tag == "div")
    );
}

#[test]
fn indents_nested_list_markers() {
    let render = render_html(
        "mem://nested-lists",
        br#"
            <html><body>
              <ul>
                <li>Parent
                  <ul>
                    <li>Child
                      <ol start="2">
                        <li>Grandchild</li>
                      </ol>
                    </li>
                  </ul>
                </li>
                <li>Sibling</li>
              </ul>
            </body></html>
            "#,
        BrowserRenderOptions {
            width: 80,
            ..BrowserRenderOptions::default()
        },
    );

    assert_eq!(render.text, "- Parent\no Child\n2. Grandchild\n- Sibling");
    assert_eq!(
        render.display_list,
        vec![
            DisplayCommand::Text {
                x: 0,
                y: 0,
                text: "- Parent".to_owned(),
            },
            DisplayCommand::Text {
                x: 2,
                y: 1,
                text: "o Child".to_owned(),
            },
            DisplayCommand::Text {
                x: 4,
                y: 2,
                text: "2. Grandchild".to_owned(),
            },
            DisplayCommand::Text {
                x: 0,
                y: 3,
                text: "- Sibling".to_owned(),
            },
        ]
    );
}

#[test]
fn unordered_list_markers_use_all_list_ancestor_depth() {
    let render = render_html(
        "mem://mixed-list-depth",
        br#"
            <html><body>
              <ol>
                <li>Ordered parent
                  <ul>
                    <li>Unordered child
                      <ol>
                        <li>Ordered grandchild
                          <ul>
                            <li>Deep unordered</li>
                          </ul>
                        </li>
                      </ol>
                    </li>
                  </ul>
                </li>
              </ol>
            </body></html>
            "#,
        BrowserRenderOptions {
            width: 80,
            ..BrowserRenderOptions::default()
        },
    );

    assert_eq!(
        render.text,
        "1. Ordered parent\no Unordered child\n1. Ordered grandchild\n* Deep unordered"
    );
    assert_eq!(
        render
            .display_list
            .iter()
            .filter_map(|command| match command {
                DisplayCommand::Text { text, .. } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>(),
        vec![
            "1. Ordered parent",
            "o Unordered child",
            "1. Ordered grandchild",
            "* Deep unordered",
        ]
    );
}

#[test]
fn renders_unordered_list_type_markers() {
    let render = render_html(
        "mem://unordered-types",
        br#"
            <html><body>
              <ul type="circle">
                <li>Circle parent</li>
                <li type="square">Square item</li>
                <li type="disc">Disc item</li>
                <li>Circle again</li>
              </ul>
              <ul type="square">
                <li>Square parent</li>
              </ul>
            </body></html>
            "#,
        BrowserRenderOptions {
            width: 80,
            ..BrowserRenderOptions::default()
        },
    );

    assert_eq!(
        render.text,
        "o Circle parent\n* Square item\n- Disc item\no Circle again\n* Square parent"
    );
    assert_eq!(
        render.display_list,
        vec![
            DisplayCommand::Text {
                x: 0,
                y: 0,
                text: "o Circle parent".to_owned(),
            },
            DisplayCommand::Text {
                x: 0,
                y: 1,
                text: "* Square item".to_owned(),
            },
            DisplayCommand::Text {
                x: 0,
                y: 2,
                text: "- Disc item".to_owned(),
            },
            DisplayCommand::Text {
                x: 0,
                y: 3,
                text: "o Circle again".to_owned(),
            },
            DisplayCommand::Text {
                x: 0,
                y: 4,
                text: "* Square parent".to_owned(),
            },
        ]
    );
}

#[test]
fn preserves_preformatted_text_spaces_and_blank_lines() {
    let render = render_html(
        "mem://preformatted",
        br#"<html><body><pre>fn main() {
  println!("hi");

done</pre><p style="white-space: pre">A  B
C</p></body></html>"#,
        BrowserRenderOptions {
            width: 80,
            ..BrowserRenderOptions::default()
        },
    );

    assert_eq!(
        render.text,
        "fn main() {\n  println!(\"hi\");\n\ndone\nA  B\nC"
    );
    assert_eq!(
        render.display_list,
        vec![
            DisplayCommand::Text {
                x: 0,
                y: 0,
                text: "fn main() {".to_owned(),
            },
            DisplayCommand::Text {
                x: 0,
                y: 1,
                text: "  println!(\"hi\");".to_owned(),
            },
            DisplayCommand::Text {
                x: 0,
                y: 3,
                text: "done".to_owned(),
            },
            DisplayCommand::Text {
                x: 0,
                y: 4,
                text: "A  B".to_owned(),
            },
            DisplayCommand::Text {
                x: 0,
                y: 5,
                text: "C".to_owned(),
            },
        ]
    );
}

#[test]
fn css_white_space_nowrap_suppresses_soft_wrapping() {
    let render = render_html(
        "mem://nowrap",
        br#"
            <html><body>
              <p>Alpha Beta Gamma Delta</p>
              <p style="white-space: nowrap">Alpha Beta Gamma Delta</p>
            </body></html>
            "#,
        BrowserRenderOptions {
            width: 20,
            ..BrowserRenderOptions::default()
        },
    );

    assert_eq!(
        render.text,
        "Alpha Beta Gamma\nDelta\nAlpha Beta Gamma Delta"
    );
    assert_eq!(
        render.display_list,
        vec![
            DisplayCommand::Text {
                x: 0,
                y: 0,
                text: "Alpha Beta Gamma".to_owned(),
            },
            DisplayCommand::Text {
                x: 0,
                y: 1,
                text: "Delta".to_owned(),
            },
            DisplayCommand::Text {
                x: 0,
                y: 2,
                text: "Alpha Beta Gamma Delta".to_owned(),
            },
        ]
    );
}

#[test]
fn inline_text_spacing_preserves_punctuation_boundaries() {
    let render = render_html(
        "mem://inline-punctuation-spacing",
        br##"
            <html><body>
              <p>The <a href="/war">Baptist War</a>, broke out.[<a href="#cite">59</a>]</p>
              <p><span>Alpha</span><span>Beta</span><span> Gamma</span><span> Delta</span></p>
              <p><span>[</span><a href="#ref">12</a><span>]</span> marker</p>
            </body></html>
            "##,
        BrowserRenderOptions {
            width: 80,
            ..BrowserRenderOptions::default()
        },
    );

    assert_eq!(
        render.text,
        "The Baptist War, broke out.[59]\nAlphaBeta Gamma Delta\n[12] marker"
    );
}

#[test]
fn inline_text_spacing_preserves_source_whitespace_between_elements() {
    let render = render_html(
        "mem://inline-element-whitespace",
        br#"
            <html><body>
              <p><span>Alpha</span> <span>Beta</span>
                 <span>Gamma</span><span>!</span></p>
              <div>
                <span>Block</span>
              </div>
              <span>Tail</span>
            </body></html>
            "#,
        BrowserRenderOptions {
            width: 80,
            ..BrowserRenderOptions::default()
        },
    );

    assert_eq!(render.text, "Alpha Beta Gamma!\nBlock\nTail");
}

#[test]
fn css_inline_background_color_paints_text_run_underlay() {
    let render = render_html(
        "mem://css-inline-background",
        br#"
            <html><body>
              <p><span style="background-color: silver">Mark</span></p>
            </body></html>
            "#,
        BrowserRenderOptions {
            width: 20,
            ..BrowserRenderOptions::default()
        },
    );

    assert_eq!(render.text, "Mark");
    assert_eq!(
        render.display_list,
        vec![
            DisplayCommand::Rect {
                x: 0,
                y: 0,
                width: 4,
                height: 1,
                shade: 192,
            },
            DisplayCommand::Text {
                x: 0,
                y: 0,
                text: "Mark".to_owned(),
            },
        ]
    );
}

#[test]
fn css_white_space_pre_line_preserves_newlines_and_wraps() {
    let render = render_html(
        "mem://pre-line",
        br#"
            <html><body>
              <p style="white-space: pre-line">Alpha   Beta
Gamma Delta Epsilon Zeta</p>
            </body></html>
            "#,
        BrowserRenderOptions {
            width: 20,
            ..BrowserRenderOptions::default()
        },
    );

    assert_eq!(render.text, "Alpha Beta\nGamma Delta Epsilon\nZeta");
    assert_eq!(
        render.display_list,
        vec![
            DisplayCommand::Text {
                x: 0,
                y: 0,
                text: "Alpha Beta".to_owned(),
            },
            DisplayCommand::Text {
                x: 0,
                y: 1,
                text: "Gamma Delta Epsilon".to_owned(),
            },
            DisplayCommand::Text {
                x: 0,
                y: 2,
                text: "Zeta".to_owned(),
            },
        ]
    );
}

#[test]
fn css_white_space_pre_wrap_preserves_spaces_and_soft_wraps() {
    let render = render_html(
        "mem://pre-wrap",
        br#"
            <html><body>
              <p style="white-space: pre-wrap">A  B
ABCDEFGHIJKLMNOPQRSTUV</p>
            </body></html>
            "#,
        BrowserRenderOptions {
            width: 20,
            ..BrowserRenderOptions::default()
        },
    );

    assert_eq!(render.text, "A  B\nABCDEFGHIJKLMNOPQRST\nUV");
    assert_eq!(
        render.display_list,
        vec![
            DisplayCommand::Text {
                x: 0,
                y: 0,
                text: "A  B".to_owned(),
            },
            DisplayCommand::Text {
                x: 0,
                y: 1,
                text: "ABCDEFGHIJKLMNOPQRST".to_owned(),
            },
            DisplayCommand::Text {
                x: 0,
                y: 2,
                text: "UV".to_owned(),
            },
        ]
    );
}

#[test]
fn css_white_space_break_spaces_preserves_spaces_and_soft_wraps() {
    let render = render_html(
        "mem://break-spaces",
        br#"
            <html><body>
              <p style="white-space: break-spaces">A    B
C</p>
            </body></html>
            "#,
        BrowserRenderOptions {
            width: 3,
            ..BrowserRenderOptions::default()
        },
    );

    assert_eq!(render.text, "A  \n  B\nC");
    assert_eq!(
        render.display_list,
        vec![
            DisplayCommand::Text {
                x: 0,
                y: 0,
                text: "A  ".to_owned(),
            },
            DisplayCommand::Text {
                x: 0,
                y: 1,
                text: "  B".to_owned(),
            },
            DisplayCommand::Text {
                x: 0,
                y: 2,
                text: "C".to_owned(),
            },
        ]
    );
}

#[test]
fn wbr_creates_zero_width_soft_break_opportunity() {
    let render = render_html(
        "mem://wbr",
        br#"
            <html><body>
              <p>Alpha<wbr>Beta Gamma</p>
              <p>Fit<wbr>Here</p>
            </body></html>
            "#,
        BrowserRenderOptions {
            width: 9,
            ..BrowserRenderOptions::default()
        },
    );

    assert_eq!(render.text, "AlphaBeta\nGamma\nFitHere");
    assert_eq!(
        render.display_list,
        vec![
            DisplayCommand::Text {
                x: 0,
                y: 0,
                text: "AlphaBeta".to_owned(),
            },
            DisplayCommand::Text {
                x: 0,
                y: 1,
                text: "Gamma".to_owned(),
            },
            DisplayCommand::Text {
                x: 0,
                y: 2,
                text: "FitHere".to_owned(),
            },
        ]
    );
}

#[test]
fn css_line_height_adds_visual_row_spacing() {
    let render = render_html(
        "mem://line-height",
        br#"
            <html><body>
              <p style="line-height: 24px">Tall line</p>
              <p>Normal line</p>
            </body></html>
            "#,
        BrowserRenderOptions {
            width: 80,
            ..BrowserRenderOptions::default()
        },
    );

    assert_eq!(render.text, "Tall line\n\nNormal line");
    assert_eq!(
        render.display_list,
        vec![
            DisplayCommand::Text {
                x: 0,
                y: 0,
                text: "Tall line".to_owned(),
            },
            DisplayCommand::Text {
                x: 0,
                y: 2,
                text: "Normal line".to_owned(),
            },
        ]
    );
}

#[test]
fn css_font_size_scales_hero_text_layout_and_viewport_paint() {
    let render = render_html(
        "mem://font-size-scale",
        br#"
            <html>
              <head>
                <style>
                  .hero {
                    font-size: clamp(32px, 6vw, 72px);
                    line-height: 48px;
                  }
                </style>
              </head>
              <body>
                <div class="hero">Hero</div>
                <div>Body</div>
              </body>
            </html>
            "#,
        BrowserRenderOptions {
            width: 24,
            ..BrowserRenderOptions::default()
        },
    );

    assert_eq!(render.text, "HHHeeerrrooo\n\n\n\nBody");
    assert_eq!(
        render.display_list,
        vec![
            DisplayCommand::Text {
                x: 0,
                y: 0,
                text: "HHHeeerrrooo".to_owned(),
            },
            DisplayCommand::Text {
                x: 0,
                y: 1,
                text: "HHHeeerrrooo".to_owned(),
            },
            DisplayCommand::Text {
                x: 0,
                y: 2,
                text: "HHHeeerrrooo".to_owned(),
            },
            DisplayCommand::Text {
                x: 0,
                y: 4,
                text: "Body".to_owned(),
            },
        ]
    );
    assert_eq!(
        display_command_bounds(&render.display_list[0]),
        DisplayCommandBounds {
            x: 0,
            y: 0,
            width: 12,
            height: 1,
        }
    );

    let viewport = browser_text_viewport(
        &render,
        BrowserTextViewportOptions {
            width: 16,
            height: 5,
            ..BrowserTextViewportOptions::default()
        },
    );
    assert_eq!(
        viewport.lines,
        vec![
            "Hero".to_owned(),
            String::new(),
            String::new(),
            String::new(),
            "Body".to_owned(),
        ]
    );
    let raster_options = BrowserRasterOptions {
        viewport_width: Some(16),
        viewport_height: Some(5),
        ..BrowserRasterOptions::default()
    };
    let raster = rasterize_render(&render, raster_options).expect("rasterize scaled text");
    let first_row_glyph = raster_options
        .padding_y
        .saturating_add(2)
        .saturating_mul(raster.width)
        .saturating_add(raster_options.padding_x.saturating_add(1));
    assert_eq!(raster.pixels[first_row_glyph], 0);
    let duplicate_row_pixel = raster_options
        .padding_y
        .saturating_add(raster_options.cell_height)
        .saturating_add(2)
        .saturating_mul(raster.width)
        .saturating_add(raster_options.padding_x.saturating_add(1));
    assert_eq!(raster.pixels[duplicate_row_pixel], 255);
    assert_eq!(
        collapse_repeated_glyph_runs("TTrruuvveettaa  DDaattaa").as_deref(),
        Some("Truveta Data")
    );
    assert_eq!(collapse_repeated_glyph_runs("bookkeeper"), None);
}

#[test]
fn raster_glyphs_preserve_lowercase_and_common_punctuation() {
    let unknown = glyph_rows('\u{2603}');
    assert_ne!(glyph_rows('a'), glyph_rows('A'));
    assert_ne!(glyph_rows('q'), glyph_rows('Q'));
    for ch in ['@', '#', '%', '*', '|', '<', '>'] {
        assert_ne!(glyph_rows(ch), unknown, "{ch} should not use unknown glyph");
    }
    let cell_width = BrowserRasterOptions::default().cell_width;
    assert!(raster_glyph_advance('i', cell_width) < raster_glyph_advance('m', cell_width));
    assert!(raster_glyph_advance(' ', cell_width) < cell_width);

    let render = BrowserRender {
        source: "mem://raster-typography".to_owned(),
        title: String::new(),
        viewport_width: 12,
        dom_node_count: 0,
        css_rule_count: 0,
        layout_box_count: 0,
        layout_boxes: Vec::new(),
        paint_command_count: 1,
        links: Vec::new(),
        forms: Vec::new(),
        resources: Vec::new(),
        fragment_targets: Vec::new(),
        decoded_images: Vec::new(),
        hit_targets: vec![DisplayHitTarget::default()],
        display_list: vec![DisplayCommand::Text {
            x: 0,
            y: 0,
            text: "aA@#%".to_owned(),
        }],
        text: "aA@#%".to_owned(),
    };

    let raster_options = BrowserRasterOptions::default();
    let raster = rasterize_render(&render, raster_options).expect("rasterize typography glyphs");
    let lowercase_top_pixel = raster_options
        .padding_y
        .saturating_add(2)
        .saturating_mul(raster.width)
        .saturating_add(raster_options.padding_x.saturating_add(2));
    let uppercase_top_pixel = raster_options
        .padding_y
        .saturating_add(2)
        .saturating_mul(raster.width)
        .saturating_add(
            raster_options
                .padding_x
                .saturating_add(raster_options.cell_width)
                .saturating_add(2),
        );
    assert_eq!(raster.pixels[lowercase_top_pixel], 255);
    assert_eq!(raster.pixels[uppercase_top_pixel], 0);
    assert_eq!(
        display_command_bounds(&render.display_list[0]),
        DisplayCommandBounds {
            x: 0,
            y: 0,
            width: 5,
            height: 1,
        }
    );

    let density_render = BrowserRender {
        source: "mem://raster-density".to_owned(),
        title: String::new(),
        viewport_width: 8,
        dom_node_count: 0,
        css_rule_count: 0,
        layout_box_count: 0,
        layout_boxes: Vec::new(),
        paint_command_count: 1,
        links: Vec::new(),
        forms: Vec::new(),
        resources: Vec::new(),
        fragment_targets: Vec::new(),
        decoded_images: Vec::new(),
        hit_targets: vec![DisplayHitTarget::default()],
        display_list: vec![DisplayCommand::Text {
            x: 0,
            y: 0,
            text: "im".to_owned(),
        }],
        text: "im".to_owned(),
    };
    let density_raster =
        rasterize_render(&density_render, raster_options).expect("rasterize density glyphs");
    let early_m_pixel = raster_options
        .padding_y
        .saturating_add(4)
        .saturating_mul(density_raster.width)
        .saturating_add(raster_options.padding_x.saturating_add(6));
    let monospace_m_left = raster_options
        .padding_x
        .saturating_add(raster_options.cell_width)
        .saturating_add(1);
    assert!(raster_options.padding_x.saturating_add(6) < monospace_m_left);
    assert_eq!(density_raster.pixels[early_m_pixel], 0);
    assert_eq!(
        display_command_bounds(&density_render.display_list[0]),
        DisplayCommandBounds {
            x: 0,
            y: 0,
            width: 2,
            height: 1,
        }
    );
}

#[test]
fn css_text_indent_offsets_first_line_and_affects_wrap_width() {
    let render = render_html(
        "mem://text-indent",
        br#"
            <html><body>
              <p style="text-indent: 16px">Alpha Beta Gamma Delta</p>
              <p>Plain text</p>
            </body></html>
            "#,
        BrowserRenderOptions {
            width: 20,
            ..BrowserRenderOptions::default()
        },
    );

    assert_eq!(render.text, "  Alpha Beta Gamma\nDelta\nPlain text");
    assert_eq!(
        render.display_list,
        vec![
            DisplayCommand::Text {
                x: 0,
                y: 0,
                text: "  Alpha Beta Gamma".to_owned(),
            },
            DisplayCommand::Text {
                x: 0,
                y: 1,
                text: "Delta".to_owned(),
            },
            DisplayCommand::Text {
                x: 0,
                y: 2,
                text: "Plain text".to_owned(),
            },
        ]
    );
}

#[test]
fn css_relative_length_units_affect_layout_text_metrics() {
    let render = render_html(
        "mem://relative-length-units",
        br#"
            <html><body>
              <div style="width:4rem; padding-left:1em; margin-left:1rem">Alpha Beta</div>
              <p style="text-indent:2em">Indented words</p>
              <p style="word-spacing:1ch">Wide gap</p>
              <p style="letter-spacing:0.5rem">AB</p>
              <p style="line-height:1rem">Tall</p>
              <p>After</p>
            </body></html>
            "#,
        BrowserRenderOptions {
            width: 24,
            ..BrowserRenderOptions::default()
        },
    );

    assert_eq!(
        render.text,
        "Alpha\nBeta\n    Indented words\nWide  gap\nA B\nTall\n\nAfter"
    );
    assert_eq!(
        render.display_list,
        vec![
            DisplayCommand::Text {
                x: 4,
                y: 0,
                text: "Alpha".to_owned(),
            },
            DisplayCommand::Text {
                x: 4,
                y: 1,
                text: "Beta".to_owned(),
            },
            DisplayCommand::Text {
                x: 0,
                y: 2,
                text: "    Indented words".to_owned(),
            },
            DisplayCommand::Text {
                x: 0,
                y: 3,
                text: "Wide  gap".to_owned(),
            },
            DisplayCommand::Text {
                x: 0,
                y: 4,
                text: "A B".to_owned(),
            },
            DisplayCommand::Text {
                x: 0,
                y: 5,
                text: "Tall".to_owned(),
            },
            DisplayCommand::Text {
                x: 0,
                y: 7,
                text: "After".to_owned(),
            },
        ]
    );
}

#[test]
fn css_root_custom_properties_resolve_supported_layout_values() {
    let render = render_html(
        "mem://css-custom-properties",
        br#"
            <html><head><style>
              :root {
                --ink: blue;
                --panel: silver;
                --inset: 16px;
              }
              .card {
                color: var(--ink);
                background-color: var(--panel);
                padding-left: var(--inset);
              }
            </style></head><body>
              <div class="card">Tone</div>
              <p style="color: var(--ink); text-indent: var(--inset)">Inline tone</p>
            </body></html>
            "#,
        BrowserRenderOptions {
            width: 20,
            ..BrowserRenderOptions::default()
        },
    );

    let blue_shade = rgb_to_luma(0, 0, 255);
    assert_eq!(render.text, "Tone\n  Inline tone");
    assert_eq!(
        render.display_list,
        vec![
            DisplayCommand::Rect {
                x: 0,
                y: 0,
                width: 20,
                height: 1,
                shade: 192,
            },
            DisplayCommand::StyledText {
                x: 2,
                y: 0,
                text: "Tone".to_owned(),
                shade: blue_shade,
            },
            DisplayCommand::StyledText {
                x: 0,
                y: 1,
                text: "  Inline tone".to_owned(),
                shade: blue_shade,
            },
        ]
    );
}

#[test]
fn css_overflow_wrap_break_word_breaks_long_words() {
    let render = render_html(
        "mem://overflow-wrap",
        br#"
            <html><body>
              <p style="overflow-wrap: break-word">ABCDEFGHIJK</p>
              <p>ABCDEFGHIJK</p>
            </body></html>
            "#,
        BrowserRenderOptions {
            width: 8,
            ..BrowserRenderOptions::default()
        },
    );

    assert_eq!(render.text, "ABCDEFGH\nIJK\nABCDEFGHIJK");
    assert_eq!(
        render.display_list,
        vec![
            DisplayCommand::Text {
                x: 0,
                y: 0,
                text: "ABCDEFGH".to_owned(),
            },
            DisplayCommand::Text {
                x: 0,
                y: 1,
                text: "IJK".to_owned(),
            },
            DisplayCommand::Text {
                x: 0,
                y: 2,
                text: "ABCDEFGHIJK".to_owned(),
            },
        ]
    );
}

#[test]
fn css_word_break_break_all_breaks_long_words() {
    let render = render_html(
        "mem://word-break",
        br#"
            <html><body>
              <p style="word-break: break-all">One AlphaBetaGamma</p>
              <p style="word-break: normal">AlphaBetaGamma</p>
            </body></html>
            "#,
        BrowserRenderOptions {
            width: 10,
            ..BrowserRenderOptions::default()
        },
    );

    assert_eq!(render.text, "One AlphaB\netaGamma\nAlphaBetaGamma");
    assert_eq!(
        render.display_list,
        vec![
            DisplayCommand::Text {
                x: 0,
                y: 0,
                text: "One AlphaB".to_owned(),
            },
            DisplayCommand::Text {
                x: 0,
                y: 1,
                text: "etaGamma".to_owned(),
            },
            DisplayCommand::Text {
                x: 0,
                y: 2,
                text: "AlphaBetaGamma".to_owned(),
            },
        ]
    );
}

#[test]
fn css_text_transform_changes_rendered_text() {
    let render = render_html(
        "mem://text-transform",
        br#"
            <html><head><style>
              .shout { text-transform: uppercase }
              .quiet { text-transform: lowercase }
              .plain { text-transform: none }
            </style></head><body>
              <p class="shout">Rust browser <span class="quiet">ENGINE</span> <span class="plain">MiXeD</span></p>
              <p style="text-transform: lowercase">LOUD Text</p>
            </body></html>
            "#,
        BrowserRenderOptions {
            width: 80,
            ..BrowserRenderOptions::default()
        },
    );

    assert_eq!(render.text, "RUST BROWSER engine MiXeD\nloud text");
    assert_eq!(
        render.display_list,
        vec![
            DisplayCommand::Text {
                x: 0,
                y: 0,
                text: "RUST BROWSER engine MiXeD".to_owned(),
            },
            DisplayCommand::Text {
                x: 0,
                y: 1,
                text: "loud text".to_owned(),
            },
        ]
    );
}

#[test]
fn css_text_transform_capitalize_uses_word_boundaries() {
    let render = render_html(
        "mem://text-transform-capitalize",
        br#"
            <html><head><style>
              .title { text-transform: capitalize }
              .plain { text-transform: none }
            </style></head><body>
              <p class="title">rust-browser layout ENGINE <span class="plain">raw mix</span> again</p>
            </body></html>
            "#,
        BrowserRenderOptions {
            width: 80,
            ..BrowserRenderOptions::default()
        },
    );

    assert_eq!(render.text, "Rust-Browser Layout ENGINE raw mix Again");
    assert_eq!(
        render.display_list,
        vec![DisplayCommand::Text {
            x: 0,
            y: 0,
            text: "Rust-Browser Layout ENGINE raw mix Again".to_owned(),
        }]
    );
}

#[test]
fn css_letter_spacing_expands_text_runs_and_wrap_width() {
    let render = render_html(
        "mem://letter-spacing",
        br#"
            <html><body>
              <p style="letter-spacing: 8px">AB CD EF GH IJ KL</p>
              <p style="letter-spacing: 8px">Wide <span style="letter-spacing: normal">gap ok</span> end more</p>
            </body></html>
            "#,
        BrowserRenderOptions {
            width: 20,
            ..BrowserRenderOptions::default()
        },
    );

    assert_eq!(
        render.text,
        "A B C D E F G H I J\nK L\nW i d e gap ok e n d\nm o r e"
    );
    assert_eq!(
        render.display_list,
        vec![
            DisplayCommand::Text {
                x: 0,
                y: 0,
                text: "A B C D E F G H I J".to_owned(),
            },
            DisplayCommand::Text {
                x: 0,
                y: 1,
                text: "K L".to_owned(),
            },
            DisplayCommand::Text {
                x: 0,
                y: 2,
                text: "W i d e gap ok e n d".to_owned(),
            },
            DisplayCommand::Text {
                x: 0,
                y: 3,
                text: "m o r e".to_owned(),
            },
        ]
    );
}

#[test]
fn css_subcell_letter_spacing_accumulates_without_exploding_text_gaps() {
    let render = render_html(
        "mem://fractional-letter-spacing",
        br#"
            <html><body>
              <p style="letter-spacing: 1px">ABCDEFGHIJKLMNOP</p>
            </body></html>
            "#,
        BrowserRenderOptions {
            width: 80,
            ..BrowserRenderOptions::default()
        },
    );

    assert_eq!(render.text, "ABCDEFGH IJKLMNOP");
    assert_eq!(
        render.display_list,
        vec![DisplayCommand::Text {
            x: 0,
            y: 0,
            text: "ABCDEFGH IJKLMNOP".to_owned(),
        }]
    );
}

#[test]
fn css_letter_spacing_applies_inside_pre_wrap_segments() {
    let render = render_html(
        "mem://letter-spacing-pre-wrap",
        br#"
            <html><body>
              <p style="white-space: pre-wrap; letter-spacing: 8px">ABCDEFGHIJK</p>
            </body></html>
            "#,
        BrowserRenderOptions {
            width: 20,
            ..BrowserRenderOptions::default()
        },
    );

    assert_eq!(render.text, "A B C D E F G H I J\nK");
    assert_eq!(
        render.display_list,
        vec![
            DisplayCommand::Text {
                x: 0,
                y: 0,
                text: "A B C D E F G H I J".to_owned(),
            },
            DisplayCommand::Text {
                x: 0,
                y: 1,
                text: "K".to_owned(),
            },
        ]
    );
}

#[test]
fn css_word_spacing_expands_inter_word_gaps_and_wrap_width() {
    let render = render_html(
        "mem://word-spacing",
        br#"
            <html><body>
              <p style="word-spacing: 16px">Alpha Beta Gamma Delta</p>
              <p style="word-spacing: 16px">Wide <span style="word-spacing: normal">normal gap</span> done</p>
            </body></html>
            "#,
        BrowserRenderOptions {
            width: 20,
            ..BrowserRenderOptions::default()
        },
    );

    assert_eq!(
        render.text,
        "Alpha   Beta   Gamma\nDelta\nWide   normal gap\ndone"
    );
    assert_eq!(
        render.display_list,
        vec![
            DisplayCommand::Text {
                x: 0,
                y: 0,
                text: "Alpha   Beta   Gamma".to_owned(),
            },
            DisplayCommand::Text {
                x: 0,
                y: 1,
                text: "Delta".to_owned(),
            },
            DisplayCommand::Text {
                x: 0,
                y: 2,
                text: "Wide   normal gap".to_owned(),
            },
            DisplayCommand::Text {
                x: 0,
                y: 3,
                text: "done".to_owned(),
            },
        ]
    );
}

#[test]
fn flows_simple_table_cells_across_rows() {
    let render = render_html(
        "mem://table",
        br#"
            <html><body>
              <table>
                <thead>
                  <tr><th>Name</th><th>Status</th></tr>
                </thead>
                <tbody>
                  <tr><td>Parser</td><td>running</td></tr>
                  <tr><td>Layout</td><td>next</td></tr>
                </tbody>
              </table>
            </body></html>
            "#,
        BrowserRenderOptions {
            width: 80,
            ..BrowserRenderOptions::default()
        },
    );

    assert_eq!(render.text, "Name    Status\nParser  running\nLayout  next");
    assert_eq!(
        render.display_list,
        vec![
            DisplayCommand::Text {
                x: 0,
                y: 0,
                text: "Name    Status".to_owned(),
            },
            DisplayCommand::Text {
                x: 0,
                y: 1,
                text: "Parser  running".to_owned(),
            },
            DisplayCommand::Text {
                x: 0,
                y: 2,
                text: "Layout  next".to_owned(),
            },
        ]
    );
    assert!(hit_test_target_node(&render, 0, 1).is_some());
    assert_eq!(hit_test_target_node(&render, 6, 1), None);
    assert!(hit_test_target_node(&render, 8, 1).is_some());
}

#[test]
fn css_table_display_values_use_table_flow() {
    let render = render_html(
        "mem://css-table-display",
        br#"
            <html><head>
              <style>
                .table { display: table }
                .row { display: table-row }
                .cell { display: table-cell }
              </style>
            </head><body>
              <div class="table">
                <div class="row"><span class="cell">Name</span><span class="cell">Status</span></div>
                <div class="row"><span class="cell">Parser</span><span class="cell">running</span></div>
              </div>
              <p>After</p>
            </body></html>
            "#,
        BrowserRenderOptions {
            width: 80,
            ..BrowserRenderOptions::default()
        },
    );

    assert_eq!(render.text, "Name    Status\nParser  running\nAfter");
    assert_eq!(
        render
            .layout_boxes
            .iter()
            .filter(|layout_box| layout_box.kind == "table-cell")
            .map(|layout_box| (layout_box.tag.as_str(), layout_box.kind.as_str()))
            .collect::<Vec<_>>(),
        vec![
            ("span", "table-cell"),
            ("span", "table-cell"),
            ("span", "table-cell"),
            ("span", "table-cell"),
        ]
    );
    assert!(hit_test_target_node(&render, 0, 1).is_some());
    assert_eq!(hit_test_target_node(&render, 6, 1), None);
    assert!(hit_test_target_node(&render, 8, 1).is_some());
}

#[test]
fn table_colspan_spans_multiple_columns() {
    let render = render_html(
        "mem://table-colspan",
        br#"
            <html><body>
              <table>
                <tr><th colspan="2">Product</th><th>Price</th></tr>
                <tr><td>Browser</td><td>Engine</td><td>Fast</td></tr>
                <tr><td colspan="2">Total cost</td><td>Zero</td></tr>
              </table>
            </body></html>
            "#,
        BrowserRenderOptions {
            width: 80,
            ..BrowserRenderOptions::default()
        },
    );

    assert_eq!(
        render.text,
        "Product          Price\nBrowser  Engine  Fast\nTotal cost       Zero"
    );
    assert_eq!(
        render.display_list,
        vec![
            DisplayCommand::Text {
                x: 0,
                y: 0,
                text: "Product          Price".to_owned(),
            },
            DisplayCommand::Text {
                x: 0,
                y: 1,
                text: "Browser  Engine  Fast".to_owned(),
            },
            DisplayCommand::Text {
                x: 0,
                y: 2,
                text: "Total cost       Zero".to_owned(),
            },
        ]
    );
    assert!(hit_test_target_node(&render, 1, 0).is_some());
    assert_eq!(hit_test_target_node(&render, 9, 0), None);
    assert!(hit_test_target_node(&render, 17, 0).is_some());
}

#[test]
fn table_rowspan_offsets_following_rows() {
    let render = render_html(
        "mem://table-rowspan",
        br#"
            <html><body>
              <table>
                <tr><th rowspan="2">Area</th><th>Metric</th></tr>
                <tr><td>Layout</td></tr>
                <tr><td>Speed</td><td>Fast</td></tr>
              </table>
            </body></html>
            "#,
        BrowserRenderOptions {
            width: 80,
            ..BrowserRenderOptions::default()
        },
    );

    assert_eq!(render.text, "Area   Metric\n       Layout\nSpeed  Fast");
    assert_eq!(
        render.display_list,
        vec![
            DisplayCommand::Text {
                x: 0,
                y: 0,
                text: "Area   Metric".to_owned(),
            },
            DisplayCommand::Text {
                x: 0,
                y: 1,
                text: "       Layout".to_owned(),
            },
            DisplayCommand::Text {
                x: 0,
                y: 2,
                text: "Speed  Fast".to_owned(),
            },
        ]
    );
    assert!(hit_test_target_node(&render, 1, 0).is_some());
    assert_eq!(hit_test_target_node(&render, 1, 1), None);
    assert!(hit_test_target_node(&render, 8, 1).is_some());
}

#[test]
fn table_cell_widths_set_column_minimums() {
    let render = render_html(
        "mem://table-cell-widths",
        br#"
            <html><body>
              <table>
                <tr><th style="width:80px">Name</th><th width="64">Status</th><th>Phase</th></tr>
                <tr><td>A</td><td>Ok</td><td>Go</td></tr>
              </table>
            </body></html>
            "#,
        BrowserRenderOptions {
            width: 80,
            ..BrowserRenderOptions::default()
        },
    );

    assert_eq!(
        render.text,
        "Name        Status   Phase\nA           Ok       Go"
    );
    assert_eq!(
        render.display_list,
        vec![
            DisplayCommand::Text {
                x: 0,
                y: 0,
                text: "Name        Status   Phase".to_owned(),
            },
            DisplayCommand::Text {
                x: 0,
                y: 1,
                text: "A           Ok       Go".to_owned(),
            },
        ]
    );
    assert!(hit_test_target_node(&render, 0, 1).is_some());
    assert_eq!(hit_test_target_node(&render, 6, 1), None);
    assert!(hit_test_target_node(&render, 12, 1).is_some());
    assert!(hit_test_target_node(&render, 21, 1).is_some());
}

#[test]
fn table_colgroup_widths_set_column_minimums() {
    let render = render_html(
        "mem://table-colgroup-widths",
        br#"
            <html><body>
              <table>
                <colgroup>
                  <col span="2" style="width:64px">
                  <col width="80">
                  <col>
                </colgroup>
                <tr><th>A</th><th>B</th><th>C</th><th>D</th></tr>
                <tr><td>One</td><td>Two</td><td>Three</td><td>Four</td></tr>
              </table>
            </body></html>
            "#,
        BrowserRenderOptions {
            width: 80,
            ..BrowserRenderOptions::default()
        },
    );

    assert_eq!(
        render.text,
        "A         B        C          D\nOne       Two      Three      Four"
    );
    assert_eq!(
        render.display_list,
        vec![
            DisplayCommand::Text {
                x: 0,
                y: 0,
                text: "A         B        C          D".to_owned(),
            },
            DisplayCommand::Text {
                x: 0,
                y: 1,
                text: "One       Two      Three      Four".to_owned(),
            },
        ]
    );
    assert!(hit_test_target_node(&render, 0, 1).is_some());
    assert_eq!(hit_test_target_node(&render, 5, 1), None);
    assert!(hit_test_target_node(&render, 10, 1).is_some());
    assert!(hit_test_target_node(&render, 19, 1).is_some());
    assert!(hit_test_target_node(&render, 30, 1).is_some());
}

#[test]
fn table_row_collection_ignores_nested_tables() {
    let parsed = parse_html(
        br#"
            <html><body>
              <table id="outer">
                <tr>
                  <td>
                    Outer
                    <table id="inner">
                      <tr><td>Nested</td><td>Row</td></tr>
                    </table>
                  </td>
                  <td>Side</td>
                </tr>
                <tr><td>Bottom</td><td>Cell</td></tr>
              </table>
            </body></html>
            "#,
    );
    let css_cascade = parse_css("");
    let outer_table = find_first_matching_selector(&parsed.dom, "table#outer").unwrap();
    let rows = table_rows(&parsed.dom, outer_table, &css_cascade);

    assert_eq!(rows.len(), 2);
    assert_eq!(
        rows.iter()
            .map(|&row| table_row_cell_count(&parsed.dom, row, &css_cascade))
            .collect::<Vec<_>>(),
        vec![2, 2]
    );
}

#[test]
fn indents_blockquotes_and_definition_descriptions_by_default() {
    let render = render_html(
        "mem://prose-indent",
        br#"
            <html><body>
              <blockquote><p>Quoted document text</p></blockquote>
              <dl>
                <dt>Engine</dt>
                <dd>Rust layout pipeline</dd>
              </dl>
            </body></html>
            "#,
        BrowserRenderOptions {
            width: 80,
            ..BrowserRenderOptions::default()
        },
    );

    assert_eq!(
        render.text,
        "Quoted document text\nEngine\nRust layout pipeline"
    );
    assert_eq!(
        render.display_list,
        vec![
            DisplayCommand::Text {
                x: 4,
                y: 0,
                text: "Quoted document text".to_owned(),
            },
            DisplayCommand::Text {
                x: 0,
                y: 1,
                text: "Engine".to_owned(),
            },
            DisplayCommand::Text {
                x: 4,
                y: 2,
                text: "Rust layout pipeline".to_owned(),
            },
        ]
    );
}

#[test]
fn links_have_default_text_shade_with_css_override() {
    let render = render_html(
        "mem://default-link-text-shade",
        br#"
            <html><head><style>
              a.override { color: red }
            </style></head><body>
              <p><a href="/profile">Profile <span>details</span></a></p>
              <p><a name="anchor">Anchor only</a></p>
              <p><a class="override" href="/alert">Alert</a></p>
            </body></html>
            "#,
        BrowserRenderOptions {
            width: 80,
            ..BrowserRenderOptions::default()
        },
    );

    assert_eq!(render.text, "Profile details\nAnchor only\nAlert");
    assert_eq!(
        render.display_list,
        vec![
            DisplayCommand::StyledText {
                x: 0,
                y: 0,
                text: "Profile details".to_owned(),
                shade: 28,
            },
            DisplayCommand::Text {
                x: 0,
                y: 1,
                text: "Anchor only".to_owned(),
            },
            DisplayCommand::StyledText {
                x: 0,
                y: 2,
                text: "Alert".to_owned(),
                shade: 76,
            },
        ]
    );
}

#[test]
fn search_and_hgroup_use_block_flow_by_default() {
    let render = render_html(
        "mem://semantic-block-defaults",
        br#"
            <html><body>
              <span>Before</span>
              <search><label>Find</label> <input value="rust"></search>
              <hgroup><h1>Title</h1><p>Subtitle</p></hgroup>
              <span>After</span>
            </body></html>
            "#,
        BrowserRenderOptions {
            width: 80,
            ..BrowserRenderOptions::default()
        },
    );

    assert_eq!(render.text, "Before\nFind [rust]\nTitle\nSubtitle\nAfter");
    assert_eq!(
        render.display_list,
        vec![
            DisplayCommand::Text {
                x: 0,
                y: 0,
                text: "Before".to_owned(),
            },
            DisplayCommand::Text {
                x: 0,
                y: 1,
                text: "Find [rust]".to_owned(),
            },
            DisplayCommand::Text {
                x: 0,
                y: 3,
                text: "Title".to_owned(),
            },
            DisplayCommand::Text {
                x: 0,
                y: 5,
                text: "Subtitle".to_owned(),
            },
            DisplayCommand::Text {
                x: 0,
                y: 6,
                text: "After".to_owned(),
            },
        ]
    );
}

#[test]
fn headings_have_default_vertical_margins() {
    let render = render_html(
        "mem://heading-default-margins",
        br#"
            <html><body>
              <p>Intro</p>
              <h1>Profile</h1>
              <p>Summary</p>
              <h3>Research</h3>
              <p>After</p>
            </body></html>
            "#,
        BrowserRenderOptions {
            width: 80,
            ..BrowserRenderOptions::default()
        },
    );

    assert_eq!(render.text, "Intro\nProfile\nSummary\nResearch\nAfter");
    assert_eq!(
        render.display_list,
        vec![
            DisplayCommand::Text {
                x: 0,
                y: 0,
                text: "Intro".to_owned(),
            },
            DisplayCommand::Text {
                x: 0,
                y: 2,
                text: "Profile".to_owned(),
            },
            DisplayCommand::Text {
                x: 0,
                y: 4,
                text: "Summary".to_owned(),
            },
            DisplayCommand::Text {
                x: 0,
                y: 6,
                text: "Research".to_owned(),
            },
            DisplayCommand::Text {
                x: 0,
                y: 8,
                text: "After".to_owned(),
            },
        ]
    );
}

#[test]
fn css_visibility_hidden_reserves_layout_without_painting() {
    let render = render_html(
        "mem://visibility-hidden",
        br#"
            <html><body>
              <p>Before <span style="visibility:hidden">Hidden</span> After</p>
              <p style="visibility:hidden">Ghost <span style="visibility:visible">Shown</span> Gone</p>
              <hr style="visibility:hidden">
              <p>Tail</p>
            </body></html>
            "#,
        BrowserRenderOptions {
            width: 80,
            ..BrowserRenderOptions::default()
        },
    );

    assert_eq!(
        render.text,
        "Before        After
      Shown     
Tail"
    );
    assert_eq!(
        render.display_list,
        vec![
            DisplayCommand::Text {
                x: 0,
                y: 0,
                text: "Before".to_owned(),
            },
            DisplayCommand::Text {
                x: 13,
                y: 0,
                text: " After".to_owned(),
            },
            DisplayCommand::Text {
                x: 5,
                y: 1,
                text: " Shown".to_owned(),
            },
            DisplayCommand::Text {
                x: 0,
                y: 3,
                text: "Tail".to_owned(),
            },
        ]
    );
}

#[test]
fn css_zero_opacity_suppresses_paint_without_collapsing_layout() {
    let render = render_html(
        "mem://opacity-zero-paint",
        br#"
            <html>
              <head>
                <style>
                  #mega-menu-wrap #mega-menu > li.mega-menu-item {
                    display: inline-block;
                  }
                  #mega-menu-wrap #mega-menu li.mega-menu-item > ul.mega-sub-menu {
                    display: block;
                    opacity: 0;
                    position: absolute;
                  }
                </style>
              </head>
              <body>
                <p>Before <span style="opacity:0">Hidden</span> After</p>
                <nav id="mega-menu-wrap">
                  <ul id="mega-menu">
                    <li class="mega-menu-item">
                      <a>Data</a>
                      <ul class="mega-sub-menu">
                        <li>Truveta Data</li>
                        <li style="opacity:1">Capabilities</li>
                      </ul>
                    </li>
                  </ul>
                </nav>
                <p>Hero copy</p>
              </body>
            </html>
            "#,
        BrowserRenderOptions {
            width: 40,
            ..BrowserRenderOptions::default()
        },
    );

    assert_eq!(render.text, "Before        After\nData\nHero copy");
    assert!(
        render
            .display_list
            .iter()
            .filter_map(|command| match command {
                DisplayCommand::Text { text, .. } | DisplayCommand::StyledText { text, .. } => {
                    Some(text.as_str())
                }
                _ => None,
            })
            .all(|text| !text.contains("Hidden")
                && !text.contains("Truveta Data")
                && !text.contains("Capabilities"))
    );
}

#[test]
fn css_animation_fill_and_negative_z_media_keep_hero_text_visible() {
    let render = render_html(
        "mem://truveta-hero-paint",
        br#"
            <html>
              <head>
                <style>
                  .hero { position: relative; height: 48px; }
                  .hero-copy { position: absolute; top: 0; }
                  .hero-media { position: absolute; top: 0; z-index: -1; }
                  .animated-fill {
                    opacity: 0;
                    -webkit-animation-fill-mode: both !important;
                    animation-fill-mode: both !important;
                    animation-name: et_pb_fade;
                  }
                  .animated-shorthand {
                    opacity: 0;
                    animation: et_pb_fade 1s forwards;
                  }
                  .hidden-menu { opacity: 0; }
                </style>
              </head>
              <body>
                <section class="hero">
                  <div class="hero-copy">Saving Lives with Data</div>
                  <img class="hero-media" alt="" width="320" height="48">
                </section>
                <div class="animated-fill">Truveta Data</div>
                <div class="animated-shorthand">Truveta Intelligence</div>
                <div class="hidden-menu">Hidden Mega Menu</div>
              </body>
            </html>
            "#,
        BrowserRenderOptions {
            width: 40,
            ..BrowserRenderOptions::default()
        },
    );

    let paint_order = render
        .display_list
        .iter()
        .filter_map(|command| match command {
            DisplayCommand::Image { .. } => Some("image".to_owned()),
            DisplayCommand::Text { text, .. } | DisplayCommand::StyledText { text, .. } => {
                Some(text.clone())
            }
            DisplayCommand::Rect { .. } | DisplayCommand::BackgroundImage { .. } => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        paint_order,
        vec![
            "image".to_owned(),
            "Saving Lives with Data".to_owned(),
            "Truveta Data".to_owned(),
            "Truveta Intelligence".to_owned(),
        ]
    );
    assert!(
        !render
            .display_list
            .iter()
            .filter_map(|command| match command {
                DisplayCommand::Text { text, .. } | DisplayCommand::StyledText { text, .. } => {
                    Some(text.as_str())
                }
                _ => None,
            })
            .any(|text| text.contains("Hidden Mega Menu"))
    );

    let viewport = browser_text_viewport(
        &render,
        BrowserTextViewportOptions {
            width: 40,
            height: 6,
            ..BrowserTextViewportOptions::default()
        },
    );
    assert!(viewport.lines[0].starts_with("Saving Lives with Data"));
    assert!(viewport.lines.join("\n").contains("Truveta Data"));
    assert!(viewport.lines.join("\n").contains("Truveta Intelligence"));
}

#[test]
fn text_viewport_and_raster_keep_text_foreground_over_later_media() {
    let render = render_html(
        "mem://truveta-readable-foreground",
        br#"
            <html>
              <head>
                <style>
                  .hero { position: relative; height: 6px; }
                  .copy { position: absolute; top: 0; left: 0; }
                  .media { position: absolute; top: 0; left: 0; width: 24px; height: 4px; }
                </style>
              </head>
              <body>
                <section class="hero">
                  <div class="copy">A Hero</div>
                  <img class="media" alt="" width="24" height="4">
                </section>
              </body>
            </html>
            "#,
        BrowserRenderOptions {
            width: 32,
            ..BrowserRenderOptions::default()
        },
    );

    let paint_order = render
        .display_list
        .iter()
        .filter_map(|command| match command {
            DisplayCommand::Text { text, .. } | DisplayCommand::StyledText { text, .. } => {
                Some(text.as_str())
            }
            DisplayCommand::Image { .. } => Some("image"),
            DisplayCommand::Rect { .. } | DisplayCommand::BackgroundImage { .. } => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(paint_order, vec!["A Hero", "image"]);
    let (text_x, text_y) = render
        .display_list
        .iter()
        .find_map(|command| match command {
            DisplayCommand::Text { x, y, text } | DisplayCommand::StyledText { x, y, text, .. }
                if text == "A Hero" =>
            {
                Some((*x, *y))
            }
            _ => None,
        })
        .expect("foreground text position");
    let (image_x, image_y, image_width, image_height) = render
        .display_list
        .iter()
        .find_map(|command| match command {
            DisplayCommand::Image {
                x,
                y,
                width,
                height,
                ..
            } => Some((*x, *y, *width, *height)),
            _ => None,
        })
        .expect("overlapping image position");
    assert!(image_x <= text_x && text_x < image_x.saturating_add(image_width));
    assert!(image_y <= text_y && text_y < image_y.saturating_add(image_height));

    let viewport_height = text_y.saturating_add(4).max(4);
    let viewport_width = text_x
        .saturating_add("A Hero".len())
        .saturating_add(4)
        .max(24);

    let viewport = browser_text_viewport(
        &render,
        BrowserTextViewportOptions {
            width: viewport_width,
            height: viewport_height,
            ..BrowserTextViewportOptions::default()
        },
    );
    assert!(
        viewport
            .lines
            .get(text_y)
            .is_some_and(|line| line.contains("A Hero"))
    );

    let raster_options = BrowserRasterOptions {
        viewport_width: Some(viewport_width),
        viewport_height: Some(viewport_height),
        ..BrowserRasterOptions::default()
    };
    let raster = rasterize_render(&render, raster_options).expect("rasterize readable foreground");
    let glyph_pixel_x = raster_options
        .padding_x
        .saturating_add(text_x.saturating_mul(raster_options.cell_width))
        .saturating_add(2);
    let glyph_pixel_y = raster_options
        .padding_y
        .saturating_add(text_y.saturating_mul(raster_options.cell_height))
        .saturating_add(2);
    let glyph_pixel = glyph_pixel_y
        .saturating_mul(raster.width)
        .saturating_add(glyph_pixel_x);
    assert_eq!(raster.pixels[glyph_pixel], 0);
}

#[test]
fn post_visual_viewports_keep_text_and_fills_readable() {
    let render = render_html(
        "mem://post-visual-readable-fill",
        br#"
            <html><body>
              <section style="height:240px; background-color:black; color:black">
                Readable hero
              </section>
            </body></html>
            "#,
        BrowserRenderOptions {
            width: 24,
            ..BrowserRenderOptions::default()
        },
    );

    let scrolled = browser_text_viewport(
        &render,
        BrowserTextViewportOptions {
            y: 3,
            width: 24,
            height: 4,
            ..BrowserTextViewportOptions::default()
        },
    );
    let scrolled_text = scrolled.lines.join("\n");
    assert!(scrolled_text.contains("Readable hero"));
    assert!(scrolled_text.contains('#'));
    assert!(
        scrolled
            .lines
            .iter()
            .all(|line| line.chars().filter(|ch| *ch == '#').count() < 8)
    );

    let raster_options = BrowserRasterOptions {
        viewport_width: Some(24),
        viewport_height: Some(2),
        ..BrowserRasterOptions::default()
    };
    let raster = rasterize_render(&render, raster_options).expect("rasterize readable contrast");
    let glyph_pixel_x = raster_options.padding_x.saturating_add(2);
    let glyph_pixel_y = raster_options.padding_y.saturating_add(2);
    let glyph_pixel = glyph_pixel_y
        .saturating_mul(raster.width)
        .saturating_add(glyph_pixel_x);
    assert_eq!(raster.pixels[glyph_pixel], 255);

    let scrolled_raster_options = BrowserRasterOptions {
        viewport_y: Some(3),
        viewport_width: Some(24),
        viewport_height: Some(4),
        ..BrowserRasterOptions::default()
    };
    let scrolled_raster =
        rasterize_render(&render, scrolled_raster_options).expect("rasterize scrolled context");
    let scrolled_glyph_pixel_x = scrolled_raster_options.padding_x.saturating_add(2);
    let scrolled_glyph_pixel_y = scrolled_raster_options.padding_y.saturating_add(2);
    let scrolled_glyph_pixel = scrolled_glyph_pixel_y
        .saturating_mul(scrolled_raster.width)
        .saturating_add(scrolled_glyph_pixel_x);
    assert_eq!(scrolled_raster.pixels[scrolled_glyph_pixel], 255);
}

#[test]
fn scrolled_visual_viewports_use_nearby_document_flow_text_context() {
    let render = render_html(
        "mem://document-flow-readable-scroll-context",
        br#"
            <html><body>
              <h2 style="margin:0">Care pathways</h2>
              <p style="margin:0">Patient outcomes summary</p>
              <section style="height:240px; background-color:black"></section>
              <ul style="margin:0">
                <li>Follow up item</li>
              </ul>
              <table>
                <tr><td>Measure</td><td>Trend</td></tr>
              </table>
            </body></html>
            "#,
        BrowserRenderOptions {
            width: 32,
            ..BrowserRenderOptions::default()
        },
    );

    let scrolled = browser_text_viewport(
        &render,
        BrowserTextViewportOptions {
            y: 5,
            width: 32,
            height: 4,
            ..BrowserTextViewportOptions::default()
        },
    );
    let scrolled_text = scrolled.lines.join("\n");
    assert!(scrolled_text.contains("Patient outcomes summary"));
    assert!(scrolled_text.contains('#'));
    assert!(
        scrolled
            .lines
            .iter()
            .all(|line| line.chars().filter(|ch| *ch == '#').count() < 10)
    );

    let raster_options = BrowserRasterOptions {
        viewport_y: Some(5),
        viewport_width: Some(32),
        viewport_height: Some(4),
        ..BrowserRasterOptions::default()
    };
    let raster =
        rasterize_render(&render, raster_options).expect("rasterize document flow context");
    let glyph_pixel_x = raster_options.padding_x.saturating_add(2);
    let glyph_pixel_y = raster_options.padding_y.saturating_add(2);
    let glyph_pixel = glyph_pixel_y
        .saturating_mul(raster.width)
        .saturating_add(glyph_pixel_x);
    assert_eq!(raster.pixels[glyph_pixel], 255);
}

#[test]
fn sparse_visual_viewports_prioritize_meaningful_document_flow_text() {
    let render = render_html(
        "mem://sparse-visual-flow-priority",
        br#"
            <html><body>
              <h2 style="margin:0">Care pathways</h2>
              <p style="margin:0">Patient outcomes summary</p>
              <section style="height:240px; background-color:black; color:black">
                ID
              </section>
              <ul style="margin:0"><li>Follow up item</li></ul>
              <table><tr><td>Measure</td><td>Trend</td></tr></table>
            </body></html>
            "#,
        BrowserRenderOptions {
            width: 32,
            ..BrowserRenderOptions::default()
        },
    );

    let scrolled = browser_text_viewport(
        &render,
        BrowserTextViewportOptions {
            y: 2,
            width: 32,
            height: 4,
            ..BrowserTextViewportOptions::default()
        },
    );
    let scrolled_text = scrolled.lines.join("\n");
    assert!(scrolled_text.contains("ID"));
    assert!(scrolled_text.contains("Patient outcomes summary"));
    assert!(
        scrolled
            .lines
            .iter()
            .all(|line| line.chars().filter(|ch| *ch == '#').count() < 10)
    );

    let raster_options = BrowserRasterOptions {
        viewport_y: Some(2),
        viewport_width: Some(32),
        viewport_height: Some(4),
        ..BrowserRasterOptions::default()
    };
    let raster = rasterize_render(&render, raster_options)
        .expect("rasterize meaningful document flow context");
    let context_pixel_x = raster_options.padding_x.saturating_add(2);
    let context_pixel_y = raster_options
        .padding_y
        .saturating_add(raster_options.cell_height)
        .saturating_add(2);
    let context_pixel = context_pixel_y
        .saturating_mul(raster.width)
        .saturating_add(context_pixel_x);
    assert_eq!(raster.pixels[context_pixel], 255);
}

#[test]
fn pinned_headers_do_not_suppress_visual_viewport_body_context() {
    let render = render_html(
        "mem://pinned-header-readable-body-context",
        br#"
            <html><body>
              <div style="position:fixed; top:0">Pinned navigation menu</div>
              <p style="margin:0">Patient outcomes summary</p>
              <section style="height:240px; background-color:black"></section>
              <ul style="margin:0"><li>Follow up item</li></ul>
            </body></html>
            "#,
        BrowserRenderOptions {
            width: 36,
            ..BrowserRenderOptions::default()
        },
    );

    let scrolled = browser_text_viewport(
        &render,
        BrowserTextViewportOptions {
            y: 4,
            width: 36,
            height: 4,
            ..BrowserTextViewportOptions::default()
        },
    );
    let scrolled_text = scrolled.lines.join("\n");
    assert!(scrolled_text.contains("Pinned navigation menu"));
    assert!(scrolled_text.contains("Patient outcomes summary"));
    assert!(scrolled_text.contains('#'));

    let raster_options = BrowserRasterOptions {
        viewport_y: Some(4),
        viewport_width: Some(36),
        viewport_height: Some(4),
        ..BrowserRasterOptions::default()
    };
    let raster = rasterize_render(&render, raster_options).expect("rasterize pinned body context");
    let context_pixel_x = raster_options.padding_x.saturating_add(1);
    let context_pixel_y = raster_options
        .padding_y
        .saturating_add(raster_options.cell_height)
        .saturating_add(2);
    let context_pixel = context_pixel_y
        .saturating_mul(raster.width)
        .saturating_add(context_pixel_x);
    assert_eq!(raster.pixels[context_pixel], 255);
}

#[test]
fn mixed_media_viewports_preserve_nearby_body_context() {
    let render = render_html(
        "mem://mixed-media-readable-body-context",
        br#"
            <html>
              <head>
                <style>
                  .hero { position: relative; height: 240px; background-color: black; color: black; }
                  .eyebrow { position: absolute; top: 0; left: 0; margin: 0; }
                  .summary { position: absolute; top: 80px; left: 0; margin: 0; }
                </style>
              </head>
              <body>
                <section class="hero">
                  <h1 class="eyebrow">Clinical evidence platform</h1>
                  <p class="summary">Trusted patient data networks</p>
                </section>
              </body>
            </html>
            "#,
        BrowserRenderOptions {
            width: 36,
            ..BrowserRenderOptions::default()
        },
    );

    let viewport = browser_text_viewport(
        &render,
        BrowserTextViewportOptions {
            width: 36,
            height: 4,
            ..BrowserTextViewportOptions::default()
        },
    );
    let viewport_text = viewport.lines.join("\n");
    assert!(viewport_text.contains("Clinical evidence platform"));
    assert!(viewport_text.contains("Trusted patient data networks"));
    assert!(viewport_text.contains('#'));

    let raster_options = BrowserRasterOptions {
        viewport_width: Some(36),
        viewport_height: Some(4),
        ..BrowserRasterOptions::default()
    };
    let raster = rasterize_render(&render, raster_options).expect("rasterize mixed media context");
    let context_pixel_x = raster_options.padding_x.saturating_add(1);
    let context_pixel_y = raster_options
        .padding_y
        .saturating_add(raster_options.cell_height)
        .saturating_add(2);
    let context_pixel = context_pixel_y
        .saturating_mul(raster.width)
        .saturating_add(context_pixel_x);
    assert_eq!(raster.pixels[context_pixel], 255);
}

#[test]
fn decoded_image_viewports_preserve_following_body_context() {
    let image_url = tiny_test_jpeg_data_url();
    let cached_image = decoded_image_entry("mem://decoded-image-context", &image_url)
        .expect("decode tiny jpeg fixture");
    let html = format!(
        r#"
            <html><body>
              <img src="{image_url}" width="36" height="6" alt="">
              <p style="margin:0">Readable body evidence</p>
            </body></html>
            "#
    );
    let render = render_html_prepared_with_inputs(
        "mem://decoded-image-context",
        html.as_bytes(),
        BrowserRenderOptions {
            width: 36,
            ..BrowserRenderOptions::default()
        },
        RenderPreparation {
            external_css: &[],
            external_scripts: &[],
            click_target: None,
            local_storage: None,
            session_storage: None,
            cached_images: &[cached_image],
        },
    )
    .expect("render with decoded image fixture");

    let image_bounds = render
        .display_list
        .iter()
        .find_map(|command| match command {
            DisplayCommand::Image {
                width,
                height,
                decoded_width,
                decoded_height,
                ..
            } => Some((*width, *height, *decoded_width, *decoded_height)),
            _ => None,
        })
        .expect("decoded image command");
    assert!(image_bounds.0 > 0);
    assert!(image_bounds.1 > 0);
    assert_eq!(image_bounds.2, Some(2));
    assert_eq!(image_bounds.3, Some(2));

    let viewport = browser_text_viewport(
        &render,
        BrowserTextViewportOptions {
            width: 36,
            height: 4,
            ..BrowserTextViewportOptions::default()
        },
    );
    let viewport_text = viewport.lines.join("\n");
    assert!(viewport_text.contains('@'));
    assert!(viewport_text.contains("Readable body evidence"));

    let raster_options = BrowserRasterOptions {
        viewport_width: Some(36),
        viewport_height: Some(4),
        ..BrowserRasterOptions::default()
    };
    let raster =
        rasterize_render(&render, raster_options).expect("rasterize decoded image context");
    let context_pixel_x = raster_options.padding_x.saturating_add(1);
    let context_pixel_y = raster_options
        .padding_y
        .saturating_add(raster_options.cell_height)
        .saturating_add(2);
    let context_pixel = context_pixel_y
        .saturating_mul(raster.width)
        .saturating_add(context_pixel_x);
    assert_eq!(raster.pixels[context_pixel], 0);
}

#[test]
fn scrolled_image_viewports_choose_nearest_body_context() {
    let image_url = tiny_test_jpeg_data_url();
    let render = BrowserRender {
        source: "mem://scrolled-image-context".to_owned(),
        title: String::new(),
        viewport_width: 36,
        dom_node_count: 0,
        css_rule_count: 0,
        layout_box_count: 0,
        layout_boxes: Vec::new(),
        paint_command_count: 3,
        links: Vec::new(),
        forms: Vec::new(),
        resources: Vec::new(),
        fragment_targets: Vec::new(),
        decoded_images: Vec::new(),
        hit_targets: vec![DisplayHitTarget::default(); 3],
        display_list: vec![
            DisplayCommand::Text {
                x: 0,
                y: 0,
                text: "Navigation overview".to_owned(),
            },
            DisplayCommand::Image {
                x: 0,
                y: 1,
                width: 36,
                height: 16,
                shade: 220,
                alt: None,
                url: Some(image_url),
                decoded_width: Some(2),
                decoded_height: Some(2),
                decoded_hash: None,
            },
            DisplayCommand::Text {
                x: 0,
                y: 17,
                text: "Readable body evidence".to_owned(),
            },
        ],
        text: "Navigation overview\nReadable body evidence".to_owned(),
    };

    let scrolled = browser_text_viewport(
        &render,
        BrowserTextViewportOptions {
            y: 12,
            width: 36,
            height: 4,
            ..BrowserTextViewportOptions::default()
        },
    );
    let scrolled_text = scrolled.lines.join("\n");
    assert!(scrolled_text.contains('@'));
    assert!(scrolled_text.contains("Readable body evidence"));
    assert!(!scrolled_text.contains("Navigation overview"));
    assert!(
        scrolled
            .lines
            .first()
            .is_some_and(|line| !line.contains("Readable body evidence"))
    );
    assert!(
        scrolled
            .lines
            .get(3)
            .is_some_and(|line| line.contains("Readable body evidence"))
    );

    let raster_options = BrowserRasterOptions {
        viewport_y: Some(12),
        viewport_width: Some(36),
        viewport_height: Some(4),
        ..BrowserRasterOptions::default()
    };
    let raster =
        rasterize_render(&render, raster_options).expect("rasterize scrolled image context");
    let overlay_row_y = raster_options
        .padding_y
        .saturating_add(3usize.saturating_mul(raster_options.cell_height));
    let overlay_row_end = overlay_row_y.saturating_add(raster_options.cell_height);
    let overlay_col_end = raster_options.padding_x.saturating_add(
        "Readable body evidence"
            .len()
            .saturating_mul(raster_options.cell_width),
    );
    let mut overlay_glyph_pixels = 0usize;
    for y in overlay_row_y..overlay_row_end {
        for x in raster_options.padding_x..overlay_col_end {
            let index = y.saturating_mul(raster.width).saturating_add(x);
            if raster.pixels.get(index).is_some_and(|pixel| *pixel == 255) {
                overlay_glyph_pixels = overlay_glyph_pixels.saturating_add(1);
            }
        }
    }
    assert!(overlay_glyph_pixels >= 8);
}

#[test]
fn image_viewports_project_adjacent_caption_and_body_lines() {
    let image_url = tiny_test_jpeg_data_url();
    let render = BrowserRender {
        source: "mem://image-card-context".to_owned(),
        title: String::new(),
        viewport_width: 36,
        dom_node_count: 0,
        css_rule_count: 0,
        layout_box_count: 0,
        layout_boxes: Vec::new(),
        paint_command_count: 3,
        links: Vec::new(),
        forms: Vec::new(),
        resources: Vec::new(),
        fragment_targets: Vec::new(),
        decoded_images: Vec::new(),
        hit_targets: vec![DisplayHitTarget::default(); 3],
        display_list: vec![
            DisplayCommand::Image {
                x: 0,
                y: 1,
                width: 36,
                height: 16,
                shade: 220,
                alt: None,
                url: Some(image_url),
                decoded_width: Some(2),
                decoded_height: Some(2),
                decoded_hash: None,
            },
            DisplayCommand::Text {
                x: 0,
                y: 16,
                text: "Card insight headline".to_owned(),
            },
            DisplayCommand::Text {
                x: 0,
                y: 17,
                text: "Supporting caption detail".to_owned(),
            },
        ],
        text: "Card insight headline\nSupporting caption detail".to_owned(),
    };

    let scrolled = browser_text_viewport(
        &render,
        BrowserTextViewportOptions {
            y: 10,
            width: 36,
            height: 6,
            ..BrowserTextViewportOptions::default()
        },
    );
    let scrolled_text = scrolled.lines.join("\n");
    assert!(scrolled_text.contains('@'));
    assert!(
        scrolled
            .lines
            .get(4)
            .is_some_and(|line| line.contains("Card insight headline"))
    );
    assert!(
        scrolled
            .lines
            .get(5)
            .is_some_and(|line| line.contains("Supporting caption detail"))
    );
    assert!(scrolled_text.matches('@').count() > 8);

    let raster_options = BrowserRasterOptions {
        viewport_y: Some(10),
        viewport_width: Some(36),
        viewport_height: Some(6),
        ..BrowserRasterOptions::default()
    };
    let raster = rasterize_render(&render, raster_options).expect("rasterize image card context");
    let overlay_rows = [4usize, 5usize];
    for row in overlay_rows {
        let overlay_row_y = raster_options
            .padding_y
            .saturating_add(row.saturating_mul(raster_options.cell_height));
        let overlay_row_end = overlay_row_y.saturating_add(raster_options.cell_height);
        let overlay_col_end = raster_options
            .padding_x
            .saturating_add(24usize.saturating_mul(raster_options.cell_width));
        let mut overlay_glyph_pixels = 0usize;
        for y in overlay_row_y..overlay_row_end {
            for x in raster_options.padding_x..overlay_col_end {
                let index = y.saturating_mul(raster.width).saturating_add(x);
                if raster.pixels.get(index).is_some_and(|pixel| *pixel == 255) {
                    overlay_glyph_pixels = overlay_glyph_pixels.saturating_add(1);
                }
            }
        }
        assert!(overlay_glyph_pixels >= 8);
    }
}

#[test]
fn visual_section_viewports_project_heading_caption_and_list_context() {
    let render = BrowserRender {
        source: "mem://visual-section-context".to_owned(),
        title: String::new(),
        viewport_width: 40,
        dom_node_count: 0,
        css_rule_count: 0,
        layout_box_count: 0,
        layout_boxes: Vec::new(),
        paint_command_count: 4,
        links: Vec::new(),
        forms: Vec::new(),
        resources: Vec::new(),
        fragment_targets: Vec::new(),
        decoded_images: Vec::new(),
        hit_targets: vec![DisplayHitTarget::default(); 4],
        display_list: vec![
            DisplayCommand::Image {
                x: 0,
                y: 0,
                width: 40,
                height: 16,
                shade: 220,
                alt: None,
                url: None,
                decoded_width: None,
                decoded_height: None,
                decoded_hash: None,
            },
            DisplayCommand::Text {
                x: 0,
                y: 9,
                text: "Patient story headline".to_owned(),
            },
            DisplayCommand::Text {
                x: 0,
                y: 16,
                text: "Outcome caption detail".to_owned(),
            },
            DisplayCommand::Text {
                x: 0,
                y: 17,
                text: "- Trial enrollment summary".to_owned(),
            },
        ],
        text: "Patient story headline\nOutcome caption detail\n- Trial enrollment summary"
            .to_owned(),
    };

    let scrolled = browser_text_viewport(
        &render,
        BrowserTextViewportOptions {
            y: 8,
            width: 40,
            height: 6,
            ..BrowserTextViewportOptions::default()
        },
    );
    let scrolled_text = scrolled.lines.join("\n");
    assert!(scrolled_text.contains('@'));
    assert!(
        scrolled
            .lines
            .get(3)
            .is_some_and(|line| line.contains("Patient story headline"))
    );
    assert!(
        scrolled
            .lines
            .get(4)
            .is_some_and(|line| line.contains("Outcome caption detail"))
    );
    assert!(
        scrolled
            .lines
            .get(5)
            .is_some_and(|line| line.contains("- Trial enrollment summary"))
    );

    let raster_options = BrowserRasterOptions {
        viewport_y: Some(8),
        viewport_width: Some(40),
        viewport_height: Some(6),
        ..BrowserRasterOptions::default()
    };
    let raster =
        rasterize_render(&render, raster_options).expect("rasterize visual section context");
    for (row, text) in [
        (3usize, "Patient story headline"),
        (4usize, "Outcome caption detail"),
        (5usize, "- Trial enrollment summary"),
    ] {
        let row_y = raster_options
            .padding_y
            .saturating_add(row.saturating_mul(raster_options.cell_height));
        let row_end = row_y.saturating_add(raster_options.cell_height);
        let col_end = raster_options
            .padding_x
            .saturating_add(text.len().saturating_mul(raster_options.cell_width));
        let mut glyph_pixels = 0usize;
        for y in row_y..row_end {
            for x in raster_options.padding_x..col_end {
                let index = y.saturating_mul(raster.width).saturating_add(x);
                if raster.pixels.get(index).is_some_and(|pixel| *pixel == 0) {
                    glyph_pixels = glyph_pixels.saturating_add(1);
                }
            }
        }
        assert!(glyph_pixels >= 8, "expected raster glyphs for {text}");
    }
}

#[test]
fn raster_dominant_image_viewports_keep_section_text_band() {
    let render = BrowserRender {
        source: "mem://dominant-image-section-context".to_owned(),
        title: String::new(),
        viewport_width: 40,
        dom_node_count: 0,
        css_rule_count: 0,
        layout_box_count: 0,
        layout_boxes: Vec::new(),
        paint_command_count: 4,
        links: Vec::new(),
        forms: Vec::new(),
        resources: Vec::new(),
        fragment_targets: Vec::new(),
        decoded_images: Vec::new(),
        hit_targets: vec![DisplayHitTarget::default(); 4],
        display_list: vec![
            DisplayCommand::Image {
                x: 0,
                y: 0,
                width: 40,
                height: 10,
                shade: 220,
                alt: None,
                url: None,
                decoded_width: None,
                decoded_height: None,
                decoded_hash: None,
            },
            DisplayCommand::Text {
                x: 0,
                y: 3,
                text: "Hero insight headline".to_owned(),
            },
            DisplayCommand::Text {
                x: 0,
                y: 4,
                text: "Visible summary row".to_owned(),
            },
            DisplayCommand::Text {
                x: 0,
                y: 10,
                text: "Adjacent body detail".to_owned(),
            },
        ],
        text: "Hero insight headline\nVisible summary row\nAdjacent body detail".to_owned(),
    };

    let raster_options = BrowserRasterOptions {
        viewport_y: Some(2),
        viewport_width: Some(40),
        viewport_height: Some(6),
        ..BrowserRasterOptions::default()
    };
    let raster =
        rasterize_render(&render, raster_options).expect("rasterize dominant image context");
    for (row, text) in [
        (3usize, "Hero insight headline"),
        (4usize, "Visible summary row"),
        (5usize, "Adjacent body detail"),
    ] {
        let row_y = raster_options
            .padding_y
            .saturating_add(row.saturating_mul(raster_options.cell_height));
        let row_end = row_y.saturating_add(raster_options.cell_height);
        let col_end = raster_options
            .padding_x
            .saturating_add(text.len().saturating_mul(raster_options.cell_width));
        let mut glyph_pixels = 0usize;
        for y in row_y..row_end {
            for x in raster_options.padding_x..col_end {
                let index = y.saturating_mul(raster.width).saturating_add(x);
                if raster.pixels.get(index).is_some_and(|pixel| *pixel == 0) {
                    glyph_pixels = glyph_pixels.saturating_add(1);
                }
            }
        }
        assert!(glyph_pixels >= 8, "expected raster glyphs for {text}");
    }

    assert_eq!(
        display_command_bounds(&render.display_list[0]),
        DisplayCommandBounds {
            x: 0,
            y: 0,
            width: 40,
            height: 10,
        }
    );
    assert_eq!(
        display_command_bounds(&render.display_list[3]),
        DisplayCommandBounds {
            x: 0,
            y: 10,
            width: "Adjacent body detail".len(),
            height: 1,
        }
    );
}

#[test]
fn css_viewport_units_size_first_viewport_hero() {
    let render = render_html(
        "mem://viewport-unit-hero",
        br#"
            <html>
              <head>
                <style>
                  .hero {
                    position: relative;
                    inline-size: 100dvw;
                    width: 100vw;
                    block-size: 100vh;
                    min-block-size: 100svh;
                    background-color: white;
                  }
                  .hero-copy {
                    position: absolute;
                    top: 0;
                  }
                </style>
              </head>
              <body>
                <section class="hero">
                  <div class="hero-copy">Hero headline</div>
                </section>
                <p>Below fold</p>
              </body>
            </html>
            "#,
        BrowserRenderOptions {
            width: 100,
            ..BrowserRenderOptions::default()
        },
    );

    assert!(render.display_list.iter().any(|command| matches!(
        command,
        DisplayCommand::Rect {
            x: 0,
            y: 0,
            width: 100,
            height: 44,
            ..
        }
    )));
    assert!(render.display_list.iter().any(|command| matches!(
        command,
        DisplayCommand::Text { y: 44, text, .. }
            | DisplayCommand::StyledText { y: 44, text, .. }
            if text == "Below fold"
    )));

    let viewport = browser_text_viewport(
        &render,
        BrowserTextViewportOptions {
            width: 100,
            height: 44,
            ..BrowserTextViewportOptions::default()
        },
    );
    assert!(viewport.lines[0].starts_with("Hero headline"));
    assert!(!viewport.lines.join("\n").contains("Below fold"));
}

#[test]
fn css_position_absolute_and_fixed_do_not_advance_normal_flow() {
    let render = render_html(
        "mem://positioned-out-of-flow",
        br#"
            <html><body>
              <p>Before</p>
              <div style="position:absolute">Overlay</div>
              <p>After</p>
              <div style="position:fixed">Pinned</div>
              <p>Tail</p>
            </body></html>
            "#,
        BrowserRenderOptions {
            width: 80,
            ..BrowserRenderOptions::default()
        },
    );

    assert_eq!(render.text, "Before\nAfter\nTail");
    assert_eq!(
        render
            .display_list
            .iter()
            .filter_map(|command| match command {
                DisplayCommand::Text { x, y, text } => Some((*x, *y, text.as_str())),
                _ => None,
            })
            .collect::<Vec<_>>(),
        vec![
            (0, 0, "Before"),
            (0, 1, "Overlay"),
            (0, 1, "After"),
            (0, 2, "Pinned"),
            (0, 2, "Tail"),
        ]
    );
}

#[test]
fn css_fixed_position_paints_relative_to_scrolled_viewport() {
    let render = render_html(
        "mem://fixed-position-scrolled-viewport",
        br#"
            <html><body>
              <div style="position:fixed; top:0">Pinned nav</div>
              <div style="height:96px"></div>
              <p>After</p>
            </body></html>
            "#,
        BrowserRenderOptions {
            width: 30,
            ..BrowserRenderOptions::default()
        },
    );

    assert_eq!(render.text, "After");

    let viewport = browser_text_viewport(
        &render,
        BrowserTextViewportOptions {
            y: 3,
            width: 30,
            height: 3,
            ..BrowserTextViewportOptions::default()
        },
    );
    assert_eq!(
        viewport.lines,
        vec!["Pinned nav".to_owned(), String::new(), String::new()]
    );
    assert_eq!(viewport.visible_command_count, 1);

    let raster_options = BrowserRasterOptions {
        viewport_y: Some(3),
        viewport_width: Some(30),
        viewport_height: Some(3),
        ..BrowserRasterOptions::default()
    };
    let raster = rasterize_render(&render, raster_options).unwrap();
    let report = raster_report(&render, &raster, raster_options);
    assert_eq!(report.visible_command_count, 1);
    assert!(raster.non_background_pixels() > 0);
}

#[test]
fn css_sticky_position_paints_relative_to_scrolled_viewport() {
    let render = render_html(
        "mem://sticky-position-scrolled-viewport",
        br#"
            <html><body>
              <div style="height:24px"></div>
              <div style="position:sticky; top:0">Sticky nav</div>
              <div style="height:96px"></div>
              <p>After</p>
            </body></html>
            "#,
        BrowserRenderOptions {
            width: 30,
            ..BrowserRenderOptions::default()
        },
    );

    let viewport = browser_text_viewport(
        &render,
        BrowserTextViewportOptions {
            y: 5,
            width: 30,
            height: 3,
            ..BrowserTextViewportOptions::default()
        },
    );
    assert_eq!(
        viewport.lines,
        vec!["Sticky nav".to_owned(), String::new(), String::new()]
    );
    assert_eq!(viewport.visible_command_count, 1);

    let raster_options = BrowserRasterOptions {
        viewport_y: Some(5),
        viewport_width: Some(30),
        viewport_height: Some(3),
        ..BrowserRasterOptions::default()
    };
    let raster = rasterize_render(&render, raster_options).unwrap();
    let report = raster_report(&render, &raster, raster_options);
    assert_eq!(report.visible_command_count, 1);
    assert!(raster.non_background_pixels() > 0);
}

#[test]
fn css_absolute_top_uses_positioned_containing_block_start() {
    let render = render_html(
        "mem://absolute-top-containing-block",
        br#"
            <html><body>
              <p>Before</p>
              <section style="position:relative; height:72px; background-color:#eeeeee">
                <div style="height:60px"></div>
                <div style="position:absolute; top:0">Hero overlay</div>
              </section>
              <p>After</p>
            </body></html>
            "#,
        BrowserRenderOptions {
            width: 40,
            ..BrowserRenderOptions::default()
        },
    );

    assert_eq!(render.text, "Before\nAfter");
    assert_eq!(
        render
            .display_list
            .iter()
            .filter_map(|command| match command {
                DisplayCommand::Text { x, y, text } => Some((*x, *y, text.as_str())),
                _ => None,
            })
            .collect::<Vec<_>>(),
        vec![(0, 0, "Before"), (0, 1, "Hero overlay"), (0, 7, "After"),]
    );
}

#[test]
fn css_positioned_horizontal_offsets_and_translate_project_overlay_paint() {
    let render = render_html(
        "mem://positioned-horizontal-projection",
        br#"
            <html><body>
              <section style="position:relative; height:60px">
                <div style="position:absolute; top:0; left:50%; width:80px; transform:translateX(-50%)">Centered</div>
                <div style="position:absolute; top:12px; right:16px; width:80px">Right</div>
                <div style="position:absolute; top:24px; left:8px; width:80px; translate:8px 12px">Shifted</div>
              </section>
              <p>After</p>
            </body></html>
            "#,
        BrowserRenderOptions {
            width: 40,
            ..BrowserRenderOptions::default()
        },
    );

    assert_eq!(render.text, "After");
    assert_eq!(
        render
            .display_list
            .iter()
            .filter_map(|command| match command {
                DisplayCommand::Text { x, y, text } => Some((*x, *y, text.as_str())),
                _ => None,
            })
            .collect::<Vec<_>>(),
        vec![
            (15, 0, "Centered"),
            (28, 1, "Right"),
            (2, 3, "Shifted"),
            (0, 5, "After"),
        ]
    );
}

#[test]
fn css_positive_z_index_positions_foreground_above_later_media() {
    let render = render_html(
        "mem://z-index-stacking",
        br#"
            <html><head><style>
              .hero { position: relative; height: 48px; }
              .copy { position: absolute; top: 0; z-index: 2; }
              .media { position: absolute; top: 0; z-index: 1; }
            </style></head><body>
              <section class="hero">
                <div class="copy">Hero copy</div>
                <img class="media" alt="decor" width="80" height="24">
              </section>
            </body></html>
            "#,
        BrowserRenderOptions {
            width: 40,
            ..BrowserRenderOptions::default()
        },
    );

    assert_eq!(
        render
            .display_list
            .iter()
            .filter_map(|command| match command {
                DisplayCommand::Image { .. } => Some("image".to_owned()),
                DisplayCommand::Text { text, .. } => Some(text.clone()),
                _ => None,
            })
            .collect::<Vec<_>>(),
        vec!["image".to_owned(), "Hero copy".to_owned()]
    );
}

#[test]
fn narrow_float_columns_clear_before_body_text() {
    let render = render_html(
        "mem://narrow-float-column-readable-flow",
        br#"
            <html><body>
              <img alt="left visual" width="144" height="48" style="float:left">
              <img alt="right visual" width="144" height="48" style="float:right">
              <p style="margin:0">Readable body text after crowded media</p>
            </body></html>
            "#,
        BrowserRenderOptions {
            width: 40,
            ..BrowserRenderOptions::default()
        },
    );

    let image_rows = render
        .display_list
        .iter()
        .filter_map(|command| match command {
            DisplayCommand::Image { y, height, .. } => Some((*y, *height)),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(image_rows.len(), 2);
    let image_bottom = image_rows
        .iter()
        .map(|(y, height)| y.saturating_add(*height))
        .max()
        .expect("image bottom");

    let body_text = render
        .display_list
        .iter()
        .find_map(|command| match command {
            DisplayCommand::Text { x, y, text } if text.contains("Readable body text") => {
                Some((*x, *y, text.as_str()))
            }
            _ => None,
        })
        .expect("body text display command");
    assert_eq!(body_text.0, 0);
    assert!(body_text.1 >= image_bottom);
    assert_eq!(body_text.2, "Readable body text after crowded media");

    let viewport = browser_text_viewport(
        &render,
        BrowserTextViewportOptions {
            y: 0,
            width: 40,
            height: 8,
            ..BrowserTextViewportOptions::default()
        },
    );
    let viewport_text = viewport.lines.join("\n");
    assert!(viewport_text.contains('@'));
    assert!(
        viewport
            .lines
            .get(body_text.1)
            .is_some_and(|line| line.contains("Readable body text"))
    );
}

#[test]
fn css_overflow_hidden_clips_paint_and_flow_extent() {
    let render = render_html(
        "mem://overflow-hidden",
        br#"
            <html><body>
              <div style="height:12px; overflow:hidden; background-color:#d0d0d0">
                <p>Visible</p>
                <p>Hidden</p>
                <img alt="late image" width="80" height="24">
              </div>
              <p>After fixed</p>
              <div style="max-height:12px; overflow:hidden">
                <p>Max visible</p>
                <p>Max hidden</p>
              </div>
              <p>Tail</p>
            </body></html>
            "#,
        BrowserRenderOptions {
            width: 40,
            ..BrowserRenderOptions::default()
        },
    );

    assert_eq!(
        render.display_list,
        vec![
            DisplayCommand::Rect {
                x: 0,
                y: 0,
                width: 40,
                height: 1,
                shade: 208,
            },
            DisplayCommand::Text {
                x: 0,
                y: 0,
                text: "Visible".to_owned(),
            },
            DisplayCommand::Text {
                x: 0,
                y: 1,
                text: "After fixed".to_owned(),
            },
            DisplayCommand::Text {
                x: 0,
                y: 2,
                text: "Max visible".to_owned(),
            },
            DisplayCommand::Text {
                x: 0,
                y: 3,
                text: "Tail".to_owned(),
            },
        ]
    );

    let viewport = browser_text_viewport(
        &render,
        BrowserTextViewportOptions {
            width: 40,
            height: 5,
            ..BrowserTextViewportOptions::default()
        },
    );
    assert!(!viewport.lines.join("\n").contains("Hidden"));
    assert_eq!(viewport.document_height, 4);
}

#[test]
fn css_floating_images_wrap_following_text_rows() {
    let render = render_html(
        "mem://image-floats",
        br#"
              <html><body>
              <img alt="portrait" width="16" height="24" style="float:left">
              <p>Alpha Beta Gamma Pad</p>
              <p>Tail</p>
              <img alt="badge" width="16" height="12" style="float:right">
              <p>Right Float Text Pad</p>
            </body></html>
            "#,
        BrowserRenderOptions {
            width: 20,
            ..BrowserRenderOptions::default()
        },
    );

    assert_eq!(
        render.text,
        "Alpha Beta Gamma\nPad\nTail\nRight Float Text\nPad"
    );
    assert_eq!(
        render.display_list,
        vec![
            DisplayCommand::Image {
                x: 0,
                y: 0,
                width: 2,
                height: 2,
                shade: 220,
                alt: Some("portrait".to_owned()),
                url: None,
                decoded_width: None,
                decoded_height: None,
                decoded_hash: None,
            },
            DisplayCommand::Text {
                x: 2,
                y: 0,
                text: "Alpha Beta Gamma".to_owned(),
            },
            DisplayCommand::Text {
                x: 2,
                y: 1,
                text: "Pad".to_owned(),
            },
            DisplayCommand::Text {
                x: 0,
                y: 2,
                text: "Tail".to_owned(),
            },
            DisplayCommand::Image {
                x: 18,
                y: 3,
                width: 2,
                height: 1,
                shade: 220,
                alt: Some("badge".to_owned()),
                url: None,
                decoded_width: None,
                decoded_height: None,
                decoded_hash: None,
            },
            DisplayCommand::Text {
                x: 0,
                y: 3,
                text: "Right Float Text".to_owned(),
            },
            DisplayCommand::Text {
                x: 0,
                y: 4,
                text: "Pad".to_owned(),
            },
        ]
    );
}

#[test]
fn css_background_image_paints_block_underlay() {
    let render = render_html(
        "mem://background-image",
        br#"
            <html><body>
              <div style="width:80px; height:24px; background-color:white; background-image:url(https://example.test/hero.png)">Hero</div>
              <p>After</p>
            </body></html>
            "#,
        BrowserRenderOptions {
            width: 20,
            ..BrowserRenderOptions::default()
        },
    );

    assert_eq!(render.text, "Hero\nAfter");
    assert_eq!(
        render.display_list,
        vec![
            DisplayCommand::Rect {
                x: 0,
                y: 0,
                width: 10,
                height: 2,
                shade: 255,
            },
            DisplayCommand::BackgroundImage {
                x: 0,
                y: 0,
                width: 10,
                height: 2,
                shade: 220,
                url: Some("https://example.test/hero.png".to_owned()),
                decoded_width: None,
                decoded_height: None,
                decoded_hash: None,
                size: BackgroundImageSize::Auto,
                position: BackgroundImagePosition {
                    x_percent: 0,
                    y_percent: 0,
                },
                repeat: BackgroundImageRepeat::Repeat,
            },
            DisplayCommand::Text {
                x: 0,
                y: 0,
                text: "Hero".to_owned(),
            },
            DisplayCommand::Text {
                x: 0,
                y: 2,
                text: "After".to_owned(),
            },
        ]
    );
}

#[test]
fn css_background_image_respects_contain_position_and_no_repeat() {
    let image_url = "data:image/svg+xml,%3Csvg%20xmlns='http://www.w3.org/2000/svg'%20width='8'%20height='12'%3E%3Crect%20width='8'%20height='12'%20fill='black'/%3E%3C/svg%3E";
    let html = format!(
        r#"
            <html><body>
              <div style="width:16px; height:12px; background-image:url({image_url}); background-size:contain; background-position:right top; background-repeat:no-repeat"></div>
            </body></html>
            "#
    );
    let render = render_html(
        "mem://background-image-fit",
        html.as_bytes(),
        BrowserRenderOptions {
            width: 4,
            ..BrowserRenderOptions::default()
        },
    );

    let raster = rasterize_render(&render, BrowserRasterOptions::default()).unwrap();
    let left_cell_x = BrowserRasterOptions::default().padding_x;
    let right_cell_x = left_cell_x + BrowserRasterOptions::default().cell_width;
    let sample_y = BrowserRasterOptions::default().padding_y;
    assert_eq!(raster.pixels[sample_y * raster.width + left_cell_x], 255);
    assert_eq!(raster.pixels[sample_y * raster.width + right_cell_x], 0);
    assert!(render.display_list.iter().any(|command| {
        matches!(
            command,
            DisplayCommand::BackgroundImage {
                size: BackgroundImageSize::Contain,
                position: BackgroundImagePosition {
                    x_percent: 100,
                    y_percent: 0
                },
                repeat: BackgroundImageRepeat::NoRepeat,
                ..
            }
        )
    }));
}

#[test]
fn css_background_image_set_selects_supported_layer_fallback() {
    let render = render_html(
        "mem://background-image-set",
        br#"
            <html><body>
              <div style="width:80px; height:24px; background-image:image-set(url(https://example.test/hero.avif) type('image/avif') 1x, url('https://example.test/hero.webp') type('image/webp') 1x), linear-gradient(red, blue); background-size:contain, cover; background-position:right top, center; background-repeat:no-repeat, repeat">Hero Copy</div>
            </body></html>
            "#,
        BrowserRenderOptions {
            width: 20,
            ..BrowserRenderOptions::default()
        },
    );

    let background = render
        .display_list
        .iter()
        .find_map(|command| match command {
            DisplayCommand::BackgroundImage {
                url,
                size,
                position,
                repeat,
                ..
            } => Some((url.as_deref(), *size, *position, *repeat)),
            _ => None,
        });
    assert_eq!(
        background,
        Some((
            Some("https://example.test/hero.webp"),
            BackgroundImageSize::Contain,
            BackgroundImagePosition {
                x_percent: 100,
                y_percent: 0,
            },
            BackgroundImageRepeat::NoRepeat,
        ))
    );
    assert!(
        render
            .display_list
            .iter()
            .position(|command| matches!(command, DisplayCommand::BackgroundImage { .. }))
            < render.display_list.iter().position(|command| matches!(
                command,
                DisplayCommand::Text { text, .. } if text == "Hero Copy"
            ))
    );
}

#[test]
fn decoded_image_and_background_downscale_with_area_samples() {
    let image_url = "mem://stripe-image".to_owned();
    let background_url = "mem://stripe-background".to_owned();
    let mut pixels = Vec::new();
    for _ in 0..12 {
        for x in 0..16 {
            pixels.push(if x % 2 == 0 { 0 } else { 255 });
        }
    }
    let decoded = DecodedImage {
        width: 16,
        height: 12,
        pixels,
        rgb_pixels: None,
    };
    let image_entry = DecodedImageEntry {
        url: image_url.clone(),
        width: decoded.width,
        height: decoded.height,
        pixel_hash: decoded.pixel_hash(),
        image: decoded.clone(),
    };
    let background_entry = DecodedImageEntry {
        url: background_url.clone(),
        width: decoded.width,
        height: decoded.height,
        pixel_hash: decoded.pixel_hash(),
        image: decoded,
    };
    let render = BrowserRender {
        source: "mem://decoded-stripes".to_owned(),
        title: String::new(),
        viewport_width: 4,
        dom_node_count: 0,
        css_rule_count: 0,
        layout_box_count: 0,
        layout_boxes: Vec::new(),
        paint_command_count: 2,
        links: Vec::new(),
        forms: Vec::new(),
        resources: Vec::new(),
        fragment_targets: Vec::new(),
        decoded_images: vec![image_entry, background_entry],
        hit_targets: vec![DisplayHitTarget::default(); 2],
        display_list: vec![
            DisplayCommand::Image {
                x: 0,
                y: 0,
                width: 1,
                height: 1,
                shade: 220,
                alt: None,
                url: Some(image_url),
                decoded_width: Some(16),
                decoded_height: Some(12),
                decoded_hash: None,
            },
            DisplayCommand::BackgroundImage {
                x: 1,
                y: 0,
                width: 1,
                height: 1,
                shade: 220,
                url: Some(background_url),
                decoded_width: Some(16),
                decoded_height: Some(12),
                decoded_hash: None,
                size: BackgroundImageSize::Contain,
                position: BackgroundImagePosition {
                    x_percent: 0,
                    y_percent: 0,
                },
                repeat: BackgroundImageRepeat::NoRepeat,
            },
        ],
        text: String::new(),
    };

    let raster_options = BrowserRasterOptions::default();
    let raster = rasterize_render(&render, raster_options).expect("rasterize decoded stripes");
    let image_sample = raster_options
        .padding_y
        .saturating_mul(raster.width)
        .saturating_add(raster_options.padding_x);
    let background_sample = raster_options
        .padding_y
        .saturating_mul(raster.width)
        .saturating_add(
            raster_options
                .padding_x
                .saturating_add(raster_options.cell_width),
        );
    assert_eq!(raster.pixels[image_sample], 127);
    assert_eq!(raster.pixels[background_sample], 127);

    assert_eq!(
        display_command_bounds(&render.display_list[0]),
        DisplayCommandBounds {
            x: 0,
            y: 0,
            width: 1,
            height: 1,
        }
    );
    assert_eq!(
        display_command_bounds(&render.display_list[1]),
        DisplayCommandBounds {
            x: 1,
            y: 0,
            width: 1,
            height: 1,
        }
    );
}

#[test]
fn rgba_raster_paints_decoded_image_color_and_preserves_grayscale_fallback() {
    let red_url = "mem://red-image".to_owned();
    let green_url = "mem://green-background".to_owned();
    let gray_url = "mem://gray-fallback".to_owned();
    let red = DecodedImage {
        width: 1,
        height: 1,
        pixels: vec![64],
        rgb_pixels: Some(vec![200, 0, 0]),
    };
    let green = DecodedImage {
        width: 1,
        height: 1,
        pixels: vec![72],
        rgb_pixels: Some(vec![0, 180, 0]),
    };
    let gray = DecodedImage {
        width: 1,
        height: 1,
        pixels: vec![90],
        rgb_pixels: None,
    };
    let render = BrowserRender {
        source: "mem://rgba-color-paint".to_owned(),
        title: String::new(),
        viewport_width: 4,
        dom_node_count: 0,
        css_rule_count: 0,
        layout_box_count: 0,
        layout_boxes: Vec::new(),
        paint_command_count: 4,
        links: Vec::new(),
        forms: Vec::new(),
        resources: Vec::new(),
        fragment_targets: Vec::new(),
        decoded_images: vec![
            DecodedImageEntry {
                url: red_url.clone(),
                width: red.width,
                height: red.height,
                pixel_hash: red.pixel_hash(),
                image: red,
            },
            DecodedImageEntry {
                url: green_url.clone(),
                width: green.width,
                height: green.height,
                pixel_hash: green.pixel_hash(),
                image: green,
            },
            DecodedImageEntry {
                url: gray_url.clone(),
                width: gray.width,
                height: gray.height,
                pixel_hash: gray.pixel_hash(),
                image: gray,
            },
        ],
        hit_targets: vec![DisplayHitTarget::default(); 4],
        display_list: vec![
            DisplayCommand::Image {
                x: 0,
                y: 0,
                width: 1,
                height: 1,
                shade: 220,
                alt: None,
                url: Some(red_url),
                decoded_width: Some(1),
                decoded_height: Some(1),
                decoded_hash: None,
            },
            DisplayCommand::BackgroundImage {
                x: 1,
                y: 0,
                width: 1,
                height: 1,
                shade: 220,
                url: Some(green_url),
                decoded_width: Some(1),
                decoded_height: Some(1),
                decoded_hash: None,
                size: BackgroundImageSize::Auto,
                position: BackgroundImagePosition {
                    x_percent: 0,
                    y_percent: 0,
                },
                repeat: BackgroundImageRepeat::NoRepeat,
            },
            DisplayCommand::Image {
                x: 2,
                y: 0,
                width: 1,
                height: 1,
                shade: 220,
                alt: None,
                url: Some(gray_url),
                decoded_width: Some(1),
                decoded_height: Some(1),
                decoded_hash: None,
            },
            DisplayCommand::Text {
                x: 0,
                y: 0,
                text: "A".to_owned(),
            },
        ],
        text: "A".to_owned(),
    };

    let raster_options = BrowserRasterOptions::default();
    let rgba = rasterize_render_rgba(&render, raster_options).expect("rasterize rgba color");
    let pixel = |x: usize, y: usize| {
        let index = y
            .saturating_mul(rgba.width)
            .saturating_add(x)
            .saturating_mul(4);
        &rgba.pixels[index..index.saturating_add(4)]
    };
    let red_sample_x = raster_options.padding_x;
    let green_sample_x = raster_options
        .padding_x
        .saturating_add(raster_options.cell_width);
    let gray_sample_x = raster_options
        .padding_x
        .saturating_add(raster_options.cell_width.saturating_mul(2));
    let sample_y = raster_options.padding_y;
    let text_sample_x = raster_options.padding_x.saturating_add(2);
    let text_sample_y = raster_options.padding_y.saturating_add(2);

    assert_eq!(pixel(red_sample_x, sample_y), &[200, 0, 0, 255]);
    assert_eq!(pixel(green_sample_x, sample_y), &[0, 180, 0, 255]);
    assert_eq!(pixel(gray_sample_x, sample_y), &[90, 90, 90, 255]);
    assert_eq!(pixel(text_sample_x, text_sample_y), &[255, 255, 255, 255]);
}

#[test]
fn viewport_scroll_offsets_clamp_and_crop_text_with_images() {
    let image_url = "mem://viewport-image".to_owned();
    let decoded = DecodedImage {
        width: 1,
        height: 1,
        pixels: vec![76],
        rgb_pixels: Some(vec![210, 20, 20]),
    };
    let render = BrowserRender {
        source: "mem://viewport-scroll-crop".to_owned(),
        title: String::new(),
        viewport_width: 12,
        dom_node_count: 0,
        css_rule_count: 0,
        layout_box_count: 0,
        layout_boxes: Vec::new(),
        paint_command_count: 3,
        links: Vec::new(),
        forms: Vec::new(),
        resources: Vec::new(),
        fragment_targets: Vec::new(),
        decoded_images: vec![DecodedImageEntry {
            url: image_url.clone(),
            width: decoded.width,
            height: decoded.height,
            pixel_hash: decoded.pixel_hash(),
            image: decoded,
        }],
        hit_targets: vec![DisplayHitTarget::default(); 3],
        display_list: vec![
            DisplayCommand::Text {
                x: 0,
                y: 0,
                text: "Header".to_owned(),
            },
            DisplayCommand::Image {
                x: 0,
                y: 1,
                width: 3,
                height: 2,
                shade: 180,
                alt: None,
                url: Some(image_url),
                decoded_width: Some(1),
                decoded_height: Some(1),
                decoded_hash: None,
            },
            DisplayCommand::Text {
                x: 0,
                y: 3,
                text: "AAAA".to_owned(),
            },
        ],
        text: "Header\nAAAA".to_owned(),
    };

    let requested = BrowserViewportState {
        x: 99,
        y: 99,
        width: 6,
        height: 3,
    };
    let document_viewport = browser_document_viewport(&render, requested, None);
    assert_eq!(document_viewport.viewport.x, 6);
    assert_eq!(document_viewport.viewport.y, 1);
    assert_eq!(document_viewport.max_scroll_x, 6);
    assert_eq!(document_viewport.max_scroll_y, 1);

    let previous = BrowserViewportState {
        x: 0,
        y: 0,
        width: 6,
        height: 3,
    };
    let scrolled = browser_document_viewport(
        &render,
        BrowserViewportState { y: 1, ..previous },
        Some(previous),
    );
    assert_eq!(scrolled.scroll_delta_y, 1);
    assert!(!scrolled.full_repaint);
    assert!(scrolled.reused_area > 0);

    let raster_options = BrowserRasterOptions {
        viewport_x: Some(99),
        viewport_y: Some(99),
        viewport_width: Some(6),
        viewport_height: Some(3),
        ..BrowserRasterOptions::default()
    };
    let raster = rasterize_render_rgba(&render, raster_options).expect("rasterize clamped slice");
    let report = rgba_raster_report(&render, &raster, raster_options);
    assert_eq!(report.raster_viewport_x, Some(6));
    assert_eq!(report.raster_viewport_y, Some(1));
    assert_eq!(report.visible_command_count, 0);

    let scrolled_raster_options = BrowserRasterOptions {
        viewport_y: Some(1),
        viewport_width: Some(6),
        viewport_height: Some(3),
        ..BrowserRasterOptions::default()
    };
    let scrolled_raster =
        rasterize_render_rgba(&render, scrolled_raster_options).expect("rasterize scrolled slice");
    fn pixel(raster: &BrowserRgbaRaster, x: usize, y: usize) -> &[u8] {
        let index = y
            .saturating_mul(raster.width)
            .saturating_add(x)
            .saturating_mul(4);
        &raster.pixels[index..index.saturating_add(4)]
    }
    assert_eq!(
        pixel(
            &scrolled_raster,
            scrolled_raster_options.padding_x,
            scrolled_raster_options.padding_y
        ),
        &[210, 20, 20, 255]
    );
    let text_y_start = scrolled_raster_options
        .padding_y
        .saturating_add(scrolled_raster_options.cell_height.saturating_mul(2));
    let text_y_end = text_y_start
        .saturating_add(scrolled_raster_options.cell_height)
        .min(scrolled_raster.height);
    let text_x_start = scrolled_raster_options.padding_x;
    let text_x_end = text_x_start
        .saturating_add(scrolled_raster_options.cell_width.saturating_mul(4))
        .min(scrolled_raster.width);
    let text_has_ink = (text_y_start..text_y_end).any(|y| {
        (text_x_start..text_x_end).any(|x| {
            let pixel = pixel(&scrolled_raster, x, y);
            pixel[3] == 255 && pixel[0] == pixel[1] && pixel[1] == pixel[2] && pixel[0] != 255
        })
    });
    assert!(
        text_has_ink,
        "expected text ink in raster row {text_y_start}..{text_y_end} columns {text_x_start}..{text_x_end}"
    );
}

#[test]
fn render_fidelity_scrolled_viewport_keeps_rgb_css_text_and_decoded_image() {
    let image_url = "mem://render-fidelity-photo".to_owned();
    let decoded = DecodedImage {
        width: 1,
        height: 1,
        pixels: vec![76],
        rgb_pixels: Some(vec![200, 24, 24]),
    };
    let decoded_entry = DecodedImageEntry {
        url: image_url.clone(),
        width: decoded.width,
        height: decoded.height,
        pixel_hash: decoded.pixel_hash(),
        image: decoded,
    };
    let html = format!(
        r#"
            <html><body>
              <section style="background-color: rgb(10, 20, 30); color: rgb(240 240 240); height: 48px">
                <img src="{image_url}" width="16" height="16" alt="">
                <p style="margin:0">Readable mixed body</p>
              </section>
            </body></html>
            "#
    );
    let render = render_html_prepared_with_inputs(
        "mem://render-fidelity",
        html.as_bytes(),
        BrowserRenderOptions {
            width: 32,
            ..BrowserRenderOptions::default()
        },
        RenderPreparation {
            external_css: &[],
            external_scripts: &[],
            click_target: None,
            local_storage: None,
            session_storage: None,
            cached_images: &[decoded_entry],
        },
    )
    .expect("render mixed rgb css and decoded image fixture");

    assert!(render.display_list.iter().any(|command| matches!(
        command,
        DisplayCommand::Rect {
            shade,
            ..
        } if *shade == rgb_to_luma(10, 20, 30)
    )));
    assert!(render.display_list.iter().any(|command| matches!(
        command,
        DisplayCommand::StyledText {
            text,
            shade,
            ..
        } if text == "Readable mixed body" && *shade == rgb_to_luma(240, 240, 240)
    )));

    let raster_options = BrowserRasterOptions {
        viewport_y: Some(1),
        viewport_width: Some(32),
        viewport_height: Some(3),
        ..BrowserRasterOptions::default()
    };
    let rgba = rasterize_render_rgba(&render, raster_options)
        .expect("rasterize scrolled rgb css and decoded image");
    let pixel = |x: usize, y: usize| {
        let index = y
            .saturating_mul(rgba.width)
            .saturating_add(x)
            .saturating_mul(4);
        &rgba.pixels[index..index.saturating_add(4)]
    };

    assert_eq!(
        pixel(raster_options.padding_x, raster_options.padding_y),
        &[200, 24, 24, 255]
    );

    let text_row_y = raster_options
        .padding_y
        .saturating_add(raster_options.cell_height.saturating_mul(1));
    let text_row_end = text_row_y
        .saturating_add(raster_options.cell_height)
        .min(rgba.height);
    let text_col_end = raster_options
        .padding_x
        .saturating_add(
            "Readable mixed body"
                .len()
                .saturating_mul(raster_options.cell_width),
        )
        .min(rgba.width);
    let mut bright_text_pixels = 0usize;
    for y in text_row_y..text_row_end {
        for x in raster_options.padding_x..text_col_end {
            if pixel(x, y) == &[240, 240, 240, 255] {
                bright_text_pixels = bright_text_pixels.saturating_add(1);
            }
        }
    }
    assert!(bright_text_pixels >= 8);
}

#[tokio::test]
async fn viewport_scroll_normal_rerender_attaches_decoded_resource_cache_images() {
    let image_url = tiny_test_jpeg_data_url();
    let html = format!(
        r#"
            <html><body>
              <img src="{image_url}" width="16" height="16" alt="cached">
            </body></html>
            "#
    );
    let options = BrowserRenderOptions {
        width: 24,
        ..BrowserRenderOptions::default()
    };
    let (page_state, profiled) = render_html_prepared_with_state(
        "mem://session-cache-auto-image",
        html.as_bytes(),
        options,
        RenderPreparation {
            external_css: &[],
            external_scripts: &[],
            click_target: None,
            local_storage: None,
            session_storage: None,
            cached_images: &[],
        },
    )
    .expect("prepare page without cached images");
    assert!(page_state.cached_images.is_empty());

    let mut session = BrowserSession::new(options);
    session.push_entry(
        "mem://session-cache-auto-image".to_owned(),
        html.as_bytes().to_vec(),
        page_state,
        profiled.render,
    );
    let resource = BrowserResource {
        kind: "image".to_owned(),
        initiator: "img".to_owned(),
        url: image_url.clone(),
        resolved: image_url,
        rel: None,
        media: None,
        alt: Some("cached".to_owned()),
        type_hint: Some("image/jpeg".to_owned()),
    };
    let fetch = fetch_resource_with_cache(
        resource,
        1024,
        &mut session.cookie_jar,
        &mut session.resource_cache,
    )
    .await;
    assert_eq!(fetch.status, "cached");
    assert!(session.entries[0].page_state.cached_images.is_empty());

    let rerender = session.render_entry_page_state(0);
    let decoded_size = rerender
        .display_list
        .iter()
        .find_map(|command| match command {
            DisplayCommand::Image {
                decoded_width,
                decoded_height,
                ..
            } => Some((*decoded_width, *decoded_height)),
            _ => None,
        });
    assert_eq!(decoded_size, Some((Some(2), Some(2))));
    assert!(!rerender.decoded_images.is_empty());
}

#[test]
fn css_height_controls_block_and_image_extent() {
    let render = render_html(
        "mem://css-height",
        br#"
            <html><body>
              <img alt="plot" width="80" height="48" style="width:24px; height:24px">
              <div style="height:24px"></div>
              <p>After</p>
            </body></html>
            "#,
        BrowserRenderOptions {
            width: 80,
            ..BrowserRenderOptions::default()
        },
    );

    assert_eq!(render.text, "After");
    assert_eq!(
        render.display_list,
        vec![
            DisplayCommand::Image {
                x: 0,
                y: 0,
                width: 3,
                height: 2,
                shade: 220,
                alt: Some("plot".to_owned()),
                url: None,
                decoded_width: None,
                decoded_height: None,
                decoded_hash: None,
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
fn css_image_single_axis_size_preserves_intrinsic_aspect_ratio() {
    let render = render_html(
        "mem://css-image-aspect-ratio",
        br#"
            <html><body>
              <img alt="wide" width="80" height="48" style="width:24px">
              <img alt="tall" width="80" height="48" style="height:24px">
              <p>After</p>
            </body></html>
            "#,
        BrowserRenderOptions {
            width: 80,
            ..BrowserRenderOptions::default()
        },
    );

    assert_eq!(render.text, "After");
    assert_eq!(
        render.display_list,
        vec![
            DisplayCommand::Image {
                x: 0,
                y: 0,
                width: 3,
                height: 2,
                shade: 220,
                alt: Some("wide".to_owned()),
                url: None,
                decoded_width: None,
                decoded_height: None,
                decoded_hash: None,
            },
            DisplayCommand::Image {
                x: 0,
                y: 2,
                width: 5,
                height: 2,
                shade: 220,
                alt: Some("tall".to_owned()),
                url: None,
                decoded_width: None,
                decoded_height: None,
                decoded_hash: None,
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
fn css_percent_media_dimensions_preserve_readable_flow() {
    let render = render_html(
        "mem://css-percent-media-dimensions",
        br#"
            <html><body>
              <img alt="hero" width="400" height="200" style="width:50%; max-height:10%">
              <img alt="badge" width="50%" height="24">
              <div style="width:50%; margin-left:auto; margin-right:auto">Centered</div>
              <p>After media</p>
            </body></html>
            "#,
        BrowserRenderOptions {
            width: 20,
            ..BrowserRenderOptions::default()
        },
    );

    assert_eq!(render.text, "Centered\nAfter media");
    assert_eq!(
        render.display_list,
        vec![
            DisplayCommand::Image {
                x: 0,
                y: 0,
                width: 10,
                height: 4,
                shade: 220,
                alt: Some("hero".to_owned()),
                url: None,
                decoded_width: None,
                decoded_height: None,
                decoded_hash: None,
            },
            DisplayCommand::Image {
                x: 0,
                y: 4,
                width: 10,
                height: 2,
                shade: 220,
                alt: Some("badge".to_owned()),
                url: None,
                decoded_width: None,
                decoded_height: None,
                decoded_hash: None,
            },
            DisplayCommand::Text {
                x: 5,
                y: 6,
                text: "Centered".to_owned(),
            },
            DisplayCommand::Text {
                x: 0,
                y: 7,
                text: "After media".to_owned(),
            },
        ]
    );
}

#[test]
fn css_max_height_limits_image_placeholder_extent() {
    let render = render_html(
        "mem://css-max-height-image",
        br#"
            <html><body>
              <img alt="wide" width="80" height="48" style="max-height:24px">
              <img alt="floor" width="80" height="12" style="max-height:24px; min-height:36px">
              <p>After</p>
            </body></html>
            "#,
        BrowserRenderOptions {
            width: 80,
            ..BrowserRenderOptions::default()
        },
    );

    assert_eq!(render.text, "After");
    assert_eq!(
        render.display_list,
        vec![
            DisplayCommand::Image {
                x: 0,
                y: 0,
                width: 10,
                height: 2,
                shade: 220,
                alt: Some("wide".to_owned()),
                url: None,
                decoded_width: None,
                decoded_height: None,
                decoded_hash: None,
            },
            DisplayCommand::Image {
                x: 0,
                y: 2,
                width: 10,
                height: 3,
                shade: 220,
                alt: Some("floor".to_owned()),
                url: None,
                decoded_width: None,
                decoded_height: None,
                decoded_hash: None,
            },
            DisplayCommand::Text {
                x: 0,
                y: 5,
                text: "After".to_owned(),
            },
        ]
    );
}

#[test]
fn unresolved_image_placeholders_are_clamped_to_keep_text_visible() {
    let render = render_html(
        "mem://unresolved-image-placeholder-clamp",
        br#"
            <html><body>
              <img alt="hero" src="https://example.invalid/hero.jpg" width="800" height="480">
              <p>Contact details</p>
            </body></html>
            "#,
        BrowserRenderOptions {
            width: 60,
            ..BrowserRenderOptions::default()
        },
    );

    assert_eq!(render.text, "Contact details");
    assert_eq!(
        render.display_list,
        vec![
            DisplayCommand::Image {
                x: 0,
                y: 0,
                width: 60,
                height: 8,
                shade: 220,
                alt: Some("hero".to_owned()),
                url: None,
                decoded_width: None,
                decoded_height: None,
                decoded_hash: None,
            },
            DisplayCommand::Text {
                x: 0,
                y: 8,
                text: "Contact details".to_owned(),
            },
        ]
    );
}

#[test]
fn metadata_and_fallback_elements_do_not_render_in_body_flow() {
    let render = render_html(
        "mem://hidden-metadata-elements",
        br#"
            <html><body>
              <p>Before metadata</p>
              <title>Body title should not paint</title>
              <datalist id="choices">
                <option value="rust">Rust fallback option</option>
              </datalist>
              <noembed>Legacy fallback text</noembed>
              <p>After metadata</p>
            </body></html>
            "#,
        BrowserRenderOptions {
            width: 80,
            ..BrowserRenderOptions::default()
        },
    );

    assert_eq!(render.text, "Before metadata\nAfter metadata");
    assert_eq!(
        render.display_list,
        vec![
            DisplayCommand::Text {
                x: 0,
                y: 0,
                text: "Before metadata".to_owned(),
            },
            DisplayCommand::Text {
                x: 0,
                y: 1,
                text: "After metadata".to_owned(),
            },
        ]
    );
}

#[test]
fn renders_replaced_media_elements_as_placeholders() {
    let render = render_html(
        "mem://replaced-media",
        br#"
            <html><body>
              <iframe title="Map" src="https://example.test/map" width="80" height="24"></iframe>
              <object data="chart.svg" width="40" height="12"></object>
              <video poster="poster.png" width="64" height="24"></video>
              <audio src="sound.mp3"></audio>
              <p>After</p>
            </body></html>
            "#,
        BrowserRenderOptions {
            width: 80,
            ..BrowserRenderOptions::default()
        },
    );

    assert_eq!(render.text, "After");
    assert_eq!(
        render.display_list,
        vec![
            DisplayCommand::Image {
                x: 0,
                y: 0,
                width: 10,
                height: 2,
                shade: 220,
                alt: Some("Map".to_owned()),
                url: None,
                decoded_width: None,
                decoded_height: None,
                decoded_hash: None,
            },
            DisplayCommand::Image {
                x: 0,
                y: 2,
                width: 5,
                height: 1,
                shade: 220,
                alt: Some("object".to_owned()),
                url: None,
                decoded_width: None,
                decoded_height: None,
                decoded_hash: None,
            },
            DisplayCommand::Image {
                x: 0,
                y: 3,
                width: 8,
                height: 2,
                shade: 220,
                alt: Some("video".to_owned()),
                url: None,
                decoded_width: None,
                decoded_height: None,
                decoded_hash: None,
            },
            DisplayCommand::Image {
                x: 0,
                y: 5,
                width: 10,
                height: 1,
                shade: 220,
                alt: Some("audio".to_owned()),
                url: None,
                decoded_width: None,
                decoded_height: None,
                decoded_hash: None,
            },
            DisplayCommand::Text {
                x: 0,
                y: 6,
                text: "After".to_owned(),
            },
        ]
    );
}

#[test]
fn css_box_sizing_content_box_keeps_width_as_content_width() {
    let render = render_html(
        "mem://box-sizing",
        br#"
            <html><body>
              <div style="width:80px; padding-left:16px; padding-right:16px">AB CD EF</div>
              <div style="box-sizing:border-box; width:80px; padding-left:16px; padding-right:16px">AB CD EF</div>
            </body></html>
            "#,
        BrowserRenderOptions {
            width: 20,
            ..BrowserRenderOptions::default()
        },
    );

    assert_eq!(render.text, "AB CD EF\nAB CD\nEF");
    assert_eq!(
        render.display_list,
        vec![
            DisplayCommand::Text {
                x: 2,
                y: 0,
                text: "AB CD EF".to_owned(),
            },
            DisplayCommand::Text {
                x: 2,
                y: 1,
                text: "AB CD".to_owned(),
            },
            DisplayCommand::Text {
                x: 2,
                y: 2,
                text: "EF".to_owned(),
            },
        ]
    );
}

#[test]
fn details_summary_hides_closed_content_and_marks_state() {
    let render = render_html(
        "mem://details-summary",
        br#"
            <html><body>
              <details>
                <summary>Closed title</summary>
                <p>Hidden body</p>
              </details>
              <details open>
                <summary>Open title</summary>
                <p>Visible body</p>
              </details>
              <details><p>No summary hidden</p></details>
              <details open><p>Fallback body</p></details>
            </body></html>
            "#,
        BrowserRenderOptions {
            width: 80,
            ..BrowserRenderOptions::default()
        },
    );

    assert_eq!(
        render.text,
        "> Closed title
v Open title
Visible body
> Details
v Details
Fallback body"
    );
    assert_eq!(
        render.display_list,
        vec![
            DisplayCommand::Text {
                x: 0,
                y: 0,
                text: "> Closed title".to_owned(),
            },
            DisplayCommand::Text {
                x: 0,
                y: 1,
                text: "v Open title".to_owned(),
            },
            DisplayCommand::Text {
                x: 0,
                y: 2,
                text: "Visible body".to_owned(),
            },
            DisplayCommand::Text {
                x: 0,
                y: 3,
                text: "> Details".to_owned(),
            },
            DisplayCommand::Text {
                x: 0,
                y: 4,
                text: "v Details".to_owned(),
            },
            DisplayCommand::Text {
                x: 0,
                y: 5,
                text: "Fallback body".to_owned(),
            },
        ]
    );
}

#[test]
fn renders_common_form_controls_as_visible_inline_widgets() {
    let render = render_html(
        "mem://form-controls",
        br#"
            <html><body>
              <form>
                <input name="q" value="rust browser">
                <input type="checkbox" checked>
                <input type="radio">
                <input type="password" value="secret">
                <select name="kind">
                  <option value="docs">Docs</option>
                  <option value="news" selected>News</option>
                </select>
                <textarea name="note">ship it</textarea>
                <input type="submit" value="Go">
              </form>
            </body></html>
            "#,
        BrowserRenderOptions {
            width: 120,
            ..BrowserRenderOptions::default()
        },
    );

    assert_eq!(
        render.text,
        "[rust browser] [x] ( ) [******] [News] [ship it] [Go]"
    );
    assert_eq!(
        render.display_list,
        vec![DisplayCommand::Text {
            x: 0,
            y: 0,
            text: "[rust browser] [x] ( ) [******] [News] [ship it] [Go]".to_owned(),
        }]
    );
    assert!(hit_test_target_node(&render, 1, 0).is_some());
    assert_eq!(hit_test_target_node(&render, 14, 0), None);
    assert!(hit_test_target_node(&render, 15, 0).is_some());
}
