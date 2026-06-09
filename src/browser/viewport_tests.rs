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
                url: Some(image_url),
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
