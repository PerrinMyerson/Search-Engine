use std::collections::HashMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;
use url::form_urlencoded;

use crate::browser::{
    BrowserCookie, BrowserFocusedControl, BrowserImageRenderReport, BrowserLocalStorageEntry,
    BrowserRenderOptions, BrowserResource, BrowserResourceFetch, BrowserResourceFetchReport,
    BrowserScriptRenderReport, BrowserSession, BrowserStylesheetRenderReport,
    BrowserTextViewportOptions, browser_text_viewport,
};

use super::{
    HttpResponse, RequestTarget, ServerState, html_response, json_response,
    sanitized_search_return_href, text_response,
};

const DEFAULT_BROWSER_WIDTH: usize = 100;
const DEFAULT_BROWSER_HEIGHT: usize = 44;
const DEFAULT_BROWSER_MAX_BYTES: usize = 4 * 1024 * 1024;
const MAX_CLOSED_BROWSER_SESSIONS: usize = 12;
const MAX_BROWSER_PROFILE_HISTORY: usize = 200;
const BROWSER_PROFILE_ENV: &str = "BRUTAL_BROWSER_PROFILE";

pub(super) struct BrowserSessionRegistry {
    next_id: AtomicU64,
    next_bookmark_id: AtomicU64,
    profile_path: Option<PathBuf>,
    profile_error: Mutex<Option<String>>,
    sessions: Mutex<HashMap<String, BrowserWebSession>>,
    closed_sessions: Mutex<Vec<BrowserClosedSession>>,
    bookmarks: Mutex<Vec<BrowserStoredBookmark>>,
    profile_tabs: Mutex<Vec<BrowserStoredProfileTab>>,
    profile_history: Mutex<Vec<BrowserStoredProfileEntry>>,
    profile_closed_sessions: Mutex<Vec<BrowserStoredClosedSession>>,
}

impl Default for BrowserSessionRegistry {
    fn default() -> Self {
        Self::new_in_memory()
    }
}

impl BrowserSessionRegistry {
    fn new_in_memory() -> Self {
        Self {
            next_id: AtomicU64::new(1),
            next_bookmark_id: AtomicU64::new(1),
            profile_path: None,
            profile_error: Mutex::new(None),
            sessions: Mutex::new(HashMap::new()),
            closed_sessions: Mutex::new(Vec::new()),
            bookmarks: Mutex::new(Vec::new()),
            profile_tabs: Mutex::new(Vec::new()),
            profile_history: Mutex::new(Vec::new()),
            profile_closed_sessions: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn from_env() -> Self {
        std::env::var_os(BROWSER_PROFILE_ENV)
            .filter(|path| !path.is_empty())
            .map(PathBuf::from)
            .map_or_else(Self::new_in_memory, Self::with_profile_path)
    }

    fn with_profile_path(path: PathBuf) -> Self {
        let (profile, profile_error) = match load_browser_session_profile(&path) {
            Ok(profile) => (profile, None),
            Err(error) => (BrowserSessionProfileFile::default(), Some(error)),
        };
        let bookmarks = profile
            .bookmarks
            .into_iter()
            .filter_map(browser_stored_bookmark_from_profile_entry)
            .collect::<Vec<_>>();
        let profile_tabs = profile
            .tabs
            .into_iter()
            .filter_map(browser_stored_profile_tab_from_file)
            .collect::<Vec<_>>();
        let profile_history = profile
            .history
            .into_iter()
            .filter_map(browser_stored_profile_entry_from_file)
            .collect::<Vec<_>>();
        let profile_closed_sessions = profile
            .closed
            .into_iter()
            .filter_map(browser_stored_closed_session_from_file)
            .take(MAX_CLOSED_BROWSER_SESSIONS)
            .collect::<Vec<_>>();
        let next_bookmark_id = bookmarks
            .iter()
            .filter_map(|bookmark| browser_profile_id_number(&bookmark.id, 'b'))
            .max()
            .unwrap_or(0)
            .saturating_add(1)
            .max(1);
        Self {
            next_id: AtomicU64::new(1),
            next_bookmark_id: AtomicU64::new(next_bookmark_id),
            profile_path: Some(path),
            profile_error: Mutex::new(profile_error),
            sessions: Mutex::new(HashMap::new()),
            closed_sessions: Mutex::new(Vec::new()),
            bookmarks: Mutex::new(bookmarks),
            profile_tabs: Mutex::new(profile_tabs),
            profile_history: Mutex::new(profile_history),
            profile_closed_sessions: Mutex::new(profile_closed_sessions),
        }
    }
}

#[derive(Debug, Clone)]
struct BrowserWebSession {
    session: BrowserSession,
    width: usize,
    height: usize,
    max_bytes: usize,
    viewport_x: usize,
    viewport_y: usize,
    back_href: String,
    find_query: String,
    find_active_line: Option<usize>,
    resource_report: Option<BrowserSessionResourceReportPayload>,
}

#[derive(Debug, Clone, Serialize)]
struct BrowserSessionPayload {
    id: String,
    back_href: String,
    title: String,
    source: String,
    width: usize,
    height: usize,
    max_bytes: usize,
    viewport_x: usize,
    viewport_y: usize,
    document_width: usize,
    document_height: usize,
    max_scroll_x: usize,
    max_scroll_y: usize,
    dom_node_count: usize,
    link_count: usize,
    can_back: bool,
    can_forward: bool,
    history_len: usize,
    current_history_index: Option<usize>,
    profile_enabled: bool,
    profile_error: Option<String>,
    current_bookmarked: bool,
    bookmarks_clear_url: Option<String>,
    closed_sessions_clear_url: Option<String>,
    profile_tabs_clear_url: Option<String>,
    profile_history_clear_url: Option<String>,
    find_query: String,
    find_match_count: usize,
    find_current_index: Option<usize>,
    find_current_line: Option<usize>,
    sessions: Vec<BrowserSessionSummaryPayload>,
    closed_sessions: Vec<BrowserClosedSessionPayload>,
    bookmarks: Vec<BrowserSessionBookmarkPayload>,
    profile_history: Vec<BrowserSessionProfileEntryPayload>,
    history: Vec<BrowserSessionHistoryEntryPayload>,
    viewport: String,
    focused: Option<BrowserFocusedControl>,
    links: Vec<BrowserSessionLinkPayload>,
    form_count: usize,
    forms: Vec<BrowserSessionFormPayload>,
    cookies: Vec<BrowserCookie>,
    local_storage: Vec<BrowserLocalStorageEntry>,
    session_storage: Vec<BrowserLocalStorageEntry>,
    resource_count: usize,
    resources: Vec<BrowserResource>,
    resource_report: Option<BrowserSessionResourceReportPayload>,
}

#[derive(Debug, Clone, Serialize)]
struct BrowserSessionSummaryPayload {
    id: String,
    title: String,
    source: String,
    action_url: String,
    reload_url: String,
    duplicate_url: String,
    close_url: String,
    current: bool,
    can_close: bool,
}

#[derive(Debug)]
struct BrowserClosedSession {
    id: String,
    title: String,
    source: String,
    closed_at_unix_secs: u64,
    session: BrowserWebSession,
}

#[derive(Debug, Clone, Serialize)]
struct BrowserClosedSessionPayload {
    id: String,
    title: String,
    source: String,
    closed_at_unix_secs: u64,
    closed_at: String,
    persisted: bool,
    restore_url: String,
    new_session_url: String,
    forget_url: String,
}

#[derive(Debug, Clone)]
struct BrowserStoredBookmark {
    id: String,
    title: String,
    source: String,
}

#[derive(Debug, Clone)]
struct BrowserStoredProfileTab {
    title: String,
    source: String,
    active: bool,
    updated_at_unix_secs: u64,
}

#[derive(Debug, Clone, Serialize)]
struct BrowserSessionBookmarkPayload {
    id: String,
    title: String,
    source: String,
    action_url: String,
    new_session_url: String,
    remove_url: String,
    current: bool,
}

#[derive(Debug, Clone, Serialize)]
struct BrowserSessionProfileEntryPayload {
    index: usize,
    title: String,
    source: String,
    visited_at_unix_secs: u64,
    visited_at: String,
    action_url: String,
    new_session_url: String,
    remove_url: String,
}

#[derive(Debug, Clone)]
struct BrowserStoredProfileEntry {
    title: String,
    source: String,
    visited_at_unix_secs: u64,
}

#[derive(Debug, Clone)]
struct BrowserStoredClosedSession {
    title: String,
    source: String,
    closed_at_unix_secs: u64,
}

#[derive(Debug, Default, Deserialize, Serialize)]
struct BrowserSessionProfileFile {
    #[serde(default)]
    version: u32,
    #[serde(default)]
    bookmarks: Vec<BrowserSessionProfileBookmarkFile>,
    #[serde(default)]
    tabs: Vec<BrowserSessionProfileTabFile>,
    #[serde(default)]
    history: Vec<BrowserSessionProfileEntryFile>,
    #[serde(default)]
    closed: Vec<BrowserSessionProfileClosedFile>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct BrowserSessionProfileBookmarkFile {
    id: String,
    title: String,
    source: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct BrowserSessionProfileTabFile {
    title: String,
    source: String,
    #[serde(default)]
    active: bool,
    #[serde(default)]
    updated_at_unix_secs: u64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct BrowserSessionProfileEntryFile {
    title: String,
    source: String,
    #[serde(default)]
    visited_at_unix_secs: u64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct BrowserSessionProfileClosedFile {
    title: String,
    source: String,
    #[serde(default)]
    closed_at_unix_secs: u64,
}

#[derive(Debug, Clone, Serialize)]
struct BrowserSessionHistoryEntryPayload {
    index: usize,
    title: String,
    source: String,
    target: String,
    action_url: String,
    new_session_url: String,
    current: bool,
}

#[derive(Debug, Clone, Serialize)]
struct BrowserSessionLinkPayload {
    index: usize,
    label: String,
    url: String,
    action_url: String,
    new_session_url: String,
}

#[derive(Debug, Clone, Serialize)]
struct BrowserSessionFormPayload {
    index: usize,
    method: String,
    action: String,
    resolved_action: String,
    no_validate: bool,
    controls: Vec<BrowserSessionFormControlPayload>,
    submit_url: String,
}

#[derive(Debug, Clone, Serialize)]
struct BrowserSessionFormControlPayload {
    index: usize,
    name: String,
    kind: String,
    value: String,
    disabled: bool,
    required: bool,
    checked: bool,
    options: Vec<BrowserSessionFormOptionPayload>,
    toggle_url: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
struct BrowserSessionFormOptionPayload {
    value: String,
    label: String,
    disabled: bool,
    selected: bool,
}

#[derive(Debug, Clone, Serialize)]
struct BrowserSessionResourceReportPayload {
    action: String,
    page_source: String,
    total: usize,
    fetched: usize,
    cached: usize,
    failed: usize,
    skipped: usize,
    applied: Option<usize>,
    decoded: Option<usize>,
    resources: Vec<BrowserSessionResourceFetchPayload>,
}

#[derive(Debug, Clone, Serialize)]
struct BrowserSessionResourceFetchPayload {
    kind: String,
    url: String,
    resolved: String,
    status: String,
    source: Option<String>,
    bytes: usize,
    content_type: Option<String>,
    error: Option<String>,
}

#[derive(Debug)]
enum BrowserSessionAction {
    Current,
    Open(String),
    OpenNewSession(String),
    Back,
    Forward,
    Reload,
    Link(usize),
    LinkText(String),
    LinkSelector(String),
    LinkTextNewSession(String),
    LinkSelectorNewSession(String),
    History(usize),
    Find(String),
    FindNext,
    FindPrevious,
    ClearFind,
    ClickSelector(String),
    ClickAt {
        x: usize,
        y: usize,
    },
    FocusSelector(String),
    FocusNext,
    FocusPrevious,
    TypeText(String),
    Backspace(usize),
    ClearInput,
    Enter,
    Space,
    Choose(String),
    ClearCookies,
    ClearLocalStorage,
    ClearSessionStorage,
    AddBookmark,
    OpenBookmark(String),
    RemoveBookmark(String),
    ClearBookmarks,
    OpenProfileClosed(usize),
    RemoveProfileHistory(usize),
    ClearClosedSessions,
    ClearProfileTabs,
    ClearProfileHistory,
    RestoreClosedSession(String),
    ForgetClosedSession(String),
    ForgetProfileClosed(usize),
    FetchResources,
    ApplyStylesheets,
    RunScripts,
    LoadImages,
    ClearResourceReport,
    DuplicateSession(String),
    CloseSession(String),
    CloseOtherSessions,
    CloseSessionsToRight,
    CloseSessionsToLeft,
    CloseDuplicateSessions,
    SwitchNextSession,
    SwitchPreviousSession,
    Scroll {
        dx: isize,
        dy: isize,
    },
    Top,
    Bottom,
    Fill {
        form_index: usize,
        name: String,
        value: String,
    },
    Select {
        form_index: usize,
        control_index: usize,
        value: String,
    },
    Toggle {
        form_index: usize,
        control_index: usize,
    },
    Submit {
        form_index: usize,
    },
}

#[derive(Debug)]
enum BrowserRouteError {
    BadRequest(String),
    NotFound(String),
    Upstream(String),
}

impl BrowserRouteError {
    fn response(&self) -> HttpResponse {
        match self {
            Self::BadRequest(message) => text_response(400, "Bad Request", message),
            Self::NotFound(message) => text_response(404, "Not Found", message),
            Self::Upstream(message) => text_response(502, "Bad Gateway", message),
        }
    }
}

pub(super) async fn browser_page(target: &RequestTarget, state: &ServerState) -> HttpResponse {
    match browser_session_for_target(target, state).await {
        Ok((payload, back_href)) => {
            html_response(render_browser_session_page(&payload, &back_href))
        }
        Err(error) => error.response(),
    }
}

pub(super) async fn api_browser_session(
    target: &RequestTarget,
    state: &ServerState,
) -> HttpResponse {
    match browser_session_for_target(target, state).await {
        Ok((payload, _)) => json_response(200, "OK", &payload),
        Err(error) => error.response(),
    }
}

async fn browser_session_for_target(
    target: &RequestTarget,
    state: &ServerState,
) -> Result<(BrowserSessionPayload, String), BrowserRouteError> {
    if target.param("id").is_some() {
        state.browser_sessions.apply_target(target).await
    } else {
        state.browser_sessions.create_target(target).await
    }
}

impl BrowserSessionRegistry {
    async fn create_target(
        &self,
        target: &RequestTarget,
    ) -> Result<(BrowserSessionPayload, String), BrowserRouteError> {
        let target_url = target
            .param("url")
            .or_else(|| target.param("target"))
            .unwrap_or_default();
        if target_url.trim().is_empty() {
            return self.create_profile_tabs_target(target).await;
        }

        let width = parse_usize_param(target, "width", DEFAULT_BROWSER_WIDTH, 40, 160);
        let height = parse_usize_param(target, "height", DEFAULT_BROWSER_HEIGHT, 16, 120);
        let max_bytes = parse_usize_param(
            target,
            "max_bytes",
            DEFAULT_BROWSER_MAX_BYTES,
            64 * 1024,
            16 * 1024 * 1024,
        );
        let back_href = sanitized_search_return_href(target.param("from").as_deref());
        let mut session = BrowserSession::new(BrowserRenderOptions { width, max_bytes });
        session.navigate(&target_url).await.map_err(|error| {
            BrowserRouteError::Upstream(format!(
                "browser render failed for {target_url}: {error:#}"
            ))
        })?;

        let id = self.next_session_id();
        let mut web_session = BrowserWebSession {
            session,
            width,
            height,
            max_bytes,
            viewport_x: parse_optional_usize_param(target, "x", 0, usize::MAX)
                .or_else(|| parse_optional_usize_param(target, "viewport_x", 0, usize::MAX))
                .unwrap_or(0),
            viewport_y: 0,
            back_href,
            find_query: String::new(),
            find_active_line: None,
            resource_report: None,
        };
        reset_viewport_to_fragment(&mut web_session);
        let mut payload = browser_session_payload(&id, &mut web_session)?;
        let back_href = web_session.back_href.clone();
        self.record_browser_profile_visit(&payload).await;
        let mut sessions = self.sessions.lock().await;
        sessions.insert(id.clone(), web_session);
        self.record_browser_profile_tabs(&sessions, &id).await;
        let closed_sessions = self.closed_sessions.lock().await;
        let bookmarks = self.bookmarks.lock().await;
        self.attach_browser_session_registry_state(
            &mut payload,
            &sessions,
            &closed_sessions,
            &bookmarks,
            &id,
        )
        .await;
        Ok((payload, back_href))
    }

    async fn create_profile_tabs_target(
        &self,
        target: &RequestTarget,
    ) -> Result<(BrowserSessionPayload, String), BrowserRouteError> {
        let profile_tabs = self.profile_tabs.lock().await.clone();
        if profile_tabs.is_empty() {
            return Err(BrowserRouteError::BadRequest(
                "missing browser URL".to_owned(),
            ));
        }

        let width = parse_usize_param(target, "width", DEFAULT_BROWSER_WIDTH, 40, 160);
        let height = parse_usize_param(target, "height", DEFAULT_BROWSER_HEIGHT, 16, 120);
        let max_bytes = parse_usize_param(
            target,
            "max_bytes",
            DEFAULT_BROWSER_MAX_BYTES,
            64 * 1024,
            16 * 1024 * 1024,
        );
        let back_href = sanitized_search_return_href(target.param("from").as_deref());
        let active_index = profile_tabs.iter().position(|tab| tab.active).unwrap_or(0);
        let mut restored_sessions = Vec::new();
        for tab in &profile_tabs {
            let mut session = BrowserSession::new(BrowserRenderOptions { width, max_bytes });
            session.navigate(&tab.source).await.map_err(|error| {
                BrowserRouteError::Upstream(format!(
                    "browser profile tab restore failed for {}: {error:#}",
                    tab.source
                ))
            })?;
            let mut web_session = BrowserWebSession {
                session,
                width,
                height,
                max_bytes,
                viewport_x: parse_optional_usize_param(target, "x", 0, usize::MAX)
                    .or_else(|| parse_optional_usize_param(target, "viewport_x", 0, usize::MAX))
                    .unwrap_or(0),
                viewport_y: 0,
                back_href: back_href.clone(),
                find_query: String::new(),
                find_active_line: None,
                resource_report: None,
            };
            reset_viewport_to_fragment(&mut web_session);
            restored_sessions.push(web_session);
        }

        let mut active_id = String::new();
        let mut sessions = self.sessions.lock().await;
        for (index, web_session) in restored_sessions.into_iter().enumerate() {
            let id = self.next_session_id();
            if index == active_index {
                active_id = id.clone();
            }
            sessions.insert(id, web_session);
        }
        if active_id.is_empty() {
            active_id = browser_sorted_session_ids(&sessions)
                .into_iter()
                .next()
                .ok_or_else(|| {
                    BrowserRouteError::NotFound("no browser tabs restored".to_owned())
                })?;
        }
        self.record_browser_profile_tabs(&sessions, &active_id)
            .await;
        let (mut payload, back_href) = {
            let active = sessions.get_mut(&active_id).ok_or_else(|| {
                BrowserRouteError::NotFound(format!("browser session {active_id} not found"))
            })?;
            let payload = browser_session_payload(&active_id, active)?;
            (payload, active.back_href.clone())
        };
        let closed_sessions = self.closed_sessions.lock().await;
        let bookmarks = self.bookmarks.lock().await;
        self.attach_browser_session_registry_state(
            &mut payload,
            &sessions,
            &closed_sessions,
            &bookmarks,
            &active_id,
        )
        .await;
        Ok((payload, back_href))
    }

    async fn apply_target(
        &self,
        target: &RequestTarget,
    ) -> Result<(BrowserSessionPayload, String), BrowserRouteError> {
        let id = target.param("id").unwrap_or_default();
        if id.trim().is_empty() {
            return Err(BrowserRouteError::BadRequest(
                "missing browser session id".to_owned(),
            ));
        }
        let action = browser_action(target)?;
        let should_record_profile_visit = browser_action_records_profile_visit(&action);
        let should_record_profile_tabs = browser_action_records_profile_tabs(&action);

        let mut web_session = self.sessions.lock().await.remove(&id).ok_or_else(|| {
            BrowserRouteError::NotFound(format!("browser session {id} not found"))
        })?;

        web_session.width =
            parse_optional_usize_param(target, "width", 40, 160).unwrap_or(web_session.width);
        web_session.height =
            parse_optional_usize_param(target, "height", 16, 120).unwrap_or(web_session.height);
        web_session.max_bytes =
            parse_optional_usize_param(target, "max_bytes", 64 * 1024, 16 * 1024 * 1024)
                .unwrap_or(web_session.max_bytes);
        web_session.viewport_x = parse_optional_usize_param(target, "x", 0, usize::MAX)
            .or_else(|| parse_optional_usize_param(target, "viewport_x", 0, usize::MAX))
            .unwrap_or(web_session.viewport_x);
        if let Some(return_href) = target.param("from") {
            web_session.back_href = sanitized_search_return_href(Some(&return_href));
        }

        if let BrowserSessionAction::CloseSession(close_id) = action {
            self.sessions.lock().await.insert(id.clone(), web_session);
            return self.close_target(target, &id, &close_id).await;
        }
        if let BrowserSessionAction::RestoreClosedSession(closed_id) = action {
            self.sessions.lock().await.insert(id.clone(), web_session);
            return self.restore_closed_target(target, &id, &closed_id).await;
        }
        if let BrowserSessionAction::SwitchNextSession = action {
            self.sessions.lock().await.insert(id.clone(), web_session);
            return self
                .switch_relative_session_target(&id, BrowserSessionSwitchDirection::Next)
                .await;
        }
        if let BrowserSessionAction::SwitchPreviousSession = action {
            self.sessions.lock().await.insert(id.clone(), web_session);
            return self
                .switch_relative_session_target(&id, BrowserSessionSwitchDirection::Previous)
                .await;
        }
        if let BrowserSessionAction::OpenNewSession(url) = action {
            return self
                .open_browser_action_in_new_session_target(
                    &id,
                    web_session,
                    BrowserSessionAction::Open(url),
                )
                .await;
        }
        if let BrowserSessionAction::LinkTextNewSession(text) = action {
            return self
                .open_browser_action_in_new_session_target(
                    &id,
                    web_session,
                    BrowserSessionAction::LinkText(text),
                )
                .await;
        }
        if let BrowserSessionAction::LinkSelectorNewSession(selector) = action {
            return self
                .open_browser_action_in_new_session_target(
                    &id,
                    web_session,
                    BrowserSessionAction::LinkSelector(selector),
                )
                .await;
        }
        if let BrowserSessionAction::DuplicateSession(duplicate_id) = action {
            self.sessions.lock().await.insert(id.clone(), web_session);
            return self.duplicate_session_target(&duplicate_id).await;
        }
        if let BrowserSessionAction::CloseOtherSessions = action {
            return self
                .close_scoped_sessions_target(
                    target,
                    &id,
                    web_session,
                    BrowserSessionCloseScope::Others,
                )
                .await;
        }
        if let BrowserSessionAction::CloseSessionsToRight = action {
            return self
                .close_scoped_sessions_target(
                    target,
                    &id,
                    web_session,
                    BrowserSessionCloseScope::RightOfActive,
                )
                .await;
        }
        if let BrowserSessionAction::CloseSessionsToLeft = action {
            return self
                .close_scoped_sessions_target(
                    target,
                    &id,
                    web_session,
                    BrowserSessionCloseScope::LeftOfActive,
                )
                .await;
        }
        if let BrowserSessionAction::CloseDuplicateSessions = action {
            let active_source = current_session_source(&web_session).unwrap_or_default();
            return self
                .close_scoped_sessions_target(
                    target,
                    &id,
                    web_session,
                    BrowserSessionCloseScope::DuplicateSource(active_source),
                )
                .await;
        }

        let result = match action {
            BrowserSessionAction::AddBookmark => self
                .add_current_bookmark(&web_session)
                .await
                .and_then(|_| browser_session_payload(&id, &mut web_session)),
            BrowserSessionAction::RemoveBookmark(bookmark_id) => self
                .remove_bookmark(&bookmark_id)
                .await
                .and_then(|_| browser_session_payload(&id, &mut web_session)),
            BrowserSessionAction::ClearBookmarks => self
                .clear_bookmarks()
                .await
                .and_then(|_| browser_session_payload(&id, &mut web_session)),
            BrowserSessionAction::OpenProfileClosed(index) => {
                match self.profile_closed_source(index).await {
                    Ok(source) => match apply_browser_action(
                        BrowserSessionAction::Open(source),
                        &mut web_session,
                    )
                    .await
                    {
                        Ok(()) => self
                            .remove_profile_closed(index)
                            .await
                            .and_then(|_| browser_session_payload(&id, &mut web_session)),
                        Err(error) => Err(error),
                    },
                    Err(error) => Err(error),
                }
            }
            BrowserSessionAction::RemoveProfileHistory(index) => self
                .remove_profile_history(index)
                .await
                .and_then(|_| browser_session_payload(&id, &mut web_session)),
            BrowserSessionAction::ClearClosedSessions => self
                .clear_closed_sessions()
                .await
                .and_then(|_| browser_session_payload(&id, &mut web_session)),
            BrowserSessionAction::ClearProfileTabs => self
                .clear_profile_tabs()
                .await
                .and_then(|_| browser_session_payload(&id, &mut web_session)),
            BrowserSessionAction::ClearProfileHistory => self
                .clear_profile_history()
                .await
                .and_then(|_| browser_session_payload(&id, &mut web_session)),
            BrowserSessionAction::ForgetClosedSession(closed_id) => self
                .forget_closed_session(&closed_id)
                .await
                .and_then(|_| browser_session_payload(&id, &mut web_session)),
            BrowserSessionAction::ForgetProfileClosed(index) => self
                .remove_profile_closed(index)
                .await
                .and_then(|_| browser_session_payload(&id, &mut web_session)),
            BrowserSessionAction::OpenBookmark(bookmark_id) => {
                match self.bookmark_source(&bookmark_id).await {
                    Ok(source) => {
                        apply_browser_action(BrowserSessionAction::Open(source), &mut web_session)
                            .await
                            .and_then(|_| browser_session_payload(&id, &mut web_session))
                    }
                    Err(error) => Err(error),
                }
            }
            action => apply_browser_action(action, &mut web_session)
                .await
                .and_then(|_| browser_session_payload(&id, &mut web_session)),
        };
        let mut payload = match result {
            Ok(payload) => payload,
            Err(error) => {
                self.sessions.lock().await.insert(id.clone(), web_session);
                return Err(error);
            }
        };
        let back_href = web_session.back_href.clone();
        if should_record_profile_visit {
            self.record_browser_profile_visit(&payload).await;
        }
        let mut sessions = self.sessions.lock().await;
        sessions.insert(id.clone(), web_session);
        if should_record_profile_tabs {
            self.record_browser_profile_tabs(&sessions, &id).await;
        }
        let closed_sessions = self.closed_sessions.lock().await;
        let bookmarks = self.bookmarks.lock().await;
        self.attach_browser_session_registry_state(
            &mut payload,
            &sessions,
            &closed_sessions,
            &bookmarks,
            &id,
        )
        .await;
        Ok((payload, back_href))
    }

    async fn close_scoped_sessions_target(
        &self,
        _target: &RequestTarget,
        active_id: &str,
        mut active_session: BrowserWebSession,
        scope: BrowserSessionCloseScope,
    ) -> Result<(BrowserSessionPayload, String), BrowserRouteError> {
        let mut sessions = self.sessions.lock().await;
        let active_number = browser_session_id_number(active_id);
        let closed_ids = browser_sorted_session_ids(&sessions)
            .into_iter()
            .filter(|id| match &scope {
                BrowserSessionCloseScope::Others => true,
                BrowserSessionCloseScope::LeftOfActive => {
                    browser_session_id_number(id) < active_number
                }
                BrowserSessionCloseScope::RightOfActive => {
                    browser_session_id_number(id) > active_number
                }
                BrowserSessionCloseScope::DuplicateSource(active_source) => sessions
                    .get(id)
                    .and_then(|session| session.session.current())
                    .is_some_and(|render| render.source.as_str() == active_source.as_str()),
            })
            .collect::<Vec<_>>();
        let mut closed_sessions = self.closed_sessions.lock().await;
        for closed_id in closed_ids {
            let Some(closed_session) = sessions.remove(&closed_id) else {
                continue;
            };
            let profile_closed_session =
                browser_stored_closed_session_from_web_session(&closed_session);
            remember_closed_browser_session(&mut closed_sessions, &closed_id, closed_session);
            self.record_browser_profile_closed_session(profile_closed_session)
                .await;
        }

        let back_href = active_session.back_href.clone();
        let mut payload = browser_session_payload(active_id, &mut active_session)?;
        sessions.insert(active_id.to_owned(), active_session);
        self.record_browser_profile_tabs(&sessions, active_id).await;
        let bookmarks = self.bookmarks.lock().await;
        self.attach_browser_session_registry_state(
            &mut payload,
            &sessions,
            &closed_sessions,
            &bookmarks,
            active_id,
        )
        .await;
        Ok((payload, back_href))
    }

    async fn switch_relative_session_target(
        &self,
        active_id: &str,
        direction: BrowserSessionSwitchDirection,
    ) -> Result<(BrowserSessionPayload, String), BrowserRouteError> {
        let mut sessions = self.sessions.lock().await;
        let ordered_ids = browser_sorted_session_ids(&sessions);
        let current_index = ordered_ids
            .iter()
            .position(|id| id == active_id)
            .ok_or_else(|| {
                BrowserRouteError::NotFound(format!("browser session {active_id} not found"))
            })?;
        let target_index = if ordered_ids.len() <= 1 {
            current_index
        } else {
            match direction {
                BrowserSessionSwitchDirection::Next => (current_index + 1) % ordered_ids.len(),
                BrowserSessionSwitchDirection::Previous => {
                    if current_index == 0 {
                        ordered_ids.len() - 1
                    } else {
                        current_index - 1
                    }
                }
            }
        };
        let selected_id = ordered_ids[target_index].clone();
        let (mut payload, back_href) = {
            let selected = sessions.get_mut(&selected_id).ok_or_else(|| {
                BrowserRouteError::NotFound(format!("browser session {selected_id} not found"))
            })?;
            let payload = browser_session_payload(&selected_id, selected)?;
            (payload, selected.back_href.clone())
        };
        self.record_browser_profile_tabs(&sessions, &selected_id)
            .await;
        let closed_sessions = self.closed_sessions.lock().await;
        let bookmarks = self.bookmarks.lock().await;
        self.attach_browser_session_registry_state(
            &mut payload,
            &sessions,
            &closed_sessions,
            &bookmarks,
            &selected_id,
        )
        .await;
        Ok((payload, back_href))
    }

    async fn open_browser_action_in_new_session_target(
        &self,
        active_id: &str,
        active_session: BrowserWebSession,
        browser_action: BrowserSessionAction,
    ) -> Result<(BrowserSessionPayload, String), BrowserRouteError> {
        let mut new_session = active_session.clone();
        {
            let mut sessions = self.sessions.lock().await;
            sessions.insert(active_id.to_owned(), active_session);
        }

        apply_browser_action(browser_action, &mut new_session).await?;

        let new_id = self.next_session_id();
        let back_href = new_session.back_href.clone();
        let mut payload = browser_session_payload(&new_id, &mut new_session)?;
        self.record_browser_profile_visit(&payload).await;
        let mut sessions = self.sessions.lock().await;
        sessions.insert(new_id.clone(), new_session);
        self.record_browser_profile_tabs(&sessions, &new_id).await;
        let closed_sessions = self.closed_sessions.lock().await;
        let bookmarks = self.bookmarks.lock().await;
        self.attach_browser_session_registry_state(
            &mut payload,
            &sessions,
            &closed_sessions,
            &bookmarks,
            &new_id,
        )
        .await;
        Ok((payload, back_href))
    }

    async fn duplicate_session_target(
        &self,
        duplicate_id: &str,
    ) -> Result<(BrowserSessionPayload, String), BrowserRouteError> {
        let mut sessions = self.sessions.lock().await;
        let mut duplicated = sessions.get(duplicate_id).cloned().ok_or_else(|| {
            BrowserRouteError::NotFound(format!("browser session {duplicate_id} not found"))
        })?;
        let new_id = self.next_session_id();
        let back_href = duplicated.back_href.clone();
        let mut payload = browser_session_payload(&new_id, &mut duplicated)?;
        self.record_browser_profile_visit(&payload).await;
        sessions.insert(new_id.clone(), duplicated);
        self.record_browser_profile_tabs(&sessions, &new_id).await;
        let closed_sessions = self.closed_sessions.lock().await;
        let bookmarks = self.bookmarks.lock().await;
        self.attach_browser_session_registry_state(
            &mut payload,
            &sessions,
            &closed_sessions,
            &bookmarks,
            &new_id,
        )
        .await;
        Ok((payload, back_href))
    }

    fn next_session_id(&self) -> String {
        let next = self.next_id.fetch_add(1, AtomicOrdering::Relaxed);
        format!("s{next}")
    }

    async fn close_target(
        &self,
        target: &RequestTarget,
        active_id: &str,
        close_id: &str,
    ) -> Result<(BrowserSessionPayload, String), BrowserRouteError> {
        let mut sessions = self.sessions.lock().await;
        if !sessions.contains_key(close_id) {
            return Err(BrowserRouteError::NotFound(format!(
                "browser session {close_id} not found"
            )));
        }
        if sessions.len() <= 1 {
            return Err(BrowserRouteError::BadRequest(
                "cannot close the only browser session".to_owned(),
            ));
        }

        let ordered_ids = browser_sorted_session_ids(&sessions);
        let closed_session = sessions.remove(close_id).ok_or_else(|| {
            BrowserRouteError::NotFound(format!("browser session {close_id} not found"))
        })?;
        let profile_closed_session =
            browser_stored_closed_session_from_web_session(&closed_session);
        let mut closed_sessions = self.closed_sessions.lock().await;
        remember_closed_browser_session(&mut closed_sessions, close_id, closed_session);
        self.record_browser_profile_closed_session(profile_closed_session)
            .await;
        let selected_id = browser_fallback_session_id(&ordered_ids, &sessions, active_id, close_id)
            .ok_or_else(|| {
                BrowserRouteError::NotFound("no browser sessions remain after close".to_owned())
            })?;
        let (mut payload, back_href) = {
            let selected = sessions.get_mut(&selected_id).ok_or_else(|| {
                BrowserRouteError::NotFound(format!("browser session {selected_id} not found"))
            })?;
            selected.width =
                parse_optional_usize_param(target, "width", 40, 160).unwrap_or(selected.width);
            selected.height =
                parse_optional_usize_param(target, "height", 16, 120).unwrap_or(selected.height);
            selected.max_bytes =
                parse_optional_usize_param(target, "max_bytes", 64 * 1024, 16 * 1024 * 1024)
                    .unwrap_or(selected.max_bytes);
            selected.viewport_x = parse_optional_usize_param(target, "x", 0, usize::MAX)
                .or_else(|| parse_optional_usize_param(target, "viewport_x", 0, usize::MAX))
                .unwrap_or(selected.viewport_x);
            if let Some(return_href) = target.param("from") {
                selected.back_href = sanitized_search_return_href(Some(&return_href));
            }
            let back_href = selected.back_href.clone();
            let payload = browser_session_payload(&selected_id, selected)?;
            (payload, back_href)
        };
        self.record_browser_profile_tabs(&sessions, &selected_id)
            .await;
        let bookmarks = self.bookmarks.lock().await;
        self.attach_browser_session_registry_state(
            &mut payload,
            &sessions,
            &closed_sessions,
            &bookmarks,
            &selected_id,
        )
        .await;
        Ok((payload, back_href))
    }

    async fn restore_closed_target(
        &self,
        target: &RequestTarget,
        active_id: &str,
        closed_id: &str,
    ) -> Result<(BrowserSessionPayload, String), BrowserRouteError> {
        let mut sessions = self.sessions.lock().await;
        if !sessions.contains_key(active_id) {
            return Err(BrowserRouteError::NotFound(format!(
                "browser session {active_id} not found"
            )));
        }
        let mut closed_sessions = self.closed_sessions.lock().await;
        let closed_index = closed_sessions
            .iter()
            .position(|closed| closed.id == closed_id)
            .ok_or_else(|| {
                BrowserRouteError::NotFound(format!("closed browser session {closed_id} not found"))
            })?;
        let mut restored = closed_sessions.remove(closed_index).session;
        restored.width =
            parse_optional_usize_param(target, "width", 40, 160).unwrap_or(restored.width);
        restored.height =
            parse_optional_usize_param(target, "height", 16, 120).unwrap_or(restored.height);
        restored.max_bytes =
            parse_optional_usize_param(target, "max_bytes", 64 * 1024, 16 * 1024 * 1024)
                .unwrap_or(restored.max_bytes);
        if let Some(return_href) = target.param("from") {
            restored.back_href = sanitized_search_return_href(Some(&return_href));
        }

        let restored_id = self.next_session_id();
        let back_href = restored.back_href.clone();
        let mut payload = browser_session_payload(&restored_id, &mut restored)?;
        self.record_browser_profile_visit(&payload).await;
        sessions.insert(restored_id.clone(), restored);
        self.record_browser_profile_tabs(&sessions, &restored_id)
            .await;
        let bookmarks = self.bookmarks.lock().await;
        self.attach_browser_session_registry_state(
            &mut payload,
            &sessions,
            &closed_sessions,
            &bookmarks,
            &restored_id,
        )
        .await;
        Ok((payload, back_href))
    }

    async fn add_current_bookmark(
        &self,
        web_session: &BrowserWebSession,
    ) -> Result<(), BrowserRouteError> {
        let render = web_session.session.current().ok_or_else(|| {
            BrowserRouteError::BadRequest("browser session has no current page".to_owned())
        })?;
        let source = render.source.trim();
        if source.is_empty() {
            return Err(BrowserRouteError::BadRequest(
                "cannot bookmark an empty browser source".to_owned(),
            ));
        }
        let title = browser_session_title(render);
        {
            let mut bookmarks = self.bookmarks.lock().await;
            if let Some(bookmark) = bookmarks
                .iter_mut()
                .find(|bookmark| bookmark.source == source)
            {
                bookmark.title = title;
                drop(bookmarks);
                self.save_browser_profile().await;
                return Ok(());
            }

            let bookmark_id = self.next_bookmark_id.fetch_add(1, AtomicOrdering::Relaxed);
            bookmarks.push(BrowserStoredBookmark {
                id: format!("b{bookmark_id}"),
                title,
                source: source.to_owned(),
            });
        }
        self.save_browser_profile().await;
        Ok(())
    }

    async fn remove_bookmark(&self, bookmark_id: &str) -> Result<(), BrowserRouteError> {
        {
            let mut bookmarks = self.bookmarks.lock().await;
            let previous_len = bookmarks.len();
            bookmarks.retain(|bookmark| bookmark.id != bookmark_id);
            if bookmarks.len() == previous_len {
                return Err(BrowserRouteError::NotFound(format!(
                    "browser bookmark {bookmark_id} not found"
                )));
            }
        }
        self.save_browser_profile().await;
        Ok(())
    }

    async fn clear_bookmarks(&self) -> Result<(), BrowserRouteError> {
        {
            let mut bookmarks = self.bookmarks.lock().await;
            bookmarks.clear();
        }
        self.save_browser_profile().await;
        Ok(())
    }

    async fn bookmark_source(&self, bookmark_id: &str) -> Result<String, BrowserRouteError> {
        let bookmarks = self.bookmarks.lock().await;
        bookmarks
            .iter()
            .find(|bookmark| bookmark.id == bookmark_id)
            .map(|bookmark| bookmark.source.clone())
            .ok_or_else(|| {
                BrowserRouteError::NotFound(format!("browser bookmark {bookmark_id} not found"))
            })
    }

    async fn profile_closed_source(&self, index: usize) -> Result<String, BrowserRouteError> {
        let profile_closed_sessions = self.profile_closed_sessions.lock().await;
        profile_closed_sessions
            .get(index)
            .map(|closed| closed.source.clone())
            .ok_or_else(|| {
                BrowserRouteError::NotFound(format!(
                    "browser profile closed entry {index} not found"
                ))
            })
    }

    async fn remove_profile_closed(&self, index: usize) -> Result<(), BrowserRouteError> {
        {
            let mut profile_closed_sessions = self.profile_closed_sessions.lock().await;
            if index >= profile_closed_sessions.len() {
                return Err(BrowserRouteError::NotFound(format!(
                    "browser profile closed entry {index} not found"
                )));
            }
            profile_closed_sessions.remove(index);
        }
        self.save_browser_profile().await;
        Ok(())
    }

    async fn clear_closed_sessions(&self) -> Result<(), BrowserRouteError> {
        {
            let mut closed_sessions = self.closed_sessions.lock().await;
            closed_sessions.clear();
        }
        {
            let mut profile_closed_sessions = self.profile_closed_sessions.lock().await;
            profile_closed_sessions.clear();
        }
        self.save_browser_profile().await;
        Ok(())
    }

    async fn forget_closed_session(&self, closed_id: &str) -> Result<(), BrowserRouteError> {
        let source = {
            let mut closed_sessions = self.closed_sessions.lock().await;
            let Some(index) = closed_sessions
                .iter()
                .position(|closed| closed.id == closed_id)
            else {
                return Err(BrowserRouteError::NotFound(format!(
                    "closed browser session {closed_id} not found"
                )));
            };
            closed_sessions.remove(index).source
        };
        {
            let mut profile_closed_sessions = self.profile_closed_sessions.lock().await;
            profile_closed_sessions.retain(|closed| closed.source != source);
        }
        self.save_browser_profile().await;
        Ok(())
    }

    async fn remove_profile_history(&self, index: usize) -> Result<(), BrowserRouteError> {
        {
            let mut profile_history = self.profile_history.lock().await;
            if index >= profile_history.len() {
                return Err(BrowserRouteError::NotFound(format!(
                    "browser profile history entry {index} not found"
                )));
            }
            profile_history.remove(index);
        }
        self.save_browser_profile().await;
        Ok(())
    }

    async fn clear_profile_history(&self) -> Result<(), BrowserRouteError> {
        {
            let mut profile_history = self.profile_history.lock().await;
            profile_history.clear();
        }
        self.save_browser_profile().await;
        Ok(())
    }

    async fn clear_profile_tabs(&self) -> Result<(), BrowserRouteError> {
        {
            let mut profile_tabs = self.profile_tabs.lock().await;
            profile_tabs.clear();
        }
        self.save_browser_profile().await;
        Ok(())
    }

    async fn record_browser_profile_closed_session(
        &self,
        entry: Option<BrowserStoredClosedSession>,
    ) {
        let Some(entry) = entry else {
            return;
        };
        if self.profile_path.is_none() {
            return;
        }
        {
            let mut profile_closed_sessions = self.profile_closed_sessions.lock().await;
            profile_closed_sessions.retain(|closed| closed.source != entry.source);
            profile_closed_sessions.insert(0, entry);
            profile_closed_sessions.truncate(MAX_CLOSED_BROWSER_SESSIONS);
        }
        self.save_browser_profile().await;
    }

    async fn record_browser_profile_tabs(
        &self,
        sessions: &HashMap<String, BrowserWebSession>,
        active_id: &str,
    ) {
        if self.profile_path.is_none() {
            return;
        }
        let updated_at_unix_secs = browser_profile_now_unix_secs();
        let mut tabs = browser_sorted_session_ids(sessions)
            .into_iter()
            .filter_map(|id| {
                let session = sessions.get(&id)?;
                let render = session.session.current()?;
                let source = render.source.trim();
                if source.is_empty() {
                    return None;
                }
                Some(BrowserStoredProfileTab {
                    title: browser_session_title(render),
                    source: source.to_owned(),
                    active: id == active_id,
                    updated_at_unix_secs,
                })
            })
            .collect::<Vec<_>>();
        if !tabs.iter().any(|tab| tab.active) {
            if let Some(first) = tabs.first_mut() {
                first.active = true;
            }
        }
        {
            let mut profile_tabs = self.profile_tabs.lock().await;
            *profile_tabs = tabs;
        }
        self.save_browser_profile().await;
    }

    async fn attach_browser_session_registry_state(
        &self,
        payload: &mut BrowserSessionPayload,
        sessions: &HashMap<String, BrowserWebSession>,
        closed_sessions: &[BrowserClosedSession],
        bookmarks: &[BrowserStoredBookmark],
        current_id: &str,
    ) {
        let profile_history = self.profile_history.lock().await;
        let profile_closed_sessions = self.profile_closed_sessions.lock().await;
        let profile_error = self.profile_error.lock().await;
        attach_browser_session_registry_state(
            payload,
            sessions,
            closed_sessions,
            bookmarks,
            self.profile_path.is_some(),
            profile_error.clone(),
            &profile_history,
            &profile_closed_sessions,
            current_id,
        );
    }

    async fn record_browser_profile_visit(&self, payload: &BrowserSessionPayload) {
        if self.profile_path.is_none() || payload.source.trim().is_empty() {
            return;
        }
        {
            let mut profile_history = self.profile_history.lock().await;
            let entry = BrowserStoredProfileEntry {
                title: payload.title.clone(),
                source: payload.source.clone(),
                visited_at_unix_secs: browser_profile_now_unix_secs(),
            };
            if let Some(last) = profile_history
                .last_mut()
                .filter(|last| last.source == entry.source)
            {
                last.title = entry.title;
                last.visited_at_unix_secs = entry.visited_at_unix_secs;
            } else {
                profile_history.push(entry);
                if profile_history.len() > MAX_BROWSER_PROFILE_HISTORY {
                    let overflow = profile_history.len() - MAX_BROWSER_PROFILE_HISTORY;
                    profile_history.drain(..overflow);
                }
            }
        }
        self.save_browser_profile().await;
    }

    async fn save_browser_profile(&self) {
        let Some(path) = self.profile_path.as_deref() else {
            return;
        };
        let bookmarks = self.bookmarks.lock().await;
        let profile_tabs = self.profile_tabs.lock().await;
        let profile_history = self.profile_history.lock().await;
        let profile_closed_sessions = self.profile_closed_sessions.lock().await;
        let profile = BrowserSessionProfileFile {
            version: 1,
            bookmarks: bookmarks
                .iter()
                .map(browser_profile_bookmark_from_stored)
                .collect(),
            tabs: profile_tabs
                .iter()
                .map(browser_profile_tab_from_stored)
                .collect(),
            history: profile_history
                .iter()
                .map(browser_profile_entry_from_stored)
                .collect(),
            closed: profile_closed_sessions
                .iter()
                .map(browser_profile_closed_from_stored)
                .collect(),
        };
        let result = save_browser_session_profile(path, &profile);
        let mut profile_error = self.profile_error.lock().await;
        *profile_error = result.err();
    }
}

fn attach_browser_session_registry_state(
    payload: &mut BrowserSessionPayload,
    sessions: &HashMap<String, BrowserWebSession>,
    closed_sessions: &[BrowserClosedSession],
    bookmarks: &[BrowserStoredBookmark],
    profile_enabled: bool,
    profile_error: Option<String>,
    profile_history: &[BrowserStoredProfileEntry],
    profile_closed_sessions: &[BrowserStoredClosedSession],
    current_id: &str,
) {
    payload.sessions = browser_session_summaries(sessions, current_id);
    payload.closed_sessions =
        browser_closed_session_summaries(closed_sessions, profile_closed_sessions, payload);
    payload.closed_sessions_clear_url = (!payload.closed_sessions.is_empty())
        .then(|| browser_session_action_href(&payload.id, "clear-closed", &[], payload));
    payload.profile_enabled = profile_enabled;
    payload.profile_error = profile_error;
    payload.current_bookmarked = bookmarks
        .iter()
        .any(|bookmark| bookmark.source == payload.source);
    payload.bookmarks_clear_url = (!bookmarks.is_empty())
        .then(|| browser_session_action_href(&payload.id, "clear-bookmarks", &[], payload));
    payload.bookmarks = browser_session_bookmarks(bookmarks, payload);
    payload.profile_tabs_clear_url = profile_enabled
        .then(|| browser_session_action_href(&payload.id, "clear-profile-tabs", &[], payload));
    payload.profile_history_clear_url = profile_enabled
        .then(|| browser_session_action_href(&payload.id, "clear-profile-history", &[], payload));
    payload.profile_history = browser_session_profile_history(profile_history, payload);
}

fn browser_session_summaries(
    sessions: &HashMap<String, BrowserWebSession>,
    current_id: &str,
) -> Vec<BrowserSessionSummaryPayload> {
    let can_close = sessions.len() > 1;
    let close_href_source = sessions.get(current_id);
    let mut summaries = sessions
        .iter()
        .map(|(id, session)| {
            let (title, source) = session
                .session
                .current()
                .map(|render| (browser_session_title(render), render.source.clone()))
                .unwrap_or_else(|| ("Untitled".to_owned(), String::new()));
            let href_source = close_href_source.unwrap_or(session);
            BrowserSessionSummaryPayload {
                id: id.clone(),
                title,
                source: source.clone(),
                action_url: browser_session_action_href(id, "current", &[], session),
                reload_url: browser_session_action_href(id, "reload", &[], session),
                duplicate_url: browser_session_action_href(
                    current_id,
                    "duplicate-session",
                    &[("session", id.clone())],
                    href_source,
                ),
                close_url: close_href_source.map_or_else(String::new, |source| {
                    browser_session_action_href(
                        current_id,
                        "close-session",
                        &[("close_id", id.clone())],
                        source,
                    )
                }),
                current: id == current_id,
                can_close,
            }
        })
        .collect::<Vec<_>>();
    summaries.sort_by(|left, right| {
        browser_session_id_number(&left.id)
            .cmp(&browser_session_id_number(&right.id))
            .then_with(|| left.id.cmp(&right.id))
    });
    summaries
}

fn remember_closed_browser_session(
    closed_sessions: &mut Vec<BrowserClosedSession>,
    id: &str,
    session: BrowserWebSession,
) {
    let (title, source) = session
        .session
        .current()
        .map(|render| (browser_session_title(render), render.source.clone()))
        .unwrap_or_else(|| ("Untitled".to_owned(), String::new()));
    let closed_at_unix_secs = browser_profile_now_unix_secs();
    closed_sessions.retain(|closed| closed.id != id && closed.source != source);
    closed_sessions.insert(
        0,
        BrowserClosedSession {
            id: id.to_owned(),
            title,
            source,
            closed_at_unix_secs,
            session,
        },
    );
    closed_sessions.truncate(MAX_CLOSED_BROWSER_SESSIONS);
}

fn browser_closed_session_summaries(
    closed_sessions: &[BrowserClosedSession],
    profile_closed_sessions: &[BrowserStoredClosedSession],
    source: &BrowserSessionPayload,
) -> Vec<BrowserClosedSessionPayload> {
    let mut summaries = closed_sessions
        .iter()
        .map(|closed| BrowserClosedSessionPayload {
            id: closed.id.clone(),
            title: closed.title.clone(),
            source: closed.source.clone(),
            closed_at_unix_secs: closed.closed_at_unix_secs,
            closed_at: browser_profile_timestamp_label(closed.closed_at_unix_secs),
            persisted: false,
            restore_url: browser_session_action_href(
                &source.id,
                "restore-closed",
                &[("closed_id", closed.id.clone())],
                source,
            ),
            new_session_url: browser_session_new_session_href(&closed.source, source),
            forget_url: browser_session_action_href(
                &source.id,
                "forget-closed",
                &[("closed_id", closed.id.clone())],
                source,
            ),
        })
        .collect::<Vec<_>>();
    for (index, closed) in profile_closed_sessions.iter().enumerate() {
        if summaries
            .iter()
            .any(|summary| summary.source == closed.source)
        {
            continue;
        }
        summaries.push(BrowserClosedSessionPayload {
            id: format!("p{}", index + 1),
            title: closed.title.clone(),
            source: closed.source.clone(),
            closed_at_unix_secs: closed.closed_at_unix_secs,
            closed_at: browser_profile_timestamp_label(closed.closed_at_unix_secs),
            persisted: true,
            restore_url: browser_session_action_href(
                &source.id,
                "open-profile-closed",
                &[("closed", index.to_string())],
                source,
            ),
            new_session_url: browser_session_new_session_href(&closed.source, source),
            forget_url: browser_session_action_href(
                &source.id,
                "forget-profile-closed",
                &[("closed", index.to_string())],
                source,
            ),
        });
    }
    summaries
}

fn browser_session_bookmarks(
    bookmarks: &[BrowserStoredBookmark],
    source: &BrowserSessionPayload,
) -> Vec<BrowserSessionBookmarkPayload> {
    bookmarks
        .iter()
        .map(|bookmark| BrowserSessionBookmarkPayload {
            id: bookmark.id.clone(),
            title: bookmark.title.clone(),
            source: bookmark.source.clone(),
            action_url: browser_session_action_href(
                &source.id,
                "open-bookmark",
                &[("bookmark", bookmark.id.clone())],
                source,
            ),
            new_session_url: browser_session_new_session_href(&bookmark.source, source),
            remove_url: browser_session_action_href(
                &source.id,
                "remove-bookmark",
                &[("bookmark", bookmark.id.clone())],
                source,
            ),
            current: bookmark.source == source.source,
        })
        .collect()
}

fn browser_session_profile_history(
    profile_history: &[BrowserStoredProfileEntry],
    source: &BrowserSessionPayload,
) -> Vec<BrowserSessionProfileEntryPayload> {
    profile_history
        .iter()
        .enumerate()
        .rev()
        .take(40)
        .enumerate()
        .map(
            |(display_index, (history_index, entry))| BrowserSessionProfileEntryPayload {
                index: display_index,
                title: entry.title.clone(),
                source: entry.source.clone(),
                visited_at_unix_secs: entry.visited_at_unix_secs,
                visited_at: browser_profile_timestamp_label(entry.visited_at_unix_secs),
                action_url: browser_session_action_href(
                    &source.id,
                    "open",
                    &[("url", entry.source.clone())],
                    source,
                ),
                new_session_url: browser_session_new_session_href(&entry.source, source),
                remove_url: browser_session_action_href(
                    &source.id,
                    "remove-profile-history",
                    &[("history", history_index.to_string())],
                    source,
                ),
            },
        )
        .collect()
}

fn browser_session_id_number(id: &str) -> u64 {
    id.strip_prefix('s')
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(u64::MAX)
}

fn browser_profile_id_number(id: &str, prefix: char) -> Option<u64> {
    id.strip_prefix(prefix)?.parse::<u64>().ok()
}

fn browser_profile_now_unix_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

fn browser_profile_timestamp_label(timestamp: u64) -> String {
    if timestamp == 0 {
        return "unknown time".to_owned();
    }
    let Some(datetime) = chrono::DateTime::<chrono::Utc>::from_timestamp(timestamp as i64, 0)
    else {
        return format!("unix {timestamp}");
    };
    datetime.format("%Y-%m-%d %H:%M UTC").to_string()
}

fn load_browser_session_profile(path: &Path) -> Result<BrowserSessionProfileFile, String> {
    match std::fs::read(path) {
        Ok(bytes) if bytes.is_empty() => Ok(BrowserSessionProfileFile::default()),
        Ok(bytes) => serde_json::from_slice::<BrowserSessionProfileFile>(&bytes).map_err(|error| {
            format!(
                "failed to parse browser profile {}: {error}",
                path.display()
            )
        }),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            Ok(BrowserSessionProfileFile::default())
        }
        Err(error) => Err(format!(
            "failed to read browser profile {}: {error}",
            path.display()
        )),
    }
}

fn save_browser_session_profile(
    path: &Path,
    profile: &BrowserSessionProfileFile,
) -> Result<(), String> {
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent).map_err(|error| {
            format!(
                "failed to create browser profile directory {}: {error}",
                parent.display()
            )
        })?;
    }
    let bytes = serde_json::to_vec_pretty(profile).map_err(|error| {
        format!(
            "failed to encode browser profile {}: {error}",
            path.display()
        )
    })?;
    std::fs::write(path, bytes).map_err(|error| {
        format!(
            "failed to write browser profile {}: {error}",
            path.display()
        )
    })
}

fn browser_stored_bookmark_from_profile_entry(
    entry: BrowserSessionProfileBookmarkFile,
) -> Option<BrowserStoredBookmark> {
    let id = entry.id.trim();
    let source = entry.source.trim();
    if id.is_empty() || source.is_empty() {
        return None;
    }
    Some(BrowserStoredBookmark {
        id: id.to_owned(),
        title: browser_profile_title(&entry.title, source),
        source: source.to_owned(),
    })
}

fn browser_stored_profile_tab_from_file(
    entry: BrowserSessionProfileTabFile,
) -> Option<BrowserStoredProfileTab> {
    let source = entry.source.trim();
    if source.is_empty() {
        return None;
    }
    Some(BrowserStoredProfileTab {
        title: browser_profile_title(&entry.title, source),
        source: source.to_owned(),
        active: entry.active,
        updated_at_unix_secs: entry.updated_at_unix_secs,
    })
}

fn browser_stored_profile_entry_from_file(
    entry: BrowserSessionProfileEntryFile,
) -> Option<BrowserStoredProfileEntry> {
    let source = entry.source.trim();
    if source.is_empty() {
        return None;
    }
    Some(BrowserStoredProfileEntry {
        title: browser_profile_title(&entry.title, source),
        source: source.to_owned(),
        visited_at_unix_secs: entry.visited_at_unix_secs,
    })
}

fn browser_stored_closed_session_from_file(
    entry: BrowserSessionProfileClosedFile,
) -> Option<BrowserStoredClosedSession> {
    let source = entry.source.trim();
    if source.is_empty() {
        return None;
    }
    Some(BrowserStoredClosedSession {
        title: browser_profile_title(&entry.title, source),
        source: source.to_owned(),
        closed_at_unix_secs: entry.closed_at_unix_secs,
    })
}

fn browser_stored_closed_session_from_web_session(
    session: &BrowserWebSession,
) -> Option<BrowserStoredClosedSession> {
    let render = session.session.current()?;
    let source = render.source.trim();
    if source.is_empty() {
        return None;
    }
    Some(BrowserStoredClosedSession {
        title: browser_session_title(render),
        source: source.to_owned(),
        closed_at_unix_secs: browser_profile_now_unix_secs(),
    })
}

fn browser_profile_bookmark_from_stored(
    bookmark: &BrowserStoredBookmark,
) -> BrowserSessionProfileBookmarkFile {
    BrowserSessionProfileBookmarkFile {
        id: bookmark.id.clone(),
        title: bookmark.title.clone(),
        source: bookmark.source.clone(),
    }
}

fn browser_profile_tab_from_stored(tab: &BrowserStoredProfileTab) -> BrowserSessionProfileTabFile {
    BrowserSessionProfileTabFile {
        title: tab.title.clone(),
        source: tab.source.clone(),
        active: tab.active,
        updated_at_unix_secs: tab.updated_at_unix_secs,
    }
}

fn browser_profile_entry_from_stored(
    entry: &BrowserStoredProfileEntry,
) -> BrowserSessionProfileEntryFile {
    BrowserSessionProfileEntryFile {
        title: entry.title.clone(),
        source: entry.source.clone(),
        visited_at_unix_secs: entry.visited_at_unix_secs,
    }
}

fn browser_profile_closed_from_stored(
    entry: &BrowserStoredClosedSession,
) -> BrowserSessionProfileClosedFile {
    BrowserSessionProfileClosedFile {
        title: entry.title.clone(),
        source: entry.source.clone(),
        closed_at_unix_secs: entry.closed_at_unix_secs,
    }
}

fn browser_profile_title(title: &str, source: &str) -> String {
    if title.trim().is_empty() {
        source.to_owned()
    } else {
        title.to_owned()
    }
}

fn browser_sorted_session_ids(sessions: &HashMap<String, BrowserWebSession>) -> Vec<String> {
    let mut ids = sessions.keys().cloned().collect::<Vec<_>>();
    ids.sort_by(|left, right| {
        browser_session_id_number(left)
            .cmp(&browser_session_id_number(right))
            .then_with(|| left.cmp(right))
    });
    ids
}

fn browser_fallback_session_id(
    ordered_ids: &[String],
    sessions: &HashMap<String, BrowserWebSession>,
    active_id: &str,
    close_id: &str,
) -> Option<String> {
    if active_id != close_id && sessions.contains_key(active_id) {
        return Some(active_id.to_owned());
    }

    let close_index = ordered_ids
        .iter()
        .position(|id| id == close_id)
        .unwrap_or(ordered_ids.len());
    ordered_ids
        .iter()
        .skip(close_index.saturating_add(1))
        .find(|id| sessions.contains_key(*id))
        .cloned()
        .or_else(|| {
            ordered_ids
                .iter()
                .take(close_index)
                .rev()
                .find(|id| sessions.contains_key(*id))
                .cloned()
        })
}

async fn apply_browser_action(
    action: BrowserSessionAction,
    web_session: &mut BrowserWebSession,
) -> Result<(), BrowserRouteError> {
    match action {
        BrowserSessionAction::Current => {}
        BrowserSessionAction::Open(url) => {
            let target_url = web_session.session.resolve_current_target(&url);
            web_session
                .session
                .navigate(&target_url)
                .await
                .map_err(|error| {
                    BrowserRouteError::Upstream(format!(
                        "browser render failed for {target_url}: {error:#}"
                    ))
                })?;
            reset_viewport_after_navigation(web_session);
            clear_browser_find_active_line(web_session);
        }
        BrowserSessionAction::Back => {
            web_session
                .session
                .back()
                .map_err(|error| BrowserRouteError::BadRequest(error.to_string()))?;
            reset_viewport_after_navigation(web_session);
            clear_browser_find_active_line(web_session);
        }
        BrowserSessionAction::Forward => {
            web_session
                .session
                .forward()
                .map_err(|error| BrowserRouteError::BadRequest(error.to_string()))?;
            reset_viewport_after_navigation(web_session);
            clear_browser_find_active_line(web_session);
        }
        BrowserSessionAction::Reload => {
            web_session.session.reload().await.map_err(|error| {
                BrowserRouteError::Upstream(format!("browser reload failed: {error:#}"))
            })?;
            reset_viewport_after_navigation(web_session);
            clear_browser_find_active_line(web_session);
        }
        BrowserSessionAction::Link(index) => {
            web_session
                .session
                .activate_link(index)
                .await
                .map_err(|error| BrowserRouteError::BadRequest(error.to_string()))?;
            reset_viewport_after_navigation(web_session);
            clear_browser_find_active_line(web_session);
        }
        BrowserSessionAction::LinkText(text) => {
            web_session
                .session
                .activate_link_text(&text)
                .await
                .map_err(|error| BrowserRouteError::BadRequest(error.to_string()))?;
            reset_viewport_after_navigation(web_session);
            clear_browser_find_active_line(web_session);
        }
        BrowserSessionAction::LinkSelector(selector) => {
            web_session
                .session
                .activate_link_selector(&selector)
                .await
                .map_err(|error| BrowserRouteError::BadRequest(error.to_string()))?;
            reset_viewport_after_navigation(web_session);
            clear_browser_find_active_line(web_session);
        }
        BrowserSessionAction::History(index) => {
            apply_browser_history_entry(web_session, index)?;
            reset_viewport_after_navigation(web_session);
            clear_browser_find_active_line(web_session);
        }
        BrowserSessionAction::Find(query) => {
            web_session.find_query = query.trim().to_owned();
            if !web_session.find_query.is_empty() {
                apply_browser_find(web_session, BrowserFindDirection::First)?;
            } else {
                clear_browser_find_active_line(web_session);
            }
        }
        BrowserSessionAction::FindNext => {
            apply_browser_find(web_session, BrowserFindDirection::Next)?;
        }
        BrowserSessionAction::FindPrevious => {
            apply_browser_find(web_session, BrowserFindDirection::Previous)?;
        }
        BrowserSessionAction::ClearFind => {
            web_session.find_query.clear();
            clear_browser_find_active_line(web_session);
        }
        BrowserSessionAction::ClickSelector(selector) => {
            let before = current_session_source(web_session);
            web_session
                .session
                .click_selector_with_default_action(&selector)
                .await
                .map_err(|error| BrowserRouteError::BadRequest(error.to_string()))?;
            if current_session_source(web_session) != before {
                reset_viewport_after_navigation(web_session);
                clear_browser_find_active_line(web_session);
            }
        }
        BrowserSessionAction::ClickAt { x, y } => {
            let before = current_session_source(web_session);
            web_session
                .session
                .click_at_with_default_action(
                    web_session.viewport_x.saturating_add(x),
                    web_session.viewport_y.saturating_add(y),
                )
                .await
                .map_err(|error| BrowserRouteError::BadRequest(error.to_string()))?;
            if current_session_source(web_session) != before {
                reset_viewport_after_navigation(web_session);
                clear_browser_find_active_line(web_session);
            }
        }
        BrowserSessionAction::FocusSelector(selector) => {
            web_session
                .session
                .focus_selector(&selector)
                .map_err(|error| BrowserRouteError::BadRequest(error.to_string()))?;
        }
        BrowserSessionAction::FocusNext => {
            web_session
                .session
                .focus_next_control()
                .map_err(|error| BrowserRouteError::BadRequest(error.to_string()))?;
        }
        BrowserSessionAction::FocusPrevious => {
            web_session
                .session
                .focus_previous_control()
                .map_err(|error| BrowserRouteError::BadRequest(error.to_string()))?;
        }
        BrowserSessionAction::TypeText(text) => {
            web_session
                .session
                .type_text(&text)
                .map_err(|error| BrowserRouteError::BadRequest(error.to_string()))?;
        }
        BrowserSessionAction::Backspace(count) => {
            web_session
                .session
                .delete_text_backward(count)
                .map_err(|error| BrowserRouteError::BadRequest(error.to_string()))?;
        }
        BrowserSessionAction::ClearInput => {
            web_session
                .session
                .clear_focused_text()
                .map_err(|error| BrowserRouteError::BadRequest(error.to_string()))?;
        }
        BrowserSessionAction::Enter => {
            let before = current_session_source(web_session);
            web_session
                .session
                .submit_focused_form()
                .await
                .map_err(|error| BrowserRouteError::BadRequest(error.to_string()))?;
            if current_session_source(web_session) != before {
                reset_viewport_after_navigation(web_session);
                clear_browser_find_active_line(web_session);
            }
        }
        BrowserSessionAction::Space => {
            let focused = web_session.session.focused_control().ok_or_else(|| {
                BrowserRouteError::BadRequest(
                    "cannot toggle focused control: no focused form control".to_owned(),
                )
            })?;
            web_session
                .session
                .toggle_form_control(focused.form_index, focused.control_index)
                .map_err(|error| BrowserRouteError::BadRequest(error.to_string()))?;
        }
        BrowserSessionAction::Choose(value) => {
            web_session
                .session
                .select_focused_option(&value)
                .map_err(|error| BrowserRouteError::BadRequest(error.to_string()))?;
        }
        BrowserSessionAction::ClearCookies => {
            web_session.session.clear_cookies();
        }
        BrowserSessionAction::ClearLocalStorage => {
            web_session.session.clear_local_storage();
        }
        BrowserSessionAction::ClearSessionStorage => {
            web_session.session.clear_session_storage();
        }
        BrowserSessionAction::FetchResources => {
            let report = web_session
                .session
                .fetch_current_resources(web_session.max_bytes)
                .await
                .map_err(|error| {
                    BrowserRouteError::Upstream(format!("browser resource fetch failed: {error:#}"))
                })?;
            web_session.resource_report = Some(browser_session_resource_report_from_fetch(report));
        }
        BrowserSessionAction::ApplyStylesheets => {
            let report = web_session
                .session
                .render_current_with_stylesheets(web_session.max_bytes)
                .await
                .map_err(|error| {
                    BrowserRouteError::Upstream(format!(
                        "browser stylesheet render failed: {error:#}"
                    ))
                })?;
            web_session.resource_report =
                Some(browser_session_resource_report_from_stylesheets(report));
        }
        BrowserSessionAction::RunScripts => {
            let report = web_session
                .session
                .render_current_with_scripts(web_session.max_bytes)
                .await
                .map_err(|error| {
                    BrowserRouteError::Upstream(format!("browser script render failed: {error:#}"))
                })?;
            web_session.resource_report =
                Some(browser_session_resource_report_from_scripts(report));
        }
        BrowserSessionAction::LoadImages => {
            let report = web_session
                .session
                .render_current_with_images(web_session.max_bytes)
                .await
                .map_err(|error| {
                    BrowserRouteError::Upstream(format!("browser image render failed: {error:#}"))
                })?;
            web_session.resource_report = Some(browser_session_resource_report_from_images(report));
        }
        BrowserSessionAction::ClearResourceReport => {
            web_session.resource_report = None;
        }
        BrowserSessionAction::AddBookmark
        | BrowserSessionAction::OpenBookmark(_)
        | BrowserSessionAction::RemoveBookmark(_)
        | BrowserSessionAction::ClearBookmarks
        | BrowserSessionAction::OpenProfileClosed(_)
        | BrowserSessionAction::RemoveProfileHistory(_)
        | BrowserSessionAction::ClearClosedSessions
        | BrowserSessionAction::ClearProfileTabs
        | BrowserSessionAction::ClearProfileHistory
        | BrowserSessionAction::RestoreClosedSession(_)
        | BrowserSessionAction::ForgetClosedSession(_)
        | BrowserSessionAction::ForgetProfileClosed(_) => {
            return Err(BrowserRouteError::BadRequest(
                "browser registry actions must be handled by the registry".to_owned(),
            ));
        }
        BrowserSessionAction::OpenNewSession(_)
        | BrowserSessionAction::LinkTextNewSession(_)
        | BrowserSessionAction::LinkSelectorNewSession(_)
        | BrowserSessionAction::DuplicateSession(_)
        | BrowserSessionAction::CloseSession(_)
        | BrowserSessionAction::CloseOtherSessions
        | BrowserSessionAction::CloseSessionsToRight
        | BrowserSessionAction::CloseSessionsToLeft
        | BrowserSessionAction::CloseDuplicateSessions
        | BrowserSessionAction::SwitchNextSession
        | BrowserSessionAction::SwitchPreviousSession => {
            return Err(BrowserRouteError::BadRequest(
                "browser session registry actions must be handled by the registry".to_owned(),
            ));
        }
        BrowserSessionAction::Scroll { dx, dy } => {
            web_session.viewport_x = apply_scroll_delta(web_session.viewport_x, dx);
            web_session.viewport_y = apply_scroll_delta(web_session.viewport_y, dy);
        }
        BrowserSessionAction::Top => {
            web_session.viewport_y = 0;
        }
        BrowserSessionAction::Bottom => {
            web_session.viewport_y = usize::MAX;
        }
        BrowserSessionAction::Fill {
            form_index,
            name,
            value,
        } => {
            web_session
                .session
                .set_form_field(form_index, &name, &value)
                .map_err(|error| BrowserRouteError::BadRequest(error.to_string()))?;
        }
        BrowserSessionAction::Select {
            form_index,
            control_index,
            value,
        } => {
            web_session
                .session
                .select_form_option(form_index, control_index, &value)
                .map_err(|error| BrowserRouteError::BadRequest(error.to_string()))?;
        }
        BrowserSessionAction::Toggle {
            form_index,
            control_index,
        } => {
            web_session
                .session
                .toggle_form_control(form_index, control_index)
                .map_err(|error| BrowserRouteError::BadRequest(error.to_string()))?;
        }
        BrowserSessionAction::Submit { form_index } => {
            web_session
                .session
                .submit_form(form_index, &[])
                .await
                .map_err(|error| BrowserRouteError::BadRequest(error.to_string()))?;
            reset_viewport_after_navigation(web_session);
            clear_browser_find_active_line(web_session);
        }
    }
    Ok(())
}

fn browser_action(target: &RequestTarget) -> Result<BrowserSessionAction, BrowserRouteError> {
    let action = target.param("action").unwrap_or_else(|| {
        if target.param("url").is_some() {
            "open"
        } else {
            "current"
        }
        .to_owned()
    });
    match action.as_str() {
        "current" => Ok(BrowserSessionAction::Current),
        "open" => browser_action_url(target).map(BrowserSessionAction::Open),
        "open-new-session" | "open_new_session" | "open-new-tab" | "open_new_tab" => {
            browser_action_url(target).map(BrowserSessionAction::OpenNewSession)
        }
        "back" => Ok(BrowserSessionAction::Back),
        "forward" => Ok(BrowserSessionAction::Forward),
        "reload" => Ok(BrowserSessionAction::Reload),
        "link" => {
            let index = target
                .param("link")
                .or_else(|| target.param("index"))
                .ok_or_else(|| BrowserRouteError::BadRequest("missing link index".to_owned()))?
                .parse::<usize>()
                .map_err(|_| BrowserRouteError::BadRequest("invalid link index".to_owned()))?;
            Ok(BrowserSessionAction::Link(index))
        }
        "link-text" | "link_text" | "open-link-text" | "open_link_text" => {
            browser_action_link_text(target).map(BrowserSessionAction::LinkText)
        }
        "link-text-new-session"
        | "link_text_new_session"
        | "link-text-new-tab"
        | "link_text_new_tab"
        | "open-link-text-new-session"
        | "open_link_text_new_session"
        | "open-link-text-new-tab"
        | "open_link_text_new_tab" => {
            browser_action_link_text(target).map(BrowserSessionAction::LinkTextNewSession)
        }
        "link-selector" | "link_selector" | "open-link-selector" | "open_link_selector" => {
            browser_action_link_selector(target).map(BrowserSessionAction::LinkSelector)
        }
        "link-selector-new-session"
        | "link_selector_new_session"
        | "link-selector-new-tab"
        | "link_selector_new_tab"
        | "open-link-selector-new-session"
        | "open_link_selector_new_session"
        | "open-link-selector-new-tab"
        | "open_link_selector_new_tab" => {
            browser_action_link_selector(target).map(BrowserSessionAction::LinkSelectorNewSession)
        }
        "history" | "history-entry" | "history_entry" => {
            let index = target
                .param("history")
                .or_else(|| target.param("index"))
                .ok_or_else(|| BrowserRouteError::BadRequest("missing history index".to_owned()))?
                .parse::<usize>()
                .map_err(|_| BrowserRouteError::BadRequest("invalid history index".to_owned()))?;
            Ok(BrowserSessionAction::History(index))
        }
        "find" | "find-text" | "find_text" => Ok(BrowserSessionAction::Find(
            target
                .param("q")
                .or_else(|| target.param("query"))
                .or_else(|| target.param("text"))
                .unwrap_or_default(),
        )),
        "find-next" | "find_next" => Ok(BrowserSessionAction::FindNext),
        "find-prev" | "find_previous" | "find-previous" => Ok(BrowserSessionAction::FindPrevious),
        "clear-find" | "clear_find" => Ok(BrowserSessionAction::ClearFind),
        "click-selector" | "click_selector" | "click" => {
            let selector = target.param("selector").unwrap_or_default();
            if selector.trim().is_empty() {
                Err(BrowserRouteError::BadRequest(
                    "missing click selector".to_owned(),
                ))
            } else {
                Ok(BrowserSessionAction::ClickSelector(selector))
            }
        }
        "click-at" | "click_at" => Ok(BrowserSessionAction::ClickAt {
            x: browser_action_index(target, "x", "x coordinate")?,
            y: browser_action_index(target, "y", "y coordinate")?,
        }),
        "focus-selector" | "focus_selector" | "focus" => {
            let selector = target.param("selector").unwrap_or_default();
            if selector.trim().is_empty() {
                Err(BrowserRouteError::BadRequest(
                    "missing focus selector".to_owned(),
                ))
            } else {
                Ok(BrowserSessionAction::FocusSelector(selector))
            }
        }
        "focus-next" | "focus_next" | "tab" => Ok(BrowserSessionAction::FocusNext),
        "focus-prev" | "focus_previous" | "focus-previous" | "shift-tab" => {
            Ok(BrowserSessionAction::FocusPrevious)
        }
        "type" | "type-text" | "type_text" => Ok(BrowserSessionAction::TypeText(
            target.param("text").unwrap_or_default(),
        )),
        "backspace" => Ok(BrowserSessionAction::Backspace(
            parse_optional_usize_param(target, "count", 1, 128).unwrap_or(1),
        )),
        "clear-input" | "clear_input" => Ok(BrowserSessionAction::ClearInput),
        "enter" | "submit-focused" | "submit_focused" => Ok(BrowserSessionAction::Enter),
        "space" | "toggle-focused" | "toggle_focused" => Ok(BrowserSessionAction::Space),
        "choose" | "select-focused" | "select_focused" => {
            let value = target.param("value").ok_or_else(|| {
                BrowserRouteError::BadRequest("missing focused option value".to_owned())
            })?;
            Ok(BrowserSessionAction::Choose(value))
        }
        "clear-cookies" | "clear_cookies" => Ok(BrowserSessionAction::ClearCookies),
        "clear-local-storage" | "clear_local_storage" => {
            Ok(BrowserSessionAction::ClearLocalStorage)
        }
        "clear-session-storage" | "clear_session_storage" => {
            Ok(BrowserSessionAction::ClearSessionStorage)
        }
        "bookmark" | "add-bookmark" | "add_bookmark" => Ok(BrowserSessionAction::AddBookmark),
        "open-bookmark" | "open_bookmark" => Ok(BrowserSessionAction::OpenBookmark(
            browser_bookmark_id(target)?,
        )),
        "remove-bookmark" | "remove_bookmark" | "delete-bookmark" | "delete_bookmark" => Ok(
            BrowserSessionAction::RemoveBookmark(browser_bookmark_id(target)?),
        ),
        "clear-bookmarks" | "clear_bookmarks" | "remove-bookmarks" | "remove_bookmarks"
        | "delete-bookmarks" | "delete_bookmarks" => Ok(BrowserSessionAction::ClearBookmarks),
        "open-profile-closed"
        | "open_profile_closed"
        | "restore-profile-closed"
        | "restore_profile_closed" => Ok(BrowserSessionAction::OpenProfileClosed(
            browser_profile_closed_index(target)?,
        )),
        "remove-profile-history"
        | "remove_profile_history"
        | "delete-profile-history"
        | "delete_profile_history" => Ok(BrowserSessionAction::RemoveProfileHistory(
            browser_action_index(target, "history", "profile history index")?,
        )),
        "clear-profile-history" | "clear_profile_history" => {
            Ok(BrowserSessionAction::ClearProfileHistory)
        }
        "clear-closed" | "clear_closed" | "clear-closed-sessions" | "clear_closed_sessions" => {
            Ok(BrowserSessionAction::ClearClosedSessions)
        }
        "clear-profile-tabs"
        | "clear_profile_tabs"
        | "forget-profile-tabs"
        | "forget_profile_tabs" => Ok(BrowserSessionAction::ClearProfileTabs),
        "restore-closed" | "restore_closed" | "restore-session" | "restore_session" => Ok(
            BrowserSessionAction::RestoreClosedSession(browser_closed_session_id(target)?),
        ),
        "forget-closed" | "forget_closed" | "forget-closed-session" | "forget_closed_session" => {
            Ok(BrowserSessionAction::ForgetClosedSession(
                browser_closed_session_id(target)?,
            ))
        }
        "forget-profile-closed" | "forget_profile_closed" => Ok(
            BrowserSessionAction::ForgetProfileClosed(browser_profile_closed_index(target)?),
        ),
        "fetch-resources" | "fetch_resources" | "resources" => {
            Ok(BrowserSessionAction::FetchResources)
        }
        "apply-styles" | "apply_styles" | "styles" | "stylesheets" => {
            Ok(BrowserSessionAction::ApplyStylesheets)
        }
        "run-scripts" | "run_scripts" | "scripts" => Ok(BrowserSessionAction::RunScripts),
        "load-images" | "load_images" | "images" => Ok(BrowserSessionAction::LoadImages),
        "clear-resource-report" | "clear_resource_report" | "clear-resources-report" => {
            Ok(BrowserSessionAction::ClearResourceReport)
        }
        "duplicate" | "duplicate-tab" | "duplicate_tab" | "duplicate-session"
        | "duplicate_session" => {
            let duplicate_id = target
                .param("session")
                .or_else(|| target.param("duplicate_id"))
                .or_else(|| target.param("target_session"))
                .or_else(|| target.param("id"))
                .unwrap_or_default();
            if duplicate_id.trim().is_empty() {
                Err(BrowserRouteError::BadRequest(
                    "missing browser session to duplicate".to_owned(),
                ))
            } else {
                Ok(BrowserSessionAction::DuplicateSession(duplicate_id))
            }
        }
        "close-other-tabs"
        | "close_other_tabs"
        | "close-other-sessions"
        | "close_other_sessions" => Ok(BrowserSessionAction::CloseOtherSessions),
        "close-tabs-right"
        | "close_tabs_right"
        | "close-right-tabs"
        | "close_right_tabs"
        | "close-sessions-right"
        | "close_sessions_right" => Ok(BrowserSessionAction::CloseSessionsToRight),
        "close-tabs-left"
        | "close_tabs_left"
        | "close-left-tabs"
        | "close_left_tabs"
        | "close-sessions-left"
        | "close_sessions_left" => Ok(BrowserSessionAction::CloseSessionsToLeft),
        "close-duplicate-tabs"
        | "close_duplicate_tabs"
        | "close-duplicates"
        | "close_duplicates"
        | "close-duplicate-sessions"
        | "close_duplicate_sessions" => Ok(BrowserSessionAction::CloseDuplicateSessions),
        "next-tab" | "next_tab" | "next-session" | "next_session" | "switch-next-tab"
        | "switch_next_tab" => Ok(BrowserSessionAction::SwitchNextSession),
        "previous-tab"
        | "previous_tab"
        | "prev-tab"
        | "prev_tab"
        | "previous-session"
        | "previous_session"
        | "prev-session"
        | "prev_session"
        | "switch-previous-tab"
        | "switch_previous_tab" => Ok(BrowserSessionAction::SwitchPreviousSession),
        "close" | "close-session" | "close_session" => {
            let close_id = target
                .param("close_id")
                .or_else(|| target.param("session"))
                .or_else(|| target.param("id"))
                .unwrap_or_default();
            if close_id.trim().is_empty() {
                Err(BrowserRouteError::BadRequest(
                    "missing browser session to close".to_owned(),
                ))
            } else {
                Ok(BrowserSessionAction::CloseSession(close_id))
            }
        }
        "scroll" => {
            let dx = target
                .param("dx")
                .unwrap_or_else(|| "0".to_owned())
                .parse::<isize>()
                .map_err(|_| {
                    BrowserRouteError::BadRequest("invalid horizontal scroll delta".to_owned())
                })?;
            let dy = target
                .param("dy")
                .unwrap_or_else(|| "0".to_owned())
                .parse::<isize>()
                .map_err(|_| {
                    BrowserRouteError::BadRequest("invalid vertical scroll delta".to_owned())
                })?;
            Ok(BrowserSessionAction::Scroll { dx, dy })
        }
        "top" => Ok(BrowserSessionAction::Top),
        "bottom" => Ok(BrowserSessionAction::Bottom),
        "fill" => Ok(BrowserSessionAction::Fill {
            form_index: browser_action_index(target, "form", "form index")?,
            name: target
                .param("name")
                .ok_or_else(|| BrowserRouteError::BadRequest("missing field name".to_owned()))?,
            value: target.param("value").unwrap_or_default(),
        }),
        "select" => Ok(BrowserSessionAction::Select {
            form_index: browser_action_index(target, "form", "form index")?,
            control_index: browser_action_index(target, "control", "control index")?,
            value: target
                .param("value")
                .ok_or_else(|| BrowserRouteError::BadRequest("missing option value".to_owned()))?,
        }),
        "toggle" => Ok(BrowserSessionAction::Toggle {
            form_index: browser_action_index(target, "form", "form index")?,
            control_index: browser_action_index(target, "control", "control index")?,
        }),
        "submit" => Ok(BrowserSessionAction::Submit {
            form_index: browser_action_index(target, "form", "form index")?,
        }),
        _ => Err(BrowserRouteError::BadRequest(format!(
            "unknown browser action {action}"
        ))),
    }
}

#[derive(Debug, Clone, Copy)]
enum BrowserFindDirection {
    First,
    Next,
    Previous,
}

#[derive(Debug, Clone)]
enum BrowserSessionCloseScope {
    Others,
    LeftOfActive,
    RightOfActive,
    DuplicateSource(String),
}

#[derive(Debug, Clone, Copy)]
enum BrowserSessionSwitchDirection {
    Next,
    Previous,
}

fn browser_action_index(
    target: &RequestTarget,
    key: &str,
    label: &str,
) -> Result<usize, BrowserRouteError> {
    target
        .param(key)
        .ok_or_else(|| BrowserRouteError::BadRequest(format!("missing {label}")))?
        .parse::<usize>()
        .map_err(|_| BrowserRouteError::BadRequest(format!("invalid {label}")))
}

fn browser_action_url(target: &RequestTarget) -> Result<String, BrowserRouteError> {
    let url = target.param("url").unwrap_or_default();
    if url.trim().is_empty() {
        Err(BrowserRouteError::BadRequest(
            "missing browser URL".to_owned(),
        ))
    } else {
        Ok(url)
    }
}

fn browser_action_link_text(target: &RequestTarget) -> Result<String, BrowserRouteError> {
    let text = target
        .param("text")
        .or_else(|| target.param("q"))
        .or_else(|| target.param("label"))
        .unwrap_or_default();
    if text.trim().is_empty() {
        Err(BrowserRouteError::BadRequest(
            "missing link text".to_owned(),
        ))
    } else {
        Ok(text)
    }
}

fn browser_action_link_selector(target: &RequestTarget) -> Result<String, BrowserRouteError> {
    let selector = target.param("selector").unwrap_or_default();
    if selector.trim().is_empty() {
        Err(BrowserRouteError::BadRequest(
            "missing link selector".to_owned(),
        ))
    } else {
        Ok(selector)
    }
}

fn browser_action_records_profile_visit(action: &BrowserSessionAction) -> bool {
    matches!(
        action,
        BrowserSessionAction::Open(_)
            | BrowserSessionAction::Back
            | BrowserSessionAction::Forward
            | BrowserSessionAction::Link(_)
            | BrowserSessionAction::LinkText(_)
            | BrowserSessionAction::LinkSelector(_)
            | BrowserSessionAction::History(_)
            | BrowserSessionAction::ClickSelector(_)
            | BrowserSessionAction::ClickAt { .. }
            | BrowserSessionAction::Enter
            | BrowserSessionAction::OpenBookmark(_)
            | BrowserSessionAction::OpenProfileClosed(_)
            | BrowserSessionAction::Submit { .. }
    )
}

fn browser_action_records_profile_tabs(action: &BrowserSessionAction) -> bool {
    !matches!(action, BrowserSessionAction::ClearProfileTabs)
}

fn browser_bookmark_id(target: &RequestTarget) -> Result<String, BrowserRouteError> {
    let id = target
        .param("bookmark")
        .or_else(|| target.param("bookmark_id"))
        .or_else(|| target.param("target_bookmark"))
        .unwrap_or_default();
    if id.trim().is_empty() {
        Err(BrowserRouteError::BadRequest(
            "missing browser bookmark id".to_owned(),
        ))
    } else {
        Ok(id)
    }
}

fn browser_closed_session_id(target: &RequestTarget) -> Result<String, BrowserRouteError> {
    let id = target
        .param("closed_id")
        .or_else(|| target.param("closed_session"))
        .or_else(|| target.param("session"))
        .unwrap_or_default();
    if id.trim().is_empty() {
        Err(BrowserRouteError::BadRequest(
            "missing closed browser session id".to_owned(),
        ))
    } else {
        Ok(id)
    }
}

fn browser_profile_closed_index(target: &RequestTarget) -> Result<usize, BrowserRouteError> {
    target
        .param("closed")
        .or_else(|| target.param("closed_index"))
        .or_else(|| target.param("index"))
        .ok_or_else(|| BrowserRouteError::BadRequest("missing profile closed index".to_owned()))?
        .parse::<usize>()
        .map_err(|_| BrowserRouteError::BadRequest("invalid profile closed index".to_owned()))
}

fn apply_browser_history_entry(
    web_session: &mut BrowserWebSession,
    target_index: usize,
) -> Result<(), BrowserRouteError> {
    let history = web_session.session.snapshot();
    let current_index = history.current_index.ok_or_else(|| {
        BrowserRouteError::BadRequest("browser session has no history".to_owned())
    })?;
    if target_index >= history.entries.len() {
        return Err(BrowserRouteError::BadRequest(
            "history index out of range".to_owned(),
        ));
    }

    if target_index < current_index {
        for _ in target_index..current_index {
            web_session
                .session
                .back()
                .map_err(|error| BrowserRouteError::BadRequest(error.to_string()))?;
        }
    } else {
        for _ in current_index..target_index {
            web_session
                .session
                .forward()
                .map_err(|error| BrowserRouteError::BadRequest(error.to_string()))?;
        }
    }
    Ok(())
}

fn apply_browser_find(
    web_session: &mut BrowserWebSession,
    direction: BrowserFindDirection,
) -> Result<(), BrowserRouteError> {
    let query = web_session.find_query.trim();
    if query.is_empty() {
        return Err(BrowserRouteError::BadRequest(
            "missing browser find query".to_owned(),
        ));
    }

    let render = web_session.session.current().ok_or_else(|| {
        BrowserRouteError::BadRequest("browser session has no current page".to_owned())
    })?;
    let matches = browser_find_matches(&render.text, query);
    let current_line = web_session
        .find_active_line
        .unwrap_or(web_session.viewport_y);
    let Some(target_line) = browser_find_target_line(&matches, current_line, direction) else {
        clear_browser_find_active_line(web_session);
        return Ok(());
    };
    web_session.viewport_y = target_line;
    web_session.find_active_line = Some(target_line);
    Ok(())
}

fn browser_find_target_line(
    matches: &[usize],
    viewport_y: usize,
    direction: BrowserFindDirection,
) -> Option<usize> {
    match direction {
        BrowserFindDirection::First => matches.first().copied(),
        BrowserFindDirection::Next => matches
            .iter()
            .copied()
            .find(|line| *line > viewport_y)
            .or_else(|| matches.first().copied()),
        BrowserFindDirection::Previous => matches
            .iter()
            .rev()
            .copied()
            .find(|line| *line < viewport_y)
            .or_else(|| matches.last().copied()),
    }
}

fn browser_find_matches(text: &str, query: &str) -> Vec<usize> {
    let needle = query.trim().to_lowercase();
    if needle.is_empty() {
        return Vec::new();
    }
    text.lines()
        .enumerate()
        .filter_map(|(line, text)| text.to_lowercase().contains(&needle).then_some(line))
        .collect()
}

fn browser_find_visible_match(
    matches: &[usize],
    viewport_y: usize,
    viewport_height: usize,
) -> Option<(usize, usize)> {
    let viewport_end = viewport_y.saturating_add(viewport_height.max(1));
    matches
        .iter()
        .enumerate()
        .find(|(_, line)| **line >= viewport_y && **line < viewport_end)
        .map(|(index, line)| (index, *line))
}

fn browser_find_active_match(
    matches: &[usize],
    active_line: Option<usize>,
) -> Option<(usize, usize)> {
    let active_line = active_line?;
    matches
        .iter()
        .enumerate()
        .find(|(_, line)| **line == active_line)
        .map(|(index, line)| (index, *line))
}

fn clear_browser_find_active_line(web_session: &mut BrowserWebSession) {
    web_session.find_active_line = None;
}

fn browser_session_resource_report_from_fetch(
    report: BrowserResourceFetchReport,
) -> BrowserSessionResourceReportPayload {
    BrowserSessionResourceReportPayload {
        action: "Fetch resources".to_owned(),
        page_source: report.page_source,
        total: report.total,
        fetched: report.fetched,
        cached: report.cached,
        failed: report.failed,
        skipped: report.skipped,
        applied: None,
        decoded: None,
        resources: report
            .resources
            .into_iter()
            .map(browser_session_resource_fetch_payload)
            .collect(),
    }
}

fn browser_session_resource_report_from_stylesheets(
    report: BrowserStylesheetRenderReport,
) -> BrowserSessionResourceReportPayload {
    let (fetched, cached, failed, skipped) = browser_session_fetch_counts(&report.fetches);
    BrowserSessionResourceReportPayload {
        action: "Apply styles".to_owned(),
        page_source: report.page_source,
        total: report.stylesheet_count,
        fetched,
        cached,
        failed,
        skipped,
        applied: Some(report.applied),
        decoded: None,
        resources: report
            .fetches
            .into_iter()
            .map(browser_session_resource_fetch_payload)
            .collect(),
    }
}

fn browser_session_resource_report_from_scripts(
    report: BrowserScriptRenderReport,
) -> BrowserSessionResourceReportPayload {
    let (fetched, cached, failed, skipped) = browser_session_fetch_counts(&report.fetches);
    BrowserSessionResourceReportPayload {
        action: "Run scripts".to_owned(),
        page_source: report.page_source,
        total: report.script_count,
        fetched,
        cached,
        failed,
        skipped,
        applied: Some(report.applied),
        decoded: None,
        resources: report
            .fetches
            .into_iter()
            .map(browser_session_resource_fetch_payload)
            .collect(),
    }
}

fn browser_session_resource_report_from_images(
    report: BrowserImageRenderReport,
) -> BrowserSessionResourceReportPayload {
    let (fetched, cached, failed, skipped) = browser_session_fetch_counts(&report.fetches);
    BrowserSessionResourceReportPayload {
        action: "Load images".to_owned(),
        page_source: report.page_source,
        total: report.image_count,
        fetched,
        cached,
        failed,
        skipped,
        applied: None,
        decoded: Some(report.decoded),
        resources: report
            .fetches
            .into_iter()
            .map(browser_session_resource_fetch_payload)
            .collect(),
    }
}

fn browser_session_fetch_counts(
    resources: &[BrowserResourceFetch],
) -> (usize, usize, usize, usize) {
    let fetched = resources
        .iter()
        .filter(|resource| resource.status == "fetched")
        .count();
    let cached = resources
        .iter()
        .filter(|resource| resource.status == "cached")
        .count();
    let failed = resources
        .iter()
        .filter(|resource| resource.status == "failed")
        .count();
    let skipped = resources
        .iter()
        .filter(|resource| resource.status == "skipped")
        .count();
    (fetched, cached, failed, skipped)
}

fn browser_session_resource_fetch_payload(
    fetch: BrowserResourceFetch,
) -> BrowserSessionResourceFetchPayload {
    BrowserSessionResourceFetchPayload {
        kind: fetch.resource.kind,
        url: fetch.resource.url,
        resolved: fetch.resource.resolved,
        status: fetch.status,
        source: fetch.source,
        bytes: fetch.bytes,
        content_type: fetch.content_type,
        error: fetch.error,
    }
}

fn browser_session_payload(
    id: &str,
    web_session: &mut BrowserWebSession,
) -> Result<BrowserSessionPayload, BrowserRouteError> {
    let payload = {
        let render = web_session.session.current().ok_or_else(|| {
            BrowserRouteError::BadRequest("browser session has no current page".to_owned())
        })?;
        let viewport = browser_text_viewport(
            render,
            BrowserTextViewportOptions {
                x: web_session.viewport_x,
                y: web_session.viewport_y,
                width: web_session.width,
                height: web_session.height,
            },
        );
        let history = web_session.session.snapshot();
        let find_matches = browser_find_matches(&render.text, &web_session.find_query);
        let find_current = browser_find_active_match(&find_matches, web_session.find_active_line)
            .or_else(|| browser_find_visible_match(&find_matches, viewport.y, viewport.height));
        let can_back = history.current_index.is_some_and(|index| index > 0);
        let can_forward = history
            .current_index
            .is_some_and(|index| index + 1 < history.entries.len());
        let history_entries = history
            .entries
            .iter()
            .enumerate()
            .map(|(index, entry)| BrowserSessionHistoryEntryPayload {
                index,
                title: if entry.title.trim().is_empty() {
                    entry.source.clone()
                } else {
                    entry.title.clone()
                },
                source: entry.source.clone(),
                target: entry.target.clone(),
                action_url: browser_session_action_href(
                    id,
                    "history",
                    &[("history", index.to_string())],
                    web_session,
                ),
                new_session_url: browser_session_new_session_href(&entry.source, web_session),
                current: history.current_index == Some(index),
            })
            .collect::<Vec<_>>();
        let links = render
            .links
            .iter()
            .take(80)
            .enumerate()
            .map(|(index, link)| {
                let label = if link.text.trim().is_empty() {
                    link.resolved.clone()
                } else {
                    link.text.trim().to_owned()
                };
                BrowserSessionLinkPayload {
                    index,
                    label,
                    url: link.resolved.clone(),
                    action_url: browser_session_action_href(
                        id,
                        "link",
                        &[("link", index.to_string())],
                        web_session,
                    ),
                    new_session_url: browser_session_new_session_href(&link.resolved, web_session),
                }
            })
            .collect();
        let forms = render
            .forms
            .iter()
            .map(|form| BrowserSessionFormPayload {
                index: form.index,
                method: form.method.clone(),
                action: form.action.clone(),
                resolved_action: form.resolved_action.clone(),
                no_validate: form.no_validate,
                controls: form
                    .controls
                    .iter()
                    .enumerate()
                    .map(
                        |(control_index, control)| BrowserSessionFormControlPayload {
                            index: control_index,
                            name: control.name.clone(),
                            kind: control.kind.clone(),
                            value: control.value.clone(),
                            disabled: control.disabled,
                            required: control.required,
                            checked: control.checked,
                            options: control
                                .options
                                .iter()
                                .map(|option| BrowserSessionFormOptionPayload {
                                    value: option.value.clone(),
                                    label: option.label.clone(),
                                    disabled: option.disabled,
                                    selected: option.selected,
                                })
                                .collect(),
                            toggle_url: if form_control_is_checkable(&control.kind) {
                                Some(browser_session_action_href(
                                    id,
                                    "toggle",
                                    &[
                                        ("form", form.index.to_string()),
                                        ("control", control_index.to_string()),
                                    ],
                                    web_session,
                                ))
                            } else {
                                None
                            },
                        },
                    )
                    .collect(),
                submit_url: browser_session_action_href(
                    id,
                    "submit",
                    &[("form", form.index.to_string())],
                    web_session,
                ),
            })
            .collect::<Vec<_>>();

        BrowserSessionPayload {
            id: id.to_owned(),
            back_href: web_session.back_href.clone(),
            title: browser_session_title(render),
            source: render.source.clone(),
            width: viewport.width,
            height: viewport.height,
            max_bytes: web_session.max_bytes,
            viewport_x: viewport.x,
            viewport_y: viewport.y,
            document_width: viewport.document_width,
            document_height: viewport.document_height,
            max_scroll_x: viewport.max_scroll_x,
            max_scroll_y: viewport.max_scroll_y,
            dom_node_count: render.dom_node_count,
            link_count: render.links.len(),
            can_back,
            can_forward,
            history_len: history.entries.len(),
            current_history_index: history.current_index,
            profile_enabled: false,
            profile_error: None,
            current_bookmarked: false,
            bookmarks_clear_url: None,
            closed_sessions_clear_url: None,
            profile_tabs_clear_url: None,
            profile_history_clear_url: None,
            find_query: web_session.find_query.clone(),
            find_match_count: find_matches.len(),
            find_current_index: find_current.map(|(index, _)| index),
            find_current_line: find_current.map(|(_, line)| line),
            sessions: Vec::new(),
            closed_sessions: Vec::new(),
            bookmarks: Vec::new(),
            profile_history: Vec::new(),
            history: history_entries,
            viewport: viewport.lines.join("\n"),
            focused: web_session.session.focused_control(),
            links,
            form_count: render.forms.len(),
            forms,
            cookies: web_session.session.cookies_snapshot(),
            local_storage: web_session.session.local_storage_entries(),
            session_storage: web_session.session.session_storage_entries(),
            resource_count: render.resources.len(),
            resources: render.resources.iter().take(120).cloned().collect(),
            resource_report: web_session.resource_report.clone(),
        }
    };
    web_session.viewport_x = payload.viewport_x;
    web_session.viewport_y = payload.viewport_y;
    Ok(payload)
}

fn render_browser_session_page(payload: &BrowserSessionPayload, back_href: &str) -> String {
    let mut link_rows = String::new();
    for link in &payload.links {
        let _ = write!(
            link_rows,
            r#"<li><span>{index}</span><div class="link-body"><a href="{href}">{label}</a><div class="link-target">{url}</div><div class="link-actions"><a href="{href}">Open</a><a href="{new_href}">New session</a></div></div></li>"#,
            index = link.index + 1,
            href = html_escape::encode_double_quoted_attribute(&link.action_url),
            new_href = html_escape::encode_double_quoted_attribute(&link.new_session_url),
            label = html_escape::encode_text(&link.label),
            url = html_escape::encode_text(&link.url),
        );
    }
    if payload.link_count > payload.links.len() {
        let _ = write!(
            link_rows,
            r#"<li><span></span><div>{count} more links omitted</div></li>"#,
            count = payload.link_count - payload.links.len(),
        );
    }
    if link_rows.is_empty() {
        link_rows
            .push_str(r#"<li><span></span><div>No links found in this session page.</div></li>"#);
    }
    let link_controls = render_browser_session_link_controls(payload);
    let form_rows = render_browser_session_forms(payload);
    let click_controls = render_browser_session_click_controls(payload);
    let keyboard_controls = render_browser_session_keyboard_controls(payload);
    let inspector = render_browser_session_inspector(payload);
    let session_tabs = render_browser_session_tabs(payload);
    let closed_sessions = render_browser_session_closed_sessions(payload);
    let bookmarks = render_browser_session_bookmarks(payload);
    let profile_history = render_browser_session_profile_history(payload);
    let find_controls = render_browser_session_find_controls(payload);
    let viewport = render_browser_session_viewport(payload);

    let back_control = nav_control(
        payload.can_back,
        "Back",
        &browser_session_action_href(&payload.id, "back", &[], payload),
    );
    let forward_control = nav_control(
        payload.can_forward,
        "Forward",
        &browser_session_action_href(&payload.id, "forward", &[], payload),
    );
    let reload_href = browser_session_action_href(&payload.id, "reload", &[], payload);
    let duplicate_href = browser_session_new_session_href(&payload.source, payload);
    let previous_tab_href = browser_session_action_href(&payload.id, "previous-tab", &[], payload);
    let previous_tab_control =
        nav_control(payload.sessions.len() > 1, "Prev tab", &previous_tab_href);
    let next_tab_href = browser_session_action_href(&payload.id, "next-tab", &[], payload);
    let next_tab_control = nav_control(payload.sessions.len() > 1, "Next tab", &next_tab_href);
    let close_current_href = browser_session_action_href(
        &payload.id,
        "close-session",
        &[("close_id", payload.id.clone())],
        payload,
    );
    let close_current_control =
        nav_control(payload.sessions.len() > 1, "Close tab", &close_current_href);
    let close_others_href =
        browser_session_action_href(&payload.id, "close-other-tabs", &[], payload);
    let close_others_control = nav_control(
        payload.sessions.len() > 1,
        "Close others",
        &close_others_href,
    );
    let current_session_number = browser_session_id_number(&payload.id);
    let has_left_sessions = payload
        .sessions
        .iter()
        .any(|session| browser_session_id_number(&session.id) < current_session_number);
    let has_right_sessions = payload
        .sessions
        .iter()
        .any(|session| browser_session_id_number(&session.id) > current_session_number);
    let close_left_href = browser_session_action_href(&payload.id, "close-tabs-left", &[], payload);
    let close_left_control = nav_control(has_left_sessions, "Close left", &close_left_href);
    let close_right_href =
        browser_session_action_href(&payload.id, "close-tabs-right", &[], payload);
    let close_right_control = nav_control(has_right_sessions, "Close right", &close_right_href);
    let has_duplicate_sessions = !payload.source.trim().is_empty()
        && payload
            .sessions
            .iter()
            .any(|session| session.id != payload.id && session.source == payload.source);
    let close_duplicates_href =
        browser_session_action_href(&payload.id, "close-duplicate-tabs", &[], payload);
    let close_duplicates_control = nav_control(
        has_duplicate_sessions,
        "Close duplicates",
        &close_duplicates_href,
    );
    let restore_tab_href = payload
        .closed_sessions
        .first()
        .map(|closed| closed.restore_url.as_str())
        .unwrap_or_default();
    let restore_tab_control = nav_control(
        !payload.closed_sessions.is_empty(),
        "Restore tab",
        restore_tab_href,
    );
    let left_href = browser_session_action_href(
        &payload.id,
        "scroll",
        &[("dx", format!("-{}", payload.width.max(1) / 2))],
        payload,
    );
    let right_href = browser_session_action_href(
        &payload.id,
        "scroll",
        &[("dx", (payload.width.max(1) / 2).to_string())],
        payload,
    );
    let up_href = browser_session_action_href(
        &payload.id,
        "scroll",
        &[("dy", format!("-{}", payload.height.max(1) / 2))],
        payload,
    );
    let down_href = browser_session_action_href(
        &payload.id,
        "scroll",
        &[("dy", (payload.height.max(1) / 2).to_string())],
        payload,
    );
    let top_href = browser_session_action_href(&payload.id, "top", &[], payload);
    let bottom_href = browser_session_action_href(&payload.id, "bottom", &[], payload);

    format!(
        r#"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>{title}</title>
<style>
:root {{ color-scheme: light; font-family: ui-sans-serif, system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif; }}
body {{ margin: 0; background: #f7f7f5; color: #191a1c; }}
main {{ max-width: 1120px; margin: 0 auto; padding: 18px 18px 56px; }}
a {{ color: #123fae; text-decoration: none; font-weight: 700; overflow-wrap: anywhere; }}
a:hover {{ text-decoration: underline; }}
h1 {{ margin: 14px 0 6px; font-size: 24px; letter-spacing: 0; }}
h2 {{ margin: 24px 0 10px; font-size: 16px; letter-spacing: 0; }}
.toolbar {{ display: flex; align-items: center; flex-wrap: wrap; gap: 8px; margin-bottom: 10px; }}
.toolbar a, .toolbar span, .toolbar button {{ min-height: 32px; display: inline-flex; align-items: center; border: 1px solid #c6cbd2; border-radius: 6px; padding: 0 10px; background: #fff; color: #20242a; font-size: 13px; font-weight: 700; }}
.toolbar span {{ color: #8a929d; background: #eef0f3; }}
.toolbar form {{ display: flex; flex: 1 1 360px; min-width: 0; gap: 8px; }}
.toolbar input[type="url"] {{ flex: 1; min-width: 0; height: 32px; border: 1px solid #b7bdc5; border-radius: 6px; padding: 0 9px; font-size: 13px; background: #fff; }}
.toolbar button {{ cursor: pointer; background: #2457d6; color: #fff; border-color: #2457d6; }}
.find-bar {{ display: grid; grid-template-columns: minmax(0, 1fr) auto; gap: 8px; align-items: center; margin: 12px 0; }}
.find-bar form {{ display: grid; grid-template-columns: minmax(0, 1fr) auto; gap: 8px; min-width: 0; }}
.find-bar input[type="search"] {{ min-width: 0; height: 32px; border: 1px solid #b7bdc5; border-radius: 6px; padding: 0 9px; font-size: 13px; background: #fff; }}
.find-bar button, .find-bar a {{ min-height: 32px; display: inline-flex; align-items: center; border: 1px solid #c6cbd2; border-radius: 6px; padding: 0 10px; background: #fff; color: #20242a; font-size: 13px; font-weight: 700; cursor: pointer; }}
.find-bar button {{ background: #2457d6; color: #fff; border-color: #2457d6; }}
.find-actions {{ display: flex; flex-wrap: wrap; gap: 8px; align-items: center; justify-content: flex-end; }}
.session-shell {{ margin: 12px 0 16px; }}
.session-title {{ display: flex; align-items: baseline; justify-content: space-between; gap: 12px; margin-bottom: 8px; }}
.session-title h2 {{ margin: 0; }}
.session-tabs {{ display: grid; grid-template-columns: repeat(auto-fit, minmax(230px, 1fr)); gap: 8px; align-items: stretch; }}
.session-tab-card {{ min-width: 0; display: grid; grid-template-columns: minmax(0, 1fr) auto; gap: 8px; align-items: start; border: 1px solid #c6cbd2; border-radius: 6px; padding: 9px 10px; background: #fff; color: #20242a; }}
.session-tab-card.current {{ background: #191a1c; color: #fff; border-color: #191a1c; }}
.session-tab {{ min-width: 0; display: grid; gap: 3px; color: inherit; }}
.session-tab strong {{ display: block; min-width: 0; font-size: 13px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }}
.session-tab span {{ min-width: 0; color: inherit; opacity: 0.72; font-size: 11px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }}
.session-actions {{ display: grid; gap: 6px; justify-items: end; }}
.session-action {{ min-height: 24px; display: inline-flex; align-items: center; border: 1px solid #c6cbd2; border-radius: 6px; padding: 0 7px; background: #fff; color: #20242a; font-size: 12px; font-weight: 700; }}
.session-tab-card.current .session-action {{ border-color: #fff; }}
.session-new {{ display: grid; grid-template-columns: minmax(0, 1fr) auto; gap: 8px; border: 1px dashed #b7bdc5; border-radius: 6px; padding: 8px; background: #fff; }}
.session-new input[type="url"] {{ min-width: 0; height: 32px; border: 1px solid #b7bdc5; border-radius: 6px; padding: 0 9px; font-size: 13px; background: #fff; }}
.session-new button {{ min-height: 32px; border: 1px solid #2457d6; border-radius: 6px; padding: 0 10px; background: #2457d6; color: #fff; font-size: 13px; font-weight: 700; cursor: pointer; }}
.meta {{ color: #5d636b; font-size: 13px; overflow-wrap: anywhere; line-height: 1.45; }}
pre {{ white-space: pre-wrap; background: #fff; border: 1px solid #dfe2e6; border-radius: 6px; padding: 16px; line-height: 1.35; overflow: auto; font: 13px ui-monospace, SFMono-Regular, Menlo, Consolas, monospace; }}
pre mark {{ background: #ffe08a; color: inherit; border-radius: 2px; padding: 0 1px; }}
ol {{ list-style: none; margin: 0; padding: 0; }}
li {{ display: grid; grid-template-columns: 36px minmax(0, 1fr); gap: 8px 10px; padding: 10px 0; border-top: 1px solid #dfe2e6; }}
li span {{ color: #6b717a; font-size: 12px; padding-top: 3px; text-align: right; }}
li a {{ font-size: 14px; }}
li > div {{ grid-column: 2; color: #5d636b; font-size: 12px; overflow-wrap: anywhere; }}
.link-body {{ display: grid; gap: 4px; }}
.link-actions {{ display: flex; flex-wrap: wrap; gap: 8px; align-items: center; }}
.link-actions a {{ min-height: 28px; display: inline-flex; align-items: center; border: 1px solid #c6cbd2; border-radius: 6px; padding: 0 9px; background: #fff; color: #20242a; font-size: 12px; font-weight: 700; }}
.browser-forms {{ display: grid; gap: 12px; }}
.browser-form {{ background: #fff; border: 1px solid #dfe2e6; border-radius: 6px; padding: 12px; }}
.browser-form h3 {{ margin: 0 0 6px; font-size: 14px; letter-spacing: 0; }}
.browser-form .control {{ display: grid; grid-template-columns: 160px minmax(0, 1fr) auto; gap: 8px; align-items: center; padding: 8px 0; border-top: 1px solid #eef0f3; }}
.browser-form label {{ color: #3a3f45; font-size: 13px; font-weight: 700; overflow-wrap: anywhere; }}
.browser-form input[type="text"], .browser-form select {{ min-width: 0; height: 32px; border: 1px solid #b7bdc5; border-radius: 6px; padding: 0 9px; font-size: 13px; background: #fff; }}
.browser-form button, .browser-form .small-action {{ min-height: 32px; display: inline-flex; align-items: center; border: 1px solid #c6cbd2; border-radius: 6px; padding: 0 10px; background: #fff; color: #20242a; font-size: 13px; font-weight: 700; cursor: pointer; }}
.browser-form button.primary {{ background: #2457d6; color: #fff; border-color: #2457d6; }}
.browser-form .details {{ color: #5d636b; font-size: 12px; overflow-wrap: anywhere; }}
.browser-actions {{ display: grid; grid-template-columns: minmax(0, 1fr) minmax(280px, 360px); gap: 10px; margin: 12px 0 0; }}
.browser-action {{ display: grid; grid-template-columns: auto minmax(0, 1fr) auto; gap: 8px; align-items: center; }}
.browser-action label {{ color: #3a3f45; font-size: 13px; font-weight: 700; }}
.browser-action input {{ min-width: 0; height: 32px; border: 1px solid #b7bdc5; border-radius: 6px; padding: 0 9px; font-size: 13px; background: #fff; }}
.browser-action .point-inputs {{ min-width: 0; display: grid; grid-template-columns: 1fr 1fr; gap: 6px; }}
.browser-action button {{ min-height: 32px; border: 1px solid #2457d6; border-radius: 6px; padding: 0 10px; background: #2457d6; color: #fff; font-size: 13px; font-weight: 700; cursor: pointer; }}
.keyboard-actions {{ display: grid; gap: 10px; margin: 12px 0 0; }}
.keyboard-action-row {{ display: flex; flex-wrap: wrap; align-items: center; gap: 8px; }}
.keyboard-action-row a {{ min-height: 32px; display: inline-flex; align-items: center; border: 1px solid #c6cbd2; border-radius: 6px; padding: 0 10px; background: #fff; color: #20242a; font-size: 13px; font-weight: 700; }}
.browser-inspector {{ display: grid; gap: 14px; }}
.browser-inspector section {{ background: #fff; border: 1px solid #dfe2e6; border-radius: 6px; padding: 12px; }}
.browser-inspector h3 {{ margin: 0 0 8px; font-size: 14px; letter-spacing: 0; }}
.browser-inspector .section-title {{ display: flex; align-items: center; justify-content: space-between; gap: 10px; }}
.browser-inspector .clear-link {{ min-height: 28px; display: inline-flex; align-items: center; border: 1px solid #c6cbd2; border-radius: 6px; padding: 0 9px; background: #fff; color: #20242a; font-size: 12px; font-weight: 700; white-space: nowrap; }}
.resource-actions {{ display: flex; flex-wrap: wrap; gap: 6px; align-items: center; }}
.resource-report {{ display: grid; gap: 6px; margin: 8px 0 10px; color: #3a3f45; font-size: 12px; }}
.resource-report-summary {{ color: #5d636b; overflow-wrap: anywhere; }}
.browser-inspector table {{ width: 100%; border-collapse: collapse; table-layout: fixed; }}
.browser-inspector th, .browser-inspector td {{ border-top: 1px solid #eef0f3; padding: 7px 6px; color: #3a3f45; font-size: 12px; text-align: left; vertical-align: top; overflow-wrap: anywhere; }}
.browser-inspector th {{ color: #5d636b; font-weight: 700; }}
.browser-inspector .current-row td {{ background: #eef4ff; }}
@media (max-width: 720px) {{ .browser-actions {{ grid-template-columns: 1fr; }} .browser-action {{ grid-template-columns: 1fr; }} }}
</style>
</head>
<body>
<main>
<nav class="toolbar"><a href="{back_href}">Back to search</a>{back_control}{forward_control}<a href="{reload_href}">Reload</a>{previous_tab_control}{next_tab_control}<a href="{duplicate_href}">Duplicate tab</a>{close_current_control}{close_others_control}{close_left_control}{close_right_control}{close_duplicates_control}{restore_tab_control}<a href="{top_href}">Top</a>{left_control}<a href="{up_href}">Up</a><a href="{down_href}">Down</a>{right_control}<a href="{bottom_href}">Bottom</a></nav>
<form class="toolbar" action="/browser" method="get">
<input type="hidden" name="id" value="{id}">
<input type="hidden" name="from" value="{back_href}">
<input type="hidden" name="width" value="{width}">
<input type="hidden" name="height" value="{height}">
<input type="hidden" name="viewport_x" value="{viewport_x}">
<input type="hidden" name="max_bytes" value="{max_bytes}">
<input type="url" name="url" value="{source_attr}" aria-label="Address">
<button type="submit" name="action" value="open">Go</button><button type="submit" name="action" value="open-new-session">New tab</button>
</form>
{session_tabs}
{closed_sessions}
{bookmarks}
{profile_history}
<h1>{heading}</h1>
<div class="meta">{source}</div>
<div class="meta">rust browser session {id} · history {history_index}/{history_len} · viewport {width}x{height} at x={viewport_x} y={viewport_y} · max scroll {max_scroll_x}x{max_scroll_y} · document {doc_width}x{doc_height} · {nodes} DOM nodes · {links} links · {forms} forms</div>
{find_controls}
<pre>{viewport}</pre>
<h2>Click</h2>
<div class="browser-actions">{click_controls}</div>
<h2>Keyboard</h2>
<div class="keyboard-actions">{keyboard_controls}</div>
<h2>Forms</h2>
<div class="browser-forms">{form_rows}</div>
<h2>Inspector</h2>
<div class="browser-inspector">{inspector}</div>
<h2>Links</h2>
<div class="browser-actions">{link_controls}</div>
<ol>{link_rows}</ol>
</main>
</body>
</html>"#,
        title = html_escape::encode_text(&payload.title),
        heading = html_escape::encode_text(&payload.title),
        source = html_escape::encode_text(&payload.source),
        source_attr = html_escape::encode_double_quoted_attribute(&payload.source),
        id = html_escape::encode_double_quoted_attribute(&payload.id),
        back_href = html_escape::encode_double_quoted_attribute(back_href),
        back_control = back_control,
        forward_control = forward_control,
        reload_href = html_escape::encode_double_quoted_attribute(&reload_href),
        previous_tab_control = previous_tab_control,
        next_tab_control = next_tab_control,
        duplicate_href = html_escape::encode_double_quoted_attribute(&duplicate_href),
        close_current_control = close_current_control,
        close_others_control = close_others_control,
        close_left_control = close_left_control,
        close_right_control = close_right_control,
        close_duplicates_control = close_duplicates_control,
        restore_tab_control = restore_tab_control,
        left_control = nav_control(payload.viewport_x > 0, "Left", &left_href),
        right_control = nav_control(
            payload.viewport_x < payload.max_scroll_x,
            "Right",
            &right_href
        ),
        top_href = html_escape::encode_double_quoted_attribute(&top_href),
        up_href = html_escape::encode_double_quoted_attribute(&up_href),
        down_href = html_escape::encode_double_quoted_attribute(&down_href),
        bottom_href = html_escape::encode_double_quoted_attribute(&bottom_href),
        width = payload.width,
        height = payload.height,
        max_bytes = payload.max_bytes,
        viewport_x = payload.viewport_x,
        viewport_y = payload.viewport_y,
        max_scroll_x = payload.max_scroll_x,
        max_scroll_y = payload.max_scroll_y,
        doc_width = payload.document_width,
        doc_height = payload.document_height,
        nodes = payload.dom_node_count,
        links = payload.link_count,
        forms = payload.form_count,
        history_index = payload.current_history_index.map_or(0, |index| index + 1),
        history_len = payload.history_len,
        viewport = viewport,
        click_controls = click_controls,
        keyboard_controls = keyboard_controls,
        find_controls = find_controls,
        session_tabs = session_tabs,
        closed_sessions = closed_sessions,
        bookmarks = bookmarks,
        profile_history = profile_history,
        form_rows = form_rows,
        inspector = inspector,
        link_controls = link_controls,
        link_rows = link_rows,
    )
}

fn render_browser_session_find_controls(payload: &BrowserSessionPayload) -> String {
    let status = if payload.find_query.trim().is_empty() {
        "Find in page".to_owned()
    } else if payload.find_match_count == 0 {
        format!("0 matches for {}", payload.find_query)
    } else if let (Some(index), Some(line)) =
        (payload.find_current_index, payload.find_current_line)
    {
        format!(
            "{} of {} at line {}",
            index + 1,
            payload.find_match_count,
            line + 1
        )
    } else {
        format!("{} matches", payload.find_match_count)
    };
    let actions = if payload.find_query.trim().is_empty() {
        String::new()
    } else {
        let previous_href = browser_session_action_href(&payload.id, "find-prev", &[], payload);
        let next_href = browser_session_action_href(&payload.id, "find-next", &[], payload);
        let clear_href = browser_session_action_href(&payload.id, "clear-find", &[], payload);
        format!(
            r#"<a href="{previous_href}">Previous</a><a href="{next_href}">Next</a><a href="{clear_href}">Clear</a>"#,
            previous_href = html_escape::encode_double_quoted_attribute(&previous_href),
            next_href = html_escape::encode_double_quoted_attribute(&next_href),
            clear_href = html_escape::encode_double_quoted_attribute(&clear_href),
        )
    };

    format!(
        r#"<div class="find-bar"><form action="/browser" method="get">{common}<input type="hidden" name="action" value="find"><input type="search" name="q" value="{query}" aria-label="Find in page"><button type="submit">Find</button></form><div class="find-actions"><span class="meta">{status}</span>{actions}</div></div>"#,
        common = browser_session_common_hidden_inputs(payload),
        query = html_escape::encode_double_quoted_attribute(&payload.find_query),
        status = html_escape::encode_text(&status),
        actions = actions,
    )
}

fn render_browser_session_viewport(payload: &BrowserSessionPayload) -> String {
    render_browser_session_highlighted_text(&payload.viewport, &payload.find_query)
}

fn render_browser_session_highlighted_text(text: &str, query: &str) -> String {
    let query = query.trim();
    if query.is_empty() {
        return html_escape::encode_text(text).into_owned();
    }

    let mut output = String::new();
    let mut rest = text;
    while let Some(index) = find_ascii_case_insensitive(rest, query) {
        let Some(before) = rest.get(..index) else {
            break;
        };
        let Some(matched) = rest.get(index..index.saturating_add(query.len())) else {
            break;
        };
        output.push_str(&html_escape::encode_text(before));
        output.push_str("<mark>");
        output.push_str(&html_escape::encode_text(matched));
        output.push_str("</mark>");
        rest = &rest[index.saturating_add(query.len())..];
    }
    output.push_str(&html_escape::encode_text(rest));
    output
}

fn find_ascii_case_insensitive(haystack: &str, needle: &str) -> Option<usize> {
    if needle.is_empty() {
        return Some(0);
    }
    haystack.char_indices().find_map(|(index, _)| {
        haystack
            .get(index..index.saturating_add(needle.len()))
            .filter(|candidate| candidate.eq_ignore_ascii_case(needle))
            .map(|_| index)
    })
}

fn render_browser_session_tabs(payload: &BrowserSessionPayload) -> String {
    let mut tabs = String::new();
    for session in &payload.sessions {
        let class = if session.current {
            "session-tab-card current"
        } else {
            "session-tab-card"
        };
        let close = if session.can_close {
            format!(
                r#"<a class="session-action" href="{href}" aria-label="Close {id}">Close</a>"#,
                href = html_escape::encode_double_quoted_attribute(&session.close_url),
                id = html_escape::encode_double_quoted_attribute(&session.id),
            )
        } else {
            String::new()
        };
        let duplicate = format!(
            r#"<a class="session-action" href="{href}" aria-label="Duplicate {id}">Duplicate</a>"#,
            href = html_escape::encode_double_quoted_attribute(&session.duplicate_url),
            id = html_escape::encode_double_quoted_attribute(&session.id),
        );
        let reload = format!(
            r#"<a class="session-action" href="{href}" aria-label="Reload {id}">Reload</a>"#,
            href = html_escape::encode_double_quoted_attribute(&session.reload_url),
            id = html_escape::encode_double_quoted_attribute(&session.id),
        );
        let _ = write!(
            tabs,
            r#"<div class="{class}"><a class="session-tab" href="{href}"><strong>{id} · {title}</strong><span>{source}</span></a><div class="session-actions">{reload}{duplicate}{close}</div></div>"#,
            class = class,
            href = html_escape::encode_double_quoted_attribute(&session.action_url),
            id = html_escape::encode_text(&session.id),
            title = html_escape::encode_text(&session.title),
            source = html_escape::encode_text(&session.source),
            reload = reload,
            duplicate = duplicate,
            close = close,
        );
    }
    if tabs.is_empty() {
        tabs.push_str(r#"<span class="session-tab-card"><span class="session-tab"><strong>No sessions</strong><span>Open a URL to start.</span></span></span>"#);
    }
    let forget_saved = payload
        .profile_tabs_clear_url
        .as_ref()
        .map_or_else(String::new, |href| {
            nav_control(!payload.sessions.is_empty(), "Forget saved", href)
        });

    format!(
        r#"<section class="session-shell"><div class="session-title"><h2>Sessions</h2><div class="resource-actions"><span class="meta">{count} open</span>{forget_saved}</div></div><div class="session-tabs">{tabs}<form class="session-new" action="/browser" method="get"><input type="hidden" name="from" value="{back_href}"><input type="hidden" name="width" value="{width}"><input type="hidden" name="height" value="{height}"><input type="hidden" name="viewport_x" value="{viewport_x}"><input type="hidden" name="max_bytes" value="{max_bytes}"><input type="url" name="url" placeholder="New session URL" aria-label="New session URL"><button type="submit">New</button></form></div></section>"#,
        count = payload.sessions.len(),
        forget_saved = forget_saved,
        tabs = tabs,
        back_href = html_escape::encode_double_quoted_attribute(&payload.back_href),
        width = payload.width,
        height = payload.height,
        viewport_x = payload.viewport_x,
        max_bytes = payload.max_bytes,
    )
}

fn render_browser_session_closed_sessions(payload: &BrowserSessionPayload) -> String {
    if payload.closed_sessions.is_empty() {
        return String::new();
    }

    let mut rows = String::new();
    for closed in &payload.closed_sessions {
        let state = if closed.persisted { "saved" } else { "session" };
        let _ = write!(
            rows,
            r#"<div class="session-tab-card"><a class="session-tab" href="{restore_href}"><strong>{id} · {title}</strong><span>{state} · {closed_at} · {source}</span></a><div class="session-actions"><a class="session-action" href="{restore_href}">Restore</a><a class="session-action" href="{new_href}">New session</a><a class="session-action" href="{forget_href}">Forget</a></div></div>"#,
            restore_href = html_escape::encode_double_quoted_attribute(&closed.restore_url),
            new_href = html_escape::encode_double_quoted_attribute(&closed.new_session_url),
            forget_href = html_escape::encode_double_quoted_attribute(&closed.forget_url),
            id = html_escape::encode_text(&closed.id),
            title = html_escape::encode_text(&closed.title),
            state = state,
            closed_at = html_escape::encode_text(&closed.closed_at),
            source = html_escape::encode_text(&closed.source),
        );
    }
    let clear_control = payload
        .closed_sessions_clear_url
        .as_ref()
        .map_or_else(String::new, |href| {
            nav_control(!payload.closed_sessions.is_empty(), "Clear", href)
        });

    format!(
        r#"<section class="session-shell"><div class="session-title"><h2>Recently closed</h2><div class="resource-actions"><span class="meta">{count} closed</span>{clear_control}</div></div><div class="session-tabs">{rows}</div></section>"#,
        count = payload.closed_sessions.len(),
        clear_control = clear_control,
        rows = rows,
    )
}

fn render_browser_session_bookmarks(payload: &BrowserSessionPayload) -> String {
    let add_href = browser_session_action_href(&payload.id, "add-bookmark", &[], payload);
    let add_label = if payload.current_bookmarked {
        "Bookmarked"
    } else {
        "Add bookmark"
    };
    let add_control = nav_control(!payload.current_bookmarked, add_label, &add_href);
    let clear_control = payload
        .bookmarks_clear_url
        .as_ref()
        .map_or_else(String::new, |href| {
            nav_control(!payload.bookmarks.is_empty(), "Clear", href)
        });
    let mut rows = String::new();
    for bookmark in &payload.bookmarks {
        let class = if bookmark.current {
            "session-tab-card current"
        } else {
            "session-tab-card"
        };
        let _ = write!(
            rows,
            r#"<div class="{class}"><a class="session-tab" href="{href}"><strong>{id} · {title}</strong><span>{source}</span></a><div class="session-actions"><a class="session-action" href="{new_href}">New session</a><a class="session-action" href="{remove_href}">Remove</a></div></div>"#,
            class = class,
            href = html_escape::encode_double_quoted_attribute(&bookmark.action_url),
            id = html_escape::encode_text(&bookmark.id),
            title = html_escape::encode_text(&bookmark.title),
            source = html_escape::encode_text(&bookmark.source),
            new_href = html_escape::encode_double_quoted_attribute(&bookmark.new_session_url),
            remove_href = html_escape::encode_double_quoted_attribute(&bookmark.remove_url),
        );
    }
    if rows.is_empty() {
        rows.push_str(r#"<span class="session-tab-card"><span class="session-tab"><strong>No bookmarks</strong><span>Add the current page to keep it in this browser session.</span></span></span>"#);
    }

    format!(
        r#"<section class="session-shell"><div class="session-title"><h2>Bookmarks</h2><div class="resource-actions"><span class="meta">{count} saved</span>{add_control}{clear_control}</div></div><div class="session-tabs">{rows}</div></section>"#,
        count = payload.bookmarks.len(),
        add_control = add_control,
        clear_control = clear_control,
        rows = rows,
    )
}

fn render_browser_session_profile_history(payload: &BrowserSessionPayload) -> String {
    if !payload.profile_enabled {
        return String::new();
    }

    let error = payload
        .profile_error
        .as_ref()
        .map_or_else(String::new, |error| {
            format!(
                r#"<div class="meta">Profile error: {error}</div>"#,
                error = html_escape::encode_text(error),
            )
        });
    let mut rows = String::new();
    for entry in &payload.profile_history {
        let _ = write!(
            rows,
            r#"<div class="session-tab-card"><a class="session-tab" href="{href}"><strong>{index} · {title}</strong><span>{visited} · {source}</span></a><div class="session-actions"><a class="session-action" href="{new_href}">New session</a><a class="session-action" href="{remove_href}">Remove</a></div></div>"#,
            href = html_escape::encode_double_quoted_attribute(&entry.action_url),
            new_href = html_escape::encode_double_quoted_attribute(&entry.new_session_url),
            remove_href = html_escape::encode_double_quoted_attribute(&entry.remove_url),
            index = entry.index + 1,
            title = html_escape::encode_text(&entry.title),
            visited = html_escape::encode_text(&entry.visited_at),
            source = html_escape::encode_text(&entry.source),
        );
    }
    if rows.is_empty() {
        rows.push_str(r#"<span class="session-tab-card"><span class="session-tab"><strong>No profile history</strong><span>Visited pages will appear here.</span></span></span>"#);
    }
    let clear_control = payload
        .profile_history_clear_url
        .as_ref()
        .map_or_else(String::new, |href| {
            nav_control(!payload.profile_history.is_empty(), "Clear", href)
        });

    format!(
        r#"<section class="session-shell"><div class="session-title"><h2>Profile history</h2><div class="resource-actions"><span class="meta">{count} recent</span>{clear_control}</div></div>{error}<div class="session-tabs">{rows}</div></section>"#,
        count = payload.profile_history.len(),
        clear_control = clear_control,
        error = error,
        rows = rows,
    )
}

fn render_browser_session_click_controls(payload: &BrowserSessionPayload) -> String {
    format!(
        r##"<form class="browser-action" action="/browser" method="get">{common}<input type="hidden" name="action" value="click-selector"><label for="browser-selector">Selector</label><input id="browser-selector" type="text" name="selector" placeholder="#id, .class, button"><button type="submit">Click</button></form><form class="browser-action" action="/browser" method="get">{common}<input type="hidden" name="action" value="click-at"><label>Point</label><div class="point-inputs"><input type="number" min="0" name="x" value="0" aria-label="x"><input type="number" min="0" name="y" value="0" aria-label="y"></div><button type="submit">Click</button></form>"##,
        common = browser_session_common_hidden_inputs(payload),
    )
}

fn render_browser_session_link_controls(payload: &BrowserSessionPayload) -> String {
    format!(
        r##"<form class="browser-action" action="/browser" method="get">{common}<input type="hidden" name="action" value="link-text"><label for="browser-link-text">Text</label><input id="browser-link-text" type="text" name="text" placeholder="Visible text"><button type="submit">Open</button></form><form class="browser-action" action="/browser" method="get">{common}<input type="hidden" name="action" value="link-text-new-session"><label for="browser-link-text-new-session">Text</label><input id="browser-link-text-new-session" type="text" name="text" placeholder="Visible text"><button type="submit">New session</button></form><form class="browser-action" action="/browser" method="get">{common}<input type="hidden" name="action" value="link-selector"><label for="browser-link-selector">Selector</label><input id="browser-link-selector" type="text" name="selector" placeholder="#link, a.primary"><button type="submit">Open</button></form><form class="browser-action" action="/browser" method="get">{common}<input type="hidden" name="action" value="link-selector-new-session"><label for="browser-link-selector-new-session">Selector</label><input id="browser-link-selector-new-session" type="text" name="selector" placeholder="#link, a.primary"><button type="submit">New session</button></form>"##,
        common = browser_session_common_hidden_inputs(payload),
    )
}

fn render_browser_session_keyboard_controls(payload: &BrowserSessionPayload) -> String {
    let focused = payload.focused.as_ref().map_or_else(
        || "Focused: none".to_owned(),
        |focused| {
            format!(
                "Focused: form {} control {} {} name={} value={}",
                focused.form_index,
                focused.control_index,
                focused.kind,
                focused.name,
                focused.value
            )
        },
    );
    let tab_href = browser_session_action_href(&payload.id, "focus-next", &[], payload);
    let shift_tab_href = browser_session_action_href(&payload.id, "focus-prev", &[], payload);
    let backspace_href = browser_session_action_href(
        &payload.id,
        "backspace",
        &[("count", "1".to_owned())],
        payload,
    );
    let clear_href = browser_session_action_href(&payload.id, "clear-input", &[], payload);
    let enter_href = browser_session_action_href(&payload.id, "enter", &[], payload);
    let space_href = browser_session_action_href(&payload.id, "space", &[], payload);

    format!(
        r##"<div class="meta">{focused}</div><div class="browser-actions"><form class="browser-action" action="/browser" method="get">{common}<input type="hidden" name="action" value="focus-selector"><label for="browser-focus-selector">Focus</label><input id="browser-focus-selector" type="text" name="selector" placeholder="#field, label, button"><button type="submit">Focus</button></form><form class="browser-action" action="/browser" method="get">{common}<input type="hidden" name="action" value="type-text"><label for="browser-type-text">Type</label><input id="browser-type-text" type="text" name="text" placeholder="text"><button type="submit">Type</button></form><form class="browser-action" action="/browser" method="get">{common}<input type="hidden" name="action" value="choose"><label for="browser-choose-value">Choose</label><input id="browser-choose-value" type="text" name="value" placeholder="option value"><button type="submit">Choose</button></form></div><div class="keyboard-action-row"><a href="{tab_href}">Tab</a><a href="{shift_tab_href}">Shift Tab</a><a href="{backspace_href}">Backspace</a><a href="{clear_href}">Clear Input</a><a href="{enter_href}">Enter</a><a href="{space_href}">Space</a></div>"##,
        focused = html_escape::encode_text(&focused),
        common = browser_session_common_hidden_inputs(payload),
        tab_href = html_escape::encode_double_quoted_attribute(&tab_href),
        shift_tab_href = html_escape::encode_double_quoted_attribute(&shift_tab_href),
        backspace_href = html_escape::encode_double_quoted_attribute(&backspace_href),
        clear_href = html_escape::encode_double_quoted_attribute(&clear_href),
        enter_href = html_escape::encode_double_quoted_attribute(&enter_href),
        space_href = html_escape::encode_double_quoted_attribute(&space_href),
    )
}

fn render_browser_session_inspector(payload: &BrowserSessionPayload) -> String {
    let history = render_browser_session_history(payload);
    let cookies = render_browser_session_cookies(payload);
    let local_storage = render_browser_session_storage(
        "localStorage",
        &payload.local_storage,
        Some(&browser_session_action_href(
            &payload.id,
            "clear-local-storage",
            &[],
            payload,
        )),
    );
    let session_storage = render_browser_session_storage(
        "sessionStorage",
        &payload.session_storage,
        Some(&browser_session_action_href(
            &payload.id,
            "clear-session-storage",
            &[],
            payload,
        )),
    );
    let resources = render_browser_session_resources(payload);
    format!("{history}{cookies}{local_storage}{session_storage}{resources}")
}

fn render_browser_session_history(payload: &BrowserSessionPayload) -> String {
    let mut rows = String::new();
    for entry in &payload.history {
        let row_class = if entry.current {
            r#" class="current-row""#
        } else {
            ""
        };
        let marker = if entry.current { "current" } else { "" };
        let _ = write!(
            rows,
            r#"<tr{row_class}><td>{index}</td><td>{marker}</td><td>{title}</td><td>{source}</td><td>{target}</td><td><div class="resource-actions"><a class="clear-link" href="{href}">Open</a><a class="clear-link" href="{new_href}">New session</a></div></td></tr>"#,
            row_class = row_class,
            index = entry.index + 1,
            marker = marker,
            title = html_escape::encode_text(&entry.title),
            source = html_escape::encode_text(&entry.source),
            target = html_escape::encode_text(&entry.target),
            href = html_escape::encode_double_quoted_attribute(&entry.action_url),
            new_href = html_escape::encode_double_quoted_attribute(&entry.new_session_url),
        );
    }
    if rows.is_empty() {
        rows.push_str(r#"<tr><td colspan="6">No browser session history.</td></tr>"#);
    }
    format!(
        r#"<section><h3>History</h3><table><thead><tr><th>#</th><th>State</th><th>Title</th><th>Source</th><th>Target</th><th>Action</th></tr></thead><tbody>{rows}</tbody></table></section>"#,
    )
}

fn render_browser_session_cookies(payload: &BrowserSessionPayload) -> String {
    let clear_href = browser_session_action_href(&payload.id, "clear-cookies", &[], payload);
    let mut rows = String::new();
    for cookie in &payload.cookies {
        let flags = browser_cookie_flags(cookie);
        let _ = write!(
            rows,
            r#"<tr><td>{name}</td><td>{value}</td><td>{domain}</td><td>{path}</td><td>{flags}</td></tr>"#,
            name = html_escape::encode_text(&cookie.name),
            value = html_escape::encode_text(&cookie.value),
            domain = html_escape::encode_text(&cookie.domain),
            path = html_escape::encode_text(&cookie.path),
            flags = html_escape::encode_text(&flags),
        );
    }
    if rows.is_empty() {
        rows.push_str(r#"<tr><td colspan="5">No cookies stored in this session.</td></tr>"#);
    }
    format!(
        r#"<section><div class="section-title"><h3>Cookies ({count})</h3><a class="clear-link" href="{clear_href}">Clear</a></div><table><thead><tr><th>Name</th><th>Value</th><th>Domain</th><th>Path</th><th>Flags</th></tr></thead><tbody>{rows}</tbody></table></section>"#,
        count = payload.cookies.len(),
        clear_href = html_escape::encode_double_quoted_attribute(&clear_href),
    )
}

fn render_browser_session_storage(
    label: &str,
    entries: &[BrowserLocalStorageEntry],
    clear_href: Option<&str>,
) -> String {
    let mut rows = String::new();
    for entry in entries {
        let _ = write!(
            rows,
            r#"<tr><td>{origin}</td><td>{key}</td><td>{value}</td></tr>"#,
            origin = html_escape::encode_text(&entry.origin),
            key = html_escape::encode_text(&entry.key),
            value = html_escape::encode_text(&entry.value),
        );
    }
    if rows.is_empty() {
        let _ = write!(
            rows,
            r#"<tr><td colspan="3">No {label} entries stored in this session.</td></tr>"#,
            label = html_escape::encode_text(label),
        );
    }
    let clear = clear_href.map_or_else(String::new, |href| {
        format!(
            r#"<a class="clear-link" href="{href}">Clear</a>"#,
            href = html_escape::encode_double_quoted_attribute(href),
        )
    });
    format!(
        r#"<section><div class="section-title"><h3>{label} ({count})</h3>{clear}</div><table><thead><tr><th>Origin</th><th>Key</th><th>Value</th></tr></thead><tbody>{rows}</tbody></table></section>"#,
        label = html_escape::encode_text(label),
        count = entries.len(),
        clear = clear,
    )
}

fn render_browser_session_resources(payload: &BrowserSessionPayload) -> String {
    let fetch_href = browser_session_action_href(&payload.id, "fetch-resources", &[], payload);
    let styles_href = browser_session_action_href(&payload.id, "apply-styles", &[], payload);
    let scripts_href = browser_session_action_href(&payload.id, "run-scripts", &[], payload);
    let images_href = browser_session_action_href(&payload.id, "load-images", &[], payload);
    let clear_report = payload
        .resource_report
        .as_ref()
        .map_or_else(String::new, |_| {
            let clear_href =
                browser_session_action_href(&payload.id, "clear-resource-report", &[], payload);
            format!(
                r#"<a class="clear-link" href="{clear_href}">Clear report</a>"#,
                clear_href = html_escape::encode_double_quoted_attribute(&clear_href),
            )
        });
    let report = render_browser_session_resource_report(payload.resource_report.as_ref());
    let mut rows = String::new();
    for resource in &payload.resources {
        let detail = browser_resource_detail(resource);
        let open_href = browser_session_action_href(
            &payload.id,
            "open",
            &[("url", resource.resolved.clone())],
            payload,
        );
        let new_href = browser_session_new_session_href(&resource.resolved, payload);
        let _ = write!(
            rows,
            r#"<tr><td>{kind}</td><td>{initiator}</td><td>{url}</td><td>{resolved}</td><td>{detail}</td><td><div class="resource-actions"><a class="clear-link" href="{open_href}">Open</a><a class="clear-link" href="{new_href}">New session</a></div></td></tr>"#,
            kind = html_escape::encode_text(&resource.kind),
            initiator = html_escape::encode_text(&resource.initiator),
            url = html_escape::encode_text(&resource.url),
            resolved = html_escape::encode_text(&resource.resolved),
            detail = html_escape::encode_text(&detail),
            open_href = html_escape::encode_double_quoted_attribute(&open_href),
            new_href = html_escape::encode_double_quoted_attribute(&new_href),
        );
    }
    if payload.resource_count > payload.resources.len() {
        let _ = write!(
            rows,
            r#"<tr><td colspan="6">{count} more resources omitted.</td></tr>"#,
            count = payload.resource_count - payload.resources.len(),
        );
    }
    if rows.is_empty() {
        rows.push_str(r#"<tr><td colspan="6">No subresources discovered.</td></tr>"#);
    }
    format!(
        r#"<section><div class="section-title"><h3>Resources ({count})</h3><div class="resource-actions"><a class="clear-link" href="{fetch_href}">Fetch</a><a class="clear-link" href="{styles_href}">Apply styles</a><a class="clear-link" href="{scripts_href}">Run scripts</a><a class="clear-link" href="{images_href}">Load images</a>{clear_report}</div></div>{report}<table><thead><tr><th>Kind</th><th>Initiator</th><th>URL</th><th>Resolved</th><th>Details</th><th>Action</th></tr></thead><tbody>{rows}</tbody></table></section>"#,
        count = payload.resource_count,
        fetch_href = html_escape::encode_double_quoted_attribute(&fetch_href),
        styles_href = html_escape::encode_double_quoted_attribute(&styles_href),
        scripts_href = html_escape::encode_double_quoted_attribute(&scripts_href),
        images_href = html_escape::encode_double_quoted_attribute(&images_href),
        clear_report = clear_report,
        report = report,
    )
}

fn render_browser_session_resource_report(
    report: Option<&BrowserSessionResourceReportPayload>,
) -> String {
    let Some(report) = report else {
        return String::new();
    };

    let mut status = format!(
        "{}: total={} fetched={} cached={} failed={} skipped={}",
        report.action, report.total, report.fetched, report.cached, report.failed, report.skipped
    );
    if let Some(applied) = report.applied {
        let _ = write!(status, " applied={applied}");
    }
    if let Some(decoded) = report.decoded {
        let _ = write!(status, " decoded={decoded}");
    }

    let mut rows = String::new();
    for resource in report.resources.iter().take(20) {
        let detail = resource
            .error
            .as_deref()
            .or(resource.content_type.as_deref())
            .unwrap_or("-");
        let _ = write!(
            rows,
            r#"<tr><td>{status}</td><td>{kind}</td><td>{bytes}</td><td>{url}</td><td>{detail}</td></tr>"#,
            status = html_escape::encode_text(&resource.status),
            kind = html_escape::encode_text(&resource.kind),
            bytes = resource.bytes,
            url = html_escape::encode_text(&resource.resolved),
            detail = html_escape::encode_text(detail),
        );
    }
    if report.resources.len() > 20 {
        let _ = write!(
            rows,
            r#"<tr><td colspan="5">{count} more resource results omitted.</td></tr>"#,
            count = report.resources.len() - 20,
        );
    }
    if rows.is_empty() {
        rows.push_str(r#"<tr><td colspan="5">No resource results.</td></tr>"#);
    }

    format!(
        r#"<div class="resource-report"><div class="resource-report-summary">{summary}</div><table><thead><tr><th>Status</th><th>Kind</th><th>Bytes</th><th>Resolved</th><th>Detail</th></tr></thead><tbody>{rows}</tbody></table></div>"#,
        summary = html_escape::encode_text(&status),
        rows = rows,
    )
}

fn browser_cookie_flags(cookie: &BrowserCookie) -> String {
    let mut flags = Vec::new();
    if cookie.secure {
        flags.push("secure");
    }
    if cookie.http_only {
        flags.push("httpOnly");
    }
    if cookie.host_only {
        flags.push("hostOnly");
    }
    if flags.is_empty() {
        "-".to_owned()
    } else {
        flags.join(", ")
    }
}

fn browser_resource_detail(resource: &BrowserResource) -> String {
    let mut details = Vec::new();
    if let Some(rel) = resource.rel.as_deref().filter(|value| !value.is_empty()) {
        details.push(format!("rel={rel}"));
    }
    if let Some(media) = resource.media.as_deref().filter(|value| !value.is_empty()) {
        details.push(format!("media={media}"));
    }
    if let Some(alt) = resource.alt.as_deref().filter(|value| !value.is_empty()) {
        details.push(format!("alt={alt}"));
    }
    if let Some(type_hint) = resource
        .type_hint
        .as_deref()
        .filter(|value| !value.is_empty())
    {
        details.push(format!("type={type_hint}"));
    }
    if details.is_empty() {
        "-".to_owned()
    } else {
        details.join(" · ")
    }
}

fn render_browser_session_forms(payload: &BrowserSessionPayload) -> String {
    if payload.forms.is_empty() {
        return r#"<div class="meta">No forms found in this session page.</div>"#.to_owned();
    }

    let mut rows = String::new();
    for form in &payload.forms {
        let _ = write!(
            rows,
            r#"<section class="browser-form"><h3>Form {index}</h3><div class="details">{method} · {action}</div>"#,
            index = form.index,
            method = html_escape::encode_text(&form.method.to_ascii_uppercase()),
            action = html_escape::encode_text(&form.resolved_action),
        );

        for control in &form.controls {
            rows.push_str(&render_browser_session_control(payload, form, control));
        }

        let _ = write!(
            rows,
            r#"<div class="control"><label>Submit</label><div class="details">{method} {target}</div><a class="small-action" href="{href}">Submit form</a></div></section>"#,
            method = html_escape::encode_text(&form.method.to_ascii_uppercase()),
            target = html_escape::encode_text(&form.resolved_action),
            href = html_escape::encode_double_quoted_attribute(&form.submit_url),
        );
    }
    rows
}

fn render_browser_session_control(
    payload: &BrowserSessionPayload,
    form: &BrowserSessionFormPayload,
    control: &BrowserSessionFormControlPayload,
) -> String {
    let label = browser_form_control_label(control);
    if form_control_is_checkable(&control.kind) {
        let state = if control.checked {
            "checked"
        } else {
            "unchecked"
        };
        let disabled = if control.disabled { " disabled" } else { "" };
        let href = control.toggle_url.as_deref().unwrap_or("#");
        return format!(
            r#"<div class="control"><label>{label}</label><div class="details">{kind} · {state}{disabled}</div><a class="small-action" href="{href}">Toggle</a></div>"#,
            label = html_escape::encode_text(&label),
            kind = html_escape::encode_text(&control.kind),
            state = state,
            disabled = disabled,
            href = html_escape::encode_double_quoted_attribute(href),
        );
    }

    if !control.options.is_empty() {
        let mut options = String::new();
        for option in &control.options {
            let selected = if option.selected { " selected" } else { "" };
            let disabled = if option.disabled { " disabled" } else { "" };
            let _ = write!(
                options,
                r#"<option value="{value}"{selected}{disabled}>{label}</option>"#,
                value = html_escape::encode_double_quoted_attribute(&option.value),
                selected = selected,
                disabled = disabled,
                label = html_escape::encode_text(&option.label),
            );
        }
        return format!(
            r#"<form class="control" action="/browser" method="get">{common}<input type="hidden" name="action" value="select"><input type="hidden" name="form" value="{form_index}"><input type="hidden" name="control" value="{control_index}"><label>{label}</label><select name="value">{options}</select><button type="submit">Set</button></form>"#,
            common = browser_session_common_hidden_inputs(payload),
            form_index = form.index,
            control_index = control.index,
            label = html_escape::encode_text(&label),
            options = options,
        );
    }

    if form_control_is_fillable(&control.kind) && !control.name.is_empty() && !control.disabled {
        return format!(
            r#"<form class="control" action="/browser" method="get">{common}<input type="hidden" name="action" value="fill"><input type="hidden" name="form" value="{form_index}"><input type="hidden" name="name" value="{name_attr}"><label>{label}</label><input type="text" name="value" value="{value}"><button type="submit">Set</button></form>"#,
            common = browser_session_common_hidden_inputs(payload),
            form_index = form.index,
            name_attr = html_escape::encode_double_quoted_attribute(&control.name),
            label = html_escape::encode_text(&label),
            value = html_escape::encode_double_quoted_attribute(&control.value),
        );
    }

    format!(
        r#"<div class="control"><label>{label}</label><div class="details">{kind} · {value}</div><span class="details">read-only</span></div>"#,
        label = html_escape::encode_text(&label),
        kind = html_escape::encode_text(&control.kind),
        value = html_escape::encode_text(&control.value),
    )
}

fn browser_session_common_hidden_inputs(payload: &BrowserSessionPayload) -> String {
    format!(
        r#"<input type="hidden" name="id" value="{id}"><input type="hidden" name="from" value="{back_href}"><input type="hidden" name="width" value="{width}"><input type="hidden" name="height" value="{height}"><input type="hidden" name="viewport_x" value="{viewport_x}"><input type="hidden" name="max_bytes" value="{max_bytes}">"#,
        id = html_escape::encode_double_quoted_attribute(&payload.id),
        back_href = html_escape::encode_double_quoted_attribute(&payload.back_href),
        width = payload.width,
        height = payload.height,
        viewport_x = payload.viewport_x,
        max_bytes = payload.max_bytes,
    )
}

fn browser_form_control_label(control: &BrowserSessionFormControlPayload) -> String {
    if control.name.trim().is_empty() {
        format!("{} {}", control.kind, control.index)
    } else {
        control.name.clone()
    }
}

fn form_control_is_checkable(kind: &str) -> bool {
    kind.eq_ignore_ascii_case("checkbox") || kind.eq_ignore_ascii_case("radio")
}

fn form_control_is_fillable(kind: &str) -> bool {
    !matches!(
        kind.to_ascii_lowercase().as_str(),
        "submit" | "button" | "reset" | "image" | "file" | "checkbox" | "radio"
    )
}

fn browser_session_action_href<T: BrowserSessionHrefSource>(
    id: &str,
    action: &str,
    extra: &[(&str, String)],
    source: &T,
) -> String {
    let mut query = form_urlencoded::Serializer::new(String::new());
    query.append_pair("id", id);
    query.append_pair("action", action);
    query.append_pair("from", source.back_href());
    query.append_pair("width", &source.width().to_string());
    query.append_pair("height", &source.height().to_string());
    query.append_pair("viewport_x", &source.viewport_x().to_string());
    query.append_pair("max_bytes", &source.max_bytes().to_string());
    for (key, value) in extra {
        query.append_pair(key, value);
    }
    format!("/browser?{}", query.finish())
}

fn browser_session_new_session_href<T: BrowserSessionHrefSource>(url: &str, source: &T) -> String {
    let mut query = form_urlencoded::Serializer::new(String::new());
    query.append_pair("url", url);
    query.append_pair("from", source.back_href());
    query.append_pair("width", &source.width().to_string());
    query.append_pair("height", &source.height().to_string());
    query.append_pair("viewport_x", &source.viewport_x().to_string());
    query.append_pair("max_bytes", &source.max_bytes().to_string());
    format!("/browser?{}", query.finish())
}

trait BrowserSessionHrefSource {
    fn back_href(&self) -> &str;
    fn width(&self) -> usize;
    fn height(&self) -> usize;
    fn viewport_x(&self) -> usize;
    fn max_bytes(&self) -> usize;
}

impl BrowserSessionHrefSource for BrowserWebSession {
    fn back_href(&self) -> &str {
        &self.back_href
    }

    fn width(&self) -> usize {
        self.width
    }

    fn height(&self) -> usize {
        self.height
    }

    fn viewport_x(&self) -> usize {
        self.viewport_x
    }

    fn max_bytes(&self) -> usize {
        self.max_bytes
    }
}

impl BrowserSessionHrefSource for BrowserSessionPayload {
    fn back_href(&self) -> &str {
        &self.back_href
    }

    fn width(&self) -> usize {
        self.width
    }

    fn height(&self) -> usize {
        self.height
    }

    fn viewport_x(&self) -> usize {
        self.viewport_x
    }

    fn max_bytes(&self) -> usize {
        self.max_bytes
    }
}

fn nav_control(enabled: bool, label: &str, href: &str) -> String {
    if enabled {
        format!(
            r#"<a href="{href}">{label}</a>"#,
            href = html_escape::encode_double_quoted_attribute(href),
            label = html_escape::encode_text(label),
        )
    } else {
        format!(
            r#"<span>{label}</span>"#,
            label = html_escape::encode_text(label),
        )
    }
}

fn browser_session_title(render: &crate::browser::BrowserRender) -> String {
    if render.title.trim().is_empty() {
        render.source.clone()
    } else {
        render.title.clone()
    }
}

fn current_session_source(web_session: &BrowserWebSession) -> Option<String> {
    web_session
        .session
        .current()
        .map(|render| render.source.clone())
}

fn reset_viewport_to_fragment(web_session: &mut BrowserWebSession) {
    web_session.viewport_y = web_session
        .session
        .current()
        .and_then(|render| render.source_fragment_scroll_y())
        .unwrap_or(0);
}

fn reset_viewport_after_navigation(web_session: &mut BrowserWebSession) {
    web_session.viewport_x = 0;
    web_session.resource_report = None;
    reset_viewport_to_fragment(web_session);
}

fn apply_scroll_delta(current: usize, delta: isize) -> usize {
    if delta.is_negative() {
        current.saturating_sub(delta.unsigned_abs())
    } else {
        current.saturating_add(delta as usize)
    }
}

fn parse_usize_param(
    target: &RequestTarget,
    key: &str,
    default: usize,
    min: usize,
    max: usize,
) -> usize {
    parse_optional_usize_param(target, key, min, max).unwrap_or(default)
}

fn parse_optional_usize_param(
    target: &RequestTarget,
    key: &str,
    min: usize,
    max: usize,
) -> Option<usize> {
    target
        .param(key)
        .and_then(|value| value.parse::<usize>().ok())
        .map(|value| value.clamp(min, max))
}

#[cfg(test)]
mod tests;
