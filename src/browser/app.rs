use anyhow::{Result, anyhow, bail};
use serde::{Deserialize, Serialize};

use super::{
    BROWSER_ABOUT_BLANK_TARGET, BrowserCookie, BrowserCookieJar, BrowserFocusedControl,
    BrowserForm, BrowserHistorySnapshot, BrowserLink, BrowserLocalStorage,
    BrowserLocalStorageEntry, BrowserRasterOptions, BrowserRenderOptions, BrowserRgbaRaster,
    BrowserSession, BrowserTextViewportOptions, BrowserViewportFrame, BrowserViewportFrameReport,
    BrowserViewportState, browser_document_viewport, browser_text_viewport, browser_viewport_frame,
    ensure_static_target,
};

const BROWSER_APP_CLOSED_TAB_LIMIT: usize = 10;
const BROWSER_APP_REPORT_TEXT_MAX_CHARS: usize = 16 * 1024;
const BROWSER_APP_CACHED_FRAME_MAX_BYTES: usize = 4 * 1024 * 1024;
const BROWSER_APP_CACHED_WINDOW_FRAME_MAX_BYTES: usize = 4 * 1024 * 1024;

#[derive(Debug, Clone, Copy)]
pub struct BrowserAppOptions {
    pub render: BrowserRenderOptions,
    pub viewport_width: usize,
    pub viewport_height: usize,
    pub raster: BrowserRasterOptions,
}

impl Default for BrowserAppOptions {
    fn default() -> Self {
        let render = BrowserRenderOptions::default();
        Self {
            render,
            viewport_width: render.width,
            viewport_height: 24,
            raster: BrowserRasterOptions::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum BrowserAppAction {
    Open(String),
    Back,
    Forward,
    Reload,
    ClearCookies,
    ClearLocalStorage,
    ClearSessionStorage,
    FindText {
        query: String,
        next: bool,
    },
    FindTextPrevious {
        query: String,
    },
    NewBlankTab,
    NewTab(String),
    DuplicateTab,
    SwitchTab(usize),
    CloseTab(Option<usize>),
    RestoreClosedTab,
    Scroll {
        delta_x: isize,
        delta_y: isize,
    },
    SetViewport {
        width: usize,
        height: usize,
    },
    SetViewportOrigin {
        x: usize,
        y: usize,
    },
    Click {
        x: usize,
        y: usize,
    },
    OpenClickInBackgroundTab {
        x: usize,
        y: usize,
    },
    OpenClickInForegroundTab {
        x: usize,
        y: usize,
    },
    ClickSelector(String),
    Focus(String),
    FocusNext,
    FocusPrevious,
    TypeText(String),
    DeleteTextBackward(usize),
    ClearText,
    BlurFocused,
    SubmitFocused,
    ToggleFocused,
    ToggleControl {
        form_index: usize,
        control_index: usize,
    },
    SelectFocused(String),
    SelectControl {
        form_index: usize,
        control_index: usize,
        value: String,
    },
    ActivateLink(usize),
    ActivateLinkText(String),
    ActivateLinkSelector(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrowserAppReport {
    pub active_tab: usize,
    pub tabs: Vec<BrowserAppTabSummary>,
    pub history: BrowserHistorySnapshot,
    pub viewport: BrowserViewportState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_presented_viewport: Option<BrowserViewportState>,
    pub frame: BrowserViewportFrameReport,
    #[serde(default)]
    pub frame_pixel_bytes: usize,
    #[serde(default)]
    pub frame_dirty_pixel_area: usize,
    #[serde(default)]
    pub frame_reused_viewport_area: usize,
    #[serde(default)]
    pub frame_full_repaint: bool,
    #[serde(default)]
    pub cached_frame_pixel_bytes: usize,
    #[serde(default)]
    pub cached_frame_limit_bytes: usize,
    #[serde(default)]
    pub frame_cache_reusable: bool,
    pub focused: Option<BrowserFocusedControl>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub find: Option<BrowserAppFindState>,
    pub links: Vec<BrowserLink>,
    pub forms: Vec<BrowserForm>,
    #[serde(default)]
    pub text_chars: usize,
    #[serde(default)]
    pub visible_text_chars: usize,
    #[serde(serialize_with = "serialize_browser_app_report_text")]
    pub text: String,
    pub visible_text: Vec<String>,
    pub cookies: Vec<BrowserCookie>,
    pub local_storage: Vec<BrowserLocalStorageEntry>,
    pub session_storage: Vec<BrowserLocalStorageEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BrowserAppTabSummary {
    pub index: usize,
    pub active: bool,
    pub history_len: usize,
    pub current_history_index: Option<usize>,
    pub title: String,
    pub source: String,
    pub viewport: BrowserViewportState,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BrowserAppFindState {
    pub query: String,
    pub active_match_index: usize,
    pub match_count: usize,
    pub line: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BrowserAppWindowFrameReport {
    pub active_tab: usize,
    pub tab_count: usize,
    pub title: String,
    pub source: String,
    pub chrome_rows: usize,
    pub chrome_height: usize,
    pub content_y: usize,
    pub window_width: usize,
    pub window_height: usize,
    pub page_frame_width: usize,
    pub page_frame_height: usize,
    pub bytes_per_pixel: usize,
    #[serde(default)]
    pub raster_pixel_bytes: usize,
    #[serde(default)]
    pub page_frame_pixel_bytes: usize,
    #[serde(default)]
    pub cached_window_frame_pixel_bytes: usize,
    #[serde(default)]
    pub cached_window_frame_limit_bytes: usize,
    #[serde(default)]
    pub window_frame_cache_reusable: bool,
    pub pixel_hash: String,
    pub non_background_pixels: usize,
    pub artifact_format: String,
    pub page: BrowserViewportFrameReport,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrowserAppWindowFrame {
    pub report: BrowserAppWindowFrameReport,
    pub raster: BrowserRgbaRaster,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BrowserAppWindowFrameOptions {
    pub location_text: Option<String>,
    pub status_text: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum BrowserAppWindowHit {
    BackButton,
    ForwardButton,
    ReloadButton,
    NewTabButton,
    Tab { index: usize },
    LocationBar,
    StatusBar,
    PageViewport { x: usize, y: usize },
    PageFrame,
    ChromeBackground,
    Outside,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BrowserAppWindowClickReport {
    pub hit: BrowserAppWindowHit,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action: Option<BrowserAppAction>,
    pub applied: bool,
    pub active_tab: usize,
    pub source: String,
}

#[derive(Debug, Clone)]
pub struct BrowserApp {
    tabs: Vec<BrowserAppTab>,
    closed_tabs: Vec<BrowserAppTab>,
    active_tab: usize,
    options: BrowserAppOptions,
}

#[derive(Debug, Clone)]
struct BrowserAppTab {
    session: BrowserSession,
    viewport: BrowserViewportState,
    last_presented_viewport: Option<BrowserViewportState>,
    last_presented_frame: Option<BrowserViewportFrame>,
    last_presented_window_frame: Option<BrowserAppCachedWindowFrame>,
    content_dirty: bool,
    find: Option<BrowserAppFindState>,
}

#[derive(Debug, Clone)]
struct BrowserAppCachedWindowFrame {
    options: BrowserAppWindowFrameOptions,
    frame: BrowserAppWindowFrame,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BrowserAppFindDirection {
    Initial,
    Next,
    Previous,
}

impl BrowserApp {
    pub async fn open(target: &str, options: BrowserAppOptions) -> Result<Self> {
        Self::open_with_state(
            target,
            options,
            BrowserCookieJar::default(),
            BrowserLocalStorage::default(),
        )
        .await
    }

    pub async fn open_with_state(
        target: &str,
        options: BrowserAppOptions,
        cookie_jar: BrowserCookieJar,
        local_storage: BrowserLocalStorage,
    ) -> Result<Self> {
        ensure_static_target(target)?;
        let mut session = BrowserSession::new_with_state(options.render, cookie_jar, local_storage);
        session.navigate(target).await?;
        let viewport = initial_app_viewport(&session, options);
        Ok(Self {
            tabs: vec![BrowserAppTab {
                session,
                viewport,
                last_presented_viewport: None,
                last_presented_frame: None,
                last_presented_window_frame: None,
                content_dirty: true,
                find: None,
            }],
            closed_tabs: Vec::new(),
            active_tab: 0,
            options,
        })
    }

    pub fn active_tab(&self) -> usize {
        self.active_tab
    }

    pub fn tab_count(&self) -> usize {
        self.tabs.len()
    }

    pub fn closed_tab_count(&self) -> usize {
        self.closed_tabs.len()
    }

    pub fn active_session(&self) -> Result<&BrowserSession> {
        Ok(&self.active_tab_ref()?.session)
    }

    pub fn active_viewport(&self) -> Result<BrowserViewportState> {
        Ok(self.active_tab_ref()?.viewport)
    }

    pub fn active_find_state(&self) -> Result<Option<BrowserAppFindState>> {
        Ok(self.active_tab_ref()?.find.clone())
    }

    pub fn active_link_target_at_viewport(&self, x: usize, y: usize) -> Result<Option<String>> {
        let tab = self.active_tab_ref()?;
        let document_x = tab.viewport.x.saturating_add(x);
        let document_y = tab.viewport.y.saturating_add(y);
        Ok(tab.session.link_target_at(document_x, document_y))
    }

    pub async fn apply_action(&mut self, action: BrowserAppAction) -> Result<()> {
        match action {
            BrowserAppAction::Open(target) => self.open_in_active_tab(&target).await,
            BrowserAppAction::Back => self.history_back(),
            BrowserAppAction::Forward => self.history_forward(),
            BrowserAppAction::Reload => self.reload_active_tab().await,
            BrowserAppAction::ClearCookies => self.clear_active_cookies(),
            BrowserAppAction::ClearLocalStorage => self.clear_active_local_storage(),
            BrowserAppAction::ClearSessionStorage => self.clear_active_session_storage(),
            BrowserAppAction::FindText { query, next } => self.find_text(&query, next),
            BrowserAppAction::FindTextPrevious { query } => self.find_text_previous(&query),
            BrowserAppAction::NewBlankTab => self.new_tab(BROWSER_ABOUT_BLANK_TARGET).await,
            BrowserAppAction::NewTab(target) => self.new_tab(&target).await,
            BrowserAppAction::DuplicateTab => self.duplicate_active_tab(),
            BrowserAppAction::SwitchTab(index) => self.switch_tab(index),
            BrowserAppAction::CloseTab(index) => self.close_tab(index),
            BrowserAppAction::RestoreClosedTab => self.restore_closed_tab(),
            BrowserAppAction::Scroll { delta_x, delta_y } => self.scroll_active(delta_x, delta_y),
            BrowserAppAction::SetViewport { width, height } => self.resize_active(width, height),
            BrowserAppAction::SetViewportOrigin { x, y } => self.set_viewport_origin(x, y),
            BrowserAppAction::Click { x, y } => self.click_active(x, y).await,
            BrowserAppAction::OpenClickInBackgroundTab { x, y } => {
                self.open_click_in_background_tab(x, y).await
            }
            BrowserAppAction::OpenClickInForegroundTab { x, y } => {
                self.open_click_in_foreground_tab(x, y).await
            }
            BrowserAppAction::ClickSelector(selector) => self.click_selector(&selector).await,
            BrowserAppAction::Focus(selector) => {
                self.active_tab_mut()?.session.focus_selector(&selector)?;
                self.mark_active_content_dirty()
            }
            BrowserAppAction::FocusNext => {
                self.active_tab_mut()?.session.focus_next_control()?;
                self.mark_active_content_dirty()
            }
            BrowserAppAction::FocusPrevious => {
                self.active_tab_mut()?.session.focus_previous_control()?;
                self.mark_active_content_dirty()
            }
            BrowserAppAction::TypeText(text) => {
                self.active_tab_mut()?.session.type_text(&text)?;
                self.mark_active_content_dirty()
            }
            BrowserAppAction::DeleteTextBackward(count) => {
                self.active_tab_mut()?.session.delete_text_backward(count)?;
                self.mark_active_content_dirty()
            }
            BrowserAppAction::ClearText => {
                self.active_tab_mut()?.session.clear_focused_text()?;
                self.mark_active_content_dirty()
            }
            BrowserAppAction::BlurFocused => {
                let blurred = self.active_tab_mut()?.session.blur_focused_control()?;
                if blurred {
                    self.mark_active_content_dirty()?;
                }
                Ok(())
            }
            BrowserAppAction::SubmitFocused => self.submit_focused().await,
            BrowserAppAction::ToggleFocused => self.toggle_focused(),
            BrowserAppAction::ToggleControl {
                form_index,
                control_index,
            } => {
                self.active_tab_mut()?
                    .session
                    .toggle_form_control(form_index, control_index)?;
                self.mark_active_content_dirty()
            }
            BrowserAppAction::SelectFocused(value) => {
                self.active_tab_mut()?
                    .session
                    .select_focused_option(&value)?;
                self.mark_active_content_dirty()
            }
            BrowserAppAction::SelectControl {
                form_index,
                control_index,
                value,
            } => {
                self.active_tab_mut()?.session.select_form_option(
                    form_index,
                    control_index,
                    &value,
                )?;
                self.mark_active_content_dirty()
            }
            BrowserAppAction::ActivateLink(index) => self.activate_link(index).await,
            BrowserAppAction::ActivateLinkText(text) => self.activate_link_text(&text).await,
            BrowserAppAction::ActivateLinkSelector(selector) => {
                self.activate_link_selector(&selector).await
            }
        }
    }

    pub fn present_frame(&mut self) -> Result<BrowserViewportFrame> {
        let raster_options = self.options.raster;
        let tab = self.active_tab_mut()?;
        if let Some(frame) = tab
            .last_presented_frame
            .as_ref()
            .filter(|frame| !tab.content_dirty && frame.report.viewport.viewport == tab.viewport)
        {
            return Ok(frame.clone());
        }
        let Some(render) = tab.session.current() else {
            bail!("browser app has no current page");
        };
        let previous = (!tab.content_dirty)
            .then_some(tab.last_presented_viewport)
            .flatten();
        let frame = browser_viewport_frame(render, tab.viewport, previous, raster_options)?;
        tab.viewport = frame.report.viewport.viewport;
        tab.last_presented_viewport = Some(tab.viewport);
        tab.last_presented_frame = browser_app_frame_is_reusable(&frame).then(|| frame.clone());
        tab.content_dirty = false;
        Ok(frame)
    }

    pub fn present_window_frame(&mut self) -> Result<BrowserAppWindowFrame> {
        self.present_window_frame_with_options(BrowserAppWindowFrameOptions::default())
    }

    pub fn present_window_frame_with_options(
        &mut self,
        options: BrowserAppWindowFrameOptions,
    ) -> Result<BrowserAppWindowFrame> {
        let cached_window_frame = {
            let tab = self.active_tab_ref()?;
            tab.last_presented_window_frame
                .as_ref()
                .filter(|cached| !tab.content_dirty && cached.options == options)
                .filter(|cached| cached.frame.report.page.viewport.viewport == tab.viewport)
                .map(|cached| cached.frame.clone())
        };
        if let Some(frame) = cached_window_frame {
            return Ok(frame);
        }

        let page_frame = self.present_frame()?;
        let window_frame =
            self.window_frame_for_presented_frame_with_options(page_frame, options.clone())?;
        let tab = self.active_tab_mut()?;
        tab.last_presented_window_frame =
            browser_app_window_frame_is_reusable(&window_frame).then(|| {
                BrowserAppCachedWindowFrame {
                    options,
                    frame: window_frame.clone(),
                }
            });
        Ok(window_frame)
    }

    pub fn window_frame_for_presented_frame(
        &self,
        page_frame: BrowserViewportFrame,
    ) -> Result<BrowserAppWindowFrame> {
        self.window_frame_for_presented_frame_with_options(
            page_frame,
            BrowserAppWindowFrameOptions::default(),
        )
    }

    pub fn window_frame_for_presented_frame_with_options(
        &self,
        page_frame: BrowserViewportFrame,
        options: BrowserAppWindowFrameOptions,
    ) -> Result<BrowserAppWindowFrame> {
        let report = self.report_for_frame(page_frame.report.clone())?;
        compose_browser_app_window_frame(&report, page_frame, &options)
    }

    pub fn hit_test_window(&self, x: usize, y: usize) -> Result<BrowserAppWindowHit> {
        hit_test_browser_app_window(self, x, y)
    }

    pub async fn click_window(
        &mut self,
        x: usize,
        y: usize,
    ) -> Result<BrowserAppWindowClickReport> {
        let hit = self.hit_test_window(x, y)?;
        let action = self.window_hit_action(&hit)?;
        if let Some(action) = action.clone() {
            self.apply_action(action).await?;
        }
        let active_tab = self.active_tab;
        let source = self
            .current_source()
            .unwrap_or_else(|| "(empty)".to_owned());
        let applied = action.is_some();
        Ok(BrowserAppWindowClickReport {
            hit,
            action,
            applied,
            active_tab,
            source,
        })
    }

    pub fn report(&mut self) -> Result<BrowserAppReport> {
        let frame = self.present_frame()?;
        self.report_for_frame(frame.report)
    }

    pub fn report_for_frame(&self, frame: BrowserViewportFrameReport) -> Result<BrowserAppReport> {
        let tab = self.active_tab_ref()?;
        let current_render = tab.session.current();
        let visible_text = current_render
            .map(|render| {
                browser_text_viewport(
                    render,
                    BrowserTextViewportOptions {
                        x: frame.viewport.viewport.x,
                        y: frame.viewport.viewport.y,
                        width: frame.viewport.viewport.width,
                        height: frame.viewport.viewport.height,
                    },
                )
                .lines
            })
            .unwrap_or_default();
        let text = current_render.map_or_else(String::new, |render| render.text.clone());
        let text_chars = text.chars().count();
        let visible_text_chars = visible_text.iter().map(|line| line.chars().count()).sum();
        let cached_frame_pixel_bytes = tab
            .last_presented_frame
            .as_ref()
            .map(|frame| browser_viewport_frame_pixel_bytes(&frame.report))
            .unwrap_or(0);

        Ok(BrowserAppReport {
            active_tab: self.active_tab,
            tabs: self.tab_summaries(),
            history: tab.session.snapshot(),
            viewport: tab.viewport,
            last_presented_viewport: tab.last_presented_viewport,
            frame_pixel_bytes: browser_viewport_frame_pixel_bytes(&frame),
            frame_dirty_pixel_area: frame.dirty_pixel_area,
            frame_reused_viewport_area: frame.viewport.reused_area,
            frame_full_repaint: frame.viewport.full_repaint,
            cached_frame_pixel_bytes,
            cached_frame_limit_bytes: BROWSER_APP_CACHED_FRAME_MAX_BYTES,
            frame_cache_reusable: cached_frame_pixel_bytes > 0,
            frame,
            focused: tab.session.focused_control(),
            find: tab.find.clone(),
            links: tab.session.current_links().to_vec(),
            forms: tab.session.current_forms().to_vec(),
            text_chars,
            visible_text_chars,
            text,
            visible_text,
            cookies: tab.session.cookies_snapshot(),
            local_storage: tab.session.local_storage_entries(),
            session_storage: tab.session.session_storage_entries(),
        })
    }

    pub fn tab_summaries(&self) -> Vec<BrowserAppTabSummary> {
        self.tabs
            .iter()
            .enumerate()
            .map(|(index, tab)| browser_app_tab_summary(index, index == self.active_tab, tab))
            .collect()
    }

    async fn open_in_active_tab(&mut self, target: &str) -> Result<()> {
        let target = self
            .active_tab_ref()?
            .session
            .resolve_current_target(target);
        ensure_static_target(&target)?;
        let tab = self.active_tab_mut()?;
        tab.session.navigate(&target).await?;
        self.reset_active_viewport_to_page_start()
    }

    async fn reload_active_tab(&mut self) -> Result<()> {
        self.active_tab_mut()?.session.reload().await?;
        self.mark_active_page_dirty_for_full_repaint()?;
        self.clamp_active_viewport()
    }

    fn clear_active_cookies(&mut self) -> Result<()> {
        self.active_tab_mut()?.session.clear_cookies();
        self.mark_active_content_dirty()
    }

    fn clear_active_local_storage(&mut self) -> Result<()> {
        self.active_tab_mut()?.session.clear_local_storage();
        self.mark_active_content_dirty()
    }

    fn clear_active_session_storage(&mut self) -> Result<()> {
        self.active_tab_mut()?.session.clear_session_storage();
        self.mark_active_content_dirty()
    }

    async fn new_tab(&mut self, target: &str) -> Result<()> {
        let active = self.active_tab_ref()?;
        let target = active.session.resolve_current_target(target);
        ensure_static_target(&target)?;
        let cookies = active.session.cookies_snapshot();
        let local_storage = active.session.local_storage_snapshot();
        let mut session = BrowserSession::new_with_state(
            self.options.render,
            BrowserCookieJar::from_cookies(cookies),
            local_storage,
        );
        session.navigate(&target).await?;
        let viewport = initial_app_viewport(&session, self.options);
        self.tabs.push(BrowserAppTab {
            session,
            viewport,
            last_presented_viewport: None,
            last_presented_frame: None,
            last_presented_window_frame: None,
            content_dirty: true,
            find: None,
        });
        self.active_tab = self.tabs.len() - 1;
        Ok(())
    }

    fn duplicate_active_tab(&mut self) -> Result<()> {
        let mut tab = self.active_tab_ref()?.clone();
        tab.last_presented_viewport = None;
        tab.last_presented_frame = None;
        tab.last_presented_window_frame = None;
        tab.content_dirty = true;
        self.tabs.push(tab);
        self.active_tab = self.tabs.len() - 1;
        Ok(())
    }

    fn switch_tab(&mut self, index: usize) -> Result<()> {
        if index >= self.tabs.len() {
            bail!(
                "tab index {index} not found; {} tab(s) open",
                self.tabs.len()
            );
        }
        self.active_tab = index;
        Ok(())
    }

    fn close_tab(&mut self, index: Option<usize>) -> Result<()> {
        if self.tabs.len() == 1 {
            bail!("cannot close the last tab");
        }
        let index = index.unwrap_or(self.active_tab);
        if index >= self.tabs.len() {
            bail!(
                "tab index {index} not found; {} tab(s) open",
                self.tabs.len()
            );
        }
        let mut closed_tab = self.tabs.remove(index);
        closed_tab.last_presented_viewport = None;
        closed_tab.last_presented_frame = None;
        closed_tab.last_presented_window_frame = None;
        closed_tab.content_dirty = true;
        self.closed_tabs.push(closed_tab);
        if self.closed_tabs.len() > BROWSER_APP_CLOSED_TAB_LIMIT {
            self.closed_tabs.remove(0);
        }
        if self.active_tab > index {
            self.active_tab -= 1;
        } else if self.active_tab >= self.tabs.len() {
            self.active_tab = self.tabs.len().saturating_sub(1);
        }
        Ok(())
    }

    fn restore_closed_tab(&mut self) -> Result<()> {
        let Some(mut tab) = self.closed_tabs.pop() else {
            bail!("no closed tab to restore");
        };
        tab.last_presented_viewport = None;
        tab.last_presented_frame = None;
        tab.last_presented_window_frame = None;
        tab.content_dirty = true;
        self.tabs.push(tab);
        self.active_tab = self.tabs.len() - 1;
        Ok(())
    }

    fn history_back(&mut self) -> Result<()> {
        self.active_tab_mut()?.session.back()?;
        self.reset_active_viewport_to_page_start()
    }

    fn history_forward(&mut self) -> Result<()> {
        self.active_tab_mut()?.session.forward()?;
        self.reset_active_viewport_to_page_start()
    }

    fn scroll_active(&mut self, delta_x: isize, delta_y: isize) -> Result<()> {
        let tab = self.active_tab_mut()?;
        let default_prevented = tab.session.dispatch_wheel_event(delta_x, delta_y)?;
        if default_prevented {
            tab.last_presented_frame = None;
            tab.last_presented_window_frame = None;
            tab.content_dirty = true;
            return Ok(());
        }
        tab.viewport.x = apply_signed_viewport_delta(tab.viewport.x, delta_x);
        tab.viewport.y = apply_signed_viewport_delta(tab.viewport.y, delta_y);
        self.clamp_active_viewport()
    }

    fn resize_active(&mut self, width: usize, height: usize) -> Result<()> {
        let tab = self.active_tab_mut()?;
        tab.viewport.width = width.max(1);
        tab.viewport.height = height.max(1);
        tab.last_presented_frame = None;
        tab.last_presented_window_frame = None;
        tab.content_dirty = true;
        self.clamp_active_viewport()
    }

    fn set_viewport_origin(&mut self, x: usize, y: usize) -> Result<()> {
        let tab = self.active_tab_mut()?;
        tab.viewport.x = x;
        tab.viewport.y = y;
        self.clamp_active_viewport()
    }

    fn find_text(&mut self, query: &str, next: bool) -> Result<()> {
        let direction = if next {
            BrowserAppFindDirection::Next
        } else {
            BrowserAppFindDirection::Initial
        };
        self.find_text_in_direction(query, direction)
    }

    fn find_text_previous(&mut self, query: &str) -> Result<()> {
        self.find_text_in_direction(query, BrowserAppFindDirection::Previous)
    }

    fn find_text_in_direction(
        &mut self,
        query: &str,
        direction: BrowserAppFindDirection,
    ) -> Result<()> {
        let query = query.trim();
        if query.is_empty() {
            bail!("find requires an argument");
        }

        let (active_match_index, match_count, line) = {
            let tab = self.active_tab_ref()?;
            let Some(render) = tab.session.current() else {
                bail!("cannot find text: browser app has no current page");
            };
            let document_viewport = browser_document_viewport(render, tab.viewport, None);
            let document = browser_text_viewport(
                render,
                BrowserTextViewportOptions {
                    x: 0,
                    y: 0,
                    width: document_viewport.document_width.max(1),
                    height: document_viewport.document_height.max(1),
                },
            );
            let matches = browser_app_find_matching_lines(&document.lines, query);
            if matches.is_empty() {
                bail!("text not found: {query:?}");
            }
            let (active_match_index, line) = match direction {
                BrowserAppFindDirection::Initial => {
                    browser_app_select_find_match(&matches, tab.viewport.y)
                }
                BrowserAppFindDirection::Next => {
                    let start_y = tab
                        .find
                        .as_ref()
                        .filter(|state| state.query == query)
                        .map_or(tab.viewport.y.saturating_add(1), |state| {
                            state.line.saturating_add(1)
                        });
                    browser_app_select_find_match(&matches, start_y)
                }
                BrowserAppFindDirection::Previous => tab
                    .find
                    .as_ref()
                    .filter(|state| {
                        state.query == query && state.active_match_index < matches.len()
                    })
                    .map_or_else(
                        || browser_app_select_previous_find_match(&matches, tab.viewport.y),
                        |state| {
                            let index = state
                                .active_match_index
                                .checked_sub(1)
                                .unwrap_or(matches.len() - 1);
                            (index, matches[index])
                        },
                    ),
            };
            (active_match_index, matches.len(), line)
        };

        let tab = self.active_tab_mut()?;
        tab.viewport.x = 0;
        tab.viewport.y = line;
        tab.find = Some(BrowserAppFindState {
            query: query.to_owned(),
            active_match_index,
            match_count,
            line,
        });
        self.clamp_active_viewport()
    }

    async fn click_active(&mut self, x: usize, y: usize) -> Result<()> {
        let before = self.current_source();
        let (document_x, document_y) = {
            let viewport = self.active_tab_ref()?.viewport;
            (viewport.x.saturating_add(x), viewport.y.saturating_add(y))
        };
        self.active_tab_mut()?
            .session
            .click_at_with_default_action(document_x, document_y)
            .await?;
        self.after_potential_navigation(before)
    }

    async fn open_click_in_background_tab(&mut self, x: usize, y: usize) -> Result<()> {
        self.open_click_in_new_tab(x, y, false).await
    }

    async fn open_click_in_foreground_tab(&mut self, x: usize, y: usize) -> Result<()> {
        self.open_click_in_new_tab(x, y, true).await
    }

    async fn open_click_in_new_tab(&mut self, x: usize, y: usize, foreground: bool) -> Result<()> {
        let active_index = self.active_tab;
        let active = self.active_tab_ref()?;
        let mut tab = active.clone();
        let before_source = tab.session.current().map(|render| render.source.clone());
        let document_x = tab.viewport.x.saturating_add(x);
        let document_y = tab.viewport.y.saturating_add(y);
        if tab
            .session
            .click_at_with_default_action(document_x, document_y)
            .await
            .is_err()
        {
            return Ok(());
        }
        let after_source = tab.session.current().map(|render| render.source.clone());
        if before_source == after_source {
            return Ok(());
        }

        tab.viewport = initial_app_viewport(&tab.session, self.options);
        tab.last_presented_viewport = None;
        tab.last_presented_frame = None;
        tab.last_presented_window_frame = None;
        tab.content_dirty = true;
        tab.find = None;
        let insert_index = active_index.saturating_add(1);
        self.tabs.insert(insert_index, tab);
        self.active_tab = if foreground {
            insert_index
        } else {
            active_index
        };
        Ok(())
    }

    async fn click_selector(&mut self, selector: &str) -> Result<()> {
        let before = self.current_source();
        self.active_tab_mut()?
            .session
            .click_selector_with_default_action(selector)
            .await?;
        self.after_potential_navigation(before)
    }

    async fn submit_focused(&mut self) -> Result<()> {
        let before = self.current_source();
        self.active_tab_mut()?.session.submit_focused_form().await?;
        self.after_potential_navigation(before)
    }

    fn toggle_focused(&mut self) -> Result<()> {
        let focused = self
            .active_tab_ref()?
            .session
            .focused_control()
            .ok_or_else(|| anyhow!("cannot toggle focused control: no focused form control"))?;
        self.active_tab_mut()?
            .session
            .toggle_form_control(focused.form_index, focused.control_index)?;
        self.mark_active_content_dirty()
    }

    async fn activate_link(&mut self, index: usize) -> Result<()> {
        self.active_tab_mut()?.session.activate_link(index).await?;
        self.reset_active_viewport_to_page_start()
    }

    async fn activate_link_text(&mut self, text: &str) -> Result<()> {
        self.active_tab_mut()?
            .session
            .activate_link_text(text)
            .await?;
        self.reset_active_viewport_to_page_start()
    }

    async fn activate_link_selector(&mut self, selector: &str) -> Result<()> {
        self.active_tab_mut()?
            .session
            .activate_link_selector(selector)
            .await?;
        self.reset_active_viewport_to_page_start()
    }

    fn after_potential_navigation(&mut self, before_source: Option<String>) -> Result<()> {
        let after_source = self.current_source();
        if before_source != after_source {
            self.reset_active_viewport_to_page_start()
        } else {
            self.mark_active_content_dirty()
        }
    }

    fn reset_active_viewport_to_page_start(&mut self) -> Result<()> {
        let options = self.options;
        let tab = self.active_tab_mut()?;
        tab.viewport = initial_app_viewport(&tab.session, options);
        tab.last_presented_viewport = None;
        tab.last_presented_frame = None;
        tab.last_presented_window_frame = None;
        tab.content_dirty = true;
        tab.find = None;
        Ok(())
    }

    fn mark_active_page_dirty_for_full_repaint(&mut self) -> Result<()> {
        let tab = self.active_tab_mut()?;
        tab.last_presented_viewport = None;
        tab.last_presented_frame = None;
        tab.last_presented_window_frame = None;
        tab.content_dirty = true;
        Ok(())
    }

    fn clamp_active_viewport(&mut self) -> Result<()> {
        let tab = self.active_tab_mut()?;
        let Some(render) = tab.session.current() else {
            bail!("browser app has no current page");
        };
        let previous_viewport = tab.viewport;
        let report = browser_document_viewport(render, tab.viewport, tab.last_presented_viewport);
        tab.viewport = report.viewport;
        if tab.viewport != previous_viewport {
            tab.last_presented_frame = None;
            tab.last_presented_window_frame = None;
        }
        Ok(())
    }

    fn mark_active_content_dirty(&mut self) -> Result<()> {
        let tab = self.active_tab_mut()?;
        tab.last_presented_frame = None;
        tab.last_presented_window_frame = None;
        tab.content_dirty = true;
        Ok(())
    }

    fn current_source(&self) -> Option<String> {
        self.active_tab_ref()
            .ok()
            .and_then(|tab| tab.session.current().map(|render| render.source.clone()))
    }

    fn window_hit_action(&self, hit: &BrowserAppWindowHit) -> Result<Option<BrowserAppAction>> {
        let action = match hit {
            BrowserAppWindowHit::BackButton if self.can_go_back()? => Some(BrowserAppAction::Back),
            BrowserAppWindowHit::ForwardButton if self.can_go_forward()? => {
                Some(BrowserAppAction::Forward)
            }
            BrowserAppWindowHit::ReloadButton => Some(BrowserAppAction::Reload),
            BrowserAppWindowHit::NewTabButton => Some(BrowserAppAction::NewBlankTab),
            BrowserAppWindowHit::Tab { index } if *index != self.active_tab => {
                Some(BrowserAppAction::SwitchTab(*index))
            }
            BrowserAppWindowHit::PageViewport { x, y } => {
                Some(BrowserAppAction::Click { x: *x, y: *y })
            }
            _ => None,
        };
        Ok(action)
    }

    fn can_go_back(&self) -> Result<bool> {
        let history = self.active_session()?.snapshot();
        Ok(history.current_index.is_some_and(|index| index > 0))
    }

    fn can_go_forward(&self) -> Result<bool> {
        let history = self.active_session()?.snapshot();
        Ok(history
            .current_index
            .is_some_and(|index| index + 1 < history.entries.len()))
    }

    fn active_tab_ref(&self) -> Result<&BrowserAppTab> {
        self.tabs
            .get(self.active_tab)
            .ok_or_else(|| anyhow!("active tab {} is not open", self.active_tab))
    }

    fn active_tab_mut(&mut self) -> Result<&mut BrowserAppTab> {
        self.tabs
            .get_mut(self.active_tab)
            .ok_or_else(|| anyhow!("active tab {} is not open", self.active_tab))
    }
}

fn initial_app_viewport(
    session: &BrowserSession,
    options: BrowserAppOptions,
) -> BrowserViewportState {
    BrowserViewportState {
        x: 0,
        y: session
            .current()
            .and_then(|render| render.source_fragment_scroll_y())
            .unwrap_or(0),
        width: options.viewport_width.max(1),
        height: options.viewport_height.max(1),
    }
}

fn browser_viewport_frame_pixel_bytes(frame: &BrowserViewportFrameReport) -> usize {
    frame
        .frame_width
        .saturating_mul(frame.frame_height)
        .saturating_mul(frame.bytes_per_pixel)
}

fn browser_app_frame_is_reusable(frame: &BrowserViewportFrame) -> bool {
    !frame.report.viewport.full_repaint
        && frame.report.dirty_pixel_area == 0
        && browser_viewport_frame_pixel_bytes(&frame.report) <= BROWSER_APP_CACHED_FRAME_MAX_BYTES
}

fn browser_app_window_frame_is_reusable(frame: &BrowserAppWindowFrame) -> bool {
    frame.report.window_frame_cache_reusable
        && frame.raster.pixels.len() <= BROWSER_APP_CACHED_WINDOW_FRAME_MAX_BYTES
}

fn serialize_browser_app_report_text<S>(text: &str, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    serializer.serialize_str(&browser_app_report_text_label(text))
}

fn browser_app_report_text_label(text: &str) -> String {
    let text_chars = text.chars().count();
    if text_chars <= BROWSER_APP_REPORT_TEXT_MAX_CHARS {
        return text.to_owned();
    }

    let preview: String = text
        .chars()
        .take(BROWSER_APP_REPORT_TEXT_MAX_CHARS)
        .collect();
    format!(
        "{preview}\n[brutal-app-report-text-truncated text_chars={text_chars} retained_chars={BROWSER_APP_REPORT_TEXT_MAX_CHARS}]"
    )
}

fn browser_app_tab_summary(
    index: usize,
    active: bool,
    tab: &BrowserAppTab,
) -> BrowserAppTabSummary {
    let history = tab.session.snapshot();
    let (title, source) = tab.session.current().map_or_else(
        || ("(empty)".to_owned(), "(empty)".to_owned()),
        |render| {
            let title = if render.title.is_empty() {
                render.source.clone()
            } else {
                render.title.clone()
            };
            (title, render.source.clone())
        },
    );
    BrowserAppTabSummary {
        index,
        active,
        history_len: history.entries.len(),
        current_history_index: history.current_index,
        title,
        source,
        viewport: tab.viewport,
    }
}

fn apply_signed_viewport_delta(value: usize, delta: isize) -> usize {
    if delta >= 0 {
        value.saturating_add(delta as usize)
    } else {
        value.saturating_sub(delta.unsigned_abs())
    }
}

fn browser_app_find_matching_lines(lines: &[String], query: &str) -> Vec<usize> {
    let needle = query.to_lowercase();
    lines
        .iter()
        .enumerate()
        .filter_map(|(line, text)| text.to_lowercase().contains(&needle).then_some(line))
        .collect()
}

fn browser_app_select_find_match(matches: &[usize], start_y: usize) -> (usize, usize) {
    if let Some((index, &line)) = matches
        .iter()
        .enumerate()
        .find(|&(_, &line)| line >= start_y)
    {
        return (index, line);
    }
    (0, matches[0])
}

fn browser_app_select_previous_find_match(matches: &[usize], start_y: usize) -> (usize, usize) {
    if let Some((index, &line)) = matches
        .iter()
        .enumerate()
        .rev()
        .find(|&(_, &line)| line < start_y)
    {
        return (index, line);
    }
    let index = matches.len() - 1;
    (index, matches[index])
}

const WINDOW_CHROME_ROWS: usize = 3;
const WINDOW_CHROME_BUTTON_LINE: &str = "B F R N ";
const WINDOW_CHROME_BACK_COLUMN: usize = 0;
const WINDOW_CHROME_FORWARD_COLUMN: usize = 2;
const WINDOW_CHROME_RELOAD_COLUMN: usize = 4;
const WINDOW_CHROME_NEW_TAB_COLUMN: usize = 6;
const WINDOW_CHROME_TAB_START_COLUMN: usize = 8;

#[derive(Debug, Clone, Copy)]
struct BrowserAppWindowGeometry {
    width: usize,
    page_height: usize,
    chrome_height: usize,
    cell_width: usize,
    cell_height: usize,
    padding_x: usize,
    padding_y: usize,
    viewport: BrowserViewportState,
}

fn browser_app_window_geometry(app: &BrowserApp) -> Result<BrowserAppWindowGeometry> {
    let tab = app.active_tab_ref()?;
    let Some(render) = tab.session.current() else {
        bail!("browser app has no current page");
    };
    let raster = app.options.raster;
    let viewport_report = browser_document_viewport(render, tab.viewport, None);
    let viewport = viewport_report.viewport;
    let width = viewport
        .width
        .checked_mul(raster.cell_width)
        .and_then(|width| width.checked_add(raster.padding_x.saturating_mul(2)))
        .ok_or_else(|| anyhow!("browser app window width overflow"))?;
    let page_height = viewport
        .height
        .checked_mul(raster.cell_height)
        .and_then(|height| height.checked_add(raster.padding_y.saturating_mul(2)))
        .ok_or_else(|| anyhow!("browser app window page height overflow"))?;
    let chrome_height = browser_app_window_chrome_height(raster.cell_height, raster.padding_y)?;
    Ok(BrowserAppWindowGeometry {
        width,
        page_height,
        chrome_height,
        cell_width: raster.cell_width.max(1),
        cell_height: raster.cell_height.max(1),
        padding_x: raster.padding_x,
        padding_y: raster.padding_y,
        viewport,
    })
}

fn browser_app_window_chrome_height(cell_height: usize, padding_y: usize) -> Result<usize> {
    WINDOW_CHROME_ROWS
        .checked_mul(cell_height.max(1))
        .and_then(|height| height.checked_add(padding_y.saturating_mul(2)))
        .ok_or_else(|| anyhow!("browser app window chrome height overflow"))
}

fn hit_test_browser_app_window(
    app: &BrowserApp,
    x: usize,
    y: usize,
) -> Result<BrowserAppWindowHit> {
    let geometry = browser_app_window_geometry(app)?;
    if x >= geometry.width || y >= geometry.chrome_height.saturating_add(geometry.page_height) {
        return Ok(BrowserAppWindowHit::Outside);
    }

    if y < geometry.chrome_height {
        return hit_test_browser_app_window_chrome(app, geometry, x, y);
    }

    let page_y = y.saturating_sub(geometry.chrome_height);
    if page_y >= geometry.page_height {
        return Ok(BrowserAppWindowHit::Outside);
    }
    if x < geometry.padding_x || page_y < geometry.padding_y {
        return Ok(BrowserAppWindowHit::PageFrame);
    }
    let content_x = x.saturating_sub(geometry.padding_x);
    let content_y = page_y.saturating_sub(geometry.padding_y);
    let page_cell_x = content_x / geometry.cell_width;
    let page_cell_y = content_y / geometry.cell_height;
    if page_cell_x >= geometry.viewport.width || page_cell_y >= geometry.viewport.height {
        return Ok(BrowserAppWindowHit::PageFrame);
    }
    Ok(BrowserAppWindowHit::PageViewport {
        x: page_cell_x,
        y: page_cell_y,
    })
}

fn hit_test_browser_app_window_chrome(
    app: &BrowserApp,
    geometry: BrowserAppWindowGeometry,
    x: usize,
    y: usize,
) -> Result<BrowserAppWindowHit> {
    if x < geometry.padding_x || y < geometry.padding_y {
        return Ok(BrowserAppWindowHit::ChromeBackground);
    }
    let column = x.saturating_sub(geometry.padding_x) / geometry.cell_width;
    let row = y.saturating_sub(geometry.padding_y) / geometry.cell_height;
    match row {
        0 => hit_test_browser_app_window_tabs(app, geometry, column),
        1 => Ok(BrowserAppWindowHit::LocationBar),
        2 => Ok(BrowserAppWindowHit::StatusBar),
        _ => Ok(BrowserAppWindowHit::ChromeBackground),
    }
}

fn hit_test_browser_app_window_tabs(
    app: &BrowserApp,
    geometry: BrowserAppWindowGeometry,
    column: usize,
) -> Result<BrowserAppWindowHit> {
    let visible_columns = geometry
        .width
        .saturating_sub(geometry.padding_x.saturating_mul(2))
        / geometry.cell_width;
    if column >= visible_columns {
        return Ok(BrowserAppWindowHit::ChromeBackground);
    }
    match column {
        WINDOW_CHROME_BACK_COLUMN => return Ok(BrowserAppWindowHit::BackButton),
        WINDOW_CHROME_FORWARD_COLUMN => return Ok(BrowserAppWindowHit::ForwardButton),
        WINDOW_CHROME_RELOAD_COLUMN => return Ok(BrowserAppWindowHit::ReloadButton),
        WINDOW_CHROME_NEW_TAB_COLUMN => return Ok(BrowserAppWindowHit::NewTabButton),
        _ => {}
    }

    let mut cursor = WINDOW_CHROME_TAB_START_COLUMN;
    for tab in app.tab_summaries() {
        let segment = browser_app_window_tab_segment(&tab);
        let len = segment.chars().count();
        if column >= cursor && column < cursor.saturating_add(len) {
            return Ok(BrowserAppWindowHit::Tab { index: tab.index });
        }
        cursor = cursor.saturating_add(len).saturating_add(1);
        if cursor >= visible_columns {
            break;
        }
    }
    Ok(BrowserAppWindowHit::ChromeBackground)
}

fn compose_browser_app_window_frame(
    report: &BrowserAppReport,
    page_frame: BrowserViewportFrame,
    options: &BrowserAppWindowFrameOptions,
) -> Result<BrowserAppWindowFrame> {
    let width = page_frame.raster.width.max(1);
    let cell_width = page_frame.report.cell_width.max(1);
    let cell_height = page_frame.report.cell_height.max(1);
    let padding_x = page_frame.report.padding_x;
    let padding_y = page_frame.report.padding_y;
    let chrome_height = browser_app_window_chrome_height(cell_height, padding_y)?;
    let height = chrome_height
        .checked_add(page_frame.raster.height)
        .ok_or_else(|| anyhow!("browser app window height overflow"))?;
    let pixel_bytes = width
        .checked_mul(height)
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or_else(|| anyhow!("browser app window pixel buffer overflow"))?;
    let background = [255, 255, 255, 255];
    let mut raster = BrowserRgbaRaster {
        width,
        height,
        background,
        pixels: vec![255; pixel_bytes],
    };

    fill_window_rect(
        &mut raster,
        0,
        0,
        width,
        chrome_height,
        [236, 238, 241, 255],
    );
    fill_window_rect(
        &mut raster,
        0,
        0,
        width,
        padding_y.saturating_mul(2).saturating_add(cell_height),
        [222, 226, 232, 255],
    );
    fill_window_rect(
        &mut raster,
        0,
        chrome_height.saturating_sub(1),
        width,
        1,
        [152, 160, 171, 255],
    );

    let active = &report.tabs[report.active_tab];
    let columns = width.saturating_sub(padding_x.saturating_mul(2)) / cell_width;
    let tab_line = browser_app_window_tab_line(report, columns);
    let location_line = options
        .location_text
        .clone()
        .unwrap_or_else(|| format!("URL {}", active.source));
    let status_line = options
        .status_text
        .clone()
        .unwrap_or_else(|| browser_app_window_status_line(report));
    let rows = [tab_line, location_line, status_line];
    for (row, line) in rows.iter().enumerate() {
        draw_window_text(
            &mut raster,
            padding_x,
            padding_y.saturating_add(row.saturating_mul(cell_height)),
            cell_width,
            line,
            [31, 41, 55, 255],
        );
    }

    copy_window_rgba(&page_frame.raster, &mut raster, 0, chrome_height)?;

    let page_frame_pixel_bytes = page_frame.raster.pixels.len();
    let raster_pixel_bytes = raster.pixels.len();
    let window_frame_cache_reusable = !report.frame_full_repaint
        && report.frame_dirty_pixel_area == 0
        && raster_pixel_bytes <= BROWSER_APP_CACHED_WINDOW_FRAME_MAX_BYTES;
    let cached_window_frame_pixel_bytes = window_frame_cache_reusable
        .then_some(raster_pixel_bytes)
        .unwrap_or(0);
    let pixel_hash = raster.pixel_hash();
    let non_background_pixels = raster.non_background_pixels();
    Ok(BrowserAppWindowFrame {
        report: BrowserAppWindowFrameReport {
            active_tab: report.active_tab,
            tab_count: report.tabs.len(),
            title: active.title.clone(),
            source: active.source.clone(),
            chrome_rows: WINDOW_CHROME_ROWS,
            chrome_height,
            content_y: chrome_height,
            window_width: width,
            window_height: height,
            page_frame_width: page_frame.report.frame_width,
            page_frame_height: page_frame.report.frame_height,
            bytes_per_pixel: 4,
            raster_pixel_bytes,
            page_frame_pixel_bytes,
            cached_window_frame_pixel_bytes,
            cached_window_frame_limit_bytes: BROWSER_APP_CACHED_WINDOW_FRAME_MAX_BYTES,
            window_frame_cache_reusable,
            pixel_hash,
            non_background_pixels,
            artifact_format: "png-rgba8-browser-window".to_owned(),
            page: page_frame.report,
        },
        raster,
    })
}

fn browser_app_window_tab_line(report: &BrowserAppReport, columns: usize) -> String {
    let mut line = WINDOW_CHROME_BUTTON_LINE.to_owned();
    for tab in &report.tabs {
        if line.chars().count() > WINDOW_CHROME_BUTTON_LINE.chars().count() {
            line.push(' ');
        }
        line.push_str(&browser_app_window_tab_segment(tab));
        if line.chars().count() >= columns {
            break;
        }
    }
    truncate_window_text(&line, columns)
}

fn browser_app_window_tab_segment(tab: &BrowserAppTabSummary) -> String {
    let marker = if tab.active { '*' } else { ' ' };
    format!("[{marker}{} {}]", tab.index, tab.title)
}

fn browser_app_window_status_line(report: &BrowserAppReport) -> String {
    let history_position = report
        .history
        .current_index
        .map(|index| index + 1)
        .unwrap_or(0);
    let mut status = format!(
        "links={} forms={} history={}/{} viewport={}x{}+{}+{}",
        report.links.len(),
        report.forms.len(),
        history_position,
        report.history.entries.len(),
        report.viewport.width,
        report.viewport.height,
        report.viewport.x,
        report.viewport.y
    );
    if let Some(find) = &report.find {
        status.push_str(&format!(
            " find={}/{} {}",
            find.active_match_index + 1,
            find.match_count,
            find.query
        ));
    }
    status
}

fn truncate_window_text(text: &str, columns: usize) -> String {
    if columns == 0 {
        return String::new();
    }
    text.chars().take(columns).collect()
}

fn draw_window_text(
    raster: &mut BrowserRgbaRaster,
    x: usize,
    y: usize,
    cell_width: usize,
    text: &str,
    color: [u8; 4],
) {
    if cell_width == 0 {
        return;
    }
    for (column, ch) in text.chars().enumerate() {
        let glyph_x = x.saturating_add(column.saturating_mul(cell_width));
        draw_window_glyph(raster, glyph_x, y, ch, color);
    }
}

fn draw_window_glyph(
    raster: &mut BrowserRgbaRaster,
    cell_x: usize,
    cell_y: usize,
    ch: char,
    color: [u8; 4],
) {
    for (row, mask) in super::glyph_rows(ch).iter().enumerate() {
        for column in 0..5 {
            if (mask & (1 << (4 - column))) == 0 {
                continue;
            }
            set_window_pixel(
                raster,
                cell_x.saturating_add(1 + column),
                cell_y.saturating_add(2 + row),
                color,
            );
        }
    }
}

fn fill_window_rect(
    raster: &mut BrowserRgbaRaster,
    x: usize,
    y: usize,
    width: usize,
    height: usize,
    color: [u8; 4],
) {
    for row in y..y.saturating_add(height).min(raster.height) {
        for column in x..x.saturating_add(width).min(raster.width) {
            set_window_pixel(raster, column, row, color);
        }
    }
}

fn copy_window_rgba(
    source: &BrowserRgbaRaster,
    target: &mut BrowserRgbaRaster,
    target_x: usize,
    target_y: usize,
) -> Result<()> {
    if target_x.saturating_add(source.width) > target.width
        || target_y.saturating_add(source.height) > target.height
    {
        bail!("browser app window frame cannot fit page raster");
    }
    let source_row_bytes = source.width.saturating_mul(4);
    let target_row_bytes = target.width.saturating_mul(4);
    for row in 0..source.height {
        let source_start = row.saturating_mul(source_row_bytes);
        let source_end = source_start.saturating_add(source_row_bytes);
        let target_start = target_y
            .saturating_add(row)
            .saturating_mul(target_row_bytes)
            .saturating_add(target_x.saturating_mul(4));
        let target_end = target_start.saturating_add(source_row_bytes);
        let Some(source_slice) = source.pixels.get(source_start..source_end) else {
            bail!("browser app source raster row is truncated");
        };
        let Some(target_slice) = target.pixels.get_mut(target_start..target_end) else {
            bail!("browser app target raster row is truncated");
        };
        target_slice.copy_from_slice(source_slice);
    }
    Ok(())
}

fn set_window_pixel(raster: &mut BrowserRgbaRaster, x: usize, y: usize, color: [u8; 4]) {
    let Some(index) = y
        .checked_mul(raster.width)
        .and_then(|row| row.checked_add(x))
        .and_then(|pixel| pixel.checked_mul(4))
    else {
        return;
    };
    let Some(pixel) = raster.pixels.get_mut(index..index.saturating_add(4)) else {
        return;
    };
    pixel.copy_from_slice(&color);
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;

    use super::*;

    fn app_options() -> BrowserAppOptions {
        BrowserAppOptions {
            render: BrowserRenderOptions {
                width: 40,
                ..BrowserRenderOptions::default()
            },
            viewport_width: 40,
            viewport_height: 2,
            raster: BrowserRasterOptions::default(),
        }
    }

    #[tokio::test]
    async fn browser_app_presents_viewport_frames_with_scroll_damage() {
        let mut app = BrowserApp::open(
            "bench/browser-fixtures/max-width-layout.html",
            app_options(),
        )
        .await
        .unwrap();

        let initial = app.present_frame().unwrap();
        assert!(initial.report.viewport.full_repaint);
        assert_eq!(initial.report.frame_width, 328);
        assert_eq!(initial.report.frame_height, 32);

        app.apply_action(BrowserAppAction::Scroll {
            delta_x: 0,
            delta_y: 99,
        })
        .await
        .unwrap();
        let scrolled = app.present_frame().unwrap();
        assert!(!scrolled.report.viewport.full_repaint);
        assert_eq!(
            scrolled.report.viewport.viewport,
            BrowserViewportState {
                x: 0,
                y: 1,
                width: 40,
                height: 2
            }
        );
        assert_eq!(scrolled.report.dirty_pixel_regions.len(), 1);
        assert_eq!(scrolled.report.dirty_pixel_regions[0].x, 4);
        assert_eq!(scrolled.report.dirty_pixel_regions[0].y, 16);
        assert_eq!(scrolled.report.dirty_pixel_regions[0].width, 320);
        assert_eq!(scrolled.report.dirty_pixel_regions[0].height, 12);
    }

    #[tokio::test]
    async fn browser_app_reuses_stable_presented_viewport_frames() {
        let mut app = BrowserApp::open(
            "bench/browser-fixtures/max-width-layout.html",
            app_options(),
        )
        .await
        .unwrap();

        let initial = app.present_frame().unwrap();
        assert!(initial.report.viewport.full_repaint);
        assert!(app.tabs[0].last_presented_frame.is_none());

        let stable = app.present_frame().unwrap();
        assert!(!stable.report.viewport.full_repaint);
        assert_eq!(stable.report.dirty_pixel_area, 0);
        assert!(app.tabs[0].last_presented_frame.is_some());
        assert!(browser_app_frame_is_reusable(&stable));

        let reused = app.present_frame().unwrap();
        assert_eq!(reused.report, stable.report);
        assert_eq!(reused.raster.pixels, stable.raster.pixels);

        let report = app.report_for_frame(reused.report.clone()).unwrap();
        assert!(report.frame_cache_reusable);
        assert_eq!(
            report.cached_frame_pixel_bytes,
            browser_viewport_frame_pixel_bytes(&reused.report)
        );
        assert_eq!(
            report.cached_frame_limit_bytes,
            BROWSER_APP_CACHED_FRAME_MAX_BYTES
        );

        let mut oversized = stable.clone();
        oversized.report.frame_width = BROWSER_APP_CACHED_FRAME_MAX_BYTES;
        oversized.report.frame_height = 2;
        oversized.report.bytes_per_pixel = 4;
        assert!(!browser_app_frame_is_reusable(&oversized));

        app.apply_action(BrowserAppAction::Scroll {
            delta_x: 0,
            delta_y: 99,
        })
        .await
        .unwrap();
        assert!(app.tabs[0].last_presented_frame.is_none());
    }

    #[tokio::test]
    async fn browser_app_reuses_stable_window_frames_with_bounds() {
        let mut app = BrowserApp::open(
            "bench/browser-fixtures/max-width-layout.html",
            app_options(),
        )
        .await
        .unwrap();

        let initial = app.present_window_frame().unwrap();
        assert!(initial.report.page.viewport.full_repaint);
        assert!(!initial.report.window_frame_cache_reusable);
        assert!(app.tabs[0].last_presented_window_frame.is_none());

        let stable = app.present_window_frame().unwrap();
        assert!(!stable.report.page.viewport.full_repaint);
        assert_eq!(stable.report.page.dirty_pixel_area, 0);
        assert!(stable.report.window_frame_cache_reusable);
        assert_eq!(
            stable.report.cached_window_frame_pixel_bytes,
            stable.raster.pixels.len()
        );
        assert_eq!(
            stable.report.cached_window_frame_limit_bytes,
            BROWSER_APP_CACHED_WINDOW_FRAME_MAX_BYTES
        );
        assert!(app.tabs[0].last_presented_window_frame.is_some());
        assert!(browser_app_window_frame_is_reusable(&stable));

        let reused = app.present_window_frame().unwrap();
        assert_eq!(reused.report, stable.report);
        assert_eq!(reused.raster.pixels, stable.raster.pixels);

        let mut oversized = stable.clone();
        oversized.report.window_frame_cache_reusable = true;
        oversized.raster.pixels = vec![0; BROWSER_APP_CACHED_WINDOW_FRAME_MAX_BYTES + 1];
        assert!(!browser_app_window_frame_is_reusable(&oversized));

        app.apply_action(BrowserAppAction::Scroll {
            delta_x: 0,
            delta_y: 99,
        })
        .await
        .unwrap();
        assert!(app.tabs[0].last_presented_window_frame.is_none());
    }

    #[tokio::test]
    async fn browser_app_presents_window_frame_with_chrome_and_page_pixels() {
        let mut app = BrowserApp::open("bench/browser-fixtures/static-text.html", app_options())
            .await
            .unwrap();
        app.apply_action(BrowserAppAction::NewTab("max-width-layout.html".to_owned()))
            .await
            .unwrap();

        let window = app.present_window_frame().unwrap();
        assert_eq!(window.report.tab_count, 2);
        assert_eq!(window.report.active_tab, 1);
        assert_eq!(window.report.content_y, window.report.chrome_height);
        assert_eq!(window.report.window_width, window.report.page_frame_width);
        assert!(window.report.window_height > window.report.page_frame_height);
        assert_eq!(window.report.bytes_per_pixel, 4);
        assert_eq!(window.report.artifact_format, "png-rgba8-browser-window");
        assert!(window.report.non_background_pixels > window.report.page.non_background_pixels);
        assert_eq!(window.report.raster_pixel_bytes, window.raster.pixels.len());
        assert_eq!(
            window.report.page_frame_pixel_bytes,
            window
                .report
                .page_frame_width
                .saturating_mul(window.report.page_frame_height)
                .saturating_mul(window.report.bytes_per_pixel)
        );
        assert_eq!(
            window.report.cached_window_frame_limit_bytes,
            BROWSER_APP_CACHED_WINDOW_FRAME_MAX_BYTES
        );
        assert_eq!(
            window.raster.pixels.len(),
            window
                .report
                .window_width
                .saturating_mul(window.report.window_height)
                .saturating_mul(4)
        );
        let png = window.raster.encode_png().unwrap();
        assert!(png.starts_with(b"\x89PNG\r\n\x1a\n"));
        assert!(png.windows(4).any(|chunk| chunk == b"IDAT"));
    }

    #[tokio::test]
    async fn browser_app_window_frame_options_override_chrome_text() {
        let mut app = BrowserApp::open("bench/browser-fixtures/static-text.html", app_options())
            .await
            .unwrap();
        let default = app.present_window_frame().unwrap();
        let overridden = app
            .present_window_frame_with_options(BrowserAppWindowFrameOptions {
                location_text: Some("URL > edited.html".to_owned()),
                status_text: Some("location: Enter=open Esc=cancel".to_owned()),
            })
            .unwrap();

        assert_eq!(overridden.report.title, default.report.title);
        assert_eq!(overridden.report.source, default.report.source);
        assert_eq!(overridden.report.window_width, default.report.window_width);
        assert_eq!(
            overridden.report.window_height,
            default.report.window_height
        );
        assert_ne!(overridden.report.pixel_hash, default.report.pixel_hash);
        assert_ne!(overridden.raster.pixels, default.raster.pixels);
    }

    #[tokio::test]
    async fn browser_app_window_clicks_route_chrome_and_page_actions() {
        let dir = tempdir().unwrap();
        let first = dir.path().join("first.html");
        let second = dir.path().join("second.html");
        fs::write(
            &first,
            r#"<html><head><title>First</title></head><body><a href="second.html">Second</a></body></html>"#,
        )
        .unwrap();
        fs::write(
            &second,
            r#"<html><head><title>Second</title></head><body>Arrived</body></html>"#,
        )
        .unwrap();

        let mut app = BrowserApp::open(&first.to_string_lossy(), app_options())
            .await
            .unwrap();
        let window = app.present_window_frame().unwrap();
        let page_x = window.report.page.padding_x + 1;
        let page_y = window
            .report
            .content_y
            .saturating_add(window.report.page.padding_y)
            .saturating_add(2);
        let page_click = app.click_window(page_x, page_y).await.unwrap();
        assert_eq!(
            page_click.hit,
            BrowserAppWindowHit::PageViewport { x: 0, y: 0 }
        );
        assert_eq!(
            app.active_session().unwrap().current().unwrap().title,
            "Second"
        );

        let window = app.present_window_frame().unwrap();
        let back_x = window.report.page.padding_x + 1;
        let back_y = window.report.page.padding_y + 2;
        let back_click = app.click_window(back_x, back_y).await.unwrap();
        assert_eq!(back_click.hit, BrowserAppWindowHit::BackButton);
        assert_eq!(
            app.active_session().unwrap().current().unwrap().title,
            "First"
        );

        let window = app.present_window_frame().unwrap();
        let new_tab_x = window
            .report
            .page
            .padding_x
            .saturating_add(WINDOW_CHROME_NEW_TAB_COLUMN * window.report.page.cell_width)
            .saturating_add(1);
        let new_tab_y = window.report.page.padding_y + 2;
        let new_tab_click = app.click_window(new_tab_x, new_tab_y).await.unwrap();
        assert_eq!(new_tab_click.hit, BrowserAppWindowHit::NewTabButton);
        assert_eq!(app.tab_count(), 2);
        assert_eq!(app.active_tab(), 1);

        let window = app.present_window_frame().unwrap();
        let first_tab_x = window
            .report
            .page
            .padding_x
            .saturating_add(WINDOW_CHROME_TAB_START_COLUMN * window.report.page.cell_width)
            .saturating_add(1);
        let first_tab_y = window.report.page.padding_y + 2;
        let tab_click = app.click_window(first_tab_x, first_tab_y).await.unwrap();
        assert_eq!(tab_click.hit, BrowserAppWindowHit::Tab { index: 0 });
        assert_eq!(app.active_tab(), 0);
    }

    #[tokio::test]
    async fn browser_app_window_new_tab_button_opens_blank_tab() {
        let mut app = BrowserApp::open("bench/browser-fixtures/static-text.html", app_options())
            .await
            .unwrap();
        let window = app.present_window_frame().unwrap();
        let new_tab_x = window
            .report
            .page
            .padding_x
            .saturating_add(WINDOW_CHROME_NEW_TAB_COLUMN * window.report.page.cell_width)
            .saturating_add(1);
        let new_tab_y = window.report.page.padding_y + 2;

        let new_tab_click = app.click_window(new_tab_x, new_tab_y).await.unwrap();

        assert_eq!(new_tab_click.hit, BrowserAppWindowHit::NewTabButton);
        assert_eq!(new_tab_click.action, Some(BrowserAppAction::NewBlankTab));
        assert_eq!(app.tab_count(), 2);
        assert_eq!(app.active_tab(), 1);
        assert_eq!(
            app.active_session().unwrap().current().unwrap().source,
            "about:blank"
        );
    }

    #[tokio::test]
    async fn browser_app_clicks_links_and_resets_to_full_frame() {
        let dir = tempdir().unwrap();
        let first = dir.path().join("first.html");
        let second = dir.path().join("second.html");
        fs::write(
            &first,
            r#"<html><head><title>First</title></head><body><a href="second.html">Second</a></body></html>"#,
        )
        .unwrap();
        fs::write(
            &second,
            r#"<html><head><title>Second</title></head><body>Arrived</body></html>"#,
        )
        .unwrap();

        let mut app = BrowserApp::open(&first.to_string_lossy(), app_options())
            .await
            .unwrap();
        app.present_frame().unwrap();
        app.apply_action(BrowserAppAction::Click { x: 0, y: 0 })
            .await
            .unwrap();
        let report = app.report().unwrap();

        assert_eq!(report.history.entries.len(), 2);
        assert_eq!(report.history.current_index, Some(1));
        assert_eq!(report.tabs[0].title, "Second");
        assert!(report.frame.viewport.full_repaint);
        assert!(report.frame.viewport.source.ends_with("second.html"));
    }

    #[tokio::test]
    async fn browser_app_opens_clicked_links_in_background_tab() {
        let dir = tempdir().unwrap();
        let first = dir.path().join("first.html");
        let second = dir.path().join("second.html");
        fs::write(
            &first,
            r#"<html><head><title>First</title></head><body><a href="second.html">Second</a></body></html>"#,
        )
        .unwrap();
        fs::write(
            &second,
            r#"<html><head><title>Second</title></head><body>Arrived</body></html>"#,
        )
        .unwrap();

        let mut app = BrowserApp::open(&first.to_string_lossy(), app_options())
            .await
            .unwrap();
        app.present_frame().unwrap();
        app.apply_action(BrowserAppAction::OpenClickInBackgroundTab { x: 0, y: 0 })
            .await
            .unwrap();

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
    async fn browser_app_opens_clicked_links_in_foreground_tab() {
        let dir = tempdir().unwrap();
        let first = dir.path().join("first.html");
        let second = dir.path().join("second.html");
        fs::write(
            &first,
            r#"<html><head><title>First</title></head><body><a href="second.html">Second</a></body></html>"#,
        )
        .unwrap();
        fs::write(
            &second,
            r#"<html><head><title>Second</title></head><body>Arrived</body></html>"#,
        )
        .unwrap();

        let mut app = BrowserApp::open(&first.to_string_lossy(), app_options())
            .await
            .unwrap();
        app.present_frame().unwrap();
        app.apply_action(BrowserAppAction::OpenClickInForegroundTab { x: 0, y: 0 })
            .await
            .unwrap();

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
    async fn browser_app_activates_links_by_text_and_selector() {
        let dir = tempdir().unwrap();
        let first = dir.path().join("first.html");
        let second = dir.path().join("second.html");
        let third = dir.path().join("third.html");
        fs::write(
            &first,
            r#"<html><head><title>First</title></head><body><a href="second.html">Second target</a><nav><a id="third" href="third.html">Third</a></nav></body></html>"#,
        )
        .unwrap();
        fs::write(
            &second,
            r#"<html><head><title>Second</title></head><body>Arrived second</body></html>"#,
        )
        .unwrap();
        fs::write(
            &third,
            r#"<html><head><title>Third</title></head><body>Arrived third</body></html>"#,
        )
        .unwrap();

        let mut app = BrowserApp::open(&first.to_string_lossy(), app_options())
            .await
            .unwrap();
        app.apply_action(BrowserAppAction::ActivateLinkText(
            "Second target".to_owned(),
        ))
        .await
        .unwrap();
        assert_eq!(
            app.active_session().unwrap().current().unwrap().title,
            "Second"
        );

        app.apply_action(BrowserAppAction::Back).await.unwrap();
        app.apply_action(BrowserAppAction::ActivateLinkSelector(
            "nav a#third".to_owned(),
        ))
        .await
        .unwrap();
        assert_eq!(
            app.active_session().unwrap().current().unwrap().title,
            "Third"
        );
    }

    #[tokio::test]
    async fn browser_app_opens_with_profile_state_and_can_clear_storage() {
        let dir = tempdir().unwrap();
        let set_page = dir.path().join("set.html");
        let read_page = dir.path().join("read.html");
        fs::write(
            &set_page,
            r#"<html><body><p id="out">Before</p><script>localStorage.setItem("headline", "Saved app state"); document.getElementById("out").textContent = localStorage.getItem("headline");</script></body></html>"#,
        )
        .unwrap();
        fs::write(
            &read_page,
            r#"<html><body><p id="out">Before</p><script>document.getElementById("out").textContent = localStorage.getItem("headline");</script></body></html>"#,
        )
        .unwrap();

        let first = BrowserApp::open(&set_page.to_string_lossy(), app_options())
            .await
            .unwrap();
        assert_eq!(
            first.active_session().unwrap().current().unwrap().text,
            "Saved app state"
        );
        let local_storage = first.active_session().unwrap().local_storage_snapshot();

        let mut second = BrowserApp::open_with_state(
            &read_page.to_string_lossy(),
            app_options(),
            BrowserCookieJar::default(),
            local_storage,
        )
        .await
        .unwrap();
        assert_eq!(
            second.active_session().unwrap().current().unwrap().text,
            "Saved app state"
        );

        second
            .apply_action(BrowserAppAction::ClearLocalStorage)
            .await
            .unwrap();
        assert!(second.report().unwrap().local_storage.is_empty());
    }

    #[tokio::test]
    async fn browser_app_sets_viewport_origin_and_reports_existing_frame() {
        let mut app = BrowserApp::open(
            "bench/browser-fixtures/max-width-layout.html",
            app_options(),
        )
        .await
        .unwrap();

        app.apply_action(BrowserAppAction::SetViewportOrigin { x: 99, y: 99 })
            .await
            .unwrap();
        let frame = app.present_frame().unwrap();
        assert_eq!(
            frame.report.viewport.viewport,
            BrowserViewportState {
                x: 0,
                y: 1,
                width: 40,
                height: 2
            }
        );
        let report = app.report_for_frame(frame.report.clone()).unwrap();
        assert_eq!(report.frame.pixel_hash, frame.report.pixel_hash);
        assert_eq!(
            report.frame_pixel_bytes,
            frame
                .report
                .frame_width
                .saturating_mul(frame.report.frame_height)
                .saturating_mul(frame.report.bytes_per_pixel)
        );
        assert_eq!(report.frame_dirty_pixel_area, frame.report.dirty_pixel_area);
        assert_eq!(
            report.frame_reused_viewport_area,
            frame.report.viewport.reused_area
        );
        assert_eq!(
            report.frame_full_repaint,
            frame.report.viewport.full_repaint
        );
        assert_eq!(report.viewport.y, 1);
        assert_eq!(
            report.visible_text,
            vec![
                "          column wraps words".to_owned(),
                "          cleanly      #".to_owned()
            ]
        );
    }

    #[tokio::test]
    async fn browser_app_report_serialization_bounds_large_text() {
        let dir = tempdir().unwrap();
        let page = dir.path().join("large-report-text.html");
        let repeated = "rust browser ".repeat(BROWSER_APP_REPORT_TEXT_MAX_CHARS / 13 + 32);
        fs::write(
            &page,
            format!("<html><head><title>Large</title></head><body>{repeated}</body></html>"),
        )
        .unwrap();

        let mut app = BrowserApp::open(&page.display().to_string(), app_options())
            .await
            .unwrap();
        let report = app.report().unwrap();
        let serialized = serde_json::to_value(&report).unwrap();
        let serialized_text = serialized["text"].as_str().unwrap();

        assert_eq!(report.text_chars, report.text.chars().count());
        assert!(report.text_chars > BROWSER_APP_REPORT_TEXT_MAX_CHARS);
        assert!(report.visible_text_chars <= report.text_chars);
        assert_ne!(serialized_text, report.text);
        assert!(serialized_text.contains("brutal-app-report-text-truncated"));
        assert!(serialized_text.contains(&format!("text_chars={}", report.text_chars)));
        assert!(serialized_text.chars().count() <= BROWSER_APP_REPORT_TEXT_MAX_CHARS + 128);
    }

    #[tokio::test]
    async fn browser_app_reload_preserves_viewport_and_repaints_full_frame() {
        let dir = tempdir().unwrap();
        let page = dir.path().join("reload-viewport.html");
        fs::write(
            &page,
            r#"<html><head><title>Reload Viewport</title></head><body>
<p>line one</p>
<p>line two</p>
<p>line three</p>
<p>line four</p>
<p>line five</p>
<p>line six</p>
</body></html>"#,
        )
        .unwrap();

        let mut options = app_options();
        options.viewport_height = 1;
        let mut app = BrowserApp::open(&page.to_string_lossy(), options)
            .await
            .unwrap();
        app.apply_action(BrowserAppAction::SetViewportOrigin { x: 0, y: 3 })
            .await
            .unwrap();
        let before = app.present_frame().unwrap();
        assert_eq!(before.report.viewport.viewport.y, 3);
        assert!(before.report.viewport.full_repaint);

        app.apply_action(BrowserAppAction::Reload).await.unwrap();
        assert_eq!(app.active_viewport().unwrap().y, 3);
        let after = app.present_frame().unwrap();

        assert_eq!(after.report.viewport.viewport.y, 3);
        assert!(after.report.viewport.full_repaint);
    }

    #[tokio::test]
    async fn browser_app_find_text_scrolls_and_tracks_match_state() {
        let dir = tempdir().unwrap();
        let page = dir.path().join("find.html");
        fs::write(
            &page,
            r#"<html><head><title>Find</title></head><body><p>Alpha</p><p>Beta needle</p><p>Gamma needle</p></body></html>"#,
        )
        .unwrap();

        let mut options = app_options();
        options.viewport_height = 1;
        let mut app = BrowserApp::open(&page.to_string_lossy(), options)
            .await
            .unwrap();
        app.apply_action(BrowserAppAction::FindText {
            query: "needle".to_owned(),
            next: false,
        })
        .await
        .unwrap();
        let first = app.report().unwrap();
        let first_find = first.find.clone().unwrap();
        assert_eq!(first_find.query, "needle");
        assert_eq!(first_find.active_match_index, 0);
        assert_eq!(first_find.match_count, 2);
        assert_eq!(first.viewport.y, first_find.line);
        assert!(
            first
                .visible_text
                .first()
                .is_some_and(|line| line.contains("needle"))
        );

        app.apply_action(BrowserAppAction::FindText {
            query: "needle".to_owned(),
            next: true,
        })
        .await
        .unwrap();
        let second = app.report().unwrap();
        let second_find = second.find.clone().unwrap();
        assert_eq!(second_find.active_match_index, 1);
        assert_eq!(second_find.match_count, 2);
        assert!(second_find.line > first_find.line);
        assert_eq!(second.viewport.y, second_find.line);

        app.apply_action(BrowserAppAction::FindTextPrevious {
            query: "needle".to_owned(),
        })
        .await
        .unwrap();
        let previous = app.report().unwrap();
        let previous_find = previous.find.clone().unwrap();
        assert_eq!(previous_find.active_match_index, 0);
        assert_eq!(previous_find.match_count, 2);
        assert_eq!(previous.viewport.y, first_find.line);

        let missing = app
            .apply_action(BrowserAppAction::FindText {
                query: "missing".to_owned(),
                next: false,
            })
            .await
            .unwrap_err()
            .to_string();
        assert!(missing.contains("text not found"));
    }

    #[tokio::test]
    async fn browser_app_tabs_share_profile_state_and_keep_viewports() {
        let mut app = BrowserApp::open("bench/browser-fixtures/static-text.html", app_options())
            .await
            .unwrap();
        app.apply_action(BrowserAppAction::NewTab("max-width-layout.html".to_owned()))
            .await
            .unwrap();

        assert_eq!(app.tab_count(), 2);
        assert_eq!(app.active_tab(), 1);
        app.apply_action(BrowserAppAction::Scroll {
            delta_x: 0,
            delta_y: 1,
        })
        .await
        .unwrap();
        assert_eq!(app.active_viewport().unwrap().y, 1);
        app.apply_action(BrowserAppAction::SwitchTab(0))
            .await
            .unwrap();
        assert_eq!(app.active_viewport().unwrap().y, 0);

        let report = app.report().unwrap();
        assert_eq!(report.tabs.len(), 2);
        assert!(report.tabs[0].active);
        assert_eq!(report.tabs[1].viewport.y, 1);
    }

    #[tokio::test]
    async fn browser_app_restores_most_recent_closed_tab() {
        let mut app = BrowserApp::open(
            "bench/browser-fixtures/list-marker-types.html",
            app_options(),
        )
        .await
        .unwrap();
        app.apply_action(BrowserAppAction::DuplicateTab)
            .await
            .unwrap();
        app.apply_action(BrowserAppAction::Scroll {
            delta_x: 0,
            delta_y: 3,
        })
        .await
        .unwrap();
        let closed_viewport = app.active_viewport().unwrap();
        assert!(closed_viewport.y > 0);

        app.apply_action(BrowserAppAction::CloseTab(None))
            .await
            .unwrap();
        assert_eq!(app.tab_count(), 1);
        assert_eq!(app.closed_tab_count(), 1);

        app.apply_action(BrowserAppAction::RestoreClosedTab)
            .await
            .unwrap();
        assert_eq!(app.tab_count(), 2);
        assert_eq!(app.closed_tab_count(), 0);
        assert_eq!(app.active_tab(), 1);
        assert_eq!(app.active_viewport().unwrap().y, closed_viewport.y);
        assert_eq!(
            app.active_session().unwrap().current().unwrap().source,
            "bench/browser-fixtures/list-marker-types.html"
        );

        let missing = app
            .apply_action(BrowserAppAction::RestoreClosedTab)
            .await
            .unwrap_err()
            .to_string();
        assert!(missing.contains("no closed tab"));
    }
}
