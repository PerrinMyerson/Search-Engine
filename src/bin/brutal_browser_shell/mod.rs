use std::path::Path;

use anyhow::{Context, Result, anyhow};
use brutal_search::browser::{
    BrowserCookie, BrowserCookieJar, BrowserFocusedControl, BrowserForm, BrowserFormOption,
    BrowserHistorySnapshot, BrowserLocalStorageEntry, BrowserRasterOptions, BrowserRenderOptions,
    BrowserRgbaRasterReport, BrowserSession, BrowserTextViewportOptions, BrowserTextViewportReport,
    browser_text_viewport, ensure_static_target, rasterize_render_rgba, rgba_raster_report,
};
use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BrowserFormSubmitMode {
    Auto,
    Get,
    Post,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum BrowserShellCommand {
    Open(String),
    Back,
    Forward,
    Reload,
    Location,
    Cookies,
    LocalStorage,
    SessionStorage,
    ClearCookies,
    ClearLocalStorage,
    ClearSessionStorage,
    Click(String),
    ClickAt {
        x: usize,
        y: usize,
    },
    Links,
    Forms,
    Link(BrowserShellLinkTarget),
    Focus(String),
    FocusNext,
    FocusPrevious,
    TypeText(String),
    DeleteTextBackward(usize),
    ClearText,
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
    Find {
        query: String,
        next: bool,
    },
    Fill {
        form_index: usize,
        name: String,
        value: String,
    },
    Submit {
        mode: BrowserFormSubmitMode,
        form_index: usize,
        fields: Vec<(String, String)>,
    },
    Styles,
    Scripts,
    Images,
    Resources,
    Tabs,
    NewTab(String),
    SwitchTab(usize),
    CloseTab(Option<usize>),
    Scroll(isize),
    HorizontalScroll(isize),
    Top,
    Bottom,
    Render,
    History,
    Help,
    Quit,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum BrowserShellLinkTarget {
    Index(usize),
    Text(String),
    Selector(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct BrowserShellState {
    pub(crate) viewport_x: usize,
    pub(crate) viewport_y: usize,
    pub(crate) viewport_width: usize,
    pub(crate) viewport_height: usize,
}

pub(crate) struct BrowserShellTab {
    pub(crate) session: BrowserSession,
    pub(crate) state: BrowserShellState,
}

pub(crate) struct BrowserShellTabs {
    tabs: Vec<BrowserShellTab>,
    active_tab: usize,
    options: BrowserRenderOptions,
}

#[derive(Serialize)]
pub(crate) struct BrowseReport {
    history: BrowserHistorySnapshot,
    active_tab: usize,
    tabs: Vec<BrowserShellTabSummary>,
    cookies: Vec<BrowserCookie>,
    local_storage: Vec<BrowserLocalStorageEntry>,
    session_storage: Vec<BrowserLocalStorageEntry>,
    viewport: BrowserTextViewportReport,
    frame: BrowserRgbaRasterReport,
    focused: Option<BrowserFocusedControl>,
    links: Vec<BrowserShellLink>,
    forms: Vec<BrowserShellForm>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub(crate) struct BrowserShellTabSummary {
    pub(crate) index: usize,
    pub(crate) active: bool,
    pub(crate) history_len: usize,
    pub(crate) current_history_index: Option<usize>,
    pub(crate) title: String,
    pub(crate) source: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub(crate) struct BrowserShellLink {
    pub(crate) index: usize,
    pub(crate) text: String,
    pub(crate) href: String,
    pub(crate) resolved: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub(crate) struct BrowserShellForm {
    pub(crate) index: usize,
    pub(crate) method: String,
    pub(crate) action: String,
    pub(crate) resolved_action: String,
    pub(crate) controls: Vec<BrowserShellFormControl>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub(crate) struct BrowserShellFormControl {
    pub(crate) index: usize,
    pub(crate) name: String,
    pub(crate) kind: String,
    pub(crate) value: String,
    pub(crate) disabled: bool,
    pub(crate) checked: bool,
    pub(crate) options: Vec<BrowserFormOption>,
}

impl BrowserShellTabs {
    pub(crate) fn new(
        session: BrowserSession,
        state: BrowserShellState,
        options: BrowserRenderOptions,
    ) -> Self {
        Self {
            tabs: vec![BrowserShellTab { session, state }],
            active_tab: 0,
            options,
        }
    }

    pub(crate) fn active_index(&self) -> usize {
        self.active_tab
    }

    pub(crate) fn active(&self) -> Result<&BrowserShellTab> {
        self.tabs
            .get(self.active_tab)
            .ok_or_else(|| anyhow!("active tab {} is not open", self.active_tab))
    }

    pub(crate) fn active_parts_mut(
        &mut self,
    ) -> Result<(&mut BrowserSession, &mut BrowserShellState)> {
        let tab = self
            .tabs
            .get_mut(self.active_tab)
            .ok_or_else(|| anyhow!("active tab {} is not open", self.active_tab))?;
        Ok((&mut tab.session, &mut tab.state))
    }

    pub(crate) fn summaries(&self) -> Vec<BrowserShellTabSummary> {
        self.tabs
            .iter()
            .enumerate()
            .map(|(index, tab)| {
                browser_shell_tab_summary(index, index == self.active_tab, &tab.session)
            })
            .collect()
    }

    pub(crate) async fn apply_tab_command(
        &mut self,
        command: &BrowserShellCommand,
    ) -> Result<Option<bool>> {
        match command {
            BrowserShellCommand::Tabs => Ok(Some(true)),
            BrowserShellCommand::NewTab(target) => {
                self.new_tab(target).await?;
                Ok(Some(true))
            }
            BrowserShellCommand::SwitchTab(index) => {
                self.switch_tab(*index)?;
                Ok(Some(true))
            }
            BrowserShellCommand::CloseTab(index) => {
                self.close_tab(*index)?;
                Ok(Some(true))
            }
            _ => Ok(None),
        }
    }

    async fn new_tab(&mut self, target: &str) -> Result<()> {
        let active = self.active()?;
        let target = active.session.resolve_current_target(target);
        let viewport_width = active.state.viewport_width;
        let viewport_height = active.state.viewport_height;
        let cookies = active.session.cookies_snapshot();
        let local_storage = active.session.local_storage_snapshot();
        ensure_static_target(&target)?;

        let mut session = BrowserSession::new_with_state(
            self.options,
            BrowserCookieJar::from_cookies(cookies),
            local_storage,
        );
        session.navigate(&target).await?;
        let mut state = BrowserShellState {
            viewport_x: 0,
            viewport_y: 0,
            viewport_width,
            viewport_height,
        };
        reset_browser_shell_viewport_to_current_location(&session, &mut state)?;
        self.tabs.push(BrowserShellTab { session, state });
        self.active_tab = self.tabs.len() - 1;
        Ok(())
    }

    fn switch_tab(&mut self, index: usize) -> Result<()> {
        if index >= self.tabs.len() {
            return Err(anyhow!(
                "tab index {} not found; {} tab(s) open",
                index,
                self.tabs.len()
            ));
        }
        self.active_tab = index;
        Ok(())
    }

    fn close_tab(&mut self, index: Option<usize>) -> Result<()> {
        if self.tabs.len() == 1 {
            return Err(anyhow!("cannot close the last tab"));
        }
        let index = index.unwrap_or(self.active_tab);
        if index >= self.tabs.len() {
            return Err(anyhow!(
                "tab index {} not found; {} tab(s) open",
                index,
                self.tabs.len()
            ));
        }
        self.tabs.remove(index);
        if self.active_tab > index {
            self.active_tab -= 1;
        } else if self.active_tab >= self.tabs.len() {
            self.active_tab = self.tabs.len().saturating_sub(1);
        }
        Ok(())
    }
}

pub(crate) fn browser_shell_report(
    session: &BrowserSession,
    state: BrowserShellState,
    active_tab: usize,
    tabs: Vec<BrowserShellTabSummary>,
) -> Result<BrowseReport> {
    let viewport = current_browser_shell_viewport(session, state)?;
    let clamped_state = BrowserShellState {
        viewport_x: viewport.x,
        viewport_y: viewport.y,
        ..state
    };
    Ok(BrowseReport {
        history: session.snapshot(),
        active_tab,
        tabs,
        cookies: session.cookies_snapshot(),
        local_storage: session.local_storage_entries(),
        session_storage: session.session_storage_entries(),
        viewport,
        frame: current_browser_shell_frame(session, clamped_state)?,
        focused: session.focused_control(),
        links: browser_shell_links(session),
        forms: browser_shell_forms(session),
    })
}

pub(crate) fn browser_shell_tab_summary(
    index: usize,
    active: bool,
    session: &BrowserSession,
) -> BrowserShellTabSummary {
    let history = session.snapshot();
    let (title, source) = session.current().map_or_else(
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
    BrowserShellTabSummary {
        index,
        active,
        history_len: history.entries.len(),
        current_history_index: history.current_index,
        title,
        source,
    }
}

pub(crate) fn browser_shell_links(session: &BrowserSession) -> Vec<BrowserShellLink> {
    session
        .current_links()
        .iter()
        .enumerate()
        .map(|(index, link)| BrowserShellLink {
            index,
            text: link.text.clone(),
            href: link.href.clone(),
            resolved: link.resolved.clone(),
        })
        .collect()
}

pub(crate) fn browser_shell_forms(session: &BrowserSession) -> Vec<BrowserShellForm> {
    session
        .current_forms()
        .iter()
        .map(browser_shell_form)
        .collect()
}

fn browser_shell_form(form: &BrowserForm) -> BrowserShellForm {
    BrowserShellForm {
        index: form.index,
        method: form.method.clone(),
        action: form.action.clone(),
        resolved_action: form.resolved_action.clone(),
        controls: form
            .controls
            .iter()
            .enumerate()
            .map(|(index, control)| BrowserShellFormControl {
                index,
                name: control.name.clone(),
                kind: control.kind.clone(),
                value: control.value.clone(),
                disabled: control.disabled,
                checked: control.checked,
                options: control.options.clone(),
            })
            .collect(),
    }
}

pub(crate) fn current_browser_shell_viewport(
    session: &BrowserSession,
    state: BrowserShellState,
) -> Result<BrowserTextViewportReport> {
    let Some(render) = session.current() else {
        return Err(anyhow!("browse shell has no current page"));
    };
    Ok(browser_text_viewport(
        render,
        BrowserTextViewportOptions {
            x: state.viewport_x,
            y: state.viewport_y,
            width: state.viewport_width,
            height: state.viewport_height,
        },
    ))
}

pub(crate) fn browser_shell_raster_options(state: BrowserShellState) -> BrowserRasterOptions {
    BrowserRasterOptions {
        viewport_x: Some(state.viewport_x),
        viewport_y: Some(state.viewport_y),
        viewport_width: Some(state.viewport_width),
        viewport_height: Some(state.viewport_height),
        ..BrowserRasterOptions::default()
    }
}

pub(crate) fn current_browser_shell_frame(
    session: &BrowserSession,
    state: BrowserShellState,
) -> Result<BrowserRgbaRasterReport> {
    let Some(render) = session.current() else {
        return Err(anyhow!("browse shell has no current page"));
    };
    let state = clamped_browser_shell_state(session, state)?;
    let options = browser_shell_raster_options(state);
    let raster = rasterize_render_rgba(render, options)?;
    Ok(rgba_raster_report(render, &raster, options))
}

pub(crate) fn write_browser_shell_screenshot(
    session: &BrowserSession,
    state: BrowserShellState,
    path: &Path,
) -> Result<()> {
    let Some(render) = session.current() else {
        return Err(anyhow!(
            "cannot write screenshot: session has no current page"
        ));
    };
    let state = clamped_browser_shell_state(session, state)?;
    let raster = rasterize_render_rgba(render, browser_shell_raster_options(state))?;
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent).with_context(|| {
            format!("failed to create screenshot directory {}", parent.display())
        })?;
    }
    std::fs::write(path, raster.encode_png()?)
        .with_context(|| format!("failed to write screenshot {}", path.display()))?;
    Ok(())
}

pub(crate) fn reset_browser_shell_viewport_to_current_location(
    session: &BrowserSession,
    state: &mut BrowserShellState,
) -> Result<()> {
    state.viewport_x = 0;
    state.viewport_y = session
        .current()
        .and_then(|render| render.source_fragment_scroll_y())
        .unwrap_or(0);
    clamp_browser_shell_viewport(session, state)
}

pub(crate) fn clamp_browser_shell_viewport(
    session: &BrowserSession,
    state: &mut BrowserShellState,
) -> Result<()> {
    let clamped = clamped_browser_shell_state(session, *state)?;
    state.viewport_x = clamped.viewport_x;
    state.viewport_y = clamped.viewport_y;
    Ok(())
}

fn clamped_browser_shell_state(
    session: &BrowserSession,
    state: BrowserShellState,
) -> Result<BrowserShellState> {
    let viewport = current_browser_shell_viewport(session, state)?;
    Ok(BrowserShellState {
        viewport_x: viewport.x,
        viewport_y: viewport.y,
        ..state
    })
}

pub(crate) fn parse_browser_shell_command(input: &str) -> Result<BrowserShellCommand> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Ok(BrowserShellCommand::Render);
    }
    if let Some(query) = trimmed.strip_prefix('/') {
        ensure_non_empty_argument("/", query)?;
        return Ok(BrowserShellCommand::Find {
            query: query.to_owned(),
            next: false,
        });
    }
    let (name, rest) = trimmed
        .split_once(char::is_whitespace)
        .map_or((trimmed, ""), |(name, rest)| (name, rest.trim()));
    match name.to_ascii_lowercase().as_str() {
        "open" | "go" => {
            ensure_non_empty_argument(name, rest)?;
            Ok(BrowserShellCommand::Open(rest.to_owned()))
        }
        "back" => Ok(BrowserShellCommand::Back),
        "forward" => Ok(BrowserShellCommand::Forward),
        "reload" | "refresh" => Ok(BrowserShellCommand::Reload),
        "location" | "url" | "where" => {
            ensure_no_argument(name, rest)?;
            Ok(BrowserShellCommand::Location)
        }
        "cookies" | "cookie" => {
            ensure_no_argument(name, rest)?;
            Ok(BrowserShellCommand::Cookies)
        }
        "local-storage" | "localstorage" | "storage" => {
            ensure_no_argument(name, rest)?;
            Ok(BrowserShellCommand::LocalStorage)
        }
        "session-storage" | "sessionstorage" => {
            ensure_no_argument(name, rest)?;
            Ok(BrowserShellCommand::SessionStorage)
        }
        "clear-cookies" | "clear-cookie-jar" => {
            ensure_no_argument(name, rest)?;
            Ok(BrowserShellCommand::ClearCookies)
        }
        "clear-local-storage" | "clear-storage" => {
            ensure_no_argument(name, rest)?;
            Ok(BrowserShellCommand::ClearLocalStorage)
        }
        "clear-session-storage" => {
            ensure_no_argument(name, rest)?;
            Ok(BrowserShellCommand::ClearSessionStorage)
        }
        "click" => {
            ensure_non_empty_argument(name, rest)?;
            Ok(BrowserShellCommand::Click(rest.to_owned()))
        }
        "click-at" | "tap" => parse_browser_shell_click_at(name, rest),
        "links" => Ok(BrowserShellCommand::Links),
        "forms" | "form" => Ok(BrowserShellCommand::Forms),
        "link" | "follow" | "activate" => Ok(BrowserShellCommand::Link(parse_link_target(rest)?)),
        "focus" => {
            ensure_non_empty_argument(name, rest)?;
            Ok(BrowserShellCommand::Focus(rest.to_owned()))
        }
        "tab" | "focus-next" => {
            ensure_no_argument(name, rest)?;
            Ok(BrowserShellCommand::FocusNext)
        }
        "shift-tab" | "backtab" | "focus-prev" | "focus-previous" => {
            ensure_no_argument(name, rest)?;
            Ok(BrowserShellCommand::FocusPrevious)
        }
        "type" | "type-text" => {
            ensure_non_empty_argument(name, rest)?;
            Ok(BrowserShellCommand::TypeText(rest.to_owned()))
        }
        "backspace" | "delete-backward" => Ok(BrowserShellCommand::DeleteTextBackward(
            parse_unsigned_amount(rest, name, 1)?,
        )),
        "clear-input" | "clear-text" => {
            ensure_no_argument(name, rest)?;
            Ok(BrowserShellCommand::ClearText)
        }
        "enter" | "submit-focused" => {
            ensure_no_argument(name, rest)?;
            Ok(BrowserShellCommand::SubmitFocused)
        }
        "space" | "toggle-focused" => {
            ensure_no_argument(name, rest)?;
            Ok(BrowserShellCommand::ToggleFocused)
        }
        "toggle" | "check" => parse_browser_shell_toggle(name, rest),
        "choose" | "select-focused" => {
            ensure_non_empty_argument(name, rest)?;
            Ok(BrowserShellCommand::SelectFocused(rest.to_owned()))
        }
        "select" | "select-option" => parse_browser_shell_select(name, rest),
        "find" | "search" => {
            ensure_non_empty_argument(name, rest)?;
            Ok(BrowserShellCommand::Find {
                query: rest.to_owned(),
                next: false,
            })
        }
        "find-next" | "next" => {
            ensure_non_empty_argument(name, rest)?;
            Ok(BrowserShellCommand::Find {
                query: rest.to_owned(),
                next: true,
            })
        }
        "fill" | "field" => parse_browser_shell_fill(name, rest),
        "submit" | "submit-form" => {
            parse_browser_shell_submit(BrowserFormSubmitMode::Auto, name, rest)
        }
        "submit-get" | "get-submit" => {
            parse_browser_shell_submit(BrowserFormSubmitMode::Get, name, rest)
        }
        "submit-post" | "post-submit" => {
            parse_browser_shell_submit(BrowserFormSubmitMode::Post, name, rest)
        }
        "styles" | "style" => Ok(BrowserShellCommand::Styles),
        "scripts" | "script" => Ok(BrowserShellCommand::Scripts),
        "images" | "image" => Ok(BrowserShellCommand::Images),
        "resources" | "resource" => Ok(BrowserShellCommand::Resources),
        "tabs" => {
            ensure_no_argument(name, rest)?;
            Ok(BrowserShellCommand::Tabs)
        }
        "new-tab" | "open-tab" | "tab-open" => {
            ensure_non_empty_argument(name, rest)?;
            Ok(BrowserShellCommand::NewTab(rest.to_owned()))
        }
        "switch-tab" | "use-tab" => {
            Ok(BrowserShellCommand::SwitchTab(parse_tab_index(name, rest)?))
        }
        "close-tab" => Ok(BrowserShellCommand::CloseTab(parse_optional_tab_index(
            name, rest,
        )?)),
        "scroll" => Ok(BrowserShellCommand::Scroll(parse_signed_amount(
            rest, "scroll", 1,
        )?)),
        "down" => Ok(BrowserShellCommand::Scroll(parse_signed_amount(
            rest, "down", 23,
        )?)),
        "up" => Ok(BrowserShellCommand::Scroll(-parse_signed_amount(
            rest, "up", 23,
        )?)),
        "right" => Ok(BrowserShellCommand::HorizontalScroll(parse_signed_amount(
            rest, "right", 10,
        )?)),
        "left" => Ok(BrowserShellCommand::HorizontalScroll(-parse_signed_amount(
            rest, "left", 10,
        )?)),
        "top" => Ok(BrowserShellCommand::Top),
        "bottom" => Ok(BrowserShellCommand::Bottom),
        "history" => Ok(BrowserShellCommand::History),
        "render" => {
            ensure_no_argument(name, rest)?;
            Ok(BrowserShellCommand::Render)
        }
        "help" | "?" => Ok(BrowserShellCommand::Help),
        "quit" | "exit" => Ok(BrowserShellCommand::Quit),
        _ => Err(anyhow!("unknown browse command {name:?}")),
    }
}

fn parse_tab_index(command: &str, rest: &str) -> Result<usize> {
    ensure_non_empty_argument(command, rest)?;
    let mut parts = rest.split_whitespace();
    let index = parts
        .next()
        .expect("non-empty tab index argument was checked");
    if parts.next().is_some() {
        return Err(anyhow!("{command} requires exactly one tab index"));
    }
    index
        .parse::<usize>()
        .map_err(|_| anyhow!("{command} requires an unsigned integer tab index"))
}

fn parse_optional_tab_index(command: &str, rest: &str) -> Result<Option<usize>> {
    if rest.trim().is_empty() {
        return Ok(None);
    }
    parse_tab_index(command, rest).map(Some)
}

fn parse_browser_shell_click_at(command: &str, rest: &str) -> Result<BrowserShellCommand> {
    let mut parts = rest.split_whitespace();
    let Some(x) = parts.next() else {
        return Err(anyhow!("{command} requires x and y coordinates"));
    };
    let Some(y) = parts.next() else {
        return Err(anyhow!("{command} requires x and y coordinates"));
    };
    if parts.next().is_some() {
        return Err(anyhow!("{command} requires exactly two coordinates"));
    }
    let x = x
        .parse::<usize>()
        .map_err(|_| anyhow!("{command} requires unsigned integer coordinates"))?;
    let y = y
        .parse::<usize>()
        .map_err(|_| anyhow!("{command} requires unsigned integer coordinates"))?;
    Ok(BrowserShellCommand::ClickAt { x, y })
}

fn parse_browser_shell_fill(command: &str, rest: &str) -> Result<BrowserShellCommand> {
    let mut parts = rest.split_whitespace();
    let Some(index) = parts.next() else {
        return Err(anyhow!("{command} requires a form index and name=value"));
    };
    let Some(field) = parts.next() else {
        return Err(anyhow!("{command} requires a field assignment"));
    };
    if parts.next().is_some() {
        return Err(anyhow!("{command} requires exactly one field assignment"));
    }
    let form_index = index
        .parse::<usize>()
        .map_err(|_| anyhow!("invalid form index {index:?}"))?;
    let (name, value) = parse_field_assignment(field)?;
    Ok(BrowserShellCommand::Fill {
        form_index,
        name,
        value,
    })
}

fn parse_browser_shell_toggle(command: &str, rest: &str) -> Result<BrowserShellCommand> {
    let mut parts = rest.split_whitespace();
    let Some(form_index) = parts.next() else {
        return Err(anyhow!("{command} requires a form index and control index"));
    };
    let Some(control_index) = parts.next() else {
        return Err(anyhow!("{command} requires a control index"));
    };
    if parts.next().is_some() {
        return Err(anyhow!(
            "{command} requires exactly a form index and control index"
        ));
    }
    let form_index = form_index
        .parse::<usize>()
        .map_err(|_| anyhow!("invalid form index {form_index:?}"))?;
    let control_index = control_index
        .parse::<usize>()
        .map_err(|_| anyhow!("invalid control index {control_index:?}"))?;
    Ok(BrowserShellCommand::ToggleControl {
        form_index,
        control_index,
    })
}

fn parse_browser_shell_select(command: &str, rest: &str) -> Result<BrowserShellCommand> {
    let mut parts = rest.splitn(3, char::is_whitespace);
    let Some(form_index) = parts.next().filter(|part| !part.is_empty()) else {
        return Err(anyhow!(
            "{command} requires a form index, control index, and option value"
        ));
    };
    let Some(control_index) = parts.next().filter(|part| !part.is_empty()) else {
        return Err(anyhow!(
            "{command} requires a control index and option value"
        ));
    };
    let Some(value) = parts
        .next()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return Err(anyhow!("{command} requires an option value"));
    };
    let form_index = form_index
        .parse::<usize>()
        .map_err(|_| anyhow!("invalid form index {form_index:?}"))?;
    let control_index = control_index
        .parse::<usize>()
        .map_err(|_| anyhow!("invalid control index {control_index:?}"))?;
    Ok(BrowserShellCommand::SelectControl {
        form_index,
        control_index,
        value: value.to_owned(),
    })
}

fn parse_link_target(input: &str) -> Result<BrowserShellLinkTarget> {
    let input = input.trim();
    if input.is_empty() {
        return Err(anyhow!("link requires an index"));
    }
    if let Some(text) = input.strip_prefix("text ") {
        ensure_non_empty_argument("link text", text.trim())?;
        return Ok(BrowserShellLinkTarget::Text(text.trim().to_owned()));
    }
    if let Some(selector) = input.strip_prefix("selector ") {
        ensure_non_empty_argument("link selector", selector.trim())?;
        return Ok(BrowserShellLinkTarget::Selector(selector.trim().to_owned()));
    }
    input
        .parse::<usize>()
        .map(BrowserShellLinkTarget::Index)
        .map_err(|_| anyhow!("link requires an index, `text <label>`, or `selector <selector>`"))
}

fn parse_browser_shell_submit(
    mode: BrowserFormSubmitMode,
    command: &str,
    rest: &str,
) -> Result<BrowserShellCommand> {
    let mut parts = rest.split_whitespace();
    let Some(index) = parts.next() else {
        return Err(anyhow!("{command} requires a form index"));
    };
    let form_index = index
        .parse::<usize>()
        .map_err(|_| anyhow!("invalid form index {index:?}"))?;
    let fields = parts
        .map(parse_field_assignment)
        .collect::<Result<Vec<_>>>()?;
    Ok(BrowserShellCommand::Submit {
        mode,
        form_index,
        fields,
    })
}

fn parse_field_assignment(input: &str) -> Result<(String, String)> {
    let Some((name, value)) = input.split_once('=') else {
        return Err(anyhow!("field override {input:?} must use name=value"));
    };
    if name.is_empty() {
        return Err(anyhow!("field override must include a name"));
    }
    Ok((name.to_owned(), value.to_owned()))
}

fn parse_signed_amount(input: &str, command: &str, default: isize) -> Result<isize> {
    if input.is_empty() {
        return Ok(default);
    }
    input
        .parse::<isize>()
        .map_err(|_| anyhow!("{command} requires a signed integer amount"))
}

fn parse_unsigned_amount(input: &str, command: &str, default: usize) -> Result<usize> {
    if input.is_empty() {
        return Ok(default);
    }
    input
        .parse::<usize>()
        .map_err(|_| anyhow!("{command} requires an unsigned integer amount"))
}

fn ensure_non_empty_argument(command: &str, value: &str) -> Result<()> {
    if value.is_empty() {
        Err(anyhow!("{command} requires an argument"))
    } else {
        Ok(())
    }
}

fn ensure_no_argument(command: &str, value: &str) -> Result<()> {
    if value.is_empty() {
        Ok(())
    } else {
        Err(anyhow!("{command} does not take an argument"))
    }
}
