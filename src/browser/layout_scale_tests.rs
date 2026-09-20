use super::*;

#[test]
fn css_pixel_scale_tracks_browser_raster_cell_width_for_centered_columns() {
    let render = render_html(
        "mem://browser-css-pixel-scale-column",
        br#"
            <html><head><style>
              .article { max-width: 640px; margin-inline: auto; background: rgb(240,240,240); }
            </style></head><body>
              <section class="article"><h1>Jamaica</h1><p>Column width tracks CSS pixels.</p></section>
            </body></html>
            "#,
        BrowserRenderOptions {
            width: 100,
            css_cell_width_px: 12,
            css_cell_height_px: 18,
            ..BrowserRenderOptions::default()
        },
    );
    let (x, width, height) = render
        .display_list
        .iter()
        .find_map(|command| match command {
            DisplayCommand::Rect {
                x,
                width,
                height,
                shade,
                ..
            } if *shade == 240 => Some((*x, *width, *height)),
            _ => None,
        })
        .expect("article background should paint");
    assert_eq!((x, width), (23, 54));
    assert!(height >= 2);
}

#[test]
fn flex_row_item_max_width_constrains_block_children() {
    let render = render_html(
        "mem://browser-css-pixel-scale-flex-row-item",
        br#"
            <html><head><style>
              .main { display: flex; justify-content: center; width: 100%; }
              .article { width: 100%; max-width: 640px; }
              .media { width: 100%; height: 180px; background: rgb(20,20,20); }
              .caption { background: rgb(240,240,240); }
            </style></head><body>
              <section class="main"><article class="article"><div class="media"></div><p class="caption">Jamaica caption</p></article></section>
            </body></html>
            "#,
        BrowserRenderOptions {
            width: 100,
            css_cell_width_px: 12,
            css_cell_height_px: 18,
            ..BrowserRenderOptions::default()
        },
    );
    let media = render
        .display_list
        .iter()
        .find_map(|command| match command {
            DisplayCommand::Rect {
                x,
                width,
                height,
                shade,
                ..
            } if *shade == 20 => Some((*x, *width, *height)),
            _ => None,
        })
        .expect("media background should paint inside centered article");
    let caption = render
        .display_list
        .iter()
        .find_map(|command| match command {
            DisplayCommand::Rect {
                x, width, shade, ..
            } if *shade == 240 => Some((*x, *width)),
            _ => None,
        })
        .expect("caption background should paint inside centered article");

    assert_eq!(media.0, 23);
    assert_eq!(media.1, 54);
    assert!(media.2 >= 10);
    assert_eq!(caption, (23, 54));
}

#[test]
fn flex_row_shrinks_full_width_main_column_before_fixed_sidebar() {
    let render = render_html(
        "mem://browser-css-pixel-scale-flex-shrink-main-sidebar",
        br#"
            <html><head><style>
              .article { display: flex; width: 100%; }
              .main { display: flex; justify-content: center; min-width: 0; width: 100%; }
              .wrapper { max-width: 660px; padding: 12px; width: 100%; }
              .media { background: rgb(20,20,20); height: 180px; width: 100%; }
              .sidebar { background: rgb(120,120,120); margin-right: 12px; min-width: 300px; width: 300px; }
            </style></head><body>
              <section class="article">
                <main class="main"><article class="wrapper"><div class="media"></div><p>Jamaica caption</p></article></main>
                <aside class="sidebar">Advertisement</aside>
              </section>
            </body></html>
            "#,
        BrowserRenderOptions {
            width: 100,
            css_cell_width_px: 12,
            css_cell_height_px: 18,
            ..BrowserRenderOptions::default()
        },
    );
    let media = render
        .display_list
        .iter()
        .find_map(|command| match command {
            DisplayCommand::Rect {
                x,
                width,
                height,
                shade,
                ..
            } if *shade == 20 => Some((*x, *width, *height)),
            _ => None,
        })
        .expect("media background should paint inside the shrunken main column");
    let sidebar = render
        .display_list
        .iter()
        .find_map(|command| match command {
            DisplayCommand::Rect {
                x, width, shade, ..
            } if *shade == 120 => Some((*x, *width)),
            _ => None,
        })
        .expect("sidebar background should remain in the flex row");

    assert!(
        media.0 <= 12,
        "main column should shrink before centering wrapper; media should start near the left article column, got {media:?}"
    );
    assert!(
        media.1 >= 50 && media.1 <= 60,
        "wrapper media should keep its max-width column, got {media:?}"
    );
    assert!(
        sidebar.0 >= media.0.saturating_add(media.1),
        "fixed sidebar should stay to the right of the shrunken media column, media={media:?} sidebar={sidebar:?}"
    );
}

#[test]
fn small_vertical_box_spacing_does_not_inflate_list_rows() {
    let render = render_html(
        "mem://browser-css-pixel-scale-small-list-spacing",
        br#"
            <html><head><style>
              .facts { background: rgb(0, 137, 0); font-size: 19px; line-height: 31px; padding: 18px 30px; width: 636px; }
              .facts h2 { font-size: 28px; line-height: 35px; margin: 0; }
              .facts ul { margin: 20px 0; padding: 0; }
              .facts li { line-height: 28.5px; margin: 5px 0; padding: 5px 0; }
            </style></head><body>
              <section class="facts">
                <h2>FAST FACTS</h2>
                <ul>
                  <li>OFFICIAL NAME: Jamaica</li>
                  <li>FORM OF GOVERNMENT: Parliamentary democracy</li>
                  <li>CAPITAL: Kingston</li>
                </ul>
              </section>
            </body></html>
            "#,
        BrowserRenderOptions {
            width: 100,
            css_cell_width_px: 12,
            css_cell_height_px: 18,
            ..BrowserRenderOptions::default()
        },
    );
    let list_item_rows = render
        .display_list
        .iter()
        .filter_map(|command| match command {
            DisplayCommand::Text { y, text, .. } | DisplayCommand::StyledText { y, text, .. }
                if text.contains("OFFICIAL NAME")
                    || text.contains("FORM OF GOVERNMENT")
                    || text.contains("CAPITAL") =>
            {
                Some(*y)
            }
            _ => None,
        })
        .collect::<Vec<_>>();

    assert_eq!(list_item_rows.len(), 3);
    assert!(
        list_item_rows[1].saturating_sub(list_item_rows[0]) <= 3,
        "5px list spacing should not expand to full-row top/bottom gaps: {list_item_rows:?}"
    );
    assert!(
        list_item_rows[2].saturating_sub(list_item_rows[1]) <= 3,
        "facts list rows should remain compact: {list_item_rows:?}"
    );
}
