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
