use super::*;

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
fn document_viewport_reports_clamped_state_and_dirty_regions() {
    let mut render = render_from_display_list(
        "mem://document-viewport",
        8,
        vec![
            DisplayCommand::Text {
                x: 0,
                y: 0,
                text: "ABCDEFGH".to_owned(),
            },
            DisplayCommand::Text {
                x: 0,
                y: 3,
                text: "tail".to_owned(),
            },
        ],
    );
    render.layout_box_count = 1;
    render.layout_boxes = vec![BrowserLayoutBox {
        id: 0,
        parent: None,
        node_id: 1,
        tag: "main".to_owned(),
        kind: "block".to_owned(),
        x: 0,
        y: 0,
        width: 8,
        height: 4,
        children: Vec::new(),
        command_indices: vec![0, 1],
    }];

    let initial = browser_document_viewport(
        &render,
        BrowserViewportState {
            x: 99,
            y: 99,
            width: 4,
            height: 3,
        },
        None,
    );
    assert_eq!(
        initial.viewport,
        BrowserViewportState {
            x: 4,
            y: 1,
            width: 4,
            height: 3
        }
    );
    assert_eq!(initial.max_scroll_x, 4);
    assert_eq!(initial.max_scroll_y, 1);
    assert!(initial.full_repaint);
    assert_eq!(initial.invalidated_area, 12);
    assert_eq!(initial.reused_area, 0);

    let scrolled = browser_document_viewport(
        &render,
        BrowserViewportState {
            x: 1,
            y: 1,
            width: 4,
            height: 3,
        },
        Some(BrowserViewportState {
            x: 0,
            y: 0,
            width: 4,
            height: 3,
        }),
    );

    assert_eq!(scrolled.scroll_delta_x, 1);
    assert_eq!(scrolled.scroll_delta_y, 1);
    assert!(!scrolled.full_repaint);
    assert_eq!(
        scrolled.invalidated_regions,
        vec![
            BrowserViewportRect {
                x: 0,
                y: 2,
                width: 4,
                height: 1
            },
            BrowserViewportRect {
                x: 3,
                y: 0,
                width: 1,
                height: 2
            },
        ]
    );
    assert_eq!(scrolled.invalidated_area, 6);
    assert_eq!(scrolled.reused_area, 6);
    assert_eq!(scrolled.visible_command_count, 1);
    assert_eq!(scrolled.culled_command_count, 1);
    assert_eq!(scrolled.visible_layout_box_count, 1);
    assert_eq!(scrolled.visible_layout_boxes[0].visible_width, 4);
    assert_eq!(scrolled.visible_layout_boxes[0].visible_height, 3);
}

#[test]
fn viewport_frame_maps_dirty_regions_to_rgba_pixels() {
    let render = render_from_display_list(
        "mem://viewport-frame",
        8,
        vec![
            DisplayCommand::Text {
                x: 0,
                y: 0,
                text: "ABCDEFGH".to_owned(),
            },
            DisplayCommand::Text {
                x: 0,
                y: 3,
                text: "tail".to_owned(),
            },
        ],
    );
    let raster_options = BrowserRasterOptions {
        cell_width: 8,
        cell_height: 12,
        padding_x: 4,
        padding_y: 4,
        ..BrowserRasterOptions::default()
    };

    let initial = browser_viewport_frame(
        &render,
        BrowserViewportState {
            x: 0,
            y: 0,
            width: 4,
            height: 3,
        },
        None,
        raster_options,
    )
    .unwrap();
    assert!(initial.report.viewport.full_repaint);
    assert_eq!(initial.report.frame_width, 40);
    assert_eq!(initial.report.frame_height, 44);
    assert_eq!(
        initial.report.dirty_pixel_regions,
        vec![BrowserViewportFrameDirtyRect {
            x: 0,
            y: 0,
            width: 40,
            height: 44,
            viewport_x: 0,
            viewport_y: 0,
            viewport_width: 4,
            viewport_height: 3,
        }]
    );

    let scrolled = browser_viewport_frame(
        &render,
        BrowserViewportState {
            x: 1,
            y: 1,
            width: 4,
            height: 3,
        },
        Some(BrowserViewportState {
            x: 0,
            y: 0,
            width: 4,
            height: 3,
        }),
        raster_options,
    )
    .unwrap();

    assert!(!scrolled.report.viewport.full_repaint);
    assert_eq!(
        scrolled.report.dirty_pixel_regions,
        vec![
            BrowserViewportFrameDirtyRect {
                x: 4,
                y: 28,
                width: 32,
                height: 12,
                viewport_x: 0,
                viewport_y: 2,
                viewport_width: 4,
                viewport_height: 1,
            },
            BrowserViewportFrameDirtyRect {
                x: 28,
                y: 4,
                width: 8,
                height: 24,
                viewport_x: 3,
                viewport_y: 0,
                viewport_width: 1,
                viewport_height: 2,
            },
        ]
    );
    assert_eq!(scrolled.report.dirty_pixel_area, 576);
    assert_eq!(scrolled.report.frame.raster_viewport_x, Some(1));
    assert_eq!(scrolled.report.frame.raster_viewport_y, Some(1));
    assert_eq!(scrolled.report.frame.raster_viewport_width, Some(4));
    assert_eq!(scrolled.report.frame.raster_viewport_height, Some(3));
    assert_eq!(scrolled.raster.pixels.len(), 40 * 44 * 4);
}

#[test]
fn repeated_scroll_projection_keeps_visual_hits_and_raster_rows_aligned() {
    let image_url = "mem://viewport-scroll-projection-image".to_owned();
    let decoded = DecodedImage {
        width: 1,
        height: 1,
        pixels: vec![96],
        rgb_pixels: Some(vec![40, 136, 224]),
    };
    let render = BrowserRender {
        source: "mem://viewport-scroll-projection".to_owned(),
        title: String::new(),
        viewport_width: 12,
        dom_node_count: 0,
        css_rule_count: 0,
        layout_box_count: 0,
        layout_boxes: Vec::new(),
        paint_command_count: 5,
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
        hit_targets: vec![
            DisplayHitTarget::default(),
            DisplayHitTarget::text(vec![
                TextHitTargetRun {
                    start: 1,
                    width: 1,
                    target_node: Some(12),
                },
                TextHitTargetRun {
                    start: 6,
                    width: 1,
                    target_node: Some(77),
                },
            ]),
            DisplayHitTarget::text(vec![TextHitTargetRun {
                start: 0,
                width: "Next".len(),
                target_node: Some(88),
            }]),
            DisplayHitTarget::default(),
            DisplayHitTarget::default(),
        ],
        display_list: vec![
            DisplayCommand::Text {
                x: 0,
                y: 3,
                text: "Lead".to_owned(),
            },
            DisplayCommand::Image {
                x: 0,
                y: 4,
                width: 8,
                height: 1,
                shade: 180,
                alt: None,
                url: Some(image_url.clone()),
                decoded_width: Some(1),
                decoded_height: Some(1),
                decoded_hash: None,
            },
            DisplayCommand::StyledText {
                x: 9,
                y: 4,
                text: "Next".to_owned(),
                shade: 0,
            },
            DisplayCommand::ColorRect {
                x: 0,
                y: 5,
                width: 12,
                height: 1,
                shade: 236,
                red: 236,
                green: 244,
                blue: 248,
            },
            DisplayCommand::Text {
                x: 0,
                y: 6,
                text: "End".to_owned(),
            },
        ],
        text: "Lead\nNext\nEnd".to_owned(),
    };

    let start = BrowserViewportState {
        x: 0,
        y: 3,
        width: 12,
        height: 2,
    };
    let options = BrowserRasterOptions {
        viewport_width: Some(start.width),
        viewport_height: Some(start.height),
        ..BrowserRasterOptions::default()
    };
    let frames = browser_viewport_frame_sequence(&render, start, &[(0, 1), (0, 1)], options)
        .expect("render repeated scrolled viewport projection frames");
    assert_eq!(frames.len(), 2);
    assert_eq!(
        frames
            .iter()
            .map(|frame| frame.report.viewport.viewport.y)
            .collect::<Vec<_>>(),
        vec![4, 5],
        "repeated small scrolls should advance the viewport one row at a time"
    );
    assert_eq!(
        frames[0]
            .report
            .viewport
            .visible_commands
            .iter()
            .map(|command| (
                command.command_index,
                command.kind.as_str(),
                command.visible_y
            ))
            .collect::<Vec<_>>(),
        vec![(1, "Image", 0), (2, "StyledText", 0), (3, "ColorRect", 1),],
        "first scroll frame should project the image/link row and following row without duplicates"
    );
    assert_eq!(
        frames[1]
            .report
            .viewport
            .visible_commands
            .iter()
            .map(|command| (
                command.command_index,
                command.kind.as_str(),
                command.visible_y
            ))
            .collect::<Vec<_>>(),
        vec![(3, "ColorRect", 0), (4, "Text", 1)],
        "second scroll frame should drop the mixed row and move the following row to the top"
    );

    assert_eq!(
        hit_test_target_node_in_viewport(&render, frames[0].report.viewport.viewport, 5, 0),
        Some(77),
        "viewport click should use the visible image hit column after scroll, not an inactive exact visual column"
    );
    assert_eq!(
        hit_test_target_node_in_viewport(&render, frames[0].report.viewport.viewport, 9, 0),
        Some(88),
        "adjacent visible text link should remain clickable in the same scrolled row"
    );
    assert_ne!(
        hit_test_target_node_in_viewport(&render, frames[0].report.viewport.viewport, 5, 0),
        Some(12),
        "stale hit metadata from another image column should not win the scrolled click"
    );
    assert_eq!(
        hit_test_target_node_in_viewport(&render, frames[1].report.viewport.viewport, 5, 0),
        None,
        "image target should not remain hittable after the mixed row scrolls out"
    );
    assert_eq!(
        hit_test_target_node_in_viewport(&render, frames[1].report.viewport.viewport, 9, 0),
        None,
        "text link target should not remain hittable after the mixed row scrolls out"
    );

    let pixel = |frame: &BrowserViewportFrame, x: usize, y: usize| -> [u8; 4] {
        let index = y
            .saturating_mul(frame.raster.width)
            .saturating_add(x)
            .saturating_mul(4);
        let mut value = [0u8; 4];
        value.copy_from_slice(&frame.raster.pixels[index..index.saturating_add(4)]);
        value
    };
    let image_pixel_x = options
        .padding_x
        .saturating_add(5usize.saturating_mul(options.cell_width));
    assert_eq!(
        pixel(&frames[0], image_pixel_x, options.padding_y),
        [40, 136, 224, 255],
        "decoded image color should remain visible at the first scrolled viewport row"
    );
    assert_ne!(
        pixel(&frames[1], image_pixel_x, options.padding_y),
        [40, 136, 224, 255],
        "next scroll frame should not leave stale image pixels at the top row"
    );
    assert_eq!(
        pixel(&frames[1], options.padding_x, options.padding_y),
        [236, 244, 248, 255],
        "next scroll frame should project the following color row to the top"
    );
}

#[test]
fn repeated_after_scroll_preserves_overlay_and_lower_content_viewport_hits() {
    let image_url = "mem://viewport-overlay-scroll-image".to_owned();
    let decoded = DecodedImage {
        width: 1,
        height: 1,
        pixels: vec![96],
        rgb_pixels: Some(vec![32, 156, 208]),
    };
    let render = BrowserRender {
        source: "mem://viewport-overlay-scroll".to_owned(),
        title: String::new(),
        viewport_width: 12,
        dom_node_count: 0,
        css_rule_count: 0,
        layout_box_count: 0,
        layout_boxes: Vec::new(),
        paint_command_count: 9,
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
        hit_targets: vec![
            DisplayHitTarget::default().with_viewport_fixed(true),
            DisplayHitTarget::text(vec![TextHitTargetRun {
                start: 0,
                width: "Fixed".len(),
                target_node: Some(10),
            }])
            .with_viewport_fixed(true),
            DisplayHitTarget::default().with_viewport_sticky_top(Some(1)),
            DisplayHitTarget::text(vec![TextHitTargetRun {
                start: 0,
                width: "Sticky".len(),
                target_node: Some(20),
            }])
            .with_viewport_sticky_top(Some(1)),
            DisplayHitTarget::default(),
            DisplayHitTarget::text(vec![TextHitTargetRun {
                start: 0,
                width: "Lower".len(),
                target_node: Some(77),
            }]),
            DisplayHitTarget::node(Some(91)),
            DisplayHitTarget::default(),
            DisplayHitTarget::default(),
        ],
        display_list: vec![
            DisplayCommand::ColorRect {
                x: 0,
                y: 0,
                width: 12,
                height: 1,
                shade: 245,
                red: 245,
                green: 248,
                blue: 250,
            },
            DisplayCommand::StyledText {
                x: 0,
                y: 0,
                text: "Fixed".to_owned(),
                shade: 0,
            },
            DisplayCommand::ColorRect {
                x: 0,
                y: 2,
                width: 12,
                height: 1,
                shade: 230,
                red: 230,
                green: 238,
                blue: 244,
            },
            DisplayCommand::StyledText {
                x: 0,
                y: 2,
                text: "Sticky".to_owned(),
                shade: 0,
            },
            DisplayCommand::ColorRect {
                x: 0,
                y: 6,
                width: 12,
                height: 1,
                shade: 236,
                red: 236,
                green: 244,
                blue: 248,
            },
            DisplayCommand::StyledText {
                x: 1,
                y: 6,
                text: "Lower".to_owned(),
                shade: 0,
            },
            DisplayCommand::Image {
                x: 2,
                y: 7,
                width: 3,
                height: 1,
                shade: 128,
                alt: None,
                url: Some(image_url),
                decoded_width: Some(1),
                decoded_height: Some(1),
                decoded_hash: None,
            },
            DisplayCommand::ColorRect {
                x: 0,
                y: 8,
                width: 12,
                height: 1,
                shade: 224,
                red: 224,
                green: 232,
                blue: 240,
            },
            DisplayCommand::Text {
                x: 0,
                y: 8,
                text: "Tail".to_owned(),
            },
        ],
        text: "Fixed\nSticky\nLower\nTail".to_owned(),
    };

    let start = BrowserViewportState {
        x: 0,
        y: 3,
        width: 12,
        height: 4,
    };
    let options = BrowserRasterOptions {
        viewport_width: Some(start.width),
        viewport_height: Some(start.height),
        ..BrowserRasterOptions::default()
    };
    let frames = browser_viewport_frame_sequence(&render, start, &[(0, 1), (0, 1)], options)
        .expect("render repeated overlay/lower content scroll frames");

    assert_eq!(
        frames
            .iter()
            .map(|frame| frame.report.viewport.viewport.y)
            .collect::<Vec<_>>(),
        vec![4, 5],
        "repeated scroll should advance the viewport slice without resetting"
    );
    assert!(
        frames
            .iter()
            .all(|frame| !frame.report.viewport.full_repaint),
        "small scroll frames should preserve incremental viewport projection"
    );

    let has_visible =
        |frame: &BrowserViewportFrame, command_index: usize, kind: &str, visible_y: usize| {
            frame
                .report
                .viewport
                .visible_commands
                .iter()
                .any(|command| {
                    command.command_index == command_index
                        && command.kind == kind
                        && command.visible_y == visible_y
                })
        };
    assert!(has_visible(&frames[0], 1, "StyledText", 0));
    assert!(has_visible(&frames[0], 3, "StyledText", 1));
    assert!(has_visible(&frames[0], 5, "StyledText", 2));
    assert!(has_visible(&frames[0], 6, "Image", 3));
    assert!(has_visible(&frames[1], 1, "StyledText", 0));
    assert!(has_visible(&frames[1], 3, "StyledText", 1));
    assert!(has_visible(&frames[1], 6, "Image", 2));
    assert!(has_visible(&frames[1], 8, "Text", 3));

    assert_eq!(
        hit_test_target_node_in_viewport(&render, frames[0].report.viewport.viewport, 1, 0),
        Some(10),
        "fixed overlay link should remain pinned at the viewport top"
    );
    assert_eq!(
        hit_test_target_node_in_viewport(&render, frames[0].report.viewport.viewport, 1, 1),
        Some(20),
        "sticky overlay link should remain pinned below fixed content"
    );
    assert_eq!(
        hit_test_target_node_in_viewport(&render, frames[0].report.viewport.viewport, 1, 2),
        Some(77),
        "lower link should remain clickable below the pinned overlays"
    );
    assert_eq!(
        hit_test_target_node_in_viewport(&render, frames[0].report.viewport.viewport, 2, 3),
        Some(91),
        "lower image should remain clickable in its painted row"
    );
    assert_eq!(
        hit_test_target_node_in_viewport(&render, frames[1].report.viewport.viewport, 1, 1),
        Some(20),
        "sticky overlay should beat the scrolled lower link only on the sticky row"
    );
    assert_eq!(
        hit_test_target_node_in_viewport(&render, frames[1].report.viewport.viewport, 2, 2),
        Some(91),
        "image hit should move up one viewport row after the next scroll"
    );
    assert_eq!(
        hit_test_target_node_in_viewport(&render, frames[1].report.viewport.viewport, 1, 3),
        None,
        "stale lower link should not leak into the tail row"
    );

    let pixel = |frame: &BrowserViewportFrame, x: usize, y: usize| -> [u8; 4] {
        let index = y
            .saturating_mul(frame.raster.width)
            .saturating_add(x)
            .saturating_mul(4);
        let mut value = [0u8; 4];
        value.copy_from_slice(&frame.raster.pixels[index..index.saturating_add(4)]);
        value
    };
    let row_pixel = |column: usize, row: usize| {
        (
            options
                .padding_x
                .saturating_add(column.saturating_mul(options.cell_width)),
            options
                .padding_y
                .saturating_add(row.saturating_mul(options.cell_height)),
        )
    };
    let fixed_bg = row_pixel(11, 0);
    assert_eq!(
        pixel(&frames[0], fixed_bg.0, fixed_bg.1),
        [245, 248, 250, 255],
        "fixed overlay background should stay pinned at row zero"
    );
    let sticky_bg = row_pixel(11, 1);
    assert_eq!(
        pixel(&frames[1], sticky_bg.0, sticky_bg.1),
        [230, 238, 244, 255],
        "sticky overlay background should stay pinned at row one"
    );
    let sticky_text_x_start = options.padding_x;
    let sticky_text_x_end = sticky_text_x_start
        .saturating_add("Sticky".len().saturating_mul(options.cell_width))
        .min(frames[1].raster.width);
    let sticky_text_y_start = options.padding_y.saturating_add(options.cell_height);
    let sticky_text_y_end = sticky_text_y_start
        .saturating_add(options.cell_height)
        .min(frames[1].raster.height);
    let sticky_text_has_ink = (sticky_text_y_start..sticky_text_y_end).any(|y| {
        (sticky_text_x_start..sticky_text_x_end).any(|x| pixel(&frames[1], x, y) == [0, 0, 0, 255])
    });
    assert!(
        sticky_text_has_ink,
        "sticky overlay text should stay pinned at row one"
    );
    let lower_bg = row_pixel(0, 2);
    assert_eq!(
        pixel(&frames[0], lower_bg.0, lower_bg.1),
        [236, 244, 248, 255],
        "lower linked content should paint below fixed/sticky overlays"
    );
    let image_pixel = row_pixel(2, 3);
    assert_eq!(
        pixel(&frames[0], image_pixel.0, image_pixel.1),
        [32, 156, 208, 255],
        "decoded image color should paint in the first scrolled slice"
    );
    let moved_image_pixel = row_pixel(2, 2);
    assert_eq!(
        pixel(&frames[1], moved_image_pixel.0, moved_image_pixel.1),
        [32, 156, 208, 255],
        "decoded image color should move with the viewport crop after repeated scroll"
    );
}

#[test]
fn pinned_layers_paint_above_later_normal_content_in_text_and_grayscale_viewports() {
    let render = BrowserRender {
        source: "mem://pinned-layer-text-grayscale-scroll".to_owned(),
        title: String::new(),
        viewport_width: 12,
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
        hit_targets: vec![
            DisplayHitTarget::node(Some(10)).with_viewport_fixed(true),
            DisplayHitTarget::text(vec![TextHitTargetRun {
                start: 0,
                width: "Nav".len(),
                target_node: Some(11),
            }])
            .with_viewport_fixed(true),
            DisplayHitTarget::node(Some(99)),
            DisplayHitTarget::default(),
        ],
        display_list: vec![
            DisplayCommand::Rect {
                x: 0,
                y: 0,
                width: 12,
                height: 1,
                shade: 42,
            },
            DisplayCommand::Text {
                x: 0,
                y: 0,
                text: "Nav".to_owned(),
            },
            DisplayCommand::Image {
                x: 0,
                y: 3,
                width: 12,
                height: 1,
                shade: 180,
                alt: Some("normal".to_owned()),
                url: None,
                decoded_width: None,
                decoded_height: None,
                decoded_hash: None,
            },
            DisplayCommand::Text {
                x: 0,
                y: 4,
                text: "Body".to_owned(),
            },
        ],
        text: "Nav\nnormal\nBody".to_owned(),
    };
    let viewport = BrowserViewportState {
        x: 0,
        y: 3,
        width: 12,
        height: 2,
    };

    let text = browser_text_viewport(
        &render,
        BrowserTextViewportOptions {
            y: viewport.y,
            width: viewport.width,
            height: viewport.height,
            ..BrowserTextViewportOptions::default()
        },
    );
    assert!(
        text.lines
            .first()
            .is_some_and(|line| line.starts_with("Nav")),
        "fixed text should stay readable above later normal media in the scrolled text viewport"
    );
    assert!(
        text.lines
            .first()
            .and_then(|line| line.chars().nth(8))
            .is_some_and(|ch| ch == '#'),
        "fixed underlay should cover later normal media outside text glyphs"
    );
    assert!(
        text.lines
            .first()
            .is_some_and(|line| line.chars().skip("Nav".len()).all(|ch| ch == '#')),
        "fixed underlay should represent the full pinned overlay row outside text glyphs"
    );

    let raster_options = BrowserRasterOptions {
        viewport_y: Some(viewport.y),
        viewport_width: Some(viewport.width),
        viewport_height: Some(viewport.height),
        ..BrowserRasterOptions::default()
    };
    let raster =
        rasterize_render(&render, raster_options).expect("rasterize pinned grayscale viewport");
    let sample_x = raster_options
        .padding_x
        .saturating_add(8usize.saturating_mul(raster_options.cell_width));
    let sample = raster_options
        .padding_y
        .saturating_mul(raster.width)
        .saturating_add(sample_x);
    assert_eq!(
        raster.pixels[sample], 42,
        "fixed underlay should paint over later normal media in grayscale scroll output"
    );
    assert_eq!(
        hit_test_target_node_in_viewport(&render, viewport, 8, 0),
        Some(10),
        "fixed hit target should stay aligned with the pinned visual layer"
    );
}

#[test]
fn successive_scroll_dirty_regions_merge_without_stale_mixed_raster_hits() {
    let mut dirty = vec![
        BrowserViewportFrameDirtyRect {
            x: 0,
            y: 0,
            width: 8,
            height: 12,
            viewport_x: 0,
            viewport_y: 0,
            viewport_width: 1,
            viewport_height: 1,
        },
        BrowserViewportFrameDirtyRect {
            x: 8,
            y: 0,
            width: 8,
            height: 24,
            viewport_x: 1,
            viewport_y: 0,
            viewport_width: 1,
            viewport_height: 2,
        },
        BrowserViewportFrameDirtyRect {
            x: 0,
            y: 12,
            width: 8,
            height: 12,
            viewport_x: 0,
            viewport_y: 1,
            viewport_width: 1,
            viewport_height: 1,
        },
    ];
    canonicalize_browser_viewport_frame_dirty_regions(&mut dirty);
    assert_eq!(
        dirty,
        vec![BrowserViewportFrameDirtyRect {
            x: 0,
            y: 0,
            width: 16,
            height: 24,
            viewport_x: 0,
            viewport_y: 0,
            viewport_width: 2,
            viewport_height: 2,
        }],
        "dirty-region canonicalization should merge adjacent rectangles after vertical merging creates a wider equal-height band"
    );

    let image_url = "mem://smooth-scroll-raster-continuity-image".to_owned();
    let decoded = DecodedImage {
        width: 1,
        height: 1,
        pixels: vec![96],
        rgb_pixels: Some(vec![48, 140, 220]),
    };
    let render = BrowserRender {
        source: "mem://smooth-scroll-raster-continuity".to_owned(),
        title: String::new(),
        viewport_width: 14,
        dom_node_count: 0,
        css_rule_count: 0,
        layout_box_count: 0,
        layout_boxes: Vec::new(),
        paint_command_count: 7,
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
        hit_targets: vec![
            DisplayHitTarget::default(),
            DisplayHitTarget::node(Some(41)),
            DisplayHitTarget::text(vec![TextHitTargetRun {
                start: 0,
                width: "Next".len(),
                target_node: Some(77),
            }]),
            DisplayHitTarget::default(),
            DisplayHitTarget::default(),
            DisplayHitTarget::node(Some(91)),
            DisplayHitTarget::default(),
        ],
        display_list: vec![
            DisplayCommand::Text {
                x: 0,
                y: 2,
                text: "Lead".to_owned(),
            },
            DisplayCommand::Image {
                x: 1,
                y: 3,
                width: 2,
                height: 1,
                shade: 180,
                alt: None,
                url: Some(image_url.clone()),
                decoded_width: Some(1),
                decoded_height: Some(1),
                decoded_hash: None,
            },
            DisplayCommand::StyledText {
                x: 5,
                y: 3,
                text: "Next".to_owned(),
                shade: 0,
            },
            DisplayCommand::ColorRect {
                x: 0,
                y: 4,
                width: 14,
                height: 1,
                shade: 236,
                red: 236,
                green: 244,
                blue: 248,
            },
            DisplayCommand::Text {
                x: 0,
                y: 5,
                text: "Body".to_owned(),
            },
            DisplayCommand::Image {
                x: 2,
                y: 5,
                width: 2,
                height: 1,
                shade: 180,
                alt: None,
                url: Some(image_url),
                decoded_width: Some(1),
                decoded_height: Some(1),
                decoded_hash: None,
            },
            DisplayCommand::Text {
                x: 0,
                y: 6,
                text: "Tail".to_owned(),
            },
        ],
        text: "Lead\nNext\nBody\nTail".to_owned(),
    };

    let start = BrowserViewportState {
        x: 0,
        y: 2,
        width: 14,
        height: 3,
    };
    let options = BrowserRasterOptions {
        viewport_width: Some(start.width),
        viewport_height: Some(start.height),
        ..BrowserRasterOptions::default()
    };
    let frames =
        browser_viewport_frame_sequence(&render, start, &[(0, 1), (0, 1), (0, 1)], options)
            .expect("render smooth scroll raster continuity frames");
    assert_eq!(
        frames
            .iter()
            .map(|frame| frame.report.viewport.viewport.y)
            .collect::<Vec<_>>(),
        vec![3, 4, 4],
        "successive small scrolls should move one row at a time and then clamp"
    );
    assert_eq!(
        frames
            .iter()
            .map(|frame| frame.report.viewport.scroll_delta_y)
            .collect::<Vec<_>>(),
        vec![1, 1, 0],
        "scroll deltas should reflect actual clamped viewport movement"
    );
    assert_eq!(
        frames[0]
            .report
            .viewport
            .visible_commands
            .iter()
            .map(|command| (
                command.command_index,
                command.kind.as_str(),
                command.visible_y
            ))
            .collect::<Vec<_>>(),
        vec![
            (1, "Image", 0),
            (2, "StyledText", 0),
            (3, "ColorRect", 1),
            (4, "Text", 2),
            (5, "Image", 2),
        ],
        "first scroll frame should expose image/link and following visible rows without stale duplication"
    );
    assert_eq!(
        frames[1]
            .report
            .viewport
            .visible_commands
            .iter()
            .map(|command| (
                command.command_index,
                command.kind.as_str(),
                command.visible_y
            ))
            .collect::<Vec<_>>(),
        vec![
            (3, "ColorRect", 0),
            (4, "Text", 1),
            (5, "Image", 1),
            (6, "Text", 2),
        ],
        "second scroll frame should drop stale mixed media/link rows"
    );
    assert!(
        frames[0].report.reused_pixel_area > 0 && frames[1].report.reused_pixel_area > 0,
        "incremental scroll frames should preserve reusable raster area"
    );
    assert!(
        frames[2].report.dirty_pixel_regions.is_empty(),
        "clamped no-op scroll should not report new dirty rows even when media remains visible"
    );

    assert_eq!(
        hit_test_target_node_in_viewport(&render, frames[0].report.viewport.viewport, 1, 0),
        Some(41),
        "visible decoded image should remain clickable in the first scrolled frame"
    );
    assert_eq!(
        hit_test_target_node_in_viewport(&render, frames[0].report.viewport.viewport, 5, 0),
        Some(77),
        "visible text link should remain clickable in the first scrolled frame"
    );
    assert_eq!(
        hit_test_target_node_in_viewport(&render, frames[1].report.viewport.viewport, 1, 0),
        None,
        "decoded image target should not remain hittable after it scrolls out"
    );
    assert_eq!(
        hit_test_target_node_in_viewport(&render, frames[1].report.viewport.viewport, 5, 0),
        None,
        "text link target should not remain hittable after it scrolls out"
    );
    assert_eq!(
        hit_test_target_node_in_viewport(&render, frames[1].report.viewport.viewport, 2, 1),
        Some(91),
        "still-visible lower image target should remain clickable after the mixed row scrolls out"
    );
    assert_eq!(
        hit_test_target_node_in_viewport(&render, frames[2].report.viewport.viewport, 2, 1),
        Some(91),
        "clamped no-op scroll should keep the visible image hit stable"
    );

    let pixel = |frame: &BrowserViewportFrame, x: usize, y: usize| -> [u8; 4] {
        let index = y
            .saturating_mul(frame.raster.width)
            .saturating_add(x)
            .saturating_mul(4);
        let mut value = [0u8; 4];
        value.copy_from_slice(&frame.raster.pixels[index..index.saturating_add(4)]);
        value
    };
    let image_x = options.padding_x.saturating_add(options.cell_width);
    assert_eq!(
        pixel(&frames[0], image_x, options.padding_y),
        [48, 140, 220, 255],
        "first scroll frame should paint decoded image color in the visible row"
    );
    assert_ne!(
        pixel(&frames[1], image_x, options.padding_y),
        [48, 140, 220, 255],
        "next scroll frame should not retain stale image color in the top row"
    );
    assert_eq!(
        pixel(&frames[1], options.padding_x, options.padding_y),
        [236, 244, 248, 255],
        "next scroll frame should move the following color row to the top"
    );
    let lower_image_x = options
        .padding_x
        .saturating_add(2usize.saturating_mul(options.cell_width));
    let lower_image_y = options.padding_y.saturating_add(options.cell_height);
    assert_eq!(
        pixel(&frames[1], lower_image_x, lower_image_y),
        [48, 140, 220, 255],
        "second scroll frame should keep visible lower decoded image color aligned"
    );
    assert_eq!(
        pixel(&frames[2], lower_image_x, lower_image_y),
        [48, 140, 220, 255],
        "clamped no-op scroll should preserve visible decoded image color"
    );
}

#[test]
fn continuous_scroll_reports_moved_clamped_and_rerendered_viewports() {
    let image_url = "mem://continuous-scroll-transition-image".to_owned();
    let decoded = DecodedImage {
        width: 1,
        height: 1,
        pixels: vec![128],
        rgb_pixels: Some(vec![208, 84, 52]),
    };
    let render = BrowserRender {
        source: "mem://continuous-scroll-transition".to_owned(),
        title: String::new(),
        viewport_width: 12,
        dom_node_count: 0,
        css_rule_count: 0,
        layout_box_count: 0,
        layout_boxes: Vec::new(),
        paint_command_count: 7,
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
        hit_targets: vec![
            DisplayHitTarget::default(),
            DisplayHitTarget::node(Some(41)),
            DisplayHitTarget::text(vec![TextHitTargetRun {
                start: 0,
                width: "Link".len(),
                target_node: Some(77),
            }]),
            DisplayHitTarget::default(),
            DisplayHitTarget::default(),
            DisplayHitTarget::node(Some(91)),
            DisplayHitTarget::default(),
        ],
        display_list: vec![
            DisplayCommand::Text {
                x: 0,
                y: 0,
                text: "Head".to_owned(),
            },
            DisplayCommand::Image {
                x: 1,
                y: 2,
                width: 2,
                height: 1,
                shade: 128,
                alt: None,
                url: Some(image_url.clone()),
                decoded_width: Some(1),
                decoded_height: Some(1),
                decoded_hash: None,
            },
            DisplayCommand::StyledText {
                x: 4,
                y: 2,
                text: "Link".to_owned(),
                shade: 0,
            },
            DisplayCommand::ColorRect {
                x: 0,
                y: 3,
                width: 12,
                height: 1,
                shade: 232,
                red: 232,
                green: 240,
                blue: 248,
            },
            DisplayCommand::Text {
                x: 0,
                y: 4,
                text: "Middle".to_owned(),
            },
            DisplayCommand::Image {
                x: 2,
                y: 5,
                width: 2,
                height: 1,
                shade: 128,
                alt: None,
                url: Some(image_url),
                decoded_width: Some(1),
                decoded_height: Some(1),
                decoded_hash: None,
            },
            DisplayCommand::Text {
                x: 0,
                y: 6,
                text: "Tail".to_owned(),
            },
        ],
        text: "Head\nLink\nMiddle\nTail".to_owned(),
    };

    let start = BrowserViewportState {
        x: 0,
        y: 1,
        width: 12,
        height: 3,
    };
    let options = BrowserRasterOptions {
        viewport_width: Some(start.width),
        viewport_height: Some(start.height),
        ..BrowserRasterOptions::default()
    };
    let frames = browser_viewport_frame_sequence(
        &render,
        start,
        &[(0, -1), (0, -1), (0, 1), (0, 1), (0, 1), (0, 1), (0, 1)],
        options,
    )
    .expect("render continuous viewport scroll transition frames");

    assert_eq!(
        frames
            .iter()
            .map(|frame| frame.report.viewport.viewport.y)
            .collect::<Vec<_>>(),
        vec![0, 0, 1, 2, 3, 4, 4],
        "small scroll deltas should move monotonically and clamp at both document edges"
    );
    assert_eq!(
        frames
            .iter()
            .map(|frame| frame.report.viewport.transition)
            .collect::<Vec<_>>(),
        vec![
            BrowserViewportTransition::Moved,
            BrowserViewportTransition::ClampedNoop,
            BrowserViewportTransition::Moved,
            BrowserViewportTransition::Moved,
            BrowserViewportTransition::Moved,
            BrowserViewportTransition::Moved,
            BrowserViewportTransition::ClampedNoop,
        ],
        "viewport reports should distinguish moved frames from clamped no-op scrolls"
    );
    assert_eq!(
        frames
            .iter()
            .map(|frame| frame.report.viewport.scroll_delta_y)
            .collect::<Vec<_>>(),
        vec![-1, 0, 1, 1, 1, 1, 0],
        "reported scroll deltas should reflect the actual clamped viewport movement"
    );
    assert!(
        frames[1].report.dirty_pixel_regions.is_empty()
            && frames[6].report.dirty_pixel_regions.is_empty(),
        "top and bottom clamped no-op scrolls should not mark stale raster rows dirty"
    );

    let pixel = |frame: &BrowserViewportFrame, x: usize, y: usize| -> [u8; 4] {
        let index = y
            .saturating_mul(frame.raster.width)
            .saturating_add(x)
            .saturating_mul(4);
        let mut value = [0u8; 4];
        value.copy_from_slice(&frame.raster.pixels[index..index.saturating_add(4)]);
        value
    };
    let first_image_x = options.padding_x.saturating_add(options.cell_width);
    assert_eq!(
        pixel(&frames[3], first_image_x, options.padding_y),
        [208, 84, 52, 255],
        "decoded image color should move one viewport row at a time with the raster slice"
    );
    assert_ne!(
        pixel(&frames[4], first_image_x, options.padding_y),
        [208, 84, 52, 255],
        "after the upper image scrolls away, its color should not remain stale in the top row"
    );
    assert_eq!(
        hit_test_target_node_in_viewport(&render, frames[3].report.viewport.viewport, 1, 0),
        Some(41),
        "partially scrolled visible image target should remain hittable"
    );
    assert_eq!(
        hit_test_target_node_in_viewport(&render, frames[4].report.viewport.viewport, 1, 0),
        None,
        "stale upper image target should disappear after it leaves the viewport"
    );
    assert_eq!(
        hit_test_target_node_in_viewport(&render, frames[5].report.viewport.viewport, 2, 1),
        Some(91),
        "lower decoded image target should remain aligned near the bottom clamp"
    );

    let rerender = browser_viewport_frame(
        &render,
        frames[5].report.viewport.viewport,
        Some(frames[5].report.viewport.viewport),
        options,
    )
    .expect("render explicit same-viewport rerender frame");
    assert_eq!(
        rerender.report.viewport.transition,
        BrowserViewportTransition::ExplicitRerender,
        "same requested and clamped viewport should be reported as an explicit rerender"
    );
    assert!(
        rerender.report.dirty_pixel_area > 0,
        "explicit same-viewport rerender should refresh visible media/control rows without masquerading as scroll movement"
    );
}

#[test]
fn continuous_wheel_and_page_scroll_clamps_without_resetting_visible_crop() {
    let image_url = "mem://continuous-wheel-scroll-image".to_owned();
    let decoded = DecodedImage {
        width: 1,
        height: 1,
        pixels: vec![144],
        rgb_pixels: Some(vec![64, 148, 224]),
    };
    let render = BrowserRender {
        source: "mem://continuous-wheel-scroll".to_owned(),
        title: String::new(),
        viewport_width: 10,
        dom_node_count: 0,
        css_rule_count: 0,
        layout_box_count: 0,
        layout_boxes: Vec::new(),
        paint_command_count: 6,
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
        hit_targets: vec![
            DisplayHitTarget::default(),
            DisplayHitTarget::node(Some(41)),
            DisplayHitTarget::text(vec![TextHitTargetRun {
                start: 0,
                width: 4,
                target_node: Some(77),
            }]),
            DisplayHitTarget::default(),
            DisplayHitTarget::node(Some(91)),
            DisplayHitTarget::default(),
        ],
        display_list: vec![
            DisplayCommand::Text {
                x: 0,
                y: 0,
                text: "Intro".to_owned(),
            },
            DisplayCommand::Image {
                x: 1,
                y: 1,
                width: 2,
                height: 1,
                shade: 144,
                alt: None,
                url: Some(image_url.clone()),
                decoded_width: Some(1),
                decoded_height: Some(1),
                decoded_hash: None,
            },
            DisplayCommand::StyledText {
                x: 4,
                y: 1,
                text: "Link".to_owned(),
                shade: 0,
            },
            DisplayCommand::Text {
                x: 0,
                y: 2,
                text: "Body".to_owned(),
            },
            DisplayCommand::Image {
                x: 2,
                y: 4,
                width: 2,
                height: 1,
                shade: 144,
                alt: None,
                url: Some(image_url),
                decoded_width: Some(1),
                decoded_height: Some(1),
                decoded_hash: None,
            },
            DisplayCommand::Text {
                x: 0,
                y: 5,
                text: "Tail".to_owned(),
            },
        ],
        text: "Intro\nLink\nBody\nTail".to_owned(),
    };

    let start = BrowserViewportState {
        x: 0,
        y: 0,
        width: 10,
        height: 3,
    };
    let top_page = browser_document_viewport_after_page_scroll(&render, start, 0, -1);
    assert_eq!(top_page.viewport.y, 0);
    assert_eq!(top_page.scroll_delta_y, 0);
    assert_eq!(top_page.transition, BrowserViewportTransition::ClampedNoop);
    assert!(top_page.invalidated_regions.is_empty());
    assert_eq!(
        top_page.reused_area,
        top_page.viewport.width * top_page.viewport.height
    );

    let options = BrowserRasterOptions {
        viewport_width: Some(start.width),
        viewport_height: Some(start.height),
        ..BrowserRasterOptions::default()
    };
    let frames = browser_viewport_frame_sequence(
        &render,
        start,
        &[(0, 1), (0, 1), (0, -1), (0, 1), (0, 1), (0, 1)],
        options,
    )
    .expect("render continuous wheel scroll crop frames");
    assert_eq!(
        frames
            .iter()
            .map(|frame| frame.report.viewport.viewport.y)
            .collect::<Vec<_>>(),
        vec![1, 2, 1, 2, 3, 3],
        "row-sized wheel deltas should move the viewport one crop at a time and clamp at bottom"
    );
    assert_eq!(
        frames
            .iter()
            .map(|frame| frame.report.viewport.transition)
            .collect::<Vec<_>>(),
        vec![
            BrowserViewportTransition::Moved,
            BrowserViewportTransition::Moved,
            BrowserViewportTransition::Moved,
            BrowserViewportTransition::Moved,
            BrowserViewportTransition::Moved,
            BrowserViewportTransition::ClampedNoop,
        ]
    );
    assert!(frames[5].report.dirty_pixel_regions.is_empty());

    assert_eq!(
        hit_test_target_node_in_viewport(&render, frames[0].report.viewport.viewport, 1, 0),
        Some(41),
        "decoded image target should be hittable when its row is visible after a small scroll"
    );
    assert_eq!(
        hit_test_target_node_in_viewport(&render, frames[0].report.viewport.viewport, 4, 0),
        Some(77),
        "link target beside the decoded image should remain aligned after a small scroll"
    );
    assert_eq!(
        hit_test_target_node_in_viewport(&render, frames[1].report.viewport.viewport, 4, 0),
        None,
        "stale link target should disappear after the mixed row scrolls out"
    );
    assert_eq!(
        hit_test_target_node_in_viewport(&render, frames[4].report.viewport.viewport, 2, 1),
        Some(91),
        "lower decoded image target should remain aligned near the bottom clamp"
    );

    let pixel = |frame: &BrowserViewportFrame, x: usize, y: usize| -> [u8; 4] {
        let index = y
            .saturating_mul(frame.raster.width)
            .saturating_add(x)
            .saturating_mul(4);
        let mut value = [0u8; 4];
        value.copy_from_slice(&frame.raster.pixels[index..index.saturating_add(4)]);
        value
    };
    let image_x = options.padding_x.saturating_add(options.cell_width);
    assert_eq!(
        pixel(&frames[0], image_x, options.padding_y),
        [64, 148, 224, 255],
        "first wheel scroll should keep decoded image color in the visible top crop row"
    );
    assert_ne!(
        pixel(&frames[1], image_x, options.padding_y),
        [64, 148, 224, 255],
        "next wheel scroll should not leave stale decoded image color in the top crop row"
    );
    let lower_image_x = options
        .padding_x
        .saturating_add(2usize.saturating_mul(options.cell_width));
    let lower_image_y = options.padding_y.saturating_add(options.cell_height);
    assert_eq!(
        pixel(&frames[4], lower_image_x, lower_image_y),
        [64, 148, 224, 255],
        "bottom-adjacent scroll should keep lower decoded image color aligned"
    );
    assert_eq!(
        pixel(&frames[5], lower_image_x, lower_image_y),
        [64, 148, 224, 255],
        "bottom clamped no-op should preserve the already visible crop"
    );
}

#[test]
fn key_page_home_end_scroll_preserves_mixed_viewport_hits_and_raster() {
    let image_url = "mem://key-page-scroll-image".to_owned();
    let decoded = DecodedImage {
        width: 1,
        height: 1,
        pixels: vec![121],
        rgb_pixels: Some(vec![76, 152, 228]),
    };
    let render = BrowserRender {
        source: "mem://key-page-scroll-hit-parity".to_owned(),
        title: String::new(),
        viewport_width: 16,
        dom_node_count: 0,
        css_rule_count: 0,
        layout_box_count: 1,
        layout_boxes: vec![BrowserLayoutBox {
            id: 0,
            parent: None,
            node_id: 12,
            tag: "input".to_owned(),
            kind: "form-control".to_owned(),
            x: 4,
            y: 3,
            width: 4,
            height: 1,
            children: Vec::new(),
            command_indices: vec![3],
        }],
        paint_command_count: 9,
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
        hit_targets: vec![
            DisplayHitTarget::default(),
            DisplayHitTarget::default(),
            DisplayHitTarget::node(Some(41)),
            DisplayHitTarget::node(Some(12)),
            DisplayHitTarget::text(vec![TextHitTargetRun {
                start: 0,
                width: "Next".len(),
                target_node: Some(77),
            }]),
            DisplayHitTarget::default(),
            DisplayHitTarget::default(),
            DisplayHitTarget::default(),
            DisplayHitTarget::default(),
        ],
        display_list: vec![
            DisplayCommand::Text {
                x: 0,
                y: 0,
                text: "Top".to_owned(),
            },
            DisplayCommand::Text {
                x: 0,
                y: 1,
                text: "Lead".to_owned(),
            },
            DisplayCommand::Image {
                x: 1,
                y: 2,
                width: 2,
                height: 1,
                shade: 180,
                alt: None,
                url: Some(image_url),
                decoded_width: Some(1),
                decoded_height: Some(1),
                decoded_hash: None,
            },
            DisplayCommand::Rect {
                x: 4,
                y: 3,
                width: 4,
                height: 1,
                shade: INLINE_WIDGET_BORDER_SHADE,
            },
            DisplayCommand::StyledText {
                x: 6,
                y: 4,
                text: "Next".to_owned(),
                shade: 0,
            },
            DisplayCommand::ColorRect {
                x: 0,
                y: 5,
                width: 16,
                height: 1,
                shade: 236,
                red: 236,
                green: 244,
                blue: 248,
            },
            DisplayCommand::Text {
                x: 0,
                y: 6,
                text: "Bottom".to_owned(),
            },
            DisplayCommand::Text {
                x: 0,
                y: 7,
                text: "Tail".to_owned(),
            },
            DisplayCommand::Text {
                x: 0,
                y: 8,
                text: "End".to_owned(),
            },
        ],
        text: "Top\nLead\nNext\nBottom\nTail\nEnd".to_owned(),
    };

    let start = BrowserViewportState {
        x: 0,
        y: 0,
        width: 16,
        height: 3,
    };
    let page_down = browser_document_viewport_after_key_scroll(
        &render,
        start,
        BrowserViewportKeyScroll::PageDown,
    );
    assert_eq!(page_down.viewport.y, 2);
    assert_eq!(page_down.scroll_delta_y, 2);
    assert_eq!(page_down.transition, BrowserViewportTransition::Moved);
    assert_eq!(
        page_down
            .visible_commands
            .iter()
            .map(|command| (
                command.command_index,
                command.kind.as_str(),
                command.visible_y
            ))
            .collect::<Vec<_>>(),
        vec![(2, "Image", 0), (3, "Rect", 1), (4, "StyledText", 2)],
        "PageDown should land on the same image/control/link rows that the viewport paints"
    );
    assert_eq!(
        hit_test_target_node_in_viewport(&render, page_down.viewport, 1, 0),
        Some(41),
        "PageDown decoded image hit should align with the painted image row"
    );
    assert_eq!(
        hit_test_target_node_in_viewport(&render, page_down.viewport, 4, 1),
        Some(12),
        "PageDown form hit should align with the painted control row"
    );
    assert_eq!(
        hit_test_target_node_in_viewport(&render, page_down.viewport, 6, 2),
        Some(77),
        "PageDown link hit should align with the painted link row"
    );

    let options = BrowserRasterOptions {
        viewport_width: Some(start.width),
        viewport_height: Some(start.height),
        ..BrowserRasterOptions::default()
    };
    let page_frame =
        browser_viewport_frame(&render, page_down.viewport, page_down.previous, options)
            .expect("render PageDown key-scroll frame");
    let pixel = |frame: &BrowserViewportFrame, x: usize, y: usize| -> [u8; 4] {
        let index = y
            .saturating_mul(frame.raster.width)
            .saturating_add(x)
            .saturating_mul(4);
        let mut rgba = [0u8; 4];
        rgba.copy_from_slice(&frame.raster.pixels[index..index.saturating_add(4)]);
        rgba
    };
    let image_x = options.padding_x.saturating_add(options.cell_width);
    assert_eq!(
        pixel(&page_frame, image_x, options.padding_y),
        [76, 152, 228, 255],
        "PageDown raster should preserve decoded image color in the hit-tested row"
    );

    let home = browser_document_viewport_after_key_scroll(
        &render,
        page_down.viewport,
        BrowserViewportKeyScroll::Home,
    );
    assert_eq!(home.viewport.y, 0);
    assert_eq!(home.scroll_delta_y, -2);
    assert_eq!(
        hit_test_target_node_in_viewport(&render, home.viewport, 1, 0),
        None,
        "Home should drop stale image hits from the previously scrolled viewport row"
    );

    let end = browser_document_viewport_after_key_scroll(
        &render,
        page_down.viewport,
        BrowserViewportKeyScroll::End,
    );
    assert_eq!(end.viewport.y, 6);
    assert_eq!(end.scroll_delta_y, 4);
    assert_eq!(end.max_scroll_y, 6);
    assert_eq!(
        hit_test_target_node_in_viewport(&render, end.viewport, 1, 0),
        None,
        "End should not keep stale image hits after the mixed row scrolls out"
    );
    assert_eq!(
        hit_test_target_node_in_viewport(&render, end.viewport, 4, 0),
        None,
        "End should not keep stale form hits after the mixed row scrolls out"
    );
    assert_eq!(
        hit_test_target_node_in_viewport(&render, end.viewport, 6, 0),
        None,
        "End should not keep stale link hits after the mixed row scrolls out"
    );

    let bottom_page = browser_document_viewport_after_key_scroll(
        &render,
        end.viewport,
        BrowserViewportKeyScroll::PageDown,
    );
    assert_eq!(bottom_page.viewport, end.viewport);
    assert_eq!(bottom_page.scroll_delta_y, 0);
    assert_eq!(
        bottom_page.transition,
        BrowserViewportTransition::ClampedNoop
    );
    assert!(
        bottom_page.invalidated_regions.is_empty(),
        "PageDown at the bottom should not invent dirty rows"
    );
}

#[test]
fn scrolled_visible_commands_report_pinned_layers_in_paint_order() {
    let render = BrowserRender {
        source: "mem://visible-command-paint-order".to_owned(),
        title: String::new(),
        viewport_width: 10,
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
        hit_targets: vec![
            DisplayHitTarget::node(Some(10)).with_viewport_fixed(true),
            DisplayHitTarget::default(),
            DisplayHitTarget::node(Some(41)),
            DisplayHitTarget::text(vec![TextHitTargetRun {
                start: 0,
                width: "Open".len(),
                target_node: Some(77),
            }]),
        ],
        display_list: vec![
            DisplayCommand::ColorRect {
                x: 0,
                y: 0,
                width: 10,
                height: 1,
                shade: 42,
                red: 42,
                green: 42,
                blue: 42,
            },
            DisplayCommand::Text {
                x: 0,
                y: 2,
                text: "Lead".to_owned(),
            },
            DisplayCommand::Image {
                x: 0,
                y: 3,
                width: 10,
                height: 1,
                shade: 180,
                alt: None,
                url: None,
                decoded_width: None,
                decoded_height: None,
                decoded_hash: None,
            },
            DisplayCommand::StyledText {
                x: 1,
                y: 3,
                text: "Open".to_owned(),
                shade: 0,
            },
        ],
        text: "Lead\nOpen".to_owned(),
    };
    let viewport = browser_document_viewport(
        &render,
        BrowserViewportState {
            x: 0,
            y: 3,
            width: 10,
            height: 1,
        },
        None,
    );
    assert_eq!(viewport.viewport.y, 3);
    assert_eq!(
        viewport
            .visible_commands
            .iter()
            .map(|command| command.command_index)
            .collect::<Vec<_>>(),
        vec![2, 3, 0],
        "visible command report should match viewport paint order with pinned layers last"
    );
    assert_eq!(
        viewport
            .visible_commands
            .last()
            .map(|command| (command.kind.as_str(), command.visible_y)),
        Some(("ColorRect", 0)),
        "fixed visual layer should report as the topmost painted viewport row"
    );
    assert_eq!(
        hit_test_target_node_in_viewport(&render, viewport.viewport, 8, 0),
        Some(10),
        "fixed painted layer should keep the topmost hit target aligned with the report order"
    );

    let raster_options = BrowserRasterOptions {
        viewport_y: Some(viewport.viewport.y),
        viewport_width: Some(viewport.viewport.width),
        viewport_height: Some(viewport.viewport.height),
        ..BrowserRasterOptions::default()
    };
    let rgba = rasterize_render_rgba(&render, raster_options)
        .expect("rasterize visible command paint order viewport");
    let sample_x = raster_options
        .padding_x
        .saturating_add(8usize.saturating_mul(raster_options.cell_width));
    let index = raster_options
        .padding_y
        .saturating_mul(rgba.width)
        .saturating_add(sample_x)
        .saturating_mul(4);
    assert_eq!(
        &rgba.pixels[index..index.saturating_add(4)],
        &[42, 42, 42, 255],
        "raster should show the same pinned layer reported last"
    );
}
