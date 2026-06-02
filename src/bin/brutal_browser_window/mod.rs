use std::path::PathBuf;

use anyhow::Result;
#[cfg(any(test, feature = "native-window"))]
use anyhow::ensure;
#[cfg(any(test, feature = "native-window"))]
use brutal_search::browser::{
    BROWSER_ABOUT_BLANK_TARGET, BrowserAppWindowFrameOptions, BrowserRasterOptions,
    BrowserRgbaRaster,
};
#[cfg(feature = "native-window")]
use brutal_search::browser::{
    BrowserApp, BrowserAppAction, BrowserAppOptions, BrowserAppWindowHit, BrowserCookieJar,
    BrowserLocalStorage, BrowserRenderOptions,
};
use clap::Args;

#[cfg(feature = "native-window")]
use crate::{
    load_browser_cookie_jar, load_browser_local_storage, save_browser_cookie_jar,
    save_browser_local_storage,
};

#[derive(Debug, Args)]
pub(crate) struct BrowserWindowCli {
    pub(crate) target: String,
    #[arg(long, default_value_t = 100)]
    pub(crate) width: usize,
    #[arg(long, default_value_t = 4 * 1024 * 1024)]
    pub(crate) max_bytes: usize,
    #[arg(long)]
    pub(crate) viewport_width: Option<usize>,
    #[arg(long, default_value_t = 32)]
    pub(crate) viewport_height: usize,
    #[arg(long, alias = "scroll-x", default_value_t = 0)]
    pub(crate) viewport_x: usize,
    #[arg(long, alias = "scroll-y", default_value_t = 0)]
    pub(crate) viewport_y: usize,
    #[arg(long, default_value_t = 8)]
    pub(crate) cell_width: usize,
    #[arg(long, default_value_t = 12)]
    pub(crate) cell_height: usize,
    #[arg(long)]
    pub(crate) cookie_jar: Option<PathBuf>,
    #[arg(long, alias = "local-storage-file")]
    pub(crate) local_storage: Option<PathBuf>,
}

pub(crate) async fn run_browser_window_cli(args: BrowserWindowCli) -> Result<()> {
    #[cfg(feature = "native-window")]
    {
        return native::run_native_browser_window(args).await;
    }

    #[cfg(not(feature = "native-window"))]
    {
        let _ = args;
        anyhow::bail!(
            "native browser window support is not enabled; rebuild with `cargo run --features native-window --bin brutal-browser -- window <target>`"
        );
    }
}

#[cfg(feature = "native-window")]
fn browser_window_app_options(args: &BrowserWindowCli) -> BrowserAppOptions {
    BrowserAppOptions {
        render: BrowserRenderOptions {
            width: args.width,
            max_bytes: args.max_bytes,
        },
        viewport_width: args.viewport_width.unwrap_or(args.width),
        viewport_height: args.viewport_height,
        raster: BrowserRasterOptions {
            cell_width: args.cell_width,
            cell_height: args.cell_height,
            ..BrowserRasterOptions::default()
        },
    }
}

#[cfg(any(test, feature = "native-window"))]
const BROWSER_WINDOW_TITLE_PREFIX: &str = "Blackium Starium✴";

#[cfg(any(test, feature = "native-window"))]
fn rgba_to_native_window_buffer(raster: &BrowserRgbaRaster) -> Result<Vec<u32>> {
    ensure!(
        raster.pixels.len() == raster.width.saturating_mul(raster.height).saturating_mul(4),
        "RGBA buffer length does not match raster dimensions"
    );

    Ok(raster
        .pixels
        .chunks_exact(4)
        .map(|pixel| {
            let red = pixel[0] as u32;
            let green = pixel[1] as u32;
            let blue = pixel[2] as u32;
            (red << 16) | (green << 8) | blue
        })
        .collect())
}

#[cfg(any(test, feature = "native-window"))]
fn wheel_delta_to_scroll_cells(delta: f32) -> isize {
    if delta.abs() < f32::EPSILON {
        0
    } else {
        let cells = (delta * 3.0).round() as isize;
        cells.clamp(-24, 24)
    }
}

#[cfg(any(test, feature = "native-window"))]
#[derive(Debug, Clone, PartialEq, Eq)]
enum BrowserWindowMode {
    Page,
    Location {
        text: String,
        replace_on_input: bool,
    },
    Find {
        text: String,
        replace_on_input: bool,
    },
}

#[cfg(any(test, feature = "native-window"))]
impl Default for BrowserWindowMode {
    fn default() -> Self {
        Self::Page
    }
}

#[cfg(any(test, feature = "native-window"))]
fn browser_window_location_text(mode: &BrowserWindowMode) -> Option<&str> {
    match mode {
        BrowserWindowMode::Page => None,
        BrowserWindowMode::Location { text, .. } => Some(text),
        BrowserWindowMode::Find { .. } => None,
    }
}

#[cfg(any(test, feature = "native-window"))]
fn begin_browser_window_location_input(mode: &mut BrowserWindowMode, current_source: &str) {
    *mode = BrowserWindowMode::Location {
        text: browser_window_location_prompt_text(current_source).to_owned(),
        replace_on_input: true,
    };
}

#[cfg(any(test, feature = "native-window"))]
fn browser_window_location_prompt_text(source: &str) -> &str {
    if source == BROWSER_ABOUT_BLANK_TARGET {
        ""
    } else {
        source
    }
}

#[cfg(any(test, feature = "native-window"))]
fn begin_browser_window_blank_location_input(mode: &mut BrowserWindowMode) {
    *mode = BrowserWindowMode::Location {
        text: String::new(),
        replace_on_input: false,
    };
}

#[cfg(any(test, feature = "native-window"))]
fn push_browser_window_location_text(mode: &mut BrowserWindowMode, text: &str) -> bool {
    let BrowserWindowMode::Location {
        text: location,
        replace_on_input,
    } = mode
    else {
        return false;
    };
    if *replace_on_input {
        location.clear();
        *replace_on_input = false;
    }
    location.push_str(text);
    true
}

#[cfg(any(test, feature = "native-window"))]
fn delete_browser_window_location_text_backward(mode: &mut BrowserWindowMode) -> bool {
    let BrowserWindowMode::Location {
        text,
        replace_on_input,
    } = mode
    else {
        return false;
    };
    if *replace_on_input {
        *replace_on_input = false;
        let changed = !text.is_empty();
        text.clear();
        return changed;
    }
    text.pop().is_some()
}

#[cfg(any(test, feature = "native-window"))]
fn browser_window_find_text(mode: &BrowserWindowMode) -> Option<&str> {
    match mode {
        BrowserWindowMode::Find { text, .. } => Some(text),
        _ => None,
    }
}

#[cfg(any(test, feature = "native-window"))]
fn begin_browser_window_find_input(mode: &mut BrowserWindowMode, current_query: Option<&str>) {
    let text = current_query.unwrap_or_default().to_owned();
    let replace_on_input = !text.is_empty();
    *mode = BrowserWindowMode::Find {
        text,
        replace_on_input,
    };
}

#[cfg(any(test, feature = "native-window"))]
fn push_browser_window_find_text(mode: &mut BrowserWindowMode, input: &str) -> bool {
    let BrowserWindowMode::Find {
        text,
        replace_on_input,
    } = mode
    else {
        return false;
    };
    if *replace_on_input {
        text.clear();
        *replace_on_input = false;
    }
    text.push_str(input);
    true
}

#[cfg(any(test, feature = "native-window"))]
fn delete_browser_window_find_text_backward(mode: &mut BrowserWindowMode) -> bool {
    let BrowserWindowMode::Find {
        text,
        replace_on_input,
    } = mode
    else {
        return false;
    };
    if *replace_on_input {
        *replace_on_input = false;
        let changed = !text.is_empty();
        text.clear();
        return changed;
    }
    text.pop().is_some()
}

#[cfg(any(test, feature = "native-window"))]
fn select_browser_window_prompt_text(mode: &mut BrowserWindowMode) -> bool {
    match mode {
        BrowserWindowMode::Location {
            text,
            replace_on_input,
        }
        | BrowserWindowMode::Find {
            text,
            replace_on_input,
        } => {
            if text.is_empty() || *replace_on_input {
                return false;
            }
            *replace_on_input = true;
            true
        }
        BrowserWindowMode::Page => false,
    }
}

#[cfg(any(test, feature = "native-window"))]
fn clear_browser_window_prompt_text(mode: &mut BrowserWindowMode) -> bool {
    match mode {
        BrowserWindowMode::Location {
            text,
            replace_on_input,
        }
        | BrowserWindowMode::Find {
            text,
            replace_on_input,
        } => {
            *replace_on_input = false;
            let changed = !text.is_empty();
            text.clear();
            changed
        }
        BrowserWindowMode::Page => false,
    }
}

#[cfg(any(test, feature = "native-window"))]
fn browser_window_frame_options(mode: &BrowserWindowMode) -> BrowserAppWindowFrameOptions {
    browser_window_frame_options_with_status(mode, None)
}

#[cfg(any(test, feature = "native-window"))]
fn browser_window_frame_options_with_status(
    mode: &BrowserWindowMode,
    page_status_text: Option<&str>,
) -> BrowserAppWindowFrameOptions {
    match mode {
        BrowserWindowMode::Page => BrowserAppWindowFrameOptions {
            location_text: None,
            status_text: page_status_text.map(str::to_owned),
        },
        BrowserWindowMode::Location { text, .. } => BrowserAppWindowFrameOptions {
            location_text: Some(format!("URL > {text}")),
            status_text: Some("location: Enter=open Esc=cancel Backspace=delete".to_owned()),
        },
        BrowserWindowMode::Find { text, .. } => BrowserAppWindowFrameOptions {
            location_text: Some(format!("Find > {text}")),
            status_text: Some("find: Enter=find Esc=cancel Backspace=delete".to_owned()),
        },
    }
}

#[cfg(any(test, feature = "native-window"))]
fn browser_viewport_size_for_window_pixels(
    window_width: usize,
    window_height: usize,
    raster: BrowserRasterOptions,
) -> (usize, usize) {
    let cell_width = raster.cell_width.max(1);
    let cell_height = raster.cell_height.max(1);
    let chrome_height = 3usize
        .saturating_mul(cell_height)
        .saturating_add(raster.padding_y.saturating_mul(2));
    let viewport_width =
        window_width.saturating_sub(raster.padding_x.saturating_mul(2)) / cell_width;
    let viewport_height = window_height
        .saturating_sub(chrome_height)
        .saturating_sub(raster.padding_y.saturating_mul(2))
        / cell_height;
    (viewport_width.max(1), viewport_height.max(1))
}

#[cfg(feature = "native-window")]
mod native {
    use std::cell::RefCell;
    use std::rc::Rc;

    use anyhow::{Context, Result};
    use brutal_search::browser::BrowserAppWindowFrame;
    use minifb::{
        InputCallback, Key, KeyRepeat, MouseButton, MouseMode, ScaleMode, Window, WindowOptions,
    };

    use super::*;

    #[derive(Debug, Clone, Copy, Default)]
    struct BrowserWindowModifiers {
        command: bool,
        shift: bool,
        alt: bool,
    }

    #[derive(Debug, Clone, Copy, Default)]
    struct BrowserWindowKeyResult {
        dirty: bool,
        close: bool,
    }

    struct BrowserWindowInputCapture {
        chars: Rc<RefCell<Vec<char>>>,
    }

    impl InputCallback for BrowserWindowInputCapture {
        fn add_char(&mut self, uni_char: u32) {
            if let Some(ch) = char::from_u32(uni_char)
                && !ch.is_control()
            {
                self.chars.borrow_mut().push(ch);
            }
        }
    }

    pub(super) async fn run_native_browser_window(args: BrowserWindowCli) -> Result<()> {
        let options = browser_window_app_options(&args);
        let raster_options = options.raster;
        let initial_cookie_jar = args
            .cookie_jar
            .as_deref()
            .map(load_browser_cookie_jar)
            .transpose()?
            .unwrap_or_else(BrowserCookieJar::default);
        let initial_local_storage = args
            .local_storage
            .as_deref()
            .map(load_browser_local_storage)
            .transpose()?
            .unwrap_or_else(BrowserLocalStorage::default);
        let mut app = BrowserApp::open_with_state(
            &args.target,
            options,
            initial_cookie_jar,
            initial_local_storage,
        )
        .await?;

        if args.viewport_x > 0 || args.viewport_y > 0 {
            app.apply_action(BrowserAppAction::SetViewportOrigin {
                x: args.viewport_x,
                y: args.viewport_y,
            })
            .await?;
        }

        let mut mode = BrowserWindowMode::Page;
        let mut hover_status_text: Option<String> = None;
        let mut frame = app.present_window_frame_with_options(
            browser_window_frame_options_with_status(&mode, hover_status_text.as_deref()),
        )?;
        let mut buffer = rgba_to_native_window_buffer(&frame.raster)?;
        let mut window = Window::new(
            &browser_window_title(&frame, &mode),
            frame.raster.width,
            frame.raster.height,
            WindowOptions {
                resize: true,
                scale_mode: ScaleMode::UpperLeft,
                ..WindowOptions::default()
            },
        )
        .context("failed to open native browser window")?;
        let input_chars = Rc::new(RefCell::new(Vec::new()));
        window.set_input_callback(Box::new(BrowserWindowInputCapture {
            chars: Rc::clone(&input_chars),
        }));
        let mut previous_left_down = false;
        let mut previous_middle_down = false;
        let mut previous_window_size = window.get_size();
        let mut close_requested = false;

        while window.is_open() && !close_requested {
            let mut dirty = false;

            let window_size = window.get_size();
            if window_size != previous_window_size {
                dirty |=
                    handle_browser_window_resize(&mut app, window_size, raster_options).await?;
                previous_window_size = window_size;
            }

            let typed_text = drain_browser_window_input_chars(&input_chars);
            dirty |= apply_browser_window_text_input(&mut app, &mut mode, &typed_text).await?;
            let modifiers = browser_window_modifiers(&window);
            let next_hover_status_text =
                browser_window_hover_status_for_window(&app, &mode, &window)?;
            if next_hover_status_text != hover_status_text {
                hover_status_text = next_hover_status_text;
                dirty = true;
            }
            for key in browser_window_pressed_keys(&window, &app, &mode, modifiers)? {
                let result = handle_browser_window_key(&mut app, &mut mode, key, modifiers).await?;
                dirty |= result.dirty;
                close_requested |= result.close;
            }

            if let Some((scroll_x, scroll_y)) = window.get_scroll_wheel() {
                if let Some(action) =
                    browser_window_wheel_scroll_action(scroll_x, scroll_y, modifiers)
                {
                    app.apply_action(action).await?;
                    dirty = true;
                }
            }

            let left_down = window.get_mouse_down(MouseButton::Left);
            if left_down
                && !previous_left_down
                && let Some((x, y)) = window.get_unscaled_mouse_pos(MouseMode::Discard)
            {
                let (x, y) = browser_window_mouse_position_to_pixels(x, y);
                let hit = app.hit_test_window(x, y)?;
                let result =
                    handle_browser_window_left_click(&mut app, &mut mode, x, y, hit, modifiers)
                        .await?;
                dirty |= result.dirty;
                close_requested |= result.close;
            }
            previous_left_down = left_down;

            let middle_down = window.get_mouse_down(MouseButton::Middle);
            if middle_down
                && !previous_middle_down
                && let Some((x, y)) = window.get_unscaled_mouse_pos(MouseMode::Discard)
            {
                let (x, y) = browser_window_mouse_position_to_pixels(x, y);
                let hit = app.hit_test_window(x, y)?;
                let result = handle_browser_window_middle_click(&mut app, &mut mode, hit).await?;
                dirty |= result.dirty;
                close_requested |= result.close;
            }
            previous_middle_down = middle_down;

            if dirty {
                hover_status_text = browser_window_hover_status_for_window(&app, &mode, &window)?;
                frame = app.present_window_frame_with_options(
                    browser_window_frame_options_with_status(&mode, hover_status_text.as_deref()),
                )?;
                buffer = rgba_to_native_window_buffer(&frame.raster)?;
                window.set_title(&browser_window_title(&frame, &mode));
            }

            window
                .update_with_buffer(&buffer, frame.raster.width, frame.raster.height)
                .context("failed to present browser window frame")?;
        }

        let active_session = app.active_session()?;
        if let Some(path) = args.cookie_jar.as_deref() {
            save_browser_cookie_jar(path, &active_session.cookies_snapshot())?;
        }
        if let Some(path) = args.local_storage.as_deref() {
            save_browser_local_storage(path, &active_session.local_storage_snapshot())?;
        }

        Ok(())
    }

    async fn handle_browser_window_resize(
        app: &mut BrowserApp,
        window_size: (usize, usize),
        raster_options: BrowserRasterOptions,
    ) -> Result<bool> {
        let (viewport_width, viewport_height) =
            browser_viewport_size_for_window_pixels(window_size.0, window_size.1, raster_options);
        let viewport = app.active_viewport()?;
        if viewport.width == viewport_width && viewport.height == viewport_height {
            return Ok(false);
        }
        app.apply_action(BrowserAppAction::SetViewport {
            width: viewport_width,
            height: viewport_height,
        })
        .await?;
        Ok(true)
    }

    async fn handle_browser_window_key(
        app: &mut BrowserApp,
        mode: &mut BrowserWindowMode,
        key: Key,
        modifiers: BrowserWindowModifiers,
    ) -> Result<BrowserWindowKeyResult> {
        if modifiers.command {
            if let Some(index) = browser_window_tab_shortcut_index(key, app.tab_count()) {
                if index == app.active_tab() {
                    return Ok(BrowserWindowKeyResult::default());
                }
                app.apply_action(BrowserAppAction::SwitchTab(index)).await?;
                return Ok(BrowserWindowKeyResult {
                    dirty: true,
                    close: false,
                });
            }
            match key {
                Key::A => {
                    return Ok(BrowserWindowKeyResult {
                        dirty: select_browser_window_prompt_text(mode),
                        close: false,
                    });
                }
                Key::Backspace | Key::Delete
                    if matches!(
                        mode,
                        BrowserWindowMode::Location { .. } | BrowserWindowMode::Find { .. }
                    ) =>
                {
                    return Ok(BrowserWindowKeyResult {
                        dirty: clear_browser_window_prompt_text(mode),
                        close: false,
                    });
                }
                Key::Backspace | Key::Delete
                    if browser_window_focused_control_accepts_text_input(app)? =>
                {
                    app.apply_action(BrowserAppAction::ClearText).await?;
                    return Ok(BrowserWindowKeyResult {
                        dirty: true,
                        close: false,
                    });
                }
                Key::Backspace | Key::Delete
                    if app.active_session()?.focused_control().is_some() =>
                {
                    return Ok(BrowserWindowKeyResult::default());
                }
                Key::L => {
                    let source = current_browser_window_source(app)?;
                    begin_browser_window_location_input(mode, &source);
                    return Ok(BrowserWindowKeyResult {
                        dirty: true,
                        close: false,
                    });
                }
                Key::F => {
                    let current_query = app.active_find_state()?.map(|find| find.query);
                    begin_browser_window_find_input(mode, current_query.as_deref());
                    return Ok(BrowserWindowKeyResult {
                        dirty: true,
                        close: false,
                    });
                }
                Key::G => {
                    return handle_browser_window_find_navigation(app, mode, modifiers.shift).await;
                }
                Key::Q => {
                    return Ok(BrowserWindowKeyResult {
                        dirty: false,
                        close: true,
                    });
                }
                Key::R => {
                    *mode = BrowserWindowMode::Page;
                    app.apply_action(BrowserAppAction::Reload).await?;
                    return Ok(BrowserWindowKeyResult {
                        dirty: true,
                        close: false,
                    });
                }
                Key::LeftBracket if modifiers.shift && app.tab_count() > 0 => {
                    app.apply_action(browser_window_tab_cycle_action(app, true)?)
                        .await?;
                    return Ok(BrowserWindowKeyResult {
                        dirty: true,
                        close: false,
                    });
                }
                Key::RightBracket if modifiers.shift && app.tab_count() > 0 => {
                    app.apply_action(browser_window_tab_cycle_action(app, false)?)
                        .await?;
                    return Ok(BrowserWindowKeyResult {
                        dirty: true,
                        close: false,
                    });
                }
                Key::Left | Key::LeftBracket => {
                    return handle_browser_window_history_navigation(app, mode, true).await;
                }
                Key::Right | Key::RightBracket => {
                    return handle_browser_window_history_navigation(app, mode, false).await;
                }
                Key::Up => {
                    *mode = BrowserWindowMode::Page;
                    app.apply_action(browser_window_document_start_action(app)?)
                        .await?;
                    return Ok(BrowserWindowKeyResult {
                        dirty: true,
                        close: false,
                    });
                }
                Key::Down => {
                    *mode = BrowserWindowMode::Page;
                    app.apply_action(browser_window_document_end_action(app)?)
                        .await?;
                    return Ok(BrowserWindowKeyResult {
                        dirty: true,
                        close: false,
                    });
                }
                Key::T if modifiers.shift => {
                    if app.closed_tab_count() > 0 {
                        app.apply_action(BrowserAppAction::RestoreClosedTab).await?;
                        return Ok(BrowserWindowKeyResult {
                            dirty: true,
                            close: false,
                        });
                    }
                    return Ok(BrowserWindowKeyResult::default());
                }
                Key::T => {
                    app.apply_action(BrowserAppAction::NewBlankTab).await?;
                    begin_browser_window_blank_location_input(mode);
                    return Ok(BrowserWindowKeyResult {
                        dirty: true,
                        close: false,
                    });
                }
                Key::W => {
                    if app.tab_count() > 1 {
                        app.apply_action(BrowserAppAction::CloseTab(None)).await?;
                        return Ok(BrowserWindowKeyResult {
                            dirty: true,
                            close: false,
                        });
                    }
                    return Ok(BrowserWindowKeyResult {
                        dirty: false,
                        close: true,
                    });
                }
                Key::Tab if app.tab_count() > 0 => {
                    app.apply_action(browser_window_tab_cycle_action(app, modifiers.shift)?)
                        .await?;
                    return Ok(BrowserWindowKeyResult {
                        dirty: true,
                        close: false,
                    });
                }
                Key::PageUp if app.tab_count() > 0 => {
                    app.apply_action(browser_window_tab_cycle_action(app, true)?)
                        .await?;
                    return Ok(BrowserWindowKeyResult {
                        dirty: true,
                        close: false,
                    });
                }
                Key::PageDown if app.tab_count() > 0 => {
                    app.apply_action(browser_window_tab_cycle_action(app, false)?)
                        .await?;
                    return Ok(BrowserWindowKeyResult {
                        dirty: true,
                        close: false,
                    });
                }
                _ => {}
            }
        }

        if modifiers.alt {
            match key {
                Key::D => {
                    let source = current_browser_window_source(app)?;
                    begin_browser_window_location_input(mode, &source);
                    return Ok(BrowserWindowKeyResult {
                        dirty: true,
                        close: false,
                    });
                }
                Key::Left => {
                    return handle_browser_window_history_navigation(app, mode, true).await;
                }
                Key::Right => {
                    return handle_browser_window_history_navigation(app, mode, false).await;
                }
                _ => {}
            }
        }

        if key == Key::F6 {
            let source = current_browser_window_source(app)?;
            begin_browser_window_location_input(mode, &source);
            return Ok(BrowserWindowKeyResult {
                dirty: true,
                close: false,
            });
        }

        if key == Key::F5 {
            *mode = BrowserWindowMode::Page;
            app.apply_action(BrowserAppAction::Reload).await?;
            return Ok(BrowserWindowKeyResult {
                dirty: true,
                close: false,
            });
        }

        if key == Key::F3 {
            return handle_browser_window_find_navigation(app, mode, modifiers.shift).await;
        }

        match mode {
            BrowserWindowMode::Location { .. } => {
                return handle_browser_window_location_key(app, mode, key, modifiers).await;
            }
            BrowserWindowMode::Find { .. } => {
                return handle_browser_window_find_key(app, mode, key, modifiers).await;
            }
            BrowserWindowMode::Page => {}
        }

        if matches!(key, Key::Slash)
            && !modifiers.command
            && !modifiers.shift
            && !modifiers.alt
            && app.active_session()?.focused_control().is_none()
        {
            let current_query = app.active_find_state()?.map(|find| find.query);
            begin_browser_window_find_input(mode, current_query.as_deref());
            return Ok(BrowserWindowKeyResult {
                dirty: true,
                close: false,
            });
        }

        if browser_window_focused_text_control_owns_navigation_key(app, key)? {
            return Ok(BrowserWindowKeyResult::default());
        }

        if matches!(
            key,
            Key::Up | Key::Down | Key::PageUp | Key::PageDown | Key::Home | Key::End
        ) && browser_window_focused_control_accepts_select_input(app)?
        {
            if let Some(action) = browser_window_focused_select_key_action(app, key)? {
                app.apply_action(action).await?;
                return Ok(BrowserWindowKeyResult {
                    dirty: true,
                    close: false,
                });
            }
            return Ok(BrowserWindowKeyResult::default());
        }

        let action = match key {
            Key::Escape if app.active_session()?.focused_control().is_some() => {
                Some(BrowserAppAction::BlurFocused)
            }
            Key::Escape => None,
            Key::Backspace if browser_window_focused_control_accepts_text_input(app)? => {
                Some(BrowserAppAction::DeleteTextBackward(1))
            }
            Key::Backspace if app.active_session()?.focused_control().is_some() => None,
            Key::Backspace => Some(BrowserAppAction::Back),
            Key::Enter | Key::NumPadEnter if app.active_session()?.focused_control().is_some() => {
                Some(BrowserAppAction::SubmitFocused)
            }
            Key::Space if browser_window_focused_control_accepts_space_toggle(app)? => {
                Some(BrowserAppAction::ToggleFocused)
            }
            Key::Space if app.active_session()?.focused_control().is_none() && modifiers.shift => {
                Some(browser_window_page_scroll_action(app, -1)?)
            }
            Key::Space if app.active_session()?.focused_control().is_none() => {
                Some(browser_window_page_scroll_action(app, 1)?)
            }
            Key::Tab if modifiers.shift && browser_window_has_focusable_controls(app)? => {
                Some(BrowserAppAction::FocusPrevious)
            }
            Key::Tab if browser_window_has_focusable_controls(app)? => {
                Some(BrowserAppAction::FocusNext)
            }
            Key::Tab => None,
            Key::Up => Some(BrowserAppAction::Scroll {
                delta_x: 0,
                delta_y: -3,
            }),
            Key::Down => Some(BrowserAppAction::Scroll {
                delta_x: 0,
                delta_y: 3,
            }),
            Key::Left => Some(BrowserAppAction::Scroll {
                delta_x: -8,
                delta_y: 0,
            }),
            Key::Right => Some(BrowserAppAction::Scroll {
                delta_x: 8,
                delta_y: 0,
            }),
            Key::PageUp if modifiers.shift => {
                Some(browser_window_horizontal_page_scroll_action(app, -1)?)
            }
            Key::PageDown if modifiers.shift => {
                Some(browser_window_horizontal_page_scroll_action(app, 1)?)
            }
            Key::PageUp => Some(browser_window_page_scroll_action(app, -1)?),
            Key::PageDown => Some(browser_window_page_scroll_action(app, 1)?),
            Key::Home if modifiers.shift => Some(browser_window_row_start_action(app)?),
            Key::End if modifiers.shift => Some(browser_window_row_end_action(app)?),
            Key::Home => Some(BrowserAppAction::SetViewportOrigin { x: 0, y: 0 }),
            Key::End => Some(browser_window_document_end_action(app)?),
            _ => None,
        };

        if let Some(action) = action {
            app.apply_action(action).await?;
            Ok(BrowserWindowKeyResult {
                dirty: true,
                close: false,
            })
        } else {
            Ok(BrowserWindowKeyResult::default())
        }
    }

    async fn handle_browser_window_find_navigation(
        app: &mut BrowserApp,
        mode: &mut BrowserWindowMode,
        backwards: bool,
    ) -> Result<BrowserWindowKeyResult> {
        if let Some(find) = app.active_find_state()? {
            if backwards {
                app.apply_action(BrowserAppAction::FindTextPrevious { query: find.query })
                    .await?;
            } else {
                app.apply_action(BrowserAppAction::FindText {
                    query: find.query,
                    next: true,
                })
                .await?;
            }
        } else {
            begin_browser_window_find_input(mode, None);
        }
        Ok(BrowserWindowKeyResult {
            dirty: true,
            close: false,
        })
    }

    async fn handle_browser_window_history_navigation(
        app: &mut BrowserApp,
        mode: &mut BrowserWindowMode,
        backwards: bool,
    ) -> Result<BrowserWindowKeyResult> {
        if browser_window_history_target_status(app, backwards)?.is_none() {
            return Ok(BrowserWindowKeyResult::default());
        }
        *mode = BrowserWindowMode::Page;
        let action = if backwards {
            BrowserAppAction::Back
        } else {
            BrowserAppAction::Forward
        };
        app.apply_action(action).await?;
        Ok(BrowserWindowKeyResult {
            dirty: true,
            close: false,
        })
    }

    async fn handle_browser_window_location_key(
        app: &mut BrowserApp,
        mode: &mut BrowserWindowMode,
        key: Key,
        modifiers: BrowserWindowModifiers,
    ) -> Result<BrowserWindowKeyResult> {
        match key {
            Key::Enter | Key::NumPadEnter => {
                let target = browser_window_location_text(mode)
                    .unwrap_or_default()
                    .to_owned();
                *mode = BrowserWindowMode::Page;
                if !target.trim().is_empty() {
                    if modifiers.alt {
                        app.apply_action(BrowserAppAction::NewTab(target)).await?;
                    } else {
                        app.apply_action(BrowserAppAction::Open(target)).await?;
                    }
                }
                Ok(BrowserWindowKeyResult {
                    dirty: true,
                    close: false,
                })
            }
            Key::Escape => {
                *mode = BrowserWindowMode::Page;
                Ok(BrowserWindowKeyResult {
                    dirty: true,
                    close: false,
                })
            }
            Key::Backspace | Key::Delete => Ok(BrowserWindowKeyResult {
                dirty: delete_browser_window_location_text_backward(mode),
                close: false,
            }),
            _ => Ok(BrowserWindowKeyResult::default()),
        }
    }

    async fn handle_browser_window_find_key(
        app: &mut BrowserApp,
        mode: &mut BrowserWindowMode,
        key: Key,
        modifiers: BrowserWindowModifiers,
    ) -> Result<BrowserWindowKeyResult> {
        match key {
            Key::Enter | Key::NumPadEnter => {
                let query = browser_window_find_text(mode)
                    .unwrap_or_default()
                    .to_owned();
                *mode = BrowserWindowMode::Page;
                if !query.trim().is_empty() {
                    if modifiers.shift {
                        app.apply_action(BrowserAppAction::FindTextPrevious { query })
                            .await?;
                    } else {
                        app.apply_action(BrowserAppAction::FindText { query, next: false })
                            .await?;
                    }
                }
                Ok(BrowserWindowKeyResult {
                    dirty: true,
                    close: false,
                })
            }
            Key::Escape => {
                *mode = BrowserWindowMode::Page;
                Ok(BrowserWindowKeyResult {
                    dirty: true,
                    close: false,
                })
            }
            Key::Backspace | Key::Delete => Ok(BrowserWindowKeyResult {
                dirty: delete_browser_window_find_text_backward(mode),
                close: false,
            }),
            _ => Ok(BrowserWindowKeyResult::default()),
        }
    }

    fn browser_window_tab_shortcut_index(key: Key, tab_count: usize) -> Option<usize> {
        if tab_count == 0 {
            return None;
        }
        match key {
            Key::Key1 | Key::NumPad1 => Some(0),
            Key::Key2 | Key::NumPad2 if tab_count >= 2 => Some(1),
            Key::Key3 | Key::NumPad3 if tab_count >= 3 => Some(2),
            Key::Key4 | Key::NumPad4 if tab_count >= 4 => Some(3),
            Key::Key5 | Key::NumPad5 if tab_count >= 5 => Some(4),
            Key::Key6 | Key::NumPad6 if tab_count >= 6 => Some(5),
            Key::Key7 | Key::NumPad7 if tab_count >= 7 => Some(6),
            Key::Key8 | Key::NumPad8 if tab_count >= 8 => Some(7),
            Key::Key9 | Key::NumPad9 => Some(tab_count - 1),
            _ => None,
        }
    }

    fn browser_window_pressed_keys(
        window: &Window,
        app: &BrowserApp,
        mode: &BrowserWindowMode,
        modifiers: BrowserWindowModifiers,
    ) -> Result<Vec<Key>> {
        let mut keys = window.get_keys_pressed(KeyRepeat::No);
        for key in window.get_keys_pressed(KeyRepeat::Yes) {
            if keys.contains(&key) {
                continue;
            }
            if browser_window_key_repeat_enabled(app, mode, key, modifiers)? {
                keys.push(key);
            }
        }
        Ok(keys)
    }

    fn browser_window_key_repeat_enabled(
        app: &BrowserApp,
        mode: &BrowserWindowMode,
        key: Key,
        modifiers: BrowserWindowModifiers,
    ) -> Result<bool> {
        if modifiers.command || modifiers.alt {
            return Ok(false);
        }

        match mode {
            BrowserWindowMode::Location { .. } | BrowserWindowMode::Find { .. } => {
                return Ok(matches!(key, Key::Backspace | Key::Delete));
            }
            BrowserWindowMode::Page => {}
        }

        if browser_window_focused_control_accepts_text_input(app)? {
            return Ok(matches!(key, Key::Backspace));
        }
        if browser_window_focused_control_accepts_select_input(app)? {
            return Ok(matches!(
                key,
                Key::Up | Key::Down | Key::PageUp | Key::PageDown | Key::Home | Key::End
            ));
        }
        if app.active_session()?.focused_control().is_some() {
            return Ok(false);
        }

        Ok(matches!(
            key,
            Key::Up | Key::Down | Key::Left | Key::Right | Key::PageUp | Key::PageDown | Key::Space
        ))
    }

    async fn apply_browser_window_text_input(
        app: &mut BrowserApp,
        mode: &mut BrowserWindowMode,
        text: &str,
    ) -> Result<bool> {
        if text.is_empty() {
            return Ok(false);
        }
        if push_browser_window_location_text(mode, text) {
            return Ok(true);
        }
        if push_browser_window_find_text(mode, text) {
            return Ok(true);
        }
        if browser_window_focused_control_accepts_text_input(app)? {
            app.apply_action(BrowserAppAction::TypeText(text.to_owned()))
                .await?;
            return Ok(true);
        }
        Ok(false)
    }

    fn current_browser_window_source(app: &BrowserApp) -> Result<String> {
        Ok(app
            .active_session()?
            .current()
            .map(|render| render.source.clone())
            .unwrap_or_default())
    }

    fn browser_window_page_scroll_action(
        app: &BrowserApp,
        direction: isize,
    ) -> Result<BrowserAppAction> {
        let viewport = app.active_viewport()?;
        let rows = viewport
            .height
            .saturating_sub(1)
            .max(1)
            .min(isize::MAX as usize) as isize;
        Ok(BrowserAppAction::Scroll {
            delta_x: 0,
            delta_y: rows.saturating_mul(direction.signum()),
        })
    }

    fn browser_window_horizontal_page_scroll_action(
        app: &BrowserApp,
        direction: isize,
    ) -> Result<BrowserAppAction> {
        let viewport = app.active_viewport()?;
        let columns = viewport
            .width
            .saturating_sub(1)
            .max(1)
            .min(isize::MAX as usize) as isize;
        Ok(BrowserAppAction::Scroll {
            delta_x: columns.saturating_mul(direction.signum()),
            delta_y: 0,
        })
    }

    fn browser_window_document_end_action(app: &BrowserApp) -> Result<BrowserAppAction> {
        let viewport = app.active_viewport()?;
        Ok(BrowserAppAction::SetViewportOrigin {
            x: viewport.x,
            y: usize::MAX,
        })
    }

    fn browser_window_document_start_action(app: &BrowserApp) -> Result<BrowserAppAction> {
        let viewport = app.active_viewport()?;
        Ok(BrowserAppAction::SetViewportOrigin {
            x: viewport.x,
            y: 0,
        })
    }

    fn browser_window_row_start_action(app: &BrowserApp) -> Result<BrowserAppAction> {
        let viewport = app.active_viewport()?;
        Ok(BrowserAppAction::SetViewportOrigin {
            x: 0,
            y: viewport.y,
        })
    }

    fn browser_window_row_end_action(app: &BrowserApp) -> Result<BrowserAppAction> {
        let viewport = app.active_viewport()?;
        Ok(BrowserAppAction::SetViewportOrigin {
            x: usize::MAX,
            y: viewport.y,
        })
    }

    fn browser_window_wheel_scroll_action(
        scroll_x: f32,
        scroll_y: f32,
        modifiers: BrowserWindowModifiers,
    ) -> Option<BrowserAppAction> {
        let (delta_x, delta_y) = if modifiers.shift && scroll_x.abs() < f32::EPSILON {
            (-wheel_delta_to_scroll_cells(scroll_y), 0)
        } else {
            (
                -wheel_delta_to_scroll_cells(scroll_x),
                -wheel_delta_to_scroll_cells(scroll_y),
            )
        };
        (delta_x != 0 || delta_y != 0).then_some(BrowserAppAction::Scroll { delta_x, delta_y })
    }

    fn browser_window_focused_control_accepts_space_toggle(app: &BrowserApp) -> Result<bool> {
        Ok(app
            .active_session()?
            .focused_control()
            .is_some_and(|control| {
                matches!(
                    control.kind.to_ascii_lowercase().as_str(),
                    "checkbox" | "radio"
                )
            }))
    }

    fn browser_window_focused_control_accepts_text_input(app: &BrowserApp) -> Result<bool> {
        Ok(app
            .active_session()?
            .focused_control()
            .is_some_and(|control| {
                matches!(
                    control.kind.to_ascii_lowercase().as_str(),
                    "text"
                        | "search"
                        | "url"
                        | "email"
                        | "password"
                        | "tel"
                        | "number"
                        | "date"
                        | "datetime-local"
                        | "month"
                        | "time"
                        | "week"
                        | "color"
                        | "textarea"
                )
            }))
    }

    fn browser_window_focused_control_accepts_select_input(app: &BrowserApp) -> Result<bool> {
        Ok(app
            .active_session()?
            .focused_control()
            .is_some_and(|control| control.kind.eq_ignore_ascii_case("select")))
    }

    fn browser_window_focused_select_key_action(
        app: &BrowserApp,
        key: Key,
    ) -> Result<Option<BrowserAppAction>> {
        let session = app.active_session()?;
        let Some(focused) = session.focused_control() else {
            return Ok(None);
        };
        if !focused.kind.eq_ignore_ascii_case("select") {
            return Ok(None);
        }

        let Some(control) = session
            .current_forms()
            .get(focused.form_index)
            .and_then(|form| form.controls.get(focused.control_index))
        else {
            return Ok(None);
        };
        if control.disabled || !control.kind.eq_ignore_ascii_case("select") {
            return Ok(None);
        }

        let enabled_options = control
            .options
            .iter()
            .filter(|option| !option.disabled)
            .collect::<Vec<_>>();
        if enabled_options.is_empty() {
            return Ok(None);
        }

        let current_index = enabled_options
            .iter()
            .position(|option| option.value == focused.value)
            .or_else(|| enabled_options.iter().position(|option| option.selected));
        let next_index = match (key, current_index) {
            (Key::Up, Some(index)) if index > 0 => Some(index - 1),
            (Key::Up, None) => enabled_options.len().checked_sub(1),
            (Key::Down, Some(index)) if index + 1 < enabled_options.len() => Some(index + 1),
            (Key::Down, None) => Some(0),
            (Key::Home | Key::PageUp, _) => Some(0),
            (Key::End | Key::PageDown, _) => enabled_options.len().checked_sub(1),
            _ => None,
        };
        let Some(next_index) = next_index else {
            return Ok(None);
        };

        let next_value = enabled_options[next_index].value.clone();
        if next_value == focused.value {
            return Ok(None);
        }
        Ok(Some(BrowserAppAction::SelectFocused(next_value)))
    }

    fn browser_window_focused_text_control_owns_navigation_key(
        app: &BrowserApp,
        key: Key,
    ) -> Result<bool> {
        Ok(browser_window_focused_control_accepts_text_input(app)?
            && matches!(
                key,
                Key::Up
                    | Key::Down
                    | Key::Left
                    | Key::Right
                    | Key::PageUp
                    | Key::PageDown
                    | Key::Home
                    | Key::End
            ))
    }

    fn browser_window_has_focusable_controls(app: &BrowserApp) -> Result<bool> {
        Ok(app.active_session()?.current_forms().iter().any(|form| {
            form.controls.iter().any(|control| {
                if control.disabled {
                    return false;
                }
                matches!(
                    control.kind.to_ascii_lowercase().as_str(),
                    "text"
                        | "search"
                        | "url"
                        | "email"
                        | "password"
                        | "tel"
                        | "number"
                        | "date"
                        | "datetime-local"
                        | "month"
                        | "time"
                        | "week"
                        | "color"
                        | "textarea"
                        | "select"
                        | "checkbox"
                        | "radio"
                        | "submit"
                        | "reset"
                )
            })
        }))
    }

    fn browser_window_hover_status_for_window(
        app: &BrowserApp,
        mode: &BrowserWindowMode,
        window: &Window,
    ) -> Result<Option<String>> {
        if !matches!(mode, BrowserWindowMode::Page) {
            return Ok(None);
        }
        let Some((x, y)) = window.get_unscaled_mouse_pos(MouseMode::Discard) else {
            return browser_window_viewport_status_text(app);
        };
        let (x, y) = browser_window_mouse_position_to_pixels(x, y);
        let hit = app.hit_test_window(x, y)?;
        browser_window_page_status_for_hit(app, hit)
    }

    fn browser_window_mouse_position_to_pixels(x: f32, y: f32) -> (usize, usize) {
        (
            browser_window_mouse_coordinate_to_pixel(x),
            browser_window_mouse_coordinate_to_pixel(y),
        )
    }

    fn browser_window_mouse_coordinate_to_pixel(value: f32) -> usize {
        if !value.is_finite() || value <= 0.0 {
            return 0;
        }
        value.floor() as usize
    }

    fn browser_window_hover_status_text(
        app: &BrowserApp,
        hit: BrowserAppWindowHit,
    ) -> Result<Option<String>> {
        let status = match hit {
            BrowserAppWindowHit::BackButton => browser_window_history_target_status(app, true)?
                .unwrap_or_else(|| "Back unavailable".to_owned()),
            BrowserAppWindowHit::ForwardButton => browser_window_history_target_status(app, false)?
                .unwrap_or_else(|| "Forward unavailable".to_owned()),
            BrowserAppWindowHit::ReloadButton => {
                format!("Reload {}", browser_window_active_status_label(app)?)
            }
            BrowserAppWindowHit::NewTabButton => "New tab".to_owned(),
            BrowserAppWindowHit::Tab { index } => {
                let Some(tab) = app
                    .tab_summaries()
                    .into_iter()
                    .find(|tab| tab.index == index)
                else {
                    return Ok(None);
                };
                let label = browser_window_status_label(&tab.title, &tab.source);
                if tab.active {
                    format!("Active tab {}: {label}", index + 1)
                } else {
                    format!("Switch to tab {}: {label}", index + 1)
                }
            }
            BrowserAppWindowHit::LocationBar => "Edit address".to_owned(),
            BrowserAppWindowHit::PageViewport { x, y } => {
                let Some(target) = app.active_link_target_at_viewport(x, y)? else {
                    return Ok(None);
                };
                target
            }
            BrowserAppWindowHit::StatusBar
            | BrowserAppWindowHit::PageFrame
            | BrowserAppWindowHit::ChromeBackground
            | BrowserAppWindowHit::Outside => return Ok(None),
        };
        Ok(Some(status))
    }

    fn browser_window_page_status_for_hit(
        app: &BrowserApp,
        hit: BrowserAppWindowHit,
    ) -> Result<Option<String>> {
        let fallback_to_viewport = matches!(hit, BrowserAppWindowHit::PageViewport { .. });
        let status = browser_window_hover_status_text(app, hit)?;
        if status.is_some() {
            return Ok(status);
        }
        if fallback_to_viewport {
            browser_window_viewport_status_text(app)
        } else {
            Ok(None)
        }
    }

    fn browser_window_viewport_status_text(app: &BrowserApp) -> Result<Option<String>> {
        let viewport = app.active_viewport()?;
        if viewport.x == 0 && viewport.y == 0 {
            return Ok(None);
        }
        Ok(Some(format!(
            "Viewport {}x{} at {},{}",
            viewport.width, viewport.height, viewport.x, viewport.y
        )))
    }

    fn browser_window_history_target_status(
        app: &BrowserApp,
        backwards: bool,
    ) -> Result<Option<String>> {
        let history = app.active_session()?.snapshot();
        let Some(current_index) = history.current_index else {
            return Ok(None);
        };
        let target_index = if backwards {
            current_index.checked_sub(1)
        } else {
            let next_index = current_index.saturating_add(1);
            (next_index < history.entries.len()).then_some(next_index)
        };
        let Some(target_index) = target_index else {
            return Ok(None);
        };
        let Some(entry) = history.entries.get(target_index) else {
            return Ok(None);
        };
        let label = browser_window_status_label(&entry.title, &entry.source);
        Ok(Some(if backwards {
            format!("Back to {label}")
        } else {
            format!("Forward to {label}")
        }))
    }

    fn browser_window_active_status_label(app: &BrowserApp) -> Result<String> {
        let current = app
            .active_session()?
            .current()
            .ok_or_else(|| anyhow::anyhow!("browser app has no current page"))?;
        Ok(browser_window_status_label(&current.title, &current.source))
    }

    fn browser_window_status_label(title: &str, source: &str) -> String {
        let title = title.trim();
        if title.is_empty() {
            source.to_owned()
        } else {
            title.to_owned()
        }
    }

    async fn handle_browser_window_left_click(
        app: &mut BrowserApp,
        mode: &mut BrowserWindowMode,
        window_x: usize,
        window_y: usize,
        hit: BrowserAppWindowHit,
        modifiers: BrowserWindowModifiers,
    ) -> Result<BrowserWindowKeyResult> {
        match hit {
            BrowserAppWindowHit::LocationBar => {
                let source = current_browser_window_source(app)?;
                begin_browser_window_location_input(mode, &source);
                Ok(BrowserWindowKeyResult {
                    dirty: true,
                    close: false,
                })
            }
            BrowserAppWindowHit::NewTabButton => {
                app.apply_action(BrowserAppAction::NewBlankTab).await?;
                begin_browser_window_blank_location_input(mode);
                Ok(BrowserWindowKeyResult {
                    dirty: true,
                    close: false,
                })
            }
            BrowserAppWindowHit::PageViewport { x, y } if modifiers.command => {
                let dismissed_prompt = !matches!(mode, BrowserWindowMode::Page);
                *mode = BrowserWindowMode::Page;
                let before_tabs = app.tab_count();
                let action = if modifiers.shift {
                    BrowserAppAction::OpenClickInForegroundTab { x, y }
                } else {
                    BrowserAppAction::OpenClickInBackgroundTab { x, y }
                };
                app.apply_action(action).await?;
                Ok(BrowserWindowKeyResult {
                    dirty: dismissed_prompt || app.tab_count() != before_tabs,
                    close: false,
                })
            }
            _ => {
                let dismissed_prompt = !matches!(mode, BrowserWindowMode::Page);
                *mode = BrowserWindowMode::Page;
                let report = app.click_window(window_x, window_y).await?;
                Ok(BrowserWindowKeyResult {
                    dirty: dismissed_prompt || report.applied,
                    close: false,
                })
            }
        }
    }

    async fn handle_browser_window_middle_click(
        app: &mut BrowserApp,
        mode: &mut BrowserWindowMode,
        hit: BrowserAppWindowHit,
    ) -> Result<BrowserWindowKeyResult> {
        match hit {
            BrowserAppWindowHit::Tab { index } => {
                *mode = BrowserWindowMode::Page;
                if app.tab_count() > 1 {
                    app.apply_action(BrowserAppAction::CloseTab(Some(index)))
                        .await?;
                    return Ok(BrowserWindowKeyResult {
                        dirty: true,
                        close: false,
                    });
                }
                Ok(BrowserWindowKeyResult {
                    dirty: false,
                    close: true,
                })
            }
            BrowserAppWindowHit::PageViewport { x, y } => {
                let dismissed_prompt = !matches!(mode, BrowserWindowMode::Page);
                *mode = BrowserWindowMode::Page;
                let before_tabs = app.tab_count();
                app.apply_action(BrowserAppAction::OpenClickInBackgroundTab { x, y })
                    .await?;
                Ok(BrowserWindowKeyResult {
                    dirty: dismissed_prompt || app.tab_count() != before_tabs,
                    close: false,
                })
            }
            _ => Ok(BrowserWindowKeyResult::default()),
        }
    }

    fn browser_window_tab_cycle_action(
        app: &BrowserApp,
        backwards: bool,
    ) -> Result<BrowserAppAction> {
        let tab_count = app.tab_count().max(1);
        let active_tab = app.active_tab();
        let target_tab = if backwards {
            active_tab.checked_sub(1).unwrap_or(tab_count - 1)
        } else {
            (active_tab + 1) % tab_count
        };
        Ok(BrowserAppAction::SwitchTab(target_tab))
    }

    fn browser_window_modifiers(window: &Window) -> BrowserWindowModifiers {
        BrowserWindowModifiers {
            command: window.is_key_down(Key::LeftCtrl)
                || window.is_key_down(Key::RightCtrl)
                || window.is_key_down(Key::LeftSuper)
                || window.is_key_down(Key::RightSuper),
            shift: window.is_key_down(Key::LeftShift) || window.is_key_down(Key::RightShift),
            alt: window.is_key_down(Key::LeftAlt) || window.is_key_down(Key::RightAlt),
        }
    }

    fn drain_browser_window_input_chars(chars: &Rc<RefCell<Vec<char>>>) -> String {
        chars.borrow_mut().drain(..).collect()
    }

    fn browser_window_title(frame: &BrowserAppWindowFrame, mode: &BrowserWindowMode) -> String {
        if let Some(location) = browser_window_location_text(mode) {
            return format!("{BROWSER_WINDOW_TITLE_PREFIX} - Location: {location}");
        }
        if let Some(query) = browser_window_find_text(mode) {
            return format!("{BROWSER_WINDOW_TITLE_PREFIX} - Find: {query}");
        }
        if frame.report.title.trim().is_empty() {
            BROWSER_WINDOW_TITLE_PREFIX.to_owned()
        } else {
            format!("{BROWSER_WINDOW_TITLE_PREFIX} - {}", frame.report.title)
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn browser_window_mouse_position_to_pixels_floors_fractional_coordinates() {
            assert_eq!(
                browser_window_mouse_position_to_pixels(7.99, 12.01),
                (7, 12)
            );
            assert_eq!(
                browser_window_mouse_position_to_pixels(-1.0, f32::NAN),
                (0, 0)
            );
            assert_eq!(
                browser_window_mouse_position_to_pixels(f32::INFINITY, 3.75),
                (0, 3)
            );
        }

        #[tokio::test]
        async fn browser_window_key_repeat_policy_limits_repeated_keys_by_context() {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("repeat-policy.html");
            std::fs::write(
                &path,
                r#"<!doctype html>
<html>
<head><title>Repeat Policy</title></head>
<body>
<form>
  <input name="q" value="rust">
  <select name="kind">
    <option value="alpha">Alpha</option>
    <option value="beta">Beta</option>
  </select>
</form>
<p>line 1</p><p>line 2</p><p>line 3</p><p>line 4</p>
</body>
</html>"#,
            )
            .unwrap();
            let mut app = BrowserApp::open(
                path.to_str().unwrap(),
                BrowserAppOptions {
                    render: BrowserRenderOptions {
                        width: 40,
                        ..BrowserRenderOptions::default()
                    },
                    viewport_width: 40,
                    viewport_height: 3,
                    raster: BrowserRasterOptions::default(),
                },
            )
            .await
            .unwrap();
            let page_mode = BrowserWindowMode::Page;
            let modifiers = BrowserWindowModifiers::default();

            assert!(
                browser_window_key_repeat_enabled(&app, &page_mode, Key::Down, modifiers).unwrap()
            );
            assert!(
                browser_window_key_repeat_enabled(&app, &page_mode, Key::PageDown, modifiers)
                    .unwrap()
            );
            assert!(
                browser_window_key_repeat_enabled(&app, &page_mode, Key::Space, modifiers).unwrap()
            );
            assert!(
                !browser_window_key_repeat_enabled(&app, &page_mode, Key::Tab, modifiers).unwrap()
            );
            assert!(
                !browser_window_key_repeat_enabled(
                    &app,
                    &page_mode,
                    Key::T,
                    BrowserWindowModifiers {
                        command: true,
                        shift: false,
                        alt: false,
                    },
                )
                .unwrap()
            );

            let location_mode = BrowserWindowMode::Location {
                text: "https://example.test".to_owned(),
                replace_on_input: false,
            };
            assert!(
                browser_window_key_repeat_enabled(&app, &location_mode, Key::Backspace, modifiers)
                    .unwrap()
            );
            assert!(
                browser_window_key_repeat_enabled(&app, &location_mode, Key::Delete, modifiers)
                    .unwrap()
            );
            assert!(
                !browser_window_key_repeat_enabled(&app, &location_mode, Key::Enter, modifiers)
                    .unwrap()
            );

            app.apply_action(BrowserAppAction::FocusNext).await.unwrap();
            assert_eq!(
                app.active_session()
                    .unwrap()
                    .focused_control()
                    .unwrap()
                    .kind,
                "text"
            );
            assert!(
                browser_window_key_repeat_enabled(&app, &page_mode, Key::Backspace, modifiers)
                    .unwrap()
            );
            assert!(
                !browser_window_key_repeat_enabled(&app, &page_mode, Key::Down, modifiers).unwrap()
            );
            assert!(
                !browser_window_key_repeat_enabled(&app, &page_mode, Key::Space, modifiers)
                    .unwrap()
            );

            app.apply_action(BrowserAppAction::FocusNext).await.unwrap();
            assert_eq!(
                app.active_session()
                    .unwrap()
                    .focused_control()
                    .unwrap()
                    .kind,
                "select"
            );
            assert!(
                browser_window_key_repeat_enabled(&app, &page_mode, Key::Down, modifiers).unwrap()
            );
            assert!(
                browser_window_key_repeat_enabled(&app, &page_mode, Key::Up, modifiers).unwrap()
            );
            assert!(
                !browser_window_key_repeat_enabled(&app, &page_mode, Key::Space, modifiers)
                    .unwrap()
            );
            assert!(
                !browser_window_key_repeat_enabled(&app, &page_mode, Key::Backspace, modifiers)
                    .unwrap()
            );
        }

        #[tokio::test]
        async fn browser_window_page_keys_use_viewport_sized_scrolls_and_end() {
            let mut app = BrowserApp::open(
                "bench/browser-fixtures/list-marker-types.html",
                BrowserAppOptions {
                    render: BrowserRenderOptions {
                        width: 40,
                        ..BrowserRenderOptions::default()
                    },
                    viewport_width: 40,
                    viewport_height: 4,
                    raster: BrowserRasterOptions::default(),
                },
            )
            .await
            .unwrap();
            let mut mode = BrowserWindowMode::Page;

            let page_down = handle_browser_window_key(
                &mut app,
                &mut mode,
                Key::PageDown,
                BrowserWindowModifiers::default(),
            )
            .await
            .unwrap();
            assert!(page_down.dirty);
            assert_eq!(app.active_viewport().unwrap().y, 3);

            let page_up = handle_browser_window_key(
                &mut app,
                &mut mode,
                Key::PageUp,
                BrowserWindowModifiers::default(),
            )
            .await
            .unwrap();
            assert!(page_up.dirty);
            assert_eq!(app.active_viewport().unwrap().y, 0);

            let end = handle_browser_window_key(
                &mut app,
                &mut mode,
                Key::End,
                BrowserWindowModifiers::default(),
            )
            .await
            .unwrap();
            assert!(end.dirty);
            let end_y = app.active_viewport().unwrap().y;
            assert!(end_y > 0);

            let home = handle_browser_window_key(
                &mut app,
                &mut mode,
                Key::Home,
                BrowserWindowModifiers::default(),
            )
            .await
            .unwrap();
            assert!(home.dirty);
            assert_eq!(app.active_viewport().unwrap().y, 0);
        }

        #[tokio::test]
        async fn browser_window_shift_home_end_jump_horizontal_edges() {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("wide.html");
            std::fs::write(
                &path,
                r#"<html><head><title>Wide</title></head><body>
<p>top</p>
<pre>wide-line-00000000001111111111222222222233333333334444444444</pre>
<p>bottom</p>
</body></html>"#,
            )
            .unwrap();
            let mut app = BrowserApp::open(
                path.to_str().unwrap(),
                BrowserAppOptions {
                    render: BrowserRenderOptions {
                        width: 10,
                        ..BrowserRenderOptions::default()
                    },
                    viewport_width: 10,
                    viewport_height: 2,
                    raster: BrowserRasterOptions::default(),
                },
            )
            .await
            .unwrap();
            app.apply_action(BrowserAppAction::SetViewportOrigin { x: 20, y: 1 })
                .await
                .unwrap();
            let scrolled = app.active_viewport().unwrap();
            assert!(scrolled.x > 0);
            assert_eq!(scrolled.y, 1);
            let mut mode = BrowserWindowMode::Page;
            let modifiers = BrowserWindowModifiers {
                command: false,
                shift: true,
                alt: false,
            };

            let row_start = handle_browser_window_key(&mut app, &mut mode, Key::Home, modifiers)
                .await
                .unwrap();

            assert!(row_start.dirty);
            assert_eq!(mode, BrowserWindowMode::Page);
            let at_row_start = app.active_viewport().unwrap();
            assert_eq!(at_row_start.x, 0);
            assert_eq!(at_row_start.y, scrolled.y);

            let row_end = handle_browser_window_key(&mut app, &mut mode, Key::End, modifiers)
                .await
                .unwrap();

            assert!(row_end.dirty);
            let at_row_end = app.active_viewport().unwrap();
            assert!(at_row_end.x > at_row_start.x);
            assert_eq!(at_row_end.y, scrolled.y);
        }

        #[tokio::test]
        async fn browser_window_shift_page_keys_scroll_horizontally_by_viewport() {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("wide-page.html");
            std::fs::write(
                &path,
                r#"<html><head><title>Wide Page</title></head><body>
<p>top</p>
<pre>wide-line-000000000011111111112222222222333333333344444444445555555555</pre>
<p>bottom</p>
</body></html>"#,
            )
            .unwrap();
            let mut app = BrowserApp::open(
                path.to_str().unwrap(),
                BrowserAppOptions {
                    render: BrowserRenderOptions {
                        width: 10,
                        ..BrowserRenderOptions::default()
                    },
                    viewport_width: 10,
                    viewport_height: 2,
                    raster: BrowserRasterOptions::default(),
                },
            )
            .await
            .unwrap();
            app.apply_action(BrowserAppAction::SetViewportOrigin { x: 20, y: 1 })
                .await
                .unwrap();
            let start = app.active_viewport().unwrap();
            assert_eq!(start.x, 20);
            assert_eq!(start.y, 1);
            let mut mode = BrowserWindowMode::Page;
            let modifiers = BrowserWindowModifiers {
                command: false,
                shift: true,
                alt: false,
            };

            let page_left = handle_browser_window_key(&mut app, &mut mode, Key::PageUp, modifiers)
                .await
                .unwrap();

            assert!(page_left.dirty);
            assert_eq!(mode, BrowserWindowMode::Page);
            let after_left = app.active_viewport().unwrap();
            assert_eq!(after_left.x, 11);
            assert_eq!(after_left.y, start.y);

            let page_right =
                handle_browser_window_key(&mut app, &mut mode, Key::PageDown, modifiers)
                    .await
                    .unwrap();

            assert!(page_right.dirty);
            let after_right = app.active_viewport().unwrap();
            assert_eq!(after_right.x, start.x);
            assert_eq!(after_right.y, start.y);
        }

        #[tokio::test]
        async fn browser_window_space_scrolls_by_viewport_when_page_has_focus() {
            let mut app = BrowserApp::open(
                "bench/browser-fixtures/list-marker-types.html",
                BrowserAppOptions {
                    render: BrowserRenderOptions {
                        width: 40,
                        ..BrowserRenderOptions::default()
                    },
                    viewport_width: 40,
                    viewport_height: 4,
                    raster: BrowserRasterOptions::default(),
                },
            )
            .await
            .unwrap();
            let mut mode = BrowserWindowMode::Page;

            let space_down = handle_browser_window_key(
                &mut app,
                &mut mode,
                Key::Space,
                BrowserWindowModifiers::default(),
            )
            .await
            .unwrap();
            assert!(space_down.dirty);
            assert_eq!(app.active_viewport().unwrap().y, 3);

            let space_up = handle_browser_window_key(
                &mut app,
                &mut mode,
                Key::Space,
                BrowserWindowModifiers {
                    command: false,
                    shift: true,
                    alt: false,
                },
            )
            .await
            .unwrap();
            assert!(space_up.dirty);
            assert_eq!(app.active_viewport().unwrap().y, 0);
        }

        #[tokio::test]
        async fn browser_window_tab_noops_without_focusable_controls() {
            let mut app = BrowserApp::open(
                "bench/browser-fixtures/static-text.html",
                BrowserAppOptions {
                    render: BrowserRenderOptions {
                        width: 40,
                        ..BrowserRenderOptions::default()
                    },
                    viewport_width: 40,
                    viewport_height: 4,
                    raster: BrowserRasterOptions::default(),
                },
            )
            .await
            .unwrap();
            assert!(app.active_session().unwrap().current_forms().is_empty());
            let mut mode = BrowserWindowMode::Page;

            let tab = handle_browser_window_key(
                &mut app,
                &mut mode,
                Key::Tab,
                BrowserWindowModifiers::default(),
            )
            .await
            .unwrap();

            assert!(!tab.dirty);
            assert!(!tab.close);
            assert_eq!(mode, BrowserWindowMode::Page);
        }

        #[tokio::test]
        async fn browser_window_command_up_down_jump_document_edges() {
            let mut app = BrowserApp::open(
                "bench/browser-fixtures/list-marker-types.html",
                BrowserAppOptions {
                    render: BrowserRenderOptions {
                        width: 40,
                        ..BrowserRenderOptions::default()
                    },
                    viewport_width: 40,
                    viewport_height: 4,
                    raster: BrowserRasterOptions::default(),
                },
            )
            .await
            .unwrap();
            let mut mode = BrowserWindowMode::Find {
                text: "stale prompt".to_owned(),
                replace_on_input: false,
            };
            let modifiers = BrowserWindowModifiers {
                command: true,
                shift: false,
                alt: false,
            };

            let down = handle_browser_window_key(&mut app, &mut mode, Key::Down, modifiers)
                .await
                .unwrap();
            assert!(down.dirty);
            assert_eq!(mode, BrowserWindowMode::Page);
            let bottom = app.active_viewport().unwrap().y;
            assert!(bottom > 0);

            mode = BrowserWindowMode::Location {
                text: "stale location".to_owned(),
                replace_on_input: false,
            };
            let up = handle_browser_window_key(&mut app, &mut mode, Key::Up, modifiers)
                .await
                .unwrap();
            assert!(up.dirty);
            assert_eq!(mode, BrowserWindowMode::Page);
            assert_eq!(app.active_viewport().unwrap().y, 0);
        }

        #[tokio::test]
        async fn browser_window_command_brackets_navigate_history() {
            let dir = tempfile::tempdir().unwrap();
            let first = dir.path().join("first.html");
            let second = dir.path().join("second.html");
            std::fs::write(
                &first,
                r#"<html><head><title>First</title></head><body>First</body></html>"#,
            )
            .unwrap();
            std::fs::write(
                &second,
                r#"<html><head><title>Second</title></head><body>Second</body></html>"#,
            )
            .unwrap();
            let mut app = BrowserApp::open(
                first.to_str().unwrap(),
                BrowserAppOptions {
                    render: BrowserRenderOptions {
                        width: 40,
                        ..BrowserRenderOptions::default()
                    },
                    viewport_width: 40,
                    viewport_height: 4,
                    raster: BrowserRasterOptions::default(),
                },
            )
            .await
            .unwrap();
            app.apply_action(BrowserAppAction::Open(second.to_str().unwrap().to_owned()))
                .await
                .unwrap();
            assert_eq!(
                app.active_session().unwrap().current().unwrap().title,
                "Second"
            );
            let mut mode = BrowserWindowMode::Find {
                text: "stale".to_owned(),
                replace_on_input: false,
            };
            let modifiers = BrowserWindowModifiers {
                command: true,
                shift: false,
                alt: false,
            };

            let back = handle_browser_window_key(&mut app, &mut mode, Key::LeftBracket, modifiers)
                .await
                .unwrap();
            assert!(back.dirty);
            assert_eq!(mode, BrowserWindowMode::Page);
            assert_eq!(
                app.active_session().unwrap().current().unwrap().title,
                "First"
            );

            mode = BrowserWindowMode::Location {
                text: second.to_string_lossy().into_owned(),
                replace_on_input: false,
            };
            let forward =
                handle_browser_window_key(&mut app, &mut mode, Key::RightBracket, modifiers)
                    .await
                    .unwrap();
            assert!(forward.dirty);
            assert_eq!(mode, BrowserWindowMode::Page);
            assert_eq!(
                app.active_session().unwrap().current().unwrap().title,
                "Second"
            );
        }

        #[tokio::test]
        async fn browser_window_history_shortcuts_noop_without_target() {
            let mut app = BrowserApp::open(
                "bench/browser-fixtures/static-text.html",
                BrowserAppOptions {
                    render: BrowserRenderOptions {
                        width: 40,
                        ..BrowserRenderOptions::default()
                    },
                    viewport_width: 40,
                    viewport_height: 4,
                    raster: BrowserRasterOptions::default(),
                },
            )
            .await
            .unwrap();
            let mut mode = BrowserWindowMode::Page;

            let unavailable_back = handle_browser_window_key(
                &mut app,
                &mut mode,
                Key::LeftBracket,
                BrowserWindowModifiers {
                    command: true,
                    shift: false,
                    alt: false,
                },
            )
            .await
            .unwrap();

            assert!(!unavailable_back.dirty);
            assert!(!unavailable_back.close);
            assert_eq!(mode, BrowserWindowMode::Page);
            assert_eq!(
                app.active_session().unwrap().current().unwrap().title,
                "Static Text Fixture"
            );

            mode = BrowserWindowMode::Location {
                text: "editing".to_owned(),
                replace_on_input: false,
            };
            let unavailable_forward = handle_browser_window_key(
                &mut app,
                &mut mode,
                Key::Right,
                BrowserWindowModifiers {
                    command: false,
                    shift: false,
                    alt: true,
                },
            )
            .await
            .unwrap();

            assert!(!unavailable_forward.dirty);
            assert!(!unavailable_forward.close);
            assert_eq!(
                mode,
                BrowserWindowMode::Location {
                    text: "editing".to_owned(),
                    replace_on_input: false,
                }
            );
        }

        #[tokio::test]
        async fn browser_window_command_shift_brackets_cycle_tabs() {
            let mut app = BrowserApp::open(
                "bench/browser-fixtures/static-text.html",
                BrowserAppOptions {
                    render: BrowserRenderOptions {
                        width: 40,
                        ..BrowserRenderOptions::default()
                    },
                    viewport_width: 40,
                    viewport_height: 4,
                    raster: BrowserRasterOptions::default(),
                },
            )
            .await
            .unwrap();
            app.apply_action(BrowserAppAction::DuplicateTab)
                .await
                .unwrap();
            app.apply_action(BrowserAppAction::DuplicateTab)
                .await
                .unwrap();
            assert_eq!(app.tab_count(), 3);
            assert_eq!(app.active_tab(), 2);
            let mut mode = BrowserWindowMode::Find {
                text: "still active".to_owned(),
                replace_on_input: false,
            };
            let modifiers = BrowserWindowModifiers {
                command: true,
                shift: true,
                alt: false,
            };

            let previous =
                handle_browser_window_key(&mut app, &mut mode, Key::LeftBracket, modifiers)
                    .await
                    .unwrap();
            assert!(previous.dirty);
            assert_eq!(app.active_tab(), 1);
            assert_eq!(
                mode,
                BrowserWindowMode::Find {
                    text: "still active".to_owned(),
                    replace_on_input: false,
                }
            );

            let next = handle_browser_window_key(&mut app, &mut mode, Key::RightBracket, modifiers)
                .await
                .unwrap();
            assert!(next.dirty);
            assert_eq!(app.active_tab(), 2);
        }

        #[tokio::test]
        async fn browser_window_alt_arrows_navigate_history_from_transient_mode() {
            let dir = tempfile::tempdir().unwrap();
            let first = dir.path().join("first.html");
            let second = dir.path().join("second.html");
            std::fs::write(
                &first,
                r#"<html><head><title>First</title></head><body>First</body></html>"#,
            )
            .unwrap();
            std::fs::write(
                &second,
                r#"<html><head><title>Second</title></head><body>Second</body></html>"#,
            )
            .unwrap();
            let mut app = BrowserApp::open(
                first.to_str().unwrap(),
                BrowserAppOptions {
                    render: BrowserRenderOptions {
                        width: 40,
                        ..BrowserRenderOptions::default()
                    },
                    viewport_width: 40,
                    viewport_height: 4,
                    raster: BrowserRasterOptions::default(),
                },
            )
            .await
            .unwrap();
            app.apply_action(BrowserAppAction::Open(second.to_str().unwrap().to_owned()))
                .await
                .unwrap();
            let mut mode = BrowserWindowMode::Location {
                text: second.to_string_lossy().into_owned(),
                replace_on_input: false,
            };
            let modifiers = BrowserWindowModifiers {
                command: false,
                shift: false,
                alt: true,
            };

            let back = handle_browser_window_key(&mut app, &mut mode, Key::Left, modifiers)
                .await
                .unwrap();
            assert!(back.dirty);
            assert_eq!(mode, BrowserWindowMode::Page);
            assert_eq!(
                app.active_session().unwrap().current().unwrap().title,
                "First"
            );

            let forward = handle_browser_window_key(&mut app, &mut mode, Key::Right, modifiers)
                .await
                .unwrap();
            assert!(forward.dirty);
            assert_eq!(
                app.active_session().unwrap().current().unwrap().title,
                "Second"
            );
        }

        #[tokio::test]
        async fn browser_window_command_tab_shortcuts_manage_tabs() {
            let mut app = BrowserApp::open(
                "bench/browser-fixtures/static-text.html",
                BrowserAppOptions {
                    render: BrowserRenderOptions {
                        width: 40,
                        ..BrowserRenderOptions::default()
                    },
                    viewport_width: 40,
                    viewport_height: 4,
                    raster: BrowserRasterOptions::default(),
                },
            )
            .await
            .unwrap();
            let mut mode = BrowserWindowMode::Page;
            let modifiers = BrowserWindowModifiers {
                command: true,
                shift: false,
                alt: false,
            };

            let new_tab = handle_browser_window_key(&mut app, &mut mode, Key::T, modifiers)
                .await
                .unwrap();
            assert!(new_tab.dirty);
            assert_eq!(app.tab_count(), 2);
            assert_eq!(app.active_tab(), 1);
            assert_eq!(
                mode,
                BrowserWindowMode::Location {
                    text: String::new(),
                    replace_on_input: false,
                }
            );
            assert_eq!(
                app.active_session()
                    .unwrap()
                    .current()
                    .unwrap()
                    .source
                    .as_str(),
                "about:blank"
            );

            let forward_tab = handle_browser_window_key(&mut app, &mut mode, Key::Tab, modifiers)
                .await
                .unwrap();
            assert!(forward_tab.dirty);
            assert_eq!(app.active_tab(), 0);

            let backward_tab = handle_browser_window_key(
                &mut app,
                &mut mode,
                Key::Tab,
                BrowserWindowModifiers {
                    command: true,
                    shift: true,
                    alt: false,
                },
            )
            .await
            .unwrap();
            assert!(backward_tab.dirty);
            assert_eq!(app.active_tab(), 1);

            let close_tab = handle_browser_window_key(&mut app, &mut mode, Key::W, modifiers)
                .await
                .unwrap();
            assert!(close_tab.dirty);
            assert_eq!(app.tab_count(), 1);
            assert_eq!(app.active_tab(), 0);
        }

        #[tokio::test]
        async fn browser_window_new_tab_button_opens_blank_tab_and_focuses_location() {
            let mut app = BrowserApp::open(
                "bench/browser-fixtures/static-text.html",
                BrowserAppOptions {
                    render: BrowserRenderOptions {
                        width: 40,
                        ..BrowserRenderOptions::default()
                    },
                    viewport_width: 40,
                    viewport_height: 4,
                    raster: BrowserRasterOptions::default(),
                },
            )
            .await
            .unwrap();
            let mut mode = BrowserWindowMode::Find {
                text: "old prompt".to_owned(),
                replace_on_input: false,
            };

            let result = handle_browser_window_left_click(
                &mut app,
                &mut mode,
                0,
                0,
                BrowserAppWindowHit::NewTabButton,
                BrowserWindowModifiers::default(),
            )
            .await
            .unwrap();

            assert!(result.dirty);
            assert!(!result.close);
            assert_eq!(app.tab_count(), 2);
            assert_eq!(app.active_tab(), 1);
            assert_eq!(
                app.active_session()
                    .unwrap()
                    .current()
                    .unwrap()
                    .source
                    .as_str(),
                "about:blank"
            );
            assert_eq!(
                mode,
                BrowserWindowMode::Location {
                    text: String::new(),
                    replace_on_input: false,
                }
            );
        }

        #[tokio::test]
        async fn browser_window_command_page_keys_cycle_tabs() {
            let mut app = BrowserApp::open(
                "bench/browser-fixtures/static-text.html",
                BrowserAppOptions {
                    render: BrowserRenderOptions {
                        width: 40,
                        ..BrowserRenderOptions::default()
                    },
                    viewport_width: 40,
                    viewport_height: 4,
                    raster: BrowserRasterOptions::default(),
                },
            )
            .await
            .unwrap();
            let mut mode = BrowserWindowMode::Page;
            let modifiers = BrowserWindowModifiers {
                command: true,
                shift: false,
                alt: false,
            };

            handle_browser_window_key(&mut app, &mut mode, Key::T, modifiers)
                .await
                .unwrap();
            handle_browser_window_key(&mut app, &mut mode, Key::T, modifiers)
                .await
                .unwrap();
            assert_eq!(app.tab_count(), 3);
            assert_eq!(app.active_tab(), 2);

            let previous = handle_browser_window_key(&mut app, &mut mode, Key::PageUp, modifiers)
                .await
                .unwrap();
            assert!(previous.dirty);
            assert_eq!(app.active_tab(), 1);

            let next = handle_browser_window_key(&mut app, &mut mode, Key::PageDown, modifiers)
                .await
                .unwrap();
            assert!(next.dirty);
            assert_eq!(app.active_tab(), 2);
        }

        #[tokio::test]
        async fn browser_window_location_alt_enter_opens_new_tab() {
            let mut app = BrowserApp::open(
                "bench/browser-fixtures/static-text.html",
                BrowserAppOptions {
                    render: BrowserRenderOptions {
                        width: 40,
                        ..BrowserRenderOptions::default()
                    },
                    viewport_width: 40,
                    viewport_height: 4,
                    raster: BrowserRasterOptions::default(),
                },
            )
            .await
            .unwrap();
            let mut mode = BrowserWindowMode::Location {
                text: "list-marker-types.html".to_owned(),
                replace_on_input: false,
            };

            let result = handle_browser_window_key(
                &mut app,
                &mut mode,
                Key::Enter,
                BrowserWindowModifiers {
                    command: false,
                    shift: false,
                    alt: true,
                },
            )
            .await
            .unwrap();

            assert!(result.dirty);
            assert_eq!(mode, BrowserWindowMode::Page);
            assert_eq!(app.tab_count(), 2);
            assert_eq!(app.active_tab(), 1);
            assert_eq!(
                app.active_session()
                    .unwrap()
                    .current()
                    .unwrap()
                    .source
                    .as_str(),
                "bench/browser-fixtures/list-marker-types.html"
            );
        }

        #[tokio::test]
        async fn browser_window_location_numpad_enter_opens_in_active_tab() {
            let mut app = BrowserApp::open(
                "bench/browser-fixtures/static-text.html",
                BrowserAppOptions {
                    render: BrowserRenderOptions {
                        width: 40,
                        ..BrowserRenderOptions::default()
                    },
                    viewport_width: 40,
                    viewport_height: 4,
                    raster: BrowserRasterOptions::default(),
                },
            )
            .await
            .unwrap();
            let mut mode = BrowserWindowMode::Location {
                text: "list-marker-types.html".to_owned(),
                replace_on_input: false,
            };

            let result = handle_browser_window_key(
                &mut app,
                &mut mode,
                Key::NumPadEnter,
                BrowserWindowModifiers::default(),
            )
            .await
            .unwrap();

            assert!(result.dirty);
            assert_eq!(mode, BrowserWindowMode::Page);
            assert_eq!(app.tab_count(), 1);
            assert_eq!(
                app.active_session()
                    .unwrap()
                    .current()
                    .unwrap()
                    .source
                    .as_str(),
                "bench/browser-fixtures/list-marker-types.html"
            );
        }

        #[tokio::test]
        async fn browser_window_command_w_closes_window_on_last_tab() {
            let mut app = BrowserApp::open(
                "bench/browser-fixtures/static-text.html",
                BrowserAppOptions {
                    render: BrowserRenderOptions {
                        width: 40,
                        ..BrowserRenderOptions::default()
                    },
                    viewport_width: 40,
                    viewport_height: 4,
                    raster: BrowserRasterOptions::default(),
                },
            )
            .await
            .unwrap();
            let mut mode = BrowserWindowMode::Page;

            let result = handle_browser_window_key(
                &mut app,
                &mut mode,
                Key::W,
                BrowserWindowModifiers {
                    command: true,
                    shift: false,
                    alt: false,
                },
            )
            .await
            .unwrap();

            assert!(!result.dirty);
            assert!(result.close);
            assert_eq!(app.tab_count(), 1);
        }

        #[tokio::test]
        async fn browser_window_escape_does_not_close_page_mode() {
            let mut app = BrowserApp::open(
                "bench/browser-fixtures/static-text.html",
                BrowserAppOptions {
                    render: BrowserRenderOptions {
                        width: 40,
                        ..BrowserRenderOptions::default()
                    },
                    viewport_width: 40,
                    viewport_height: 4,
                    raster: BrowserRasterOptions::default(),
                },
            )
            .await
            .unwrap();
            let mut mode = BrowserWindowMode::Page;

            let result = handle_browser_window_key(
                &mut app,
                &mut mode,
                Key::Escape,
                BrowserWindowModifiers::default(),
            )
            .await
            .unwrap();

            assert!(!result.dirty);
            assert!(!result.close);
            assert_eq!(mode, BrowserWindowMode::Page);

            mode = BrowserWindowMode::Location {
                text: "open prompt".to_owned(),
                replace_on_input: false,
            };
            let prompt_cancel = handle_browser_window_key(
                &mut app,
                &mut mode,
                Key::Escape,
                BrowserWindowModifiers::default(),
            )
            .await
            .unwrap();

            assert!(prompt_cancel.dirty);
            assert!(!prompt_cancel.close);
            assert_eq!(mode, BrowserWindowMode::Page);
        }

        #[tokio::test]
        async fn browser_window_hover_status_text_describes_chrome_controls() {
            let dir = tempfile::tempdir().unwrap();
            let first = dir.path().join("first.html");
            let second = dir.path().join("second.html");
            std::fs::write(
                &first,
                r#"<html><head><title>First</title></head><body>First</body></html>"#,
            )
            .unwrap();
            std::fs::write(
                &second,
                r#"<html><head><title>Second</title></head><body>Second</body></html>"#,
            )
            .unwrap();

            let mut app = BrowserApp::open(
                &first.to_string_lossy(),
                BrowserAppOptions {
                    render: BrowserRenderOptions {
                        width: 40,
                        ..BrowserRenderOptions::default()
                    },
                    viewport_width: 40,
                    viewport_height: 4,
                    raster: BrowserRasterOptions::default(),
                },
            )
            .await
            .unwrap();

            assert_eq!(
                browser_window_hover_status_text(&app, BrowserAppWindowHit::BackButton).unwrap(),
                Some("Back unavailable".to_owned())
            );
            assert_eq!(
                browser_window_hover_status_text(&app, BrowserAppWindowHit::ReloadButton).unwrap(),
                Some("Reload First".to_owned())
            );
            assert_eq!(
                browser_window_hover_status_text(&app, BrowserAppWindowHit::LocationBar).unwrap(),
                Some("Edit address".to_owned())
            );

            app.apply_action(BrowserAppAction::Open(
                second.to_string_lossy().into_owned(),
            ))
            .await
            .unwrap();
            assert_eq!(
                browser_window_hover_status_text(&app, BrowserAppWindowHit::BackButton).unwrap(),
                Some("Back to First".to_owned())
            );
            assert_eq!(
                browser_window_hover_status_text(&app, BrowserAppWindowHit::ForwardButton).unwrap(),
                Some("Forward unavailable".to_owned())
            );

            app.apply_action(BrowserAppAction::Back).await.unwrap();
            assert_eq!(
                browser_window_hover_status_text(&app, BrowserAppWindowHit::ForwardButton).unwrap(),
                Some("Forward to Second".to_owned())
            );

            app.apply_action(BrowserAppAction::DuplicateTab)
                .await
                .unwrap();
            assert_eq!(
                browser_window_hover_status_text(&app, BrowserAppWindowHit::Tab { index: 0 })
                    .unwrap(),
                Some("Switch to tab 1: First".to_owned())
            );
            assert_eq!(
                browser_window_hover_status_text(&app, BrowserAppWindowHit::Tab { index: 1 })
                    .unwrap(),
                Some("Active tab 2: First".to_owned())
            );
            assert_eq!(
                browser_window_hover_status_text(&app, BrowserAppWindowHit::NewTabButton).unwrap(),
                Some("New tab".to_owned())
            );
            assert_eq!(
                browser_window_hover_status_text(
                    &app,
                    BrowserAppWindowHit::PageViewport { x: 0, y: 0 },
                )
                .unwrap(),
                None
            );
        }

        #[tokio::test]
        async fn browser_window_hover_status_text_describes_page_link_target() {
            let dir = tempfile::tempdir().unwrap();
            let first = dir.path().join("first.html");
            let second = dir.path().join("second.html");
            std::fs::write(
                &first,
                r#"<html><head><title>First</title></head><body><a href="second.html">Second</a></body></html>"#,
            )
            .unwrap();
            std::fs::write(
                &second,
                r#"<html><head><title>Second</title></head><body>Second</body></html>"#,
            )
            .unwrap();

            let app = BrowserApp::open(
                &first.to_string_lossy(),
                BrowserAppOptions {
                    render: BrowserRenderOptions {
                        width: 40,
                        ..BrowserRenderOptions::default()
                    },
                    viewport_width: 40,
                    viewport_height: 4,
                    raster: BrowserRasterOptions::default(),
                },
            )
            .await
            .unwrap();

            assert_eq!(
                browser_window_hover_status_text(
                    &app,
                    BrowserAppWindowHit::PageViewport { x: 0, y: 0 },
                )
                .unwrap(),
                Some(second.to_string_lossy().into_owned())
            );
            assert_eq!(
                browser_window_hover_status_text(
                    &app,
                    BrowserAppWindowHit::PageViewport { x: 20, y: 0 },
                )
                .unwrap(),
                None
            );
        }

        #[tokio::test]
        async fn browser_window_page_status_reports_scrolled_viewport() {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("wide-page.html");
            std::fs::write(
                &path,
                r#"<html><head><title>Wide Page</title></head><body>
<p>top</p>
<pre>wide-line-000000000011111111112222222222333333333344444444445555555555</pre>
<p>bottom</p>
</body></html>"#,
            )
            .unwrap();
            let mut app = BrowserApp::open(
                path.to_str().unwrap(),
                BrowserAppOptions {
                    render: BrowserRenderOptions {
                        width: 10,
                        ..BrowserRenderOptions::default()
                    },
                    viewport_width: 10,
                    viewport_height: 2,
                    raster: BrowserRasterOptions::default(),
                },
            )
            .await
            .unwrap();

            assert_eq!(browser_window_viewport_status_text(&app).unwrap(), None);
            assert_eq!(
                browser_window_page_status_for_hit(
                    &app,
                    BrowserAppWindowHit::PageViewport { x: 0, y: 0 },
                )
                .unwrap(),
                None
            );

            app.apply_action(BrowserAppAction::SetViewportOrigin { x: 20, y: 1 })
                .await
                .unwrap();

            assert_eq!(
                browser_window_viewport_status_text(&app).unwrap(),
                Some("Viewport 10x2 at 20,1".to_owned())
            );
            assert_eq!(
                browser_window_page_status_for_hit(
                    &app,
                    BrowserAppWindowHit::PageViewport { x: 0, y: 0 },
                )
                .unwrap(),
                Some("Viewport 10x2 at 20,1".to_owned())
            );
        }

        #[tokio::test]
        async fn browser_window_middle_click_closes_tabs() {
            let mut app = BrowserApp::open(
                "bench/browser-fixtures/static-text.html",
                BrowserAppOptions {
                    render: BrowserRenderOptions {
                        width: 40,
                        ..BrowserRenderOptions::default()
                    },
                    viewport_width: 40,
                    viewport_height: 4,
                    raster: BrowserRasterOptions::default(),
                },
            )
            .await
            .unwrap();
            app.apply_action(BrowserAppAction::DuplicateTab)
                .await
                .unwrap();
            app.apply_action(BrowserAppAction::DuplicateTab)
                .await
                .unwrap();
            assert_eq!(app.tab_count(), 3);
            assert_eq!(app.active_tab(), 2);

            let mut mode = BrowserWindowMode::Find {
                text: "open prompt".to_owned(),
                replace_on_input: false,
            };
            let closed_first = handle_browser_window_middle_click(
                &mut app,
                &mut mode,
                BrowserAppWindowHit::Tab { index: 0 },
            )
            .await
            .unwrap();
            assert!(closed_first.dirty);
            assert!(!closed_first.close);
            assert_eq!(mode, BrowserWindowMode::Page);
            assert_eq!(app.tab_count(), 2);
            assert_eq!(app.active_tab(), 1);

            app.apply_action(BrowserAppAction::CloseTab(None))
                .await
                .unwrap();
            assert_eq!(app.tab_count(), 1);
            let close_window = handle_browser_window_middle_click(
                &mut app,
                &mut mode,
                BrowserAppWindowHit::Tab { index: 0 },
            )
            .await
            .unwrap();
            assert!(!close_window.dirty);
            assert!(close_window.close);
            assert_eq!(app.tab_count(), 1);
        }

        #[tokio::test]
        async fn browser_window_command_click_opens_page_link_in_background_tab() {
            let dir = tempfile::tempdir().unwrap();
            let first = dir.path().join("first.html");
            let second = dir.path().join("second.html");
            std::fs::write(
                &first,
                r#"<html><head><title>First</title></head><body><a href="second.html">Second</a></body></html>"#,
            )
            .unwrap();
            std::fs::write(
                &second,
                r#"<html><head><title>Second</title></head><body>Arrived</body></html>"#,
            )
            .unwrap();

            let mut app = BrowserApp::open(
                &first.to_string_lossy(),
                BrowserAppOptions {
                    render: BrowserRenderOptions {
                        width: 40,
                        ..BrowserRenderOptions::default()
                    },
                    viewport_width: 40,
                    viewport_height: 4,
                    raster: BrowserRasterOptions::default(),
                },
            )
            .await
            .unwrap();
            app.present_frame().unwrap();
            let mut mode = BrowserWindowMode::Find {
                text: "open prompt".to_owned(),
                replace_on_input: false,
            };

            let result = handle_browser_window_left_click(
                &mut app,
                &mut mode,
                0,
                0,
                BrowserAppWindowHit::PageViewport { x: 0, y: 0 },
                BrowserWindowModifiers {
                    command: true,
                    shift: false,
                    alt: false,
                },
            )
            .await
            .unwrap();

            assert!(result.dirty);
            assert!(!result.close);
            assert_eq!(mode, BrowserWindowMode::Page);
            assert_eq!(app.tab_count(), 2);
            assert_eq!(app.active_tab(), 0);
            assert_eq!(
                app.active_session().unwrap().current().unwrap().title,
                "First"
            );
            app.apply_action(BrowserAppAction::SwitchTab(1))
                .await
                .unwrap();
            assert_eq!(
                app.active_session().unwrap().current().unwrap().title,
                "Second"
            );
        }

        #[tokio::test]
        async fn browser_window_command_shift_click_opens_page_link_in_foreground_tab() {
            let dir = tempfile::tempdir().unwrap();
            let first = dir.path().join("first.html");
            let second = dir.path().join("second.html");
            std::fs::write(
                &first,
                r#"<html><head><title>First</title></head><body><a href="second.html">Second</a></body></html>"#,
            )
            .unwrap();
            std::fs::write(
                &second,
                r#"<html><head><title>Second</title></head><body>Arrived</body></html>"#,
            )
            .unwrap();

            let mut app = BrowserApp::open(
                &first.to_string_lossy(),
                BrowserAppOptions {
                    render: BrowserRenderOptions {
                        width: 40,
                        ..BrowserRenderOptions::default()
                    },
                    viewport_width: 40,
                    viewport_height: 4,
                    raster: BrowserRasterOptions::default(),
                },
            )
            .await
            .unwrap();
            app.present_frame().unwrap();
            let mut mode = BrowserWindowMode::Find {
                text: "open prompt".to_owned(),
                replace_on_input: false,
            };

            let result = handle_browser_window_left_click(
                &mut app,
                &mut mode,
                0,
                0,
                BrowserAppWindowHit::PageViewport { x: 0, y: 0 },
                BrowserWindowModifiers {
                    command: true,
                    shift: true,
                    alt: false,
                },
            )
            .await
            .unwrap();

            assert!(result.dirty);
            assert!(!result.close);
            assert_eq!(mode, BrowserWindowMode::Page);
            assert_eq!(app.tab_count(), 2);
            assert_eq!(app.active_tab(), 1);
            assert_eq!(
                app.active_session().unwrap().current().unwrap().title,
                "Second"
            );
            app.apply_action(BrowserAppAction::SwitchTab(0))
                .await
                .unwrap();
            assert_eq!(
                app.active_session().unwrap().current().unwrap().title,
                "First"
            );
        }

        #[tokio::test]
        async fn browser_window_left_click_dismisses_prompt_even_without_page_action() {
            let mut app = BrowserApp::open(
                "bench/browser-fixtures/static-text.html",
                BrowserAppOptions {
                    render: BrowserRenderOptions {
                        width: 40,
                        ..BrowserRenderOptions::default()
                    },
                    viewport_width: 40,
                    viewport_height: 4,
                    raster: BrowserRasterOptions::default(),
                },
            )
            .await
            .unwrap();
            let mut mode = BrowserWindowMode::Find {
                text: "open prompt".to_owned(),
                replace_on_input: false,
            };

            let chrome_background = handle_browser_window_left_click(
                &mut app,
                &mut mode,
                0,
                0,
                BrowserAppWindowHit::ChromeBackground,
                BrowserWindowModifiers::default(),
            )
            .await
            .unwrap();

            assert!(chrome_background.dirty);
            assert!(!chrome_background.close);
            assert_eq!(mode, BrowserWindowMode::Page);
            assert_eq!(app.tab_count(), 1);

            mode = BrowserWindowMode::Location {
                text: "stale prompt".to_owned(),
                replace_on_input: false,
            };
            let command_click_non_link = handle_browser_window_left_click(
                &mut app,
                &mut mode,
                0,
                0,
                BrowserAppWindowHit::PageViewport { x: 20, y: 0 },
                BrowserWindowModifiers {
                    command: true,
                    shift: false,
                    alt: false,
                },
            )
            .await
            .unwrap();

            assert!(command_click_non_link.dirty);
            assert!(!command_click_non_link.close);
            assert_eq!(mode, BrowserWindowMode::Page);
            assert_eq!(app.tab_count(), 1);
        }

        #[tokio::test]
        async fn browser_window_middle_click_opens_page_link_in_background_tab() {
            let dir = tempfile::tempdir().unwrap();
            let first = dir.path().join("first.html");
            let second = dir.path().join("second.html");
            std::fs::write(
                &first,
                r#"<html><head><title>First</title></head><body><a href="second.html">Second</a></body></html>"#,
            )
            .unwrap();
            std::fs::write(
                &second,
                r#"<html><head><title>Second</title></head><body>Arrived</body></html>"#,
            )
            .unwrap();

            let mut app = BrowserApp::open(
                &first.to_string_lossy(),
                BrowserAppOptions {
                    render: BrowserRenderOptions {
                        width: 40,
                        ..BrowserRenderOptions::default()
                    },
                    viewport_width: 40,
                    viewport_height: 4,
                    raster: BrowserRasterOptions::default(),
                },
            )
            .await
            .unwrap();
            app.present_frame().unwrap();
            let mut mode = BrowserWindowMode::Find {
                text: "open prompt".to_owned(),
                replace_on_input: false,
            };

            let result = handle_browser_window_middle_click(
                &mut app,
                &mut mode,
                BrowserAppWindowHit::PageViewport { x: 0, y: 0 },
            )
            .await
            .unwrap();

            assert!(result.dirty);
            assert!(!result.close);
            assert_eq!(mode, BrowserWindowMode::Page);
            assert_eq!(app.tab_count(), 2);
            assert_eq!(app.active_tab(), 0);
            assert_eq!(
                app.active_session().unwrap().current().unwrap().title,
                "First"
            );
            app.apply_action(BrowserAppAction::SwitchTab(1))
                .await
                .unwrap();
            assert_eq!(
                app.active_session().unwrap().current().unwrap().title,
                "Second"
            );
        }

        #[tokio::test]
        async fn browser_window_middle_click_dismisses_prompt_on_non_link_page() {
            let mut app = BrowserApp::open(
                "bench/browser-fixtures/static-text.html",
                BrowserAppOptions {
                    render: BrowserRenderOptions {
                        width: 40,
                        ..BrowserRenderOptions::default()
                    },
                    viewport_width: 40,
                    viewport_height: 4,
                    raster: BrowserRasterOptions::default(),
                },
            )
            .await
            .unwrap();
            let mut mode = BrowserWindowMode::Find {
                text: "open prompt".to_owned(),
                replace_on_input: false,
            };

            let result = handle_browser_window_middle_click(
                &mut app,
                &mut mode,
                BrowserAppWindowHit::PageViewport { x: 20, y: 0 },
            )
            .await
            .unwrap();

            assert!(result.dirty);
            assert!(!result.close);
            assert_eq!(mode, BrowserWindowMode::Page);
            assert_eq!(app.tab_count(), 1);
            assert_eq!(app.active_tab(), 0);
        }

        #[tokio::test]
        async fn browser_window_command_shift_t_restores_closed_tab() {
            let mut app = BrowserApp::open(
                "bench/browser-fixtures/static-text.html",
                BrowserAppOptions {
                    render: BrowserRenderOptions {
                        width: 40,
                        ..BrowserRenderOptions::default()
                    },
                    viewport_width: 40,
                    viewport_height: 4,
                    raster: BrowserRasterOptions::default(),
                },
            )
            .await
            .unwrap();
            let mut mode = BrowserWindowMode::Page;

            let no_closed_tab = handle_browser_window_key(
                &mut app,
                &mut mode,
                Key::T,
                BrowserWindowModifiers {
                    command: true,
                    shift: true,
                    alt: false,
                },
            )
            .await
            .unwrap();
            assert!(!no_closed_tab.dirty);
            assert_eq!(app.tab_count(), 1);

            app.apply_action(BrowserAppAction::DuplicateTab)
                .await
                .unwrap();
            app.apply_action(BrowserAppAction::CloseTab(None))
                .await
                .unwrap();
            assert_eq!(app.closed_tab_count(), 1);

            let restored = handle_browser_window_key(
                &mut app,
                &mut mode,
                Key::T,
                BrowserWindowModifiers {
                    command: true,
                    shift: true,
                    alt: false,
                },
            )
            .await
            .unwrap();
            assert!(restored.dirty);
            assert_eq!(app.tab_count(), 2);
            assert_eq!(app.closed_tab_count(), 0);
            assert_eq!(app.active_tab(), 1);
        }

        #[tokio::test]
        async fn browser_window_command_number_shortcuts_switch_tabs() {
            let mut app = BrowserApp::open(
                "bench/browser-fixtures/static-text.html",
                BrowserAppOptions {
                    render: BrowserRenderOptions {
                        width: 40,
                        ..BrowserRenderOptions::default()
                    },
                    viewport_width: 40,
                    viewport_height: 4,
                    raster: BrowserRasterOptions::default(),
                },
            )
            .await
            .unwrap();
            app.apply_action(BrowserAppAction::DuplicateTab)
                .await
                .unwrap();
            app.apply_action(BrowserAppAction::DuplicateTab)
                .await
                .unwrap();
            assert_eq!(app.tab_count(), 3);
            assert_eq!(app.active_tab(), 2);

            let mut mode = BrowserWindowMode::Page;
            let modifiers = BrowserWindowModifiers {
                command: true,
                shift: false,
                alt: false,
            };
            let first_tab = handle_browser_window_key(&mut app, &mut mode, Key::Key1, modifiers)
                .await
                .unwrap();
            assert!(first_tab.dirty);
            assert_eq!(app.active_tab(), 0);

            let second_tab =
                handle_browser_window_key(&mut app, &mut mode, Key::NumPad2, modifiers)
                    .await
                    .unwrap();
            assert!(second_tab.dirty);
            assert_eq!(app.active_tab(), 1);

            let last_tab = handle_browser_window_key(&mut app, &mut mode, Key::NumPad9, modifiers)
                .await
                .unwrap();
            assert!(last_tab.dirty);
            assert_eq!(app.active_tab(), 2);

            let active_tab = handle_browser_window_key(&mut app, &mut mode, Key::Key9, modifiers)
                .await
                .unwrap();
            assert!(!active_tab.dirty);
            assert_eq!(app.active_tab(), 2);

            let missing_tab =
                handle_browser_window_key(&mut app, &mut mode, Key::NumPad8, modifiers)
                    .await
                    .unwrap();
            assert!(!missing_tab.dirty);
            assert_eq!(app.active_tab(), 2);
        }

        #[test]
        fn browser_window_wheel_scroll_action_keeps_horizontal_delta() {
            assert_eq!(
                browser_window_wheel_scroll_action(0.0, 0.0, BrowserWindowModifiers::default()),
                None
            );
            assert_eq!(
                browser_window_wheel_scroll_action(1.0, -2.0, BrowserWindowModifiers::default()),
                Some(BrowserAppAction::Scroll {
                    delta_x: -3,
                    delta_y: 6,
                })
            );
            assert_eq!(
                browser_window_wheel_scroll_action(
                    0.0,
                    -2.0,
                    BrowserWindowModifiers {
                        command: false,
                        shift: true,
                        alt: false,
                    },
                ),
                Some(BrowserAppAction::Scroll {
                    delta_x: 6,
                    delta_y: 0,
                })
            );
        }

        #[tokio::test]
        async fn browser_window_escape_blurs_focused_control_for_page_navigation() {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("escape-blur.html");
            std::fs::write(
                &path,
                r#"<!doctype html>
<html>
<head><title>Escape Blur Fixture</title></head>
<body>
<form><input name="q" value="search"></form>
<p>top</p>
<p>middle</p>
<p>bottom</p>
</body>
</html>"#,
            )
            .unwrap();
            let mut app = BrowserApp::open(
                path.to_str().unwrap(),
                BrowserAppOptions {
                    render: BrowserRenderOptions {
                        width: 40,
                        ..BrowserRenderOptions::default()
                    },
                    viewport_width: 40,
                    viewport_height: 1,
                    raster: BrowserRasterOptions::default(),
                },
            )
            .await
            .unwrap();
            app.apply_action(BrowserAppAction::FocusNext).await.unwrap();
            assert!(app.active_session().unwrap().focused_control().is_some());

            let mut mode = BrowserWindowMode::Page;
            let focused_arrow = handle_browser_window_key(
                &mut app,
                &mut mode,
                Key::Down,
                BrowserWindowModifiers::default(),
            )
            .await
            .unwrap();
            assert!(!focused_arrow.dirty);
            assert_eq!(app.active_viewport().unwrap().y, 0);

            let blur = handle_browser_window_key(
                &mut app,
                &mut mode,
                Key::Escape,
                BrowserWindowModifiers::default(),
            )
            .await
            .unwrap();
            assert!(blur.dirty);
            assert_eq!(mode, BrowserWindowMode::Page);
            assert!(app.active_session().unwrap().focused_control().is_none());

            let page_arrow = handle_browser_window_key(
                &mut app,
                &mut mode,
                Key::Down,
                BrowserWindowModifiers::default(),
            )
            .await
            .unwrap();
            assert!(page_arrow.dirty);
            assert!(app.active_viewport().unwrap().y > 0);
        }

        #[tokio::test]
        async fn browser_window_page_click_blurs_focused_control_for_navigation() {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("click-blur.html");
            std::fs::write(
                &path,
                r#"<!doctype html>
<html>
<head><title>Click Blur Fixture</title></head>
<body>
<form><input name="q" value="search"></form>
<p>ordinary page text</p>
<p>middle</p>
<p>bottom</p>
<p>after bottom</p>
<p>final line</p>
</body>
</html>"#,
            )
            .unwrap();
            let mut app = BrowserApp::open(
                path.to_str().unwrap(),
                BrowserAppOptions {
                    render: BrowserRenderOptions {
                        width: 40,
                        ..BrowserRenderOptions::default()
                    },
                    viewport_width: 40,
                    viewport_height: 4,
                    raster: BrowserRasterOptions::default(),
                },
            )
            .await
            .unwrap();
            app.apply_action(BrowserAppAction::FocusNext).await.unwrap();
            assert!(app.active_session().unwrap().focused_control().is_some());

            let mut mode = BrowserWindowMode::Page;
            let focused_arrow = handle_browser_window_key(
                &mut app,
                &mut mode,
                Key::Down,
                BrowserWindowModifiers::default(),
            )
            .await
            .unwrap();
            assert!(!focused_arrow.dirty);
            assert_eq!(app.active_viewport().unwrap().y, 0);

            app.apply_action(BrowserAppAction::Click { x: 0, y: 1 })
                .await
                .unwrap();
            assert!(app.active_session().unwrap().focused_control().is_none());

            let page_arrow = handle_browser_window_key(
                &mut app,
                &mut mode,
                Key::Down,
                BrowserWindowModifiers::default(),
            )
            .await
            .unwrap();
            assert!(page_arrow.dirty);
            assert!(app.active_viewport().unwrap().y > 0);
        }

        #[tokio::test]
        async fn browser_window_arrow_keys_step_focused_select_without_scrolling() {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("select-arrow.html");
            std::fs::write(
                &path,
                r#"<!doctype html>
<html>
<head><title>Select Arrow Fixture</title></head>
<body>
<form>
  <select name="kind">
    <option value="alpha">Alpha</option>
    <option value="beta" disabled>Beta</option>
    <option value="gamma">Gamma</option>
  </select>
</form>
<p>ordinary page text</p>
<p>middle</p>
<p>bottom</p>
<p>after bottom</p>
<p>final line</p>
</body>
</html>"#,
            )
            .unwrap();
            let mut app = BrowserApp::open(
                path.to_str().unwrap(),
                BrowserAppOptions {
                    render: BrowserRenderOptions {
                        width: 40,
                        ..BrowserRenderOptions::default()
                    },
                    viewport_width: 40,
                    viewport_height: 3,
                    raster: BrowserRasterOptions::default(),
                },
            )
            .await
            .unwrap();
            app.apply_action(BrowserAppAction::FocusNext).await.unwrap();

            let focused = app.active_session().unwrap().focused_control().unwrap();
            assert_eq!(focused.kind, "select");
            assert_eq!(focused.value, "alpha");
            assert_eq!(app.active_viewport().unwrap().y, 0);

            let mut mode = BrowserWindowMode::Page;
            let down = handle_browser_window_key(
                &mut app,
                &mut mode,
                Key::Down,
                BrowserWindowModifiers::default(),
            )
            .await
            .unwrap();
            assert!(down.dirty);
            assert_eq!(
                app.active_session()
                    .unwrap()
                    .focused_control()
                    .unwrap()
                    .value,
                "gamma"
            );
            assert_eq!(app.active_viewport().unwrap().y, 0);

            let bottom_down = handle_browser_window_key(
                &mut app,
                &mut mode,
                Key::Down,
                BrowserWindowModifiers::default(),
            )
            .await
            .unwrap();
            assert!(!bottom_down.dirty);
            assert_eq!(
                app.active_session()
                    .unwrap()
                    .focused_control()
                    .unwrap()
                    .value,
                "gamma"
            );
            assert_eq!(app.active_viewport().unwrap().y, 0);

            let up = handle_browser_window_key(
                &mut app,
                &mut mode,
                Key::Up,
                BrowserWindowModifiers::default(),
            )
            .await
            .unwrap();
            assert!(up.dirty);
            assert_eq!(
                app.active_session()
                    .unwrap()
                    .focused_control()
                    .unwrap()
                    .value,
                "alpha"
            );
            assert_eq!(app.active_viewport().unwrap().y, 0);
        }

        #[tokio::test]
        async fn browser_window_home_end_choose_focused_select_edges_without_scrolling() {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("select-home-end.html");
            std::fs::write(
                &path,
                r#"<!doctype html>
<html>
<head><title>Select Home End Fixture</title></head>
<body>
<form>
  <select name="kind">
    <option value="disabled-start" disabled>Disabled Start</option>
    <option value="alpha">Alpha</option>
    <option value="beta" selected>Beta</option>
    <option value="gamma">Gamma</option>
    <option value="disabled-end" disabled>Disabled End</option>
  </select>
</form>
<p>ordinary page text</p>
<p>middle</p>
<p>bottom</p>
<p>after bottom</p>
<p>final line</p>
</body>
</html>"#,
            )
            .unwrap();
            let mut app = BrowserApp::open(
                path.to_str().unwrap(),
                BrowserAppOptions {
                    render: BrowserRenderOptions {
                        width: 40,
                        ..BrowserRenderOptions::default()
                    },
                    viewport_width: 40,
                    viewport_height: 3,
                    raster: BrowserRasterOptions::default(),
                },
            )
            .await
            .unwrap();
            app.apply_action(BrowserAppAction::FocusNext).await.unwrap();

            let focused = app.active_session().unwrap().focused_control().unwrap();
            assert_eq!(focused.kind, "select");
            assert_eq!(focused.value, "beta");
            assert_eq!(app.active_viewport().unwrap().y, 0);

            let mut mode = BrowserWindowMode::Page;
            let modifiers = BrowserWindowModifiers::default();
            assert!(browser_window_key_repeat_enabled(&app, &mode, Key::Home, modifiers).unwrap());
            assert!(browser_window_key_repeat_enabled(&app, &mode, Key::End, modifiers).unwrap());

            let end = handle_browser_window_key(&mut app, &mut mode, Key::End, modifiers)
                .await
                .unwrap();
            assert!(end.dirty);
            assert_eq!(
                app.active_session()
                    .unwrap()
                    .focused_control()
                    .unwrap()
                    .value,
                "gamma"
            );
            assert_eq!(app.active_viewport().unwrap().y, 0);

            let repeated_end = handle_browser_window_key(&mut app, &mut mode, Key::End, modifiers)
                .await
                .unwrap();
            assert!(!repeated_end.dirty);
            assert_eq!(
                app.active_session()
                    .unwrap()
                    .focused_control()
                    .unwrap()
                    .value,
                "gamma"
            );
            assert_eq!(app.active_viewport().unwrap().y, 0);

            let home = handle_browser_window_key(&mut app, &mut mode, Key::Home, modifiers)
                .await
                .unwrap();
            assert!(home.dirty);
            assert_eq!(
                app.active_session()
                    .unwrap()
                    .focused_control()
                    .unwrap()
                    .value,
                "alpha"
            );
            assert_eq!(app.active_viewport().unwrap().y, 0);

            let repeated_home =
                handle_browser_window_key(&mut app, &mut mode, Key::Home, modifiers)
                    .await
                    .unwrap();
            assert!(!repeated_home.dirty);
            assert_eq!(
                app.active_session()
                    .unwrap()
                    .focused_control()
                    .unwrap()
                    .value,
                "alpha"
            );
            assert_eq!(app.active_viewport().unwrap().y, 0);
        }

        #[tokio::test]
        async fn browser_window_page_keys_choose_focused_select_edges_without_scrolling() {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("select-page-keys.html");
            std::fs::write(
                &path,
                r#"<!doctype html>
<html>
<head><title>Select Page Keys Fixture</title></head>
<body>
<form>
  <select name="kind">
    <option value="disabled-start" disabled>Disabled Start</option>
    <option value="alpha">Alpha</option>
    <option value="beta" selected>Beta</option>
    <option value="gamma">Gamma</option>
    <option value="disabled-end" disabled>Disabled End</option>
  </select>
</form>
<p>ordinary page text</p>
<p>middle</p>
<p>bottom</p>
<p>after bottom</p>
<p>final line</p>
</body>
</html>"#,
            )
            .unwrap();
            let mut app = BrowserApp::open(
                path.to_str().unwrap(),
                BrowserAppOptions {
                    render: BrowserRenderOptions {
                        width: 40,
                        ..BrowserRenderOptions::default()
                    },
                    viewport_width: 40,
                    viewport_height: 3,
                    raster: BrowserRasterOptions::default(),
                },
            )
            .await
            .unwrap();
            app.apply_action(BrowserAppAction::FocusNext).await.unwrap();

            let focused = app.active_session().unwrap().focused_control().unwrap();
            assert_eq!(focused.kind, "select");
            assert_eq!(focused.value, "beta");
            assert_eq!(app.active_viewport().unwrap().y, 0);

            let mut mode = BrowserWindowMode::Page;
            let modifiers = BrowserWindowModifiers::default();
            assert!(
                browser_window_key_repeat_enabled(&app, &mode, Key::PageUp, modifiers).unwrap()
            );
            assert!(
                browser_window_key_repeat_enabled(&app, &mode, Key::PageDown, modifiers).unwrap()
            );

            let page_down =
                handle_browser_window_key(&mut app, &mut mode, Key::PageDown, modifiers)
                    .await
                    .unwrap();
            assert!(page_down.dirty);
            assert_eq!(
                app.active_session()
                    .unwrap()
                    .focused_control()
                    .unwrap()
                    .value,
                "gamma"
            );
            assert_eq!(app.active_viewport().unwrap().y, 0);

            let repeated_page_down =
                handle_browser_window_key(&mut app, &mut mode, Key::PageDown, modifiers)
                    .await
                    .unwrap();
            assert!(!repeated_page_down.dirty);
            assert_eq!(
                app.active_session()
                    .unwrap()
                    .focused_control()
                    .unwrap()
                    .value,
                "gamma"
            );
            assert_eq!(app.active_viewport().unwrap().y, 0);

            let page_up = handle_browser_window_key(&mut app, &mut mode, Key::PageUp, modifiers)
                .await
                .unwrap();
            assert!(page_up.dirty);
            assert_eq!(
                app.active_session()
                    .unwrap()
                    .focused_control()
                    .unwrap()
                    .value,
                "alpha"
            );
            assert_eq!(app.active_viewport().unwrap().y, 0);

            let repeated_page_up =
                handle_browser_window_key(&mut app, &mut mode, Key::PageUp, modifiers)
                    .await
                    .unwrap();
            assert!(!repeated_page_up.dirty);
            assert_eq!(
                app.active_session()
                    .unwrap()
                    .focused_control()
                    .unwrap()
                    .value,
                "alpha"
            );
            assert_eq!(app.active_viewport().unwrap().y, 0);
        }

        #[tokio::test]
        async fn browser_window_find_mode_enters_query_and_scrolls_to_match() {
            let mut app = BrowserApp::open(
                "bench/browser-fixtures/static-text.html",
                BrowserAppOptions {
                    render: BrowserRenderOptions {
                        width: 40,
                        ..BrowserRenderOptions::default()
                    },
                    viewport_width: 40,
                    viewport_height: 1,
                    raster: BrowserRasterOptions::default(),
                },
            )
            .await
            .unwrap();
            let mut mode = BrowserWindowMode::Page;

            let open_find = handle_browser_window_key(
                &mut app,
                &mut mode,
                Key::F,
                BrowserWindowModifiers {
                    command: true,
                    shift: false,
                    alt: false,
                },
            )
            .await
            .unwrap();
            assert!(open_find.dirty);
            assert_eq!(browser_window_find_text(&mode), Some(""));
            assert_eq!(
                browser_window_frame_options(&mode).location_text,
                Some("Find > ".to_owned())
            );

            assert!(
                apply_browser_window_text_input(&mut app, &mut mode, "Visible")
                    .await
                    .unwrap()
            );
            assert_eq!(browser_window_find_text(&mode), Some("Visible"));

            let apply_find = handle_browser_window_key(
                &mut app,
                &mut mode,
                Key::Enter,
                BrowserWindowModifiers::default(),
            )
            .await
            .unwrap();
            assert!(apply_find.dirty);
            assert_eq!(mode, BrowserWindowMode::Page);

            let report = app.report().unwrap();
            let find = report.find.unwrap();
            assert_eq!(find.query, "Visible");
            assert_eq!(find.active_match_index, 0);
            assert_eq!(report.viewport.y, find.line);
            assert!(report.viewport.y > 0);
        }

        #[tokio::test]
        async fn browser_window_find_mode_numpad_enter_applies_query() {
            let mut app = BrowserApp::open(
                "bench/browser-fixtures/static-text.html",
                BrowserAppOptions {
                    render: BrowserRenderOptions {
                        width: 40,
                        ..BrowserRenderOptions::default()
                    },
                    viewport_width: 40,
                    viewport_height: 1,
                    raster: BrowserRasterOptions::default(),
                },
            )
            .await
            .unwrap();
            let mut mode = BrowserWindowMode::Find {
                text: "Visible".to_owned(),
                replace_on_input: false,
            };

            let apply_find = handle_browser_window_key(
                &mut app,
                &mut mode,
                Key::NumPadEnter,
                BrowserWindowModifiers::default(),
            )
            .await
            .unwrap();

            assert!(apply_find.dirty);
            assert_eq!(mode, BrowserWindowMode::Page);
            let find = app.active_find_state().unwrap().unwrap();
            assert_eq!(find.query, "Visible");
            assert_eq!(find.active_match_index, 0);
        }

        #[tokio::test]
        async fn browser_window_command_find_next_reuses_active_query() {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("find-next.html");
            std::fs::write(
                &path,
                r#"<!doctype html>
<html>
<head><title>Find Next Fixture</title></head>
<body>
<p>needle first result</p>
<p>middle line</p>
<p>needle second result</p>
</body>
</html>"#,
            )
            .unwrap();
            let mut app = BrowserApp::open(
                path.to_str().unwrap(),
                BrowserAppOptions {
                    render: BrowserRenderOptions {
                        width: 40,
                        ..BrowserRenderOptions::default()
                    },
                    viewport_width: 40,
                    viewport_height: 1,
                    raster: BrowserRasterOptions::default(),
                },
            )
            .await
            .unwrap();
            app.apply_action(BrowserAppAction::FindText {
                query: "needle".to_owned(),
                next: false,
            })
            .await
            .unwrap();
            let first_find = app.active_find_state().unwrap().unwrap();
            assert_eq!(first_find.active_match_index, 0);

            let mut mode = BrowserWindowMode::Page;
            let find_next = handle_browser_window_key(
                &mut app,
                &mut mode,
                Key::G,
                BrowserWindowModifiers {
                    command: true,
                    shift: false,
                    alt: false,
                },
            )
            .await
            .unwrap();

            assert!(find_next.dirty);
            assert_eq!(mode, BrowserWindowMode::Page);
            let second_find = app.active_find_state().unwrap().unwrap();
            assert_eq!(second_find.query, "needle");
            assert_eq!(second_find.match_count, 2);
            assert_eq!(second_find.active_match_index, 1);
            assert!(second_find.line > first_find.line);
            assert_eq!(app.active_viewport().unwrap().y, second_find.line);

            let find_previous = handle_browser_window_key(
                &mut app,
                &mut mode,
                Key::G,
                BrowserWindowModifiers {
                    command: true,
                    shift: true,
                    alt: false,
                },
            )
            .await
            .unwrap();

            assert!(find_previous.dirty);
            let previous_find = app.active_find_state().unwrap().unwrap();
            assert_eq!(previous_find.query, "needle");
            assert_eq!(previous_find.match_count, 2);
            assert_eq!(previous_find.active_match_index, 0);
            assert_eq!(app.active_viewport().unwrap().y, first_find.line);
        }

        #[tokio::test]
        async fn browser_window_find_mode_shift_enter_selects_previous_match() {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("find-previous.html");
            std::fs::write(
                &path,
                r#"<!doctype html>
<html>
<head><title>Find Previous Fixture</title></head>
<body>
<p>needle first result</p>
<p>middle line</p>
<p>needle second result</p>
</body>
</html>"#,
            )
            .unwrap();
            let mut app = BrowserApp::open(
                path.to_str().unwrap(),
                BrowserAppOptions {
                    render: BrowserRenderOptions {
                        width: 40,
                        ..BrowserRenderOptions::default()
                    },
                    viewport_width: 40,
                    viewport_height: 1,
                    raster: BrowserRasterOptions::default(),
                },
            )
            .await
            .unwrap();
            app.apply_action(BrowserAppAction::FindText {
                query: "needle".to_owned(),
                next: false,
            })
            .await
            .unwrap();
            app.apply_action(BrowserAppAction::FindText {
                query: "needle".to_owned(),
                next: true,
            })
            .await
            .unwrap();
            assert_eq!(
                app.active_find_state().unwrap().unwrap().active_match_index,
                1
            );
            let mut mode = BrowserWindowMode::Find {
                text: "needle".to_owned(),
                replace_on_input: false,
            };

            let previous = handle_browser_window_key(
                &mut app,
                &mut mode,
                Key::Enter,
                BrowserWindowModifiers {
                    command: false,
                    shift: true,
                    alt: false,
                },
            )
            .await
            .unwrap();

            assert!(previous.dirty);
            assert_eq!(mode, BrowserWindowMode::Page);
            let find = app.active_find_state().unwrap().unwrap();
            assert_eq!(find.query, "needle");
            assert_eq!(find.active_match_index, 0);
            assert_eq!(app.active_viewport().unwrap().y, find.line);
        }

        #[tokio::test]
        async fn browser_window_f3_reuses_active_find_query() {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("find-f3.html");
            std::fs::write(
                &path,
                r#"<!doctype html>
<html>
<head><title>Find F3 Fixture</title></head>
<body>
<p>needle first result</p>
<p>middle line</p>
<p>needle second result</p>
</body>
</html>"#,
            )
            .unwrap();
            let mut app = BrowserApp::open(
                path.to_str().unwrap(),
                BrowserAppOptions {
                    render: BrowserRenderOptions {
                        width: 40,
                        ..BrowserRenderOptions::default()
                    },
                    viewport_width: 40,
                    viewport_height: 1,
                    raster: BrowserRasterOptions::default(),
                },
            )
            .await
            .unwrap();
            app.apply_action(BrowserAppAction::FindText {
                query: "needle".to_owned(),
                next: false,
            })
            .await
            .unwrap();
            let first_find = app.active_find_state().unwrap().unwrap();
            let mut mode = BrowserWindowMode::Page;

            let next = handle_browser_window_key(
                &mut app,
                &mut mode,
                Key::F3,
                BrowserWindowModifiers::default(),
            )
            .await
            .unwrap();

            assert!(next.dirty);
            let second_find = app.active_find_state().unwrap().unwrap();
            assert_eq!(second_find.query, "needle");
            assert_eq!(second_find.active_match_index, 1);
            assert!(second_find.line > first_find.line);

            let previous = handle_browser_window_key(
                &mut app,
                &mut mode,
                Key::F3,
                BrowserWindowModifiers {
                    command: false,
                    shift: true,
                    alt: false,
                },
            )
            .await
            .unwrap();

            assert!(previous.dirty);
            let restored_find = app.active_find_state().unwrap().unwrap();
            assert_eq!(restored_find.active_match_index, 0);
            assert_eq!(app.active_viewport().unwrap().y, first_find.line);
        }

        #[tokio::test]
        async fn browser_window_command_f_reopens_active_find_query_selected() {
            let mut app = BrowserApp::open(
                "bench/browser-fixtures/static-text.html",
                BrowserAppOptions {
                    render: BrowserRenderOptions {
                        width: 40,
                        ..BrowserRenderOptions::default()
                    },
                    viewport_width: 40,
                    viewport_height: 1,
                    raster: BrowserRasterOptions::default(),
                },
            )
            .await
            .unwrap();
            app.apply_action(BrowserAppAction::FindText {
                query: "Visible".to_owned(),
                next: false,
            })
            .await
            .unwrap();
            let mut mode = BrowserWindowMode::Page;

            let open_find = handle_browser_window_key(
                &mut app,
                &mut mode,
                Key::F,
                BrowserWindowModifiers {
                    command: true,
                    shift: false,
                    alt: false,
                },
            )
            .await
            .unwrap();

            assert!(open_find.dirty);
            assert_eq!(browser_window_find_text(&mode), Some("Visible"));
            assert!(
                apply_browser_window_text_input(&mut app, &mut mode, "Hidden")
                    .await
                    .unwrap()
            );
            assert_eq!(browser_window_find_text(&mode), Some("Hidden"));
        }

        #[tokio::test]
        async fn browser_window_command_a_selects_prompt_text_for_replacement() {
            let mut app = BrowserApp::open(
                "bench/browser-fixtures/static-text.html",
                BrowserAppOptions {
                    render: BrowserRenderOptions {
                        width: 40,
                        ..BrowserRenderOptions::default()
                    },
                    viewport_width: 40,
                    viewport_height: 4,
                    raster: BrowserRasterOptions::default(),
                },
            )
            .await
            .unwrap();
            let modifiers = BrowserWindowModifiers {
                command: true,
                shift: false,
                alt: false,
            };
            let mut location_mode = BrowserWindowMode::Location {
                text: "bench/browser-fixtures/static-text.html".to_owned(),
                replace_on_input: false,
            };

            let select_location =
                handle_browser_window_key(&mut app, &mut location_mode, Key::A, modifiers)
                    .await
                    .unwrap();

            assert!(select_location.dirty);
            assert!(
                apply_browser_window_text_input(&mut app, &mut location_mode, "about.html")
                    .await
                    .unwrap()
            );
            assert_eq!(
                browser_window_location_text(&location_mode),
                Some("about.html")
            );

            let mut find_mode = BrowserWindowMode::Find {
                text: "Visible".to_owned(),
                replace_on_input: false,
            };
            let select_find =
                handle_browser_window_key(&mut app, &mut find_mode, Key::A, modifiers)
                    .await
                    .unwrap();

            assert!(select_find.dirty);
            assert!(
                apply_browser_window_text_input(&mut app, &mut find_mode, "Hidden")
                    .await
                    .unwrap()
            );
            assert_eq!(browser_window_find_text(&find_mode), Some("Hidden"));
        }

        #[tokio::test]
        async fn browser_window_command_delete_clears_focused_text_control() {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("form.html");
            std::fs::write(
                &path,
                r#"<html><head><title>Form</title></head><body><form><input name="q" value="filled"></form></body></html>"#,
            )
            .unwrap();
            let mut app = BrowserApp::open(
                path.to_str().unwrap(),
                BrowserAppOptions {
                    render: BrowserRenderOptions {
                        width: 40,
                        ..BrowserRenderOptions::default()
                    },
                    viewport_width: 40,
                    viewport_height: 4,
                    raster: BrowserRasterOptions::default(),
                },
            )
            .await
            .unwrap();
            app.apply_action(BrowserAppAction::FocusNext).await.unwrap();
            assert_eq!(
                app.active_session()
                    .unwrap()
                    .focused_control()
                    .unwrap()
                    .value,
                "filled"
            );
            let mut mode = BrowserWindowMode::Page;
            let modifiers = BrowserWindowModifiers {
                command: true,
                shift: false,
                alt: false,
            };

            let clear_with_backspace =
                handle_browser_window_key(&mut app, &mut mode, Key::Backspace, modifiers)
                    .await
                    .unwrap();

            assert!(clear_with_backspace.dirty);
            assert_eq!(
                app.active_session()
                    .unwrap()
                    .focused_control()
                    .unwrap()
                    .value,
                ""
            );

            app.apply_action(BrowserAppAction::TypeText("new".to_owned()))
                .await
                .unwrap();
            let clear_with_delete =
                handle_browser_window_key(&mut app, &mut mode, Key::Delete, modifiers)
                    .await
                    .unwrap();

            assert!(clear_with_delete.dirty);
            assert_eq!(
                app.active_session()
                    .unwrap()
                    .focused_control()
                    .unwrap()
                    .value,
                ""
            );
        }

        #[tokio::test]
        async fn browser_window_backspace_noops_on_focused_checkbox() {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("checkbox.html");
            std::fs::write(
                &path,
                r#"<html><head><title>Checkbox</title></head><body><form><label><input type="checkbox" name="ok">Accept</label></form></body></html>"#,
            )
            .unwrap();
            let mut app = BrowserApp::open(
                path.to_str().unwrap(),
                BrowserAppOptions {
                    render: BrowserRenderOptions {
                        width: 40,
                        ..BrowserRenderOptions::default()
                    },
                    viewport_width: 40,
                    viewport_height: 4,
                    raster: BrowserRasterOptions::default(),
                },
            )
            .await
            .unwrap();
            app.apply_action(BrowserAppAction::FocusNext).await.unwrap();
            assert_eq!(
                app.active_session()
                    .unwrap()
                    .focused_control()
                    .unwrap()
                    .kind,
                "checkbox"
            );
            assert_eq!(app.active_session().unwrap().snapshot().entries.len(), 1);
            let mut mode = BrowserWindowMode::Page;

            let backspace = handle_browser_window_key(
                &mut app,
                &mut mode,
                Key::Backspace,
                BrowserWindowModifiers::default(),
            )
            .await
            .unwrap();

            assert!(!backspace.dirty);
            assert!(!backspace.close);
            assert_eq!(mode, BrowserWindowMode::Page);
            assert_eq!(app.active_session().unwrap().snapshot().entries.len(), 1);
            assert_eq!(
                app.active_session()
                    .unwrap()
                    .focused_control()
                    .unwrap()
                    .kind,
                "checkbox"
            );

            let command_delete = handle_browser_window_key(
                &mut app,
                &mut mode,
                Key::Delete,
                BrowserWindowModifiers {
                    command: true,
                    shift: false,
                    alt: false,
                },
            )
            .await
            .unwrap();

            assert!(!command_delete.dirty);
            assert!(!command_delete.close);
            assert_eq!(mode, BrowserWindowMode::Page);
            assert_eq!(
                app.active_session()
                    .unwrap()
                    .focused_control()
                    .unwrap()
                    .kind,
                "checkbox"
            );
        }

        #[tokio::test]
        async fn browser_window_slash_opens_find_without_focused_text_control() {
            let mut app = BrowserApp::open(
                "bench/browser-fixtures/static-text.html",
                BrowserAppOptions {
                    render: BrowserRenderOptions {
                        width: 40,
                        ..BrowserRenderOptions::default()
                    },
                    viewport_width: 40,
                    viewport_height: 4,
                    raster: BrowserRasterOptions::default(),
                },
            )
            .await
            .unwrap();
            let mut mode = BrowserWindowMode::Page;

            let open_find = handle_browser_window_key(
                &mut app,
                &mut mode,
                Key::Slash,
                BrowserWindowModifiers::default(),
            )
            .await
            .unwrap();

            assert!(open_find.dirty);
            assert!(!open_find.close);
            assert_eq!(browser_window_find_text(&mode), Some(""));

            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("form.html");
            std::fs::write(
                &path,
                r#"<html><head><title>Form</title></head><body><form><input name="q" value="rust"></form></body></html>"#,
            )
            .unwrap();
            let mut app = BrowserApp::open(
                path.to_str().unwrap(),
                BrowserAppOptions {
                    render: BrowserRenderOptions {
                        width: 40,
                        ..BrowserRenderOptions::default()
                    },
                    viewport_width: 40,
                    viewport_height: 4,
                    raster: BrowserRasterOptions::default(),
                },
            )
            .await
            .unwrap();
            app.apply_action(BrowserAppAction::FocusNext).await.unwrap();
            let mut mode = BrowserWindowMode::Page;

            assert!(
                apply_browser_window_text_input(&mut app, &mut mode, "/")
                    .await
                    .unwrap()
            );
            let typed_slash = handle_browser_window_key(
                &mut app,
                &mut mode,
                Key::Slash,
                BrowserWindowModifiers::default(),
            )
            .await
            .unwrap();

            assert!(!typed_slash.dirty);
            assert!(!typed_slash.close);
            assert_eq!(mode, BrowserWindowMode::Page);
            assert_eq!(
                app.active_session()
                    .unwrap()
                    .focused_control()
                    .unwrap()
                    .value,
                "rust/"
            );
        }

        #[tokio::test]
        async fn browser_window_text_control_navigation_keys_do_not_scroll_page() {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("long-form.html");
            std::fs::write(
                &path,
                r#"<html><head><title>Long Form</title></head><body>
<form><input name="q" value="rust"></form>
<p>line 1</p><p>line 2</p><p>line 3</p><p>line 4</p><p>line 5</p>
<p>line 6</p><p>line 7</p><p>line 8</p><p>line 9</p><p>line 10</p>
</body></html>"#,
            )
            .unwrap();
            let mut app = BrowserApp::open(
                path.to_str().unwrap(),
                BrowserAppOptions {
                    render: BrowserRenderOptions {
                        width: 40,
                        ..BrowserRenderOptions::default()
                    },
                    viewport_width: 40,
                    viewport_height: 3,
                    raster: BrowserRasterOptions::default(),
                },
            )
            .await
            .unwrap();
            app.apply_action(BrowserAppAction::FocusNext).await.unwrap();
            app.apply_action(BrowserAppAction::Scroll {
                delta_x: 0,
                delta_y: 5,
            })
            .await
            .unwrap();
            let scrolled_viewport = app.active_viewport().unwrap();
            assert!(scrolled_viewport.y > 0);
            let mut mode = BrowserWindowMode::Page;

            for key in [
                Key::Up,
                Key::Down,
                Key::Left,
                Key::Right,
                Key::PageUp,
                Key::PageDown,
                Key::Home,
                Key::End,
            ] {
                let before = app.active_viewport().unwrap();
                let result = handle_browser_window_key(
                    &mut app,
                    &mut mode,
                    key,
                    BrowserWindowModifiers::default(),
                )
                .await
                .unwrap();

                assert!(!result.dirty, "{key:?} should not repaint");
                assert!(!result.close);
                assert_eq!(app.active_viewport().unwrap(), before);
                assert_eq!(mode, BrowserWindowMode::Page);
                assert_eq!(
                    app.active_session()
                        .unwrap()
                        .focused_control()
                        .unwrap()
                        .value,
                    "rust"
                );
            }
        }

        #[tokio::test]
        async fn browser_window_space_toggles_focused_checkbox() {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("checkbox.html");
            std::fs::write(
                &path,
                r#"<html><head><title>Checkbox</title></head><body><form><label><input type="checkbox" name="ok">Accept</label></form></body></html>"#,
            )
            .unwrap();
            let mut app = BrowserApp::open(
                path.to_str().unwrap(),
                BrowserAppOptions {
                    render: BrowserRenderOptions {
                        width: 40,
                        ..BrowserRenderOptions::default()
                    },
                    viewport_width: 40,
                    viewport_height: 4,
                    raster: BrowserRasterOptions::default(),
                },
            )
            .await
            .unwrap();
            app.apply_action(BrowserAppAction::FocusNext).await.unwrap();
            assert_eq!(
                app.active_session()
                    .unwrap()
                    .focused_control()
                    .unwrap()
                    .kind,
                "checkbox"
            );
            assert!(app.report().unwrap().text.contains("[ ]"));
            let mut mode = BrowserWindowMode::Page;
            assert!(
                !apply_browser_window_text_input(&mut app, &mut mode, " ")
                    .await
                    .unwrap()
            );
            assert!(app.report().unwrap().text.contains("[ ]"));

            let toggle = handle_browser_window_key(
                &mut app,
                &mut mode,
                Key::Space,
                BrowserWindowModifiers::default(),
            )
            .await
            .unwrap();

            assert!(toggle.dirty);
            assert!(app.report().unwrap().text.contains("[x]"));
        }

        #[tokio::test]
        async fn browser_window_f6_focuses_location_from_any_mode() {
            let mut app = BrowserApp::open(
                "bench/browser-fixtures/static-text.html",
                BrowserAppOptions {
                    render: BrowserRenderOptions {
                        width: 40,
                        ..BrowserRenderOptions::default()
                    },
                    viewport_width: 40,
                    viewport_height: 4,
                    raster: BrowserRasterOptions::default(),
                },
            )
            .await
            .unwrap();
            let mut mode = BrowserWindowMode::Find {
                text: "open prompt".to_owned(),
                replace_on_input: false,
            };

            let result = handle_browser_window_key(
                &mut app,
                &mut mode,
                Key::F6,
                BrowserWindowModifiers::default(),
            )
            .await
            .unwrap();

            assert!(result.dirty);
            assert!(!result.close);
            assert_eq!(
                browser_window_location_text(&mode),
                Some("bench/browser-fixtures/static-text.html")
            );
        }

        #[tokio::test]
        async fn browser_window_alt_d_focuses_location_from_any_mode() {
            let mut app = BrowserApp::open(
                "bench/browser-fixtures/static-text.html",
                BrowserAppOptions {
                    render: BrowserRenderOptions {
                        width: 40,
                        ..BrowserRenderOptions::default()
                    },
                    viewport_width: 40,
                    viewport_height: 4,
                    raster: BrowserRasterOptions::default(),
                },
            )
            .await
            .unwrap();
            let mut mode = BrowserWindowMode::Find {
                text: "open prompt".to_owned(),
                replace_on_input: false,
            };

            let result = handle_browser_window_key(
                &mut app,
                &mut mode,
                Key::D,
                BrowserWindowModifiers {
                    command: false,
                    shift: false,
                    alt: true,
                },
            )
            .await
            .unwrap();

            assert!(result.dirty);
            assert!(!result.close);
            assert_eq!(
                browser_window_location_text(&mode),
                Some("bench/browser-fixtures/static-text.html")
            );
        }

        #[tokio::test]
        async fn browser_window_about_blank_focuses_empty_location_prompt() {
            let mut app = BrowserApp::open(
                "bench/browser-fixtures/static-text.html",
                BrowserAppOptions {
                    render: BrowserRenderOptions {
                        width: 40,
                        ..BrowserRenderOptions::default()
                    },
                    viewport_width: 40,
                    viewport_height: 4,
                    raster: BrowserRasterOptions::default(),
                },
            )
            .await
            .unwrap();
            app.apply_action(BrowserAppAction::NewBlankTab)
                .await
                .unwrap();
            assert_eq!(
                app.active_session()
                    .unwrap()
                    .current()
                    .unwrap()
                    .source
                    .as_str(),
                BROWSER_ABOUT_BLANK_TARGET
            );

            let mut mode = BrowserWindowMode::Page;
            let command_l = handle_browser_window_key(
                &mut app,
                &mut mode,
                Key::L,
                BrowserWindowModifiers {
                    command: true,
                    shift: false,
                    alt: false,
                },
            )
            .await
            .unwrap();
            assert!(command_l.dirty);
            assert_eq!(browser_window_location_text(&mode), Some(""));

            mode = BrowserWindowMode::Page;
            let f6 = handle_browser_window_key(
                &mut app,
                &mut mode,
                Key::F6,
                BrowserWindowModifiers::default(),
            )
            .await
            .unwrap();
            assert!(f6.dirty);
            assert_eq!(browser_window_location_text(&mode), Some(""));

            mode = BrowserWindowMode::Page;
            let click = handle_browser_window_left_click(
                &mut app,
                &mut mode,
                0,
                0,
                BrowserAppWindowHit::LocationBar,
                BrowserWindowModifiers::default(),
            )
            .await
            .unwrap();
            assert!(click.dirty);
            assert_eq!(browser_window_location_text(&mode), Some(""));
        }

        #[tokio::test]
        async fn browser_window_f5_reloads_and_returns_to_page_mode() {
            let mut app = BrowserApp::open(
                "bench/browser-fixtures/static-text.html",
                BrowserAppOptions {
                    render: BrowserRenderOptions {
                        width: 40,
                        ..BrowserRenderOptions::default()
                    },
                    viewport_width: 40,
                    viewport_height: 1,
                    raster: BrowserRasterOptions::default(),
                },
            )
            .await
            .unwrap();
            app.apply_action(BrowserAppAction::FindText {
                query: "Visible".to_owned(),
                next: false,
            })
            .await
            .unwrap();
            assert!(app.active_find_state().unwrap().is_some());
            let mut mode = BrowserWindowMode::Find {
                text: "Visible".to_owned(),
                replace_on_input: false,
            };

            let result = handle_browser_window_key(
                &mut app,
                &mut mode,
                Key::F5,
                BrowserWindowModifiers::default(),
            )
            .await
            .unwrap();

            assert!(result.dirty);
            assert!(!result.close);
            assert_eq!(mode, BrowserWindowMode::Page);
            assert_eq!(app.active_viewport().unwrap().y, 0);
            assert!(app.active_find_state().unwrap().is_none());
        }

        #[tokio::test]
        async fn browser_window_command_r_reloads_and_returns_to_page_mode() {
            let mut app = BrowserApp::open(
                "bench/browser-fixtures/static-text.html",
                BrowserAppOptions {
                    render: BrowserRenderOptions {
                        width: 40,
                        ..BrowserRenderOptions::default()
                    },
                    viewport_width: 40,
                    viewport_height: 1,
                    raster: BrowserRasterOptions::default(),
                },
            )
            .await
            .unwrap();
            app.apply_action(BrowserAppAction::FindText {
                query: "Visible".to_owned(),
                next: false,
            })
            .await
            .unwrap();
            let mut mode = BrowserWindowMode::Find {
                text: "Visible".to_owned(),
                replace_on_input: false,
            };

            let result = handle_browser_window_key(
                &mut app,
                &mut mode,
                Key::R,
                BrowserWindowModifiers {
                    command: true,
                    shift: false,
                    alt: false,
                },
            )
            .await
            .unwrap();

            assert!(result.dirty);
            assert!(!result.close);
            assert_eq!(mode, BrowserWindowMode::Page);
            assert_eq!(app.active_viewport().unwrap().y, 0);
            assert!(app.active_find_state().unwrap().is_none());
        }

        #[tokio::test]
        async fn browser_window_resize_noops_when_viewport_cells_unchanged() {
            let raster = BrowserRasterOptions {
                cell_width: 8,
                cell_height: 12,
                padding_x: 4,
                padding_y: 4,
                ..BrowserRasterOptions::default()
            };
            let mut app = BrowserApp::open(
                "bench/browser-fixtures/static-text.html",
                BrowserAppOptions {
                    render: BrowserRenderOptions {
                        width: 40,
                        ..BrowserRenderOptions::default()
                    },
                    viewport_width: 40,
                    viewport_height: 2,
                    raster,
                },
            )
            .await
            .unwrap();

            let same_cells = handle_browser_window_resize(&mut app, (329, 76), raster)
                .await
                .unwrap();
            assert!(!same_cells);
            assert_eq!(app.active_viewport().unwrap().width, 40);
            assert_eq!(app.active_viewport().unwrap().height, 2);

            let wider = handle_browser_window_resize(&mut app, (336, 76), raster)
                .await
                .unwrap();
            assert!(wider);
            assert_eq!(app.active_viewport().unwrap().width, 41);
            assert_eq!(app.active_viewport().unwrap().height, 2);

            let still_wider = handle_browser_window_resize(&mut app, (337, 76), raster)
                .await
                .unwrap();
            assert!(!still_wider);
            assert_eq!(app.active_viewport().unwrap().width, 41);
            assert_eq!(app.active_viewport().unwrap().height, 2);
        }

        #[tokio::test]
        async fn browser_window_title_uses_blackium_starium_brand() {
            let mut app = BrowserApp::open(
                "bench/browser-fixtures/static-text.html",
                BrowserAppOptions {
                    render: BrowserRenderOptions {
                        width: 40,
                        ..BrowserRenderOptions::default()
                    },
                    viewport_width: 40,
                    viewport_height: 4,
                    raster: BrowserRasterOptions::default(),
                },
            )
            .await
            .unwrap();
            let frame = app.present_window_frame().unwrap();

            assert_eq!(
                browser_window_title(&frame, &BrowserWindowMode::Page),
                "Blackium Starium✴ - Static Text Fixture"
            );
            assert_eq!(
                browser_window_title(
                    &frame,
                    &BrowserWindowMode::Location {
                        text: "https://example.com".to_owned(),
                        replace_on_input: false,
                    },
                ),
                "Blackium Starium✴ - Location: https://example.com"
            );
            assert_eq!(
                browser_window_title(
                    &frame,
                    &BrowserWindowMode::Find {
                        text: "needle".to_owned(),
                        replace_on_input: false,
                    },
                ),
                "Blackium Starium✴ - Find: needle"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rgba_to_native_window_buffer_packs_rgb_pixels() {
        let raster = BrowserRgbaRaster {
            width: 2,
            height: 1,
            background: [255, 255, 255, 255],
            pixels: vec![255, 0, 0, 255, 0, 128, 255, 64],
        };

        let buffer = rgba_to_native_window_buffer(&raster).unwrap();

        assert_eq!(buffer, vec![0xff0000, 0x0080ff]);
    }

    #[test]
    fn rgba_to_native_window_buffer_rejects_bad_dimensions() {
        let raster = BrowserRgbaRaster {
            width: 2,
            height: 1,
            background: [255, 255, 255, 255],
            pixels: vec![255, 0, 0, 255],
        };

        assert!(rgba_to_native_window_buffer(&raster).is_err());
    }

    #[test]
    fn wheel_delta_to_scroll_cells_clamps_large_device_deltas() {
        assert_eq!(wheel_delta_to_scroll_cells(0.0), 0);
        assert_eq!(wheel_delta_to_scroll_cells(1.0), 3);
        assert_eq!(wheel_delta_to_scroll_cells(-1.0), -3);
        assert_eq!(wheel_delta_to_scroll_cells(100.0), 24);
    }

    #[test]
    fn browser_window_location_mode_edits_and_reports_chrome_overrides() {
        let mut mode = BrowserWindowMode::Page;
        assert_eq!(browser_window_location_text(&mode), None);
        assert!(!push_browser_window_location_text(&mut mode, "ignored"));

        begin_browser_window_location_input(&mut mode, "first.html");
        assert_eq!(browser_window_location_text(&mode), Some("first.html"));
        assert!(push_browser_window_location_text(&mut mode, "second.html"));
        assert_eq!(browser_window_location_text(&mode), Some("second.html"));
        assert!(push_browser_window_location_text(&mut mode, "?q=rust"));
        assert_eq!(
            browser_window_location_text(&mode),
            Some("second.html?q=rust")
        );
        assert!(delete_browser_window_location_text_backward(&mut mode));
        assert_eq!(
            browser_window_location_text(&mode),
            Some("second.html?q=rus")
        );

        let options = browser_window_frame_options(&mode);
        assert_eq!(
            options.location_text,
            Some("URL > second.html?q=rus".to_owned())
        );
        assert_eq!(
            options.status_text,
            Some("location: Enter=open Esc=cancel Backspace=delete".to_owned())
        );
    }

    #[test]
    fn browser_window_location_mode_backspace_clears_selected_source() {
        let mut mode = BrowserWindowMode::Page;
        begin_browser_window_location_input(&mut mode, "https://example.com/old");

        assert!(delete_browser_window_location_text_backward(&mut mode));
        assert_eq!(browser_window_location_text(&mode), Some(""));
        assert!(push_browser_window_location_text(
            &mut mode,
            "https://example.com/new"
        ));
        assert_eq!(
            browser_window_location_text(&mode),
            Some("https://example.com/new")
        );
    }

    #[test]
    fn browser_window_prompt_selection_can_be_deleted_or_replaced() {
        let mut location_mode = BrowserWindowMode::Location {
            text: "https://example.com/old".to_owned(),
            replace_on_input: false,
        };
        assert!(select_browser_window_prompt_text(&mut location_mode));
        assert!(delete_browser_window_location_text_backward(
            &mut location_mode
        ));
        assert_eq!(browser_window_location_text(&location_mode), Some(""));

        let mut find_mode = BrowserWindowMode::Find {
            text: "needle".to_owned(),
            replace_on_input: false,
        };
        assert!(select_browser_window_prompt_text(&mut find_mode));
        assert!(push_browser_window_find_text(&mut find_mode, "replacement"));
        assert_eq!(browser_window_find_text(&find_mode), Some("replacement"));
    }

    #[test]
    fn browser_window_prompt_text_can_be_cleared() {
        let mut location_mode = BrowserWindowMode::Location {
            text: "https://example.com/old".to_owned(),
            replace_on_input: true,
        };
        assert!(clear_browser_window_prompt_text(&mut location_mode));
        assert_eq!(browser_window_location_text(&location_mode), Some(""));
        assert!(push_browser_window_location_text(
            &mut location_mode,
            "https://example.com/new"
        ));
        assert_eq!(
            browser_window_location_text(&location_mode),
            Some("https://example.com/new")
        );

        let mut find_mode = BrowserWindowMode::Find {
            text: "needle".to_owned(),
            replace_on_input: true,
        };
        assert!(clear_browser_window_prompt_text(&mut find_mode));
        assert_eq!(browser_window_find_text(&find_mode), Some(""));
        assert!(!clear_browser_window_prompt_text(&mut find_mode));
    }

    #[test]
    fn browser_viewport_size_tracks_native_window_pixels() {
        let raster = BrowserRasterOptions {
            cell_width: 8,
            cell_height: 12,
            padding_x: 4,
            padding_y: 4,
            ..BrowserRasterOptions::default()
        };

        assert_eq!(
            browser_viewport_size_for_window_pixels(328, 76, raster),
            (40, 2)
        );
        assert_eq!(
            browser_viewport_size_for_window_pixels(1, 1, raster),
            (1, 1)
        );
    }
}
