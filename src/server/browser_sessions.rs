use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use tokio::sync::{Mutex, MutexGuard, Notify};
use tokio::time::timeout;
use url::{Url, form_urlencoded};

use crate::browser::{
    BROWSER_ABOUT_BLANK_TARGET, BrowserCookie, BrowserFocusedControl, BrowserForm,
    BrowserImageRenderReport, BrowserLocalStorageEntry, BrowserRasterOptions, BrowserRender,
    BrowserRenderOptions, BrowserResource, BrowserResourceFetch, BrowserResourceFetchReport,
    BrowserScriptRenderReport, BrowserSession, BrowserStylesheetRenderReport,
    BrowserTextViewportOptions, BrowserViewportState, browser_text_viewport, rasterize_render_rgba,
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
const MAX_VISIBLE_BROWSER_PROFILE_HISTORY: usize = 40;
const DEFAULT_BULK_BACKGROUND_LINKS: usize = 16;
const MAX_BULK_BACKGROUND_LINKS: usize = 80;
const MAX_BROWSER_SESSION_RESOURCES: usize = 120;
const BROWSER_PROFILE_ENV: &str = "BRUTAL_BROWSER_PROFILE";
#[cfg(not(test))]
const BROWSER_CREATE_TARGET_TIMEOUT: Duration = Duration::from_secs(8);
#[cfg(test)]
const BROWSER_CREATE_TARGET_TIMEOUT: Duration = Duration::from_millis(100);

pub(super) struct BrowserSessionRegistry {
    next_id: AtomicU64,
    next_bookmark_id: AtomicU64,
    profile_path: Option<PathBuf>,
    profile_error: Mutex<Option<String>>,
    sessions: Mutex<HashMap<String, BrowserWebSession>>,
    in_flight_sessions: Mutex<HashMap<String, Arc<Notify>>>,
    in_flight_viewports: Mutex<HashMap<String, BrowserWebSession>>,
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
            in_flight_sessions: Mutex::new(HashMap::new()),
            in_flight_viewports: Mutex::new(HashMap::new()),
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
            in_flight_sessions: Mutex::new(HashMap::new()),
            in_flight_viewports: Mutex::new(HashMap::new()),
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
    tab_order: u64,
    width: usize,
    height: usize,
    max_bytes: usize,
    viewport_x: usize,
    viewport_y: usize,
    back_href: String,
    find_query: String,
    find_active_line: Option<usize>,
    tab_search_query: String,
    resource_report: Option<BrowserSessionResourceReportPayload>,
    action_feedback: Option<String>,
    pending_source: Option<String>,
    display_source: Option<String>,
    pinned: bool,
    tab_label: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
struct BrowserSessionPayload {
    id: String,
    back_href: String,
    title: String,
    source: String,
    #[serde(skip)]
    rendered_source: String,
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
    anchor_count: usize,
    can_back: bool,
    can_forward: bool,
    history_len: usize,
    current_history_index: Option<usize>,
    profile_enabled: bool,
    profile_error: Option<String>,
    current_bookmarked: bool,
    bookmarks_clear_url: Option<String>,
    bookmarks_background_url: Option<String>,
    links_background_url: Option<String>,
    closed_sessions_clear_url: Option<String>,
    profile_tabs_clear_url: Option<String>,
    profile_history_clear_url: Option<String>,
    find_query: String,
    find_match_count: usize,
    find_current_index: Option<usize>,
    find_current_line: Option<usize>,
    find_current_column: Option<usize>,
    find_matches: Vec<BrowserSessionFindMatchPayload>,
    tab_search_query: String,
    tab_search_results: Vec<BrowserSessionTabSearchResultPayload>,
    sessions: Vec<BrowserSessionSummaryPayload>,
    closed_sessions: Vec<BrowserClosedSessionPayload>,
    bookmarks: Vec<BrowserSessionBookmarkPayload>,
    profile_history: Vec<BrowserSessionProfileEntryPayload>,
    history: Vec<BrowserSessionHistoryEntryPayload>,
    viewport: String,
    #[serde(skip)]
    viewport_image: Option<BrowserSessionViewportImagePayload>,
    #[serde(skip)]
    viewport_image_error: Option<String>,
    #[serde(skip)]
    page_text: String,
    focused: Option<BrowserFocusedControl>,
    anchors: Vec<BrowserSessionAnchorPayload>,
    links: Vec<BrowserSessionLinkPayload>,
    form_count: usize,
    forms: Vec<BrowserSessionFormPayload>,
    cookies: Vec<BrowserCookie>,
    local_storage: Vec<BrowserLocalStorageEntry>,
    session_storage: Vec<BrowserLocalStorageEntry>,
    resource_count: usize,
    resource_image_count: usize,
    resource_stylesheet_count: usize,
    resource_script_count: usize,
    resources: Vec<BrowserSessionResourcePayload>,
    resource_report: Option<BrowserSessionResourceReportPayload>,
    action_feedback: Option<String>,
    pending_source: Option<String>,
    fast_scroll: bool,
}

#[derive(Debug, Clone)]
struct BrowserSessionViewportImagePayload {
    data_url: String,
    width: usize,
    height: usize,
}

#[derive(Debug, Clone, Copy)]
struct BrowserSessionPayloadOptions {
    render_viewport_image: bool,
    fast_scroll: bool,
}

impl Default for BrowserSessionPayloadOptions {
    fn default() -> Self {
        Self {
            render_viewport_image: true,
            fast_scroll: false,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
struct BrowserSessionSummaryPayload {
    id: String,
    position: usize,
    order: u64,
    title: String,
    page_title: String,
    label: Option<String>,
    source: String,
    action_url: String,
    reload_url: String,
    duplicate_url: String,
    duplicate_background_url: String,
    label_url: String,
    clear_label_url: String,
    move_left_url: String,
    move_right_url: String,
    close_url: String,
    pin_url: String,
    unpin_url: String,
    current: bool,
    can_close: bool,
    can_move_left: bool,
    can_move_right: bool,
    pinned: bool,
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
    background_restore_url: String,
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
    pinned: bool,
    label: Option<String>,
    updated_at_unix_secs: u64,
}

#[derive(Debug, Clone, Serialize)]
struct BrowserSessionBookmarkPayload {
    id: String,
    title: String,
    source: String,
    action_url: String,
    new_session_url: String,
    background_session_url: String,
    rename_url: String,
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
    background_session_url: String,
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
    pinned: bool,
    #[serde(default)]
    label: Option<String>,
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
    background_session_url: String,
    current: bool,
}

#[derive(Debug, Clone, Serialize)]
struct BrowserSessionFindMatchPayload {
    index: usize,
    line: usize,
    column: usize,
    current: bool,
    text: String,
    action_url: String,
    new_session_url: String,
    background_session_url: String,
}

#[derive(Debug, Clone, Serialize)]
struct BrowserSessionTabSearchResultPayload {
    id: String,
    title: String,
    page_title: String,
    label: Option<String>,
    source: String,
    pinned: bool,
    field: String,
    line: Option<usize>,
    text: String,
    action_url: String,
    reload_url: String,
    duplicate_url: String,
    duplicate_background_url: String,
    pin_url: String,
    unpin_url: String,
    close_url: String,
    current: bool,
}

#[derive(Debug, Clone, Serialize)]
struct BrowserSessionAnchorPayload {
    index: usize,
    name: String,
    y: usize,
    action_url: String,
    new_session_url: String,
    background_session_url: String,
}

#[derive(Debug, Clone, Serialize)]
struct BrowserSessionLinkPayload {
    index: usize,
    label: String,
    url: String,
    action_url: String,
    new_session_url: String,
    background_session_url: String,
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
    submit_new_session_url: String,
    submit_background_session_url: String,
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
    fill_url: Option<String>,
    type_url: Option<String>,
    clear_url: Option<String>,
    focus_url: Option<String>,
    activate_url: Option<String>,
    activate_new_session_url: Option<String>,
    activate_background_session_url: Option<String>,
    toggle_url: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
struct BrowserSessionFormOptionPayload {
    value: String,
    label: String,
    disabled: bool,
    selected: bool,
    select_url: Option<String>,
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
struct BrowserSessionResourcePayload {
    index: usize,
    kind: String,
    initiator: String,
    url: String,
    resolved: String,
    rel: Option<String>,
    media: Option<String>,
    alt: Option<String>,
    type_hint: Option<String>,
    details: String,
    open_url: String,
    new_session_url: String,
    background_session_url: String,
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

#[derive(Debug, Serialize)]
struct BrowserSessionStateExportPayload<'a> {
    format: &'static str,
    id: &'a str,
    title: &'a str,
    source: &'a str,
    viewport: BrowserSessionStateExportViewport,
    history: BrowserSessionStateExportHistory,
    history_entries: &'a [BrowserSessionHistoryEntryPayload],
    tabs: &'a [BrowserSessionSummaryPayload],
    closed_sessions: &'a [BrowserClosedSessionPayload],
    bookmarks: &'a [BrowserSessionBookmarkPayload],
    profile_history: &'a [BrowserSessionProfileEntryPayload],
    anchors: &'a [BrowserSessionAnchorPayload],
    links: &'a [BrowserSessionLinkPayload],
    forms: &'a [BrowserSessionFormPayload],
    resources: &'a [BrowserSessionResourcePayload],
    focused: Option<&'a BrowserFocusedControl>,
    find: BrowserSessionStateExportFind<'a>,
    tab_search: BrowserSessionStateExportTabSearch<'a>,
    resource_report: Option<BrowserSessionStateExportResourceReport<'a>>,
    profile: BrowserSessionStateExportProfile<'a>,
    counts: BrowserSessionStateExportCounts,
    clear_urls: BrowserSessionStateExportClearUrls<'a>,
    export_urls: BrowserSessionStateExportUrls,
    action_urls: BrowserSessionStateExportActionUrls,
    cookies: &'a [BrowserCookie],
    local_storage: &'a [BrowserLocalStorageEntry],
    session_storage: &'a [BrowserLocalStorageEntry],
}

#[derive(Debug, Serialize)]
struct BrowserSessionResourceReportExportPayload<'a> {
    format: &'static str,
    id: &'a str,
    title: &'a str,
    source: &'a str,
    resource_report: Option<&'a BrowserSessionResourceReportPayload>,
    csv_url: String,
    clear_url: Option<String>,
}

#[derive(Debug, Serialize)]
struct BrowserSessionResourcesExportPayload<'a> {
    format: &'static str,
    id: &'a str,
    title: &'a str,
    source: &'a str,
    resource_count: usize,
    displayed_resource_count: usize,
    image_count: usize,
    stylesheet_count: usize,
    script_count: usize,
    other_count: usize,
    resources: &'a [BrowserSessionResourcePayload],
    action_urls: BrowserSessionResourceActionUrls,
    csv_url: String,
    session_state_url: String,
}

#[derive(Debug, Serialize)]
struct BrowserSessionResourceActionUrls {
    fetch_resources: Option<String>,
    make_visual: Option<String>,
    apply_stylesheets: Option<String>,
    run_scripts: Option<String>,
    load_images: Option<String>,
    clear_resource_report: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct BrowserInteractionSnapshot {
    source: String,
    title: String,
    text: String,
    forms: Vec<BrowserForm>,
    link_count: usize,
}

#[derive(Debug, Serialize)]
struct BrowserSessionFormsExportPayload<'a> {
    format: &'static str,
    id: &'a str,
    title: &'a str,
    source: &'a str,
    form_count: usize,
    forms: &'a [BrowserSessionFormPayload],
    csv_url: String,
    session_state_url: String,
}

#[derive(Debug, Serialize)]
struct BrowserSessionFindExportPayload<'a> {
    format: &'static str,
    id: &'a str,
    title: &'a str,
    source: &'a str,
    query: &'a str,
    match_count: usize,
    current_index: Option<usize>,
    current_line: Option<usize>,
    current_column: Option<usize>,
    matches: &'a [BrowserSessionFindMatchPayload],
    csv_url: String,
    session_state_url: String,
}

#[derive(Debug, Serialize)]
struct BrowserSessionTabSearchExportPayload<'a> {
    format: &'static str,
    id: &'a str,
    title: &'a str,
    source: &'a str,
    query: &'a str,
    result_count: usize,
    results: &'a [BrowserSessionTabSearchResultPayload],
    action_urls: BrowserSessionTabSearchExportActionUrls,
    csv_url: String,
    session_state_url: String,
}

#[derive(Debug, Serialize)]
struct BrowserSessionTabSearchExportActionUrls {
    move_tab_search_results_front: Option<String>,
    move_tab_search_results_back: Option<String>,
    duplicate_tab_search_results: Option<String>,
    bookmark_tab_search_results: Option<String>,
    remove_tab_search_bookmarks: Option<String>,
    clear_tab_search: Option<String>,
    reload_tab_search_results: Option<String>,
    close_tab_search_results: Option<String>,
    close_tab_search_nonmatches: Option<String>,
    pin_tab_search_results: Option<String>,
    unpin_tab_search_results: Option<String>,
    label_tab_search_results: Option<String>,
    clear_tab_search_labels: Option<String>,
}

#[derive(Debug, Serialize)]
struct BrowserSessionStateExportViewport {
    width: usize,
    height: usize,
    x: usize,
    y: usize,
    document_width: usize,
    document_height: usize,
    max_scroll_x: usize,
    max_scroll_y: usize,
    max_bytes: usize,
}

#[derive(Debug, Serialize)]
struct BrowserSessionStateExportHistory {
    len: usize,
    current_index: Option<usize>,
    can_back: bool,
    can_forward: bool,
}

#[derive(Debug, Serialize)]
struct BrowserSessionStateExportFind<'a> {
    query: &'a str,
    match_count: usize,
    current_index: Option<usize>,
    current_line: Option<usize>,
    current_column: Option<usize>,
    matches: &'a [BrowserSessionFindMatchPayload],
}

#[derive(Debug, Serialize)]
struct BrowserSessionStateExportTabSearch<'a> {
    query: &'a str,
    result_count: usize,
    results: &'a [BrowserSessionTabSearchResultPayload],
}

#[derive(Debug, Serialize)]
struct BrowserSessionStateExportResourceReport<'a> {
    action: &'a str,
    page_source: &'a str,
    total: usize,
    fetched: usize,
    cached: usize,
    failed: usize,
    skipped: usize,
    applied: Option<usize>,
    decoded: Option<usize>,
    resources: usize,
    fetches: &'a [BrowserSessionResourceFetchPayload],
    csv_url: String,
    clear_url: String,
}

#[derive(Debug, Serialize)]
struct BrowserSessionStateExportProfile<'a> {
    enabled: bool,
    error: Option<&'a str>,
    current_bookmarked: bool,
}

#[derive(Debug, Serialize)]
struct BrowserSessionStateExportCounts {
    open_sessions: usize,
    pinned_tabs: usize,
    closed_sessions: usize,
    bookmarks: usize,
    profile_history: usize,
    history: usize,
    anchors: usize,
    links: usize,
    forms: usize,
    find_matches: usize,
    tab_search_results: usize,
    dom_nodes: usize,
    resources: usize,
    resource_images: usize,
    resource_stylesheets: usize,
    resource_scripts: usize,
    resource_others: usize,
    cookies: usize,
    local_storage: usize,
    session_storage: usize,
}

#[derive(Debug, Serialize)]
struct BrowserSessionStateExportClearUrls<'a> {
    cookies: Option<String>,
    local_storage: Option<String>,
    session_storage: Option<String>,
    bookmarks: Option<&'a str>,
    closed_sessions: Option<&'a str>,
    profile_tabs: Option<&'a str>,
    profile_history: Option<&'a str>,
}

#[derive(Debug, Serialize)]
struct BrowserSessionStateExportUrls {
    payload_json: String,
    session_state_json: String,
    session_state_csv: String,
    tabs_csv: String,
    closed_sessions_csv: String,
    bookmarks_csv: String,
    anchors_csv: String,
    links_csv: String,
    forms_json: String,
    forms_csv: String,
    history_csv: String,
    profile_history_csv: String,
    resources_json: String,
    resources_csv: String,
    resource_report_json: String,
    resource_report_csv: String,
    find_json: String,
    find_csv: String,
    tab_search_json: String,
    tab_search_csv: String,
    viewport_text: String,
    page_text: String,
}

#[derive(Debug, Serialize)]
struct BrowserSessionStateExportActionUrls {
    back: Option<String>,
    forward: Option<String>,
    reload: String,
    top: Option<String>,
    bottom: Option<String>,
    page_up: Option<String>,
    page_down: Option<String>,
    line_up: Option<String>,
    line_down: Option<String>,
    scroll_up: Option<String>,
    scroll_down: Option<String>,
    scroll_left: Option<String>,
    scroll_right: Option<String>,
    previous_tab: Option<String>,
    next_tab: Option<String>,
    move_tab_left: Option<String>,
    move_tab_right: Option<String>,
    move_tab_search_results_front: Option<String>,
    move_tab_search_results_back: Option<String>,
    duplicate_tab: String,
    duplicate_tab_background: String,
    duplicate_tab_search_results: Option<String>,
    close_tab: Option<String>,
    close_other_tabs: Option<String>,
    close_unpinned_tabs: Option<String>,
    pin_all_tabs: Option<String>,
    unpin_all_tabs: Option<String>,
    add_bookmark: Option<String>,
    bookmark_all_tabs: Option<String>,
    bookmark_profile_history: Option<String>,
    remove_profile_history_bookmarks: Option<String>,
    bookmark_tab_search_results: Option<String>,
    remove_tab_search_bookmarks: Option<String>,
    open_bookmarks_new_sessions: Option<String>,
    open_bookmarks_background: Option<String>,
    open_links_new_sessions: Option<String>,
    open_links_background: Option<String>,
    open_resources_new_sessions: Option<String>,
    open_resources_background: Option<String>,
    open_find_matches_new_sessions: Option<String>,
    open_find_matches_background: Option<String>,
    open_profile_history_new_sessions: Option<String>,
    open_profile_history_background: Option<String>,
    bookmark_page_links: Option<String>,
    remove_page_link_bookmarks: Option<String>,
    restore_closed_background_sessions: Option<String>,
    clear_find: Option<String>,
    clear_tab_search: Option<String>,
    reload_tab_search_results: Option<String>,
    close_tab_search_results: Option<String>,
    close_tab_search_nonmatches: Option<String>,
    pin_tab_search_results: Option<String>,
    unpin_tab_search_results: Option<String>,
    label_tab_search_results: Option<String>,
    clear_tab_search_labels: Option<String>,
    fetch_resources: Option<String>,
    make_visual: Option<String>,
    apply_stylesheets: Option<String>,
    run_scripts: Option<String>,
    load_images: Option<String>,
    clear_resource_report: Option<String>,
}

#[derive(Debug, Clone)]
enum BrowserSessionAction {
    Current,
    Open(String),
    OpenNewSession(String),
    OpenBackgroundSession(String),
    Back,
    Forward,
    Reload,
    Link(usize),
    Anchor(usize),
    AnchorNewSession(usize),
    AnchorBackgroundSession(usize),
    Resource(usize),
    LinkText(String),
    LinkSelector(String),
    LinkTextNewSession(String),
    LinkSelectorNewSession(String),
    LinkTextBackgroundSession(String),
    LinkSelectorBackgroundSession(String),
    ResourceNewSession(usize),
    LinkBackgroundSession(usize),
    OpenLinksNewSessions {
        limit: usize,
    },
    OpenLinksBackgroundSessions {
        limit: usize,
    },
    ResourceBackgroundSession(usize),
    OpenResourcesNewSessions {
        limit: usize,
    },
    OpenResourcesBackgroundSessions {
        limit: usize,
    },
    History(usize),
    Find(String),
    FindMatch(usize),
    FindMatchNewSession(usize),
    FindMatchBackgroundSession(usize),
    OpenFindMatchesNewSessions {
        limit: usize,
    },
    OpenFindMatchesBackgroundSessions {
        limit: usize,
    },
    FindNext,
    FindPrevious,
    ClearFind,
    SearchTabs(String),
    ClearTabSearch,
    ReloadTabSearchResults,
    CloseTabSearchResults,
    CloseTabSearchNonMatches,
    PinTabSearchResults,
    UnpinTabSearchResults,
    LabelTabSearchResults(String),
    ClearTabSearchLabels,
    ClickSelector(String),
    ClickAt {
        x: usize,
        y: usize,
        raster_width: Option<usize>,
        raster_height: Option<usize>,
    },
    FocusSelector(String),
    FocusControl {
        form_index: usize,
        control_index: usize,
    },
    ActivateControl {
        form_index: usize,
        control_index: usize,
    },
    ActivateControlNewSession {
        form_index: usize,
        control_index: usize,
    },
    ActivateControlBackgroundSession {
        form_index: usize,
        control_index: usize,
    },
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
    BookmarkAllTabs,
    BookmarkProfileHistory,
    RemoveProfileHistoryBookmarks,
    BookmarkPageLinks,
    RemovePageLinkBookmarks,
    BookmarkTabSearchResults,
    RemoveTabSearchBookmarks,
    OpenBookmark(String),
    RenameBookmark {
        bookmark_id: String,
        title: String,
    },
    RemoveBookmark(String),
    ClearBookmarks,
    OpenBookmarksNewSessions,
    OpenBookmarksBackgroundSessions,
    OpenProfileHistoryNewSessions {
        limit: usize,
    },
    OpenProfileHistoryBackgroundSessions {
        limit: usize,
    },
    OpenProfileClosed(usize),
    OpenProfileClosedBackgroundSession(usize),
    RemoveProfileHistory(usize),
    ClearClosedSessions,
    ClearProfileTabs,
    ClearProfileHistory,
    RestoreClosedSession(String),
    RestoreClosedBackgroundSession(String),
    RestoreClosedBackgroundSessions,
    ForgetClosedSession(String),
    ForgetProfileClosed(usize),
    FetchResources,
    MakeVisual,
    ApplyStylesheets,
    RunScripts,
    LoadImages,
    ClearResourceReport,
    DuplicateSession(String),
    DuplicateBackgroundSession(String),
    DuplicateTabSearchResults,
    CloseSession(String),
    CloseOtherSessions,
    CloseUnpinnedSessions,
    CloseSessionsToRight,
    CloseSessionsToLeft,
    CloseDuplicateSessions,
    PinSession(String),
    UnpinSession(String),
    PinAllSessions,
    UnpinAllSessions,
    MoveSessionLeft(String),
    MoveSessionRight(String),
    MoveTabSearchResultsToFront,
    MoveTabSearchResultsToBack,
    LabelSession {
        session_id: String,
        label: String,
    },
    ClearSessionLabel(String),
    SwitchNextSession,
    SwitchPreviousSession,
    JumpSession(String),
    Scroll {
        dx: isize,
        dy: isize,
    },
    Top,
    Bottom,
    PageUp,
    PageDown,
    LineUp,
    LineDown,
    Fill {
        form_index: usize,
        name: String,
        value: String,
    },
    FillControl {
        form_index: usize,
        control_index: usize,
        value: String,
    },
    TypeControl {
        form_index: usize,
        control_index: usize,
        value: String,
    },
    ClearControl {
        form_index: usize,
        control_index: usize,
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
    SubmitNewSession {
        form_index: usize,
    },
    SubmitBackgroundSession {
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

    fn browser_response(&self, target: &RequestTarget) -> HttpResponse {
        match self {
            Self::BadRequest(_) => self.response(),
            Self::NotFound(message) => browser_route_error_response(
                404,
                "Not Found",
                "Browser session unavailable",
                message,
                target,
            ),
            Self::Upstream(message) => {
                let title = if message.contains("timed out") {
                    "Browser page is still loading"
                } else {
                    "Browser page could not load"
                };
                browser_route_error_response(502, "Bad Gateway", title, message, target)
            }
        }
    }
}

pub(super) async fn browser_page(target: &RequestTarget, state: &ServerState) -> HttpResponse {
    match browser_session_for_target(target, state).await {
        Ok((payload, back_href)) => {
            html_response(if browser_session_target_wants_viewport_partial(target) {
                render_browser_session_viewport_partial(&payload)
            } else {
                render_browser_session_page_with_diagnostics(
                    &payload,
                    &back_href,
                    browser_session_target_wants_diagnostics(target),
                )
            })
        }
        Err(error) => error.browser_response(target),
    }
}

fn browser_session_target_wants_viewport_partial(target: &RequestTarget) -> bool {
    target
        .param("partial")
        .is_some_and(|value| value.eq_ignore_ascii_case("viewport"))
}

fn browser_session_target_wants_diagnostics(target: &RequestTarget) -> bool {
    ["debug", "tools", "diagnostics"].into_iter().any(|key| {
        target.param(key).is_some_and(|value| {
            matches!(
                value.to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "open"
            )
        })
    })
}

fn browser_route_error_response(
    status: u16,
    reason: &'static str,
    heading: &str,
    message: &str,
    target: &RequestTarget,
) -> HttpResponse {
    let back_href = sanitized_search_return_href(target.param("from").as_deref());
    let target_url = browser_route_recoverable_target_url(target).unwrap_or_default();
    let retry_href = browser_route_retry_href(target, &target_url);
    let recovery_hidden_inputs = browser_route_recovery_hidden_inputs(target);
    let retry_control = retry_href.as_ref().map_or_else(
        || r#"<span class="browser-error-disabled">No original URL to retry</span>"#.to_owned(),
        |href| {
            format!(
                r#"<a class="primary-action" href="{href}">Retry page</a>"#,
                href = html_escape::encode_double_quoted_attribute(href),
            )
        },
    );
    let address_value = if target_url.trim().is_empty() {
        String::new()
    } else {
        normalize_browser_address_url(&target_url)
    };
    let missing_session = matches!(reason, "Not Found");
    let status_label = if missing_session {
        "session missing"
    } else {
        "page loading"
    };
    let body_copy = if missing_session {
        "The browser server restarted or the tab expired. Start a new page or return to search."
    } else {
        "The first browser render did not finish quickly enough. The search page is still available, and you can retry this page when the site responds."
    };
    let missing_attr = if missing_session {
        r#" data-browser-missing-session="true""#
    } else {
        ""
    };
    HttpResponse {
        status,
        reason,
        content_type: "text/html; charset=utf-8",
        body: format!(
            r#"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>{heading}</title>
<style>
:root {{ color-scheme: light; font-family: ui-sans-serif, system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif; }}
body {{ margin: 0; background: #f7f7f5; color: #191a1c; }}
main {{ max-width: 960px; margin: 0 auto; padding: 18px; }}
a {{ color: #123fae; text-decoration: none; font-weight: 800; }}
a:hover {{ text-decoration: underline; }}
.browser-topbar {{ position: sticky; top: 0; z-index: 20; display: grid; gap: 6px; margin: -18px -18px 18px; padding: 8px 18px; background: rgba(247, 247, 245, 0.97); border-bottom: 1px solid #dfe2e6; }}
.browser-chrome-row {{ display: grid; grid-template-columns: auto minmax(220px, 1fr); gap: 8px; align-items: center; }}
.toolbar {{ display: flex; align-items: center; flex-wrap: wrap; gap: 8px; margin: 0; }}
.toolbar a, .toolbar span, .toolbar button {{ min-height: 32px; display: inline-flex; align-items: center; border: 1px solid #c6cbd2; border-radius: 6px; padding: 0 10px; background: #fff; color: #20242a; font-size: 13px; font-weight: 800; }}
.toolbar span {{ color: #8a929d; background: #eef0f3; }}
.toolbar form {{ display: flex; flex: 1 1 360px; min-width: 0; gap: 8px; }}
.toolbar input[name="url"] {{ flex: 1; min-width: 0; height: 32px; border: 1px solid #b7bdc5; border-radius: 6px; padding: 0 9px; font-size: 13px; background: #fff; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }}
.toolbar button, .primary-action {{ background: #2457d6 !important; border-color: #2457d6 !important; color: #fff !important; }}
.browser-chrome-status {{ display: flex; flex-wrap: wrap; gap: 6px; align-items: center; min-width: 0; color: #5d636b; font-size: 12px; font-weight: 800; }}
.viewport-state-chip {{ min-height: 24px; display: inline-flex; align-items: center; border: 1px solid #dfe2e6; border-radius: 6px; padding: 0 8px; background: #fff; color: #3a3f45; font-size: 12px; font-weight: 800; }}
.browser-error-card {{ border: 1px solid #d3d8df; border-radius: 8px; padding: 18px; background: #fff; box-shadow: 0 1px 2px rgba(25,26,28,0.06); }}
.browser-error-card h1 {{ margin: 0 0 8px; font-size: 22px; letter-spacing: 0; }}
.browser-error-card p {{ margin: 8px 0; color: #3a3f45; line-height: 1.45; }}
.browser-error-actions {{ display: flex; flex-wrap: wrap; gap: 8px; margin-top: 14px; }}
.browser-error-actions a, .browser-error-disabled {{ min-height: 32px; display: inline-flex; align-items: center; border: 1px solid #c6cbd2; border-radius: 6px; padding: 0 10px; background: #fff; color: #20242a; font-size: 13px; font-weight: 800; }}
.browser-error-disabled {{ color: #8a929d; background: #eef0f3; }}
.browser-error-detail {{ margin-top: 14px; }}
.browser-error-detail summary {{ cursor: pointer; color: #5d636b; font-size: 12px; font-weight: 900; }}
.browser-error-detail pre {{ white-space: pre-wrap; overflow-wrap: anywhere; border: 1px solid #dfe2e6; border-radius: 6px; padding: 10px; background: #f7f7f5; color: #3a3f45; }}
</style>
</head>
<body>
<main data-browser-route-error{missing_attr} data-browser-recovery-source="{address_value}">
<header class="browser-topbar">
<div class="browser-chrome-row" data-browser-chrome>
<nav class="toolbar browser-primary-nav" aria-label="Browser navigation"><span>Back</span><span>Forward</span><span>Reload</span></nav>
<form class="toolbar address-bar" method="get" action="/browser"><input data-browser-address type="text" name="url" value="{address_value}" aria-label="Address"><input type="hidden" name="from" value="{back_href}">{recovery_hidden_inputs}<button type="submit">Go</button></form>
</div>
<div class="browser-chrome-status" data-browser-chrome-status><span class="viewport-state-chip">{status_label}</span><span class="viewport-state-chip">shell ready</span></div>
</header>
<section class="browser-error-card" aria-live="polite">
<h1>{heading}</h1>
<p>{body_copy}</p>
<div class="browser-error-actions">{retry_control}<a href="{back_href}">Back to search</a><a href="/search">Search home</a></div>
<details class="browser-error-detail"><summary>Details</summary><pre>{message}</pre></details>
</section>
</main>
</body>
</html>"#,
            heading = html_escape::encode_text(heading),
            missing_attr = missing_attr,
            status_label = html_escape::encode_text(status_label),
            address_value = html_escape::encode_double_quoted_attribute(&address_value),
            back_href = html_escape::encode_double_quoted_attribute(&back_href),
            recovery_hidden_inputs = recovery_hidden_inputs,
            body_copy = html_escape::encode_text(body_copy),
            retry_control = retry_control,
            message = html_escape::encode_text(message),
        ),
    }
}

fn browser_route_retry_href(target: &RequestTarget, target_url: &str) -> Option<String> {
    let clean_target = target_url.trim();
    if clean_target.is_empty() {
        return None;
    }
    let normalized = checked_browser_address_url(clean_target).ok()?;
    let mut query = form_urlencoded::Serializer::new(String::new());
    query.append_pair("url", &normalized);
    query.append_pair(
        "from",
        &sanitized_search_return_href(target.param("from").as_deref()),
    );
    for key in ["width", "height", "viewport_x", "viewport_y", "max_bytes"] {
        if let Some(value) = target.param(key) {
            query.append_pair(key, &value);
        }
    }
    Some(format!("/browser?{}", query.finish()))
}

fn browser_route_recoverable_target_url(target: &RequestTarget) -> Option<String> {
    ["url", "target", "source"]
        .into_iter()
        .filter_map(|key| target.param(key))
        .find_map(|value| checked_browser_address_url(&value).ok())
}

fn browser_route_recovery_hidden_inputs(target: &RequestTarget) -> String {
    let mut inputs = String::new();
    for key in ["width", "height", "viewport_x", "viewport_y", "max_bytes"] {
        if let Some(value) = target.param(key) {
            let _ = write!(
                inputs,
                r#"<input type="hidden" name="{key}" value="{value}">"#,
                key = key,
                value = html_escape::encode_double_quoted_attribute(&value),
            );
        }
    }
    inputs
}

pub(super) async fn api_browser_session(
    target: &RequestTarget,
    state: &ServerState,
) -> HttpResponse {
    match browser_session_for_target(target, state).await {
        Ok((payload, _)) => browser_session_api_response(target, &payload),
        Err(error) => error.response(),
    }
}

fn browser_session_api_response(
    target: &RequestTarget,
    payload: &BrowserSessionPayload,
) -> HttpResponse {
    let format = target.param("format").unwrap_or_else(|| "json".to_owned());
    match format.trim().to_ascii_lowercase().as_str() {
        "" | "json" | "payload" | "session" => json_response(200, "OK", payload),
        "session-state" | "session_state" | "state" => {
            json_response(200, "OK", &browser_session_state_export_payload(payload))
        }
        "session-state-csv" | "session_state_csv" | "state-csv" | "state_csv" | "storage-csv"
        | "storage_csv" => browser_session_state_csv_response(payload),
        "tabs-csv" | "tabs_csv" | "sessions-csv" | "sessions_csv" => {
            browser_session_tabs_csv_response(payload)
        }
        "closed-sessions-csv" | "closed_sessions_csv" | "closed-csv" | "closed_csv" => {
            browser_session_closed_sessions_csv_response(payload)
        }
        "bookmarks-csv" | "bookmarks_csv" => browser_session_bookmarks_csv_response(payload),
        "anchors-csv" | "anchors_csv" | "fragments-csv" | "fragments_csv" => {
            browser_session_anchors_csv_response(payload)
        }
        "links-csv" | "links_csv" => browser_session_links_csv_response(payload),
        "forms-json" | "forms_json" => browser_session_forms_json_response(payload),
        "forms-csv" | "forms_csv" => browser_session_forms_csv_response(payload),
        "history-csv" | "history_csv" => browser_session_history_csv_response(payload),
        "profile-history-csv" | "profile_history_csv" => {
            browser_session_profile_history_csv_response(payload)
        }
        "resources-json" | "resources_json" => browser_session_resources_json_response(payload),
        "resources-csv" | "resources_csv" => browser_session_resources_csv_response(payload),
        "resource-report-json"
        | "resource_report_json"
        | "resources-report-json"
        | "resources_report_json" => browser_session_resource_report_json_response(payload),
        "resource-report-csv"
        | "resource_report_csv"
        | "resources-report-csv"
        | "resources_report_csv" => browser_session_resource_report_csv_response(payload),
        "find-json" | "find_json" => browser_session_find_json_response(payload),
        "find-csv" | "find_csv" => browser_session_find_csv_response(payload),
        "tab-search-json" | "tab_search_json" | "tabs-search-json" | "tabs_search_json" => {
            browser_session_tab_search_json_response(payload)
        }
        "tab-search-csv" | "tab_search_csv" | "tabs-search-csv" | "tabs_search_csv" => {
            browser_session_tab_search_csv_response(payload)
        }
        "viewport-text" | "viewport_text" | "viewport" => {
            text_response(200, "OK", &payload.viewport)
        }
        "page-text" | "page_text" | "text" => text_response(200, "OK", &payload.page_text),
        _ => text_response(
            400,
            "Bad Request",
            &format!("unsupported browser session format: {format}"),
        ),
    }
}

async fn browser_session_for_target(
    target: &RequestTarget,
    state: &ServerState,
) -> Result<(BrowserSessionPayload, String), BrowserRouteError> {
    state.browser_sessions.target(target).await
}

fn browser_target_with_session_id(target: &RequestTarget, id: &str) -> RequestTarget {
    let mut params = target
        .params
        .iter()
        .filter(|(key, _)| key != "id")
        .cloned()
        .collect::<Vec<_>>();
    params.insert(0, ("id".to_owned(), id.to_owned()));
    RequestTarget {
        path: target.path.clone(),
        params,
    }
}

fn browser_target_can_apply_action_after_create(
    target: &RequestTarget,
) -> Result<bool, BrowserRouteError> {
    if target.param("url").is_none() || target.param("action").is_none() {
        return Ok(false);
    }
    let action = browser_action(target)?;
    Ok(browser_action_can_apply_after_fresh_create(&action))
}

fn browser_action_can_apply_after_fresh_create(action: &BrowserSessionAction) -> bool {
    matches!(
        action,
        BrowserSessionAction::Current
            | BrowserSessionAction::Reload
            | BrowserSessionAction::Link(_)
            | BrowserSessionAction::LinkText(_)
            | BrowserSessionAction::LinkSelector(_)
            | BrowserSessionAction::ClickSelector(_)
            | BrowserSessionAction::ClickAt { .. }
            | BrowserSessionAction::FocusSelector(_)
            | BrowserSessionAction::FocusControl { .. }
            | BrowserSessionAction::ActivateControl { .. }
            | BrowserSessionAction::FocusNext
            | BrowserSessionAction::FocusPrevious
            | BrowserSessionAction::TypeText(_)
            | BrowserSessionAction::Backspace(_)
            | BrowserSessionAction::ClearInput
            | BrowserSessionAction::Enter
            | BrowserSessionAction::Space
            | BrowserSessionAction::Choose(_)
            | BrowserSessionAction::FetchResources
            | BrowserSessionAction::MakeVisual
            | BrowserSessionAction::ApplyStylesheets
            | BrowserSessionAction::RunScripts
            | BrowserSessionAction::LoadImages
            | BrowserSessionAction::ClearResourceReport
            | BrowserSessionAction::Scroll { .. }
            | BrowserSessionAction::Top
            | BrowserSessionAction::Bottom
            | BrowserSessionAction::PageUp
            | BrowserSessionAction::PageDown
            | BrowserSessionAction::LineUp
            | BrowserSessionAction::LineDown
            | BrowserSessionAction::Fill { .. }
            | BrowserSessionAction::FillControl { .. }
            | BrowserSessionAction::TypeControl { .. }
            | BrowserSessionAction::ClearControl { .. }
            | BrowserSessionAction::Select { .. }
            | BrowserSessionAction::Toggle { .. }
            | BrowserSessionAction::Submit { .. }
    )
}

fn browser_target_allows_xy_viewport_alias(target: &RequestTarget) -> bool {
    browser_action(target)
        .as_ref()
        .map(browser_session_action_allows_xy_viewport_alias)
        .unwrap_or(true)
}

impl BrowserSessionRegistry {
    async fn target(
        &self,
        target: &RequestTarget,
    ) -> Result<(BrowserSessionPayload, String), BrowserRouteError> {
        if target.param("id").is_some() {
            self.apply_target(target).await
        } else if browser_target_can_apply_action_after_create(target)? {
            let (payload, _) = self.create_target(target).await?;
            let target = browser_target_with_session_id(target, &payload.id);
            self.apply_target(&target).await
        } else {
            self.create_target(target).await
        }
    }

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
        let target_url = checked_browser_address_url(&target_url)?;

        let width = parse_usize_param(target, "width", DEFAULT_BROWSER_WIDTH, 40, 160);
        let height = parse_usize_param(target, "height", DEFAULT_BROWSER_HEIGHT, 16, 120);
        let max_bytes = parse_usize_param(
            target,
            "max_bytes",
            DEFAULT_BROWSER_MAX_BYTES,
            64 * 1024,
            16 * 1024 * 1024,
        );
        let navigation_target = browser_session_navigation_target(&target_url, max_bytes)?;
        let back_href = sanitized_search_return_href(target.param("from").as_deref());
        let allow_xy_viewport_alias = browser_target_allows_xy_viewport_alias(target);
        let has_explicit_viewport_y = target.param("viewport_y").is_some()
            || (allow_xy_viewport_alias && target.param("y").is_some());
        let mut session = BrowserSession::new(BrowserRenderOptions { width, max_bytes });
        let mut pending_source = None;
        let display_source = navigation_target.display_source;
        let mut action_feedback = None;
        match timeout(
            BROWSER_CREATE_TARGET_TIMEOUT,
            session.navigate(&navigation_target.target),
        )
        .await
        {
            Ok(Ok(_)) => {}
            Ok(Err(error)) => {
                session
                    .navigate(BROWSER_ABOUT_BLANK_TARGET)
                    .await
                    .map_err(|blank_error| {
                        BrowserRouteError::Upstream(format!(
                            "browser fallback shell failed after {target_url} failed: {blank_error:#}"
                        ))
                    })?;
                pending_source = Some(target_url.clone());
                action_feedback = Some(format!(
                    "Still opening {}; renderer reported: {}",
                    browser_session_feedback_excerpt(&target_url),
                    browser_session_feedback_excerpt(&error.to_string())
                ));
            }
            Err(_) => {
                session
                    .navigate(BROWSER_ABOUT_BLANK_TARGET)
                    .await
                    .map_err(|blank_error| {
                        BrowserRouteError::Upstream(format!(
                            "browser fallback shell failed after {target_url} timed out: {blank_error:#}"
                        ))
                    })?;
                pending_source = Some(target_url.clone());
                action_feedback = Some(format!(
                    "Still opening {}; initial render exceeded {}ms.",
                    browser_session_feedback_excerpt(&target_url),
                    BROWSER_CREATE_TARGET_TIMEOUT.as_millis()
                ));
            }
        }

        let id = self.next_session_id();
        let mut web_session = BrowserWebSession {
            session,
            tab_order: browser_session_id_number(&id),
            width,
            height,
            max_bytes,
            viewport_x: parse_optional_usize_param(target, "viewport_x", 0, usize::MAX)
                .or_else(|| {
                    allow_xy_viewport_alias
                        .then(|| parse_optional_usize_param(target, "x", 0, usize::MAX))
                        .flatten()
                })
                .unwrap_or(0),
            viewport_y: parse_optional_usize_param(target, "viewport_y", 0, usize::MAX)
                .or_else(|| {
                    allow_xy_viewport_alias
                        .then(|| parse_optional_usize_param(target, "y", 0, usize::MAX))
                        .flatten()
                })
                .unwrap_or(0),
            back_href,
            find_query: String::new(),
            find_active_line: None,
            tab_search_query: String::new(),
            resource_report: None,
            action_feedback,
            pending_source,
            display_source,
            pinned: false,
            tab_label: None,
        };
        if !has_explicit_viewport_y {
            reset_viewport_to_fragment(&mut web_session);
        }
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

    async fn recover_missing_session_target(
        &self,
        target: &RequestTarget,
        missing_message: &str,
    ) -> Result<Option<(BrowserSessionPayload, String)>, BrowserRouteError> {
        let target_url = target
            .param("url")
            .or_else(|| target.param("target"))
            .or_else(|| target.param("source"))
            .unwrap_or_default();
        let Some(target_url) = checked_browser_address_url(&target_url).ok() else {
            return Ok(None);
        };
        let mut params = target
            .params
            .iter()
            .filter(|(key, _)| !matches!(key.as_str(), "id" | "url" | "target" | "source"))
            .cloned()
            .collect::<Vec<_>>();
        params.insert(0, ("url".to_owned(), target_url));
        let recovered = RequestTarget {
            path: target.path.clone(),
            params,
        };
        let (mut payload, back_href) = self.create_target(&recovered).await?;
        let prior = payload.action_feedback.take();
        let recovery = format!(
            "Recovered expired browser session; reopened page in a new session ({})",
            browser_session_feedback_excerpt(missing_message)
        );
        payload.action_feedback = Some(match prior {
            Some(feedback) if !feedback.trim().is_empty() => format!("{recovery}; {feedback}"),
            _ => recovery,
        });
        Ok(Some((payload, back_href)))
    }

    async fn take_session_for_action(
        &self,
        id: &str,
        mark_in_flight: bool,
    ) -> Result<BrowserWebSession, BrowserRouteError> {
        loop {
            let mut in_flight_sessions = self.in_flight_sessions.lock().await;
            let mut sessions = self.sessions.lock().await;
            if let Some(web_session) = sessions.remove(id) {
                let in_flight_viewport = mark_in_flight.then(|| web_session.clone());
                if mark_in_flight {
                    in_flight_sessions.insert(id.to_owned(), Arc::new(Notify::new()));
                }
                drop(sessions);
                drop(in_flight_sessions);
                if let Some(in_flight_viewport) = in_flight_viewport {
                    self.in_flight_viewports
                        .lock()
                        .await
                        .insert(id.to_owned(), in_flight_viewport);
                }
                return Ok(web_session);
            }
            let Some(notify) = in_flight_sessions.get(id).cloned() else {
                return Err(BrowserRouteError::NotFound(format!(
                    "browser session {id} not found"
                )));
            };
            let notified = notify.notified();
            drop(sessions);
            drop(in_flight_sessions);
            notified.await;
        }
    }

    async fn return_session_after_action<'a>(
        &'a self,
        id: &str,
        mut web_session: BrowserWebSession,
        notify_in_flight_waiters: bool,
    ) -> MutexGuard<'a, HashMap<String, BrowserWebSession>> {
        if notify_in_flight_waiters {
            let in_flight_viewport = self.in_flight_viewports.lock().await.remove(id);
            if let Some(in_flight_viewport) = in_flight_viewport {
                web_session.width = in_flight_viewport.width;
                web_session.height = in_flight_viewport.height;
                web_session.max_bytes = in_flight_viewport.max_bytes;
                web_session.viewport_x = in_flight_viewport.viewport_x;
                web_session.viewport_y = in_flight_viewport.viewport_y;
                web_session.back_href = in_flight_viewport.back_href;
                if in_flight_viewport.action_feedback.is_some() {
                    web_session.action_feedback = in_flight_viewport.action_feedback;
                }
            }
            let mut in_flight_sessions = self.in_flight_sessions.lock().await;
            let mut sessions = self.sessions.lock().await;
            sessions.insert(id.to_owned(), web_session);
            let notify = in_flight_sessions.remove(id);
            drop(in_flight_sessions);
            if let Some(notify) = notify {
                notify.notify_waiters();
            }
            return sessions;
        }

        let mut sessions = self.sessions.lock().await;
        sessions.insert(id.to_owned(), web_session);
        sessions
    }

    async fn apply_in_flight_viewport_partial(
        &self,
        target: &RequestTarget,
        id: &str,
        action: &BrowserSessionAction,
    ) -> Result<Option<(BrowserSessionPayload, String)>, BrowserRouteError> {
        if !browser_session_target_wants_viewport_partial(target)
            || !browser_action_can_apply_in_flight_viewport_partial(action)
        {
            return Ok(None);
        }

        {
            let in_flight_sessions = self.in_flight_sessions.lock().await;
            if !in_flight_sessions.contains_key(id) {
                return Ok(None);
            }
        }

        let Some(mut web_session) = self.in_flight_viewports.lock().await.remove(id) else {
            return Ok(None);
        };

        web_session.width =
            parse_optional_usize_param(target, "width", 40, 160).unwrap_or(web_session.width);
        web_session.height =
            parse_optional_usize_param(target, "height", 16, 120).unwrap_or(web_session.height);
        web_session.max_bytes =
            parse_optional_usize_param(target, "max_bytes", 64 * 1024, 16 * 1024 * 1024)
                .unwrap_or(web_session.max_bytes);
        if let Some(return_href) = target.param("from") {
            web_session.back_href = sanitized_search_return_href(Some(&return_href));
        }
        apply_browser_action(action.clone(), &mut web_session).await?;
        let back_href = web_session.back_href.clone();
        let mut payload = browser_session_payload(id, &mut web_session)?;
        let mut sessions_view = {
            let sessions = self.sessions.lock().await;
            sessions.clone()
        };
        sessions_view.insert(id.to_owned(), web_session.clone());
        self.in_flight_viewports
            .lock()
            .await
            .insert(id.to_owned(), web_session);
        let closed_sessions = self.closed_sessions.lock().await;
        let bookmarks = self.bookmarks.lock().await;
        self.attach_browser_session_registry_state(
            &mut payload,
            &sessions_view,
            &closed_sessions,
            &bookmarks,
            id,
        )
        .await;
        Ok(Some((payload, back_href)))
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
        let has_explicit_viewport_y =
            target.param("y").is_some() || target.param("viewport_y").is_some();
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
                tab_order: 0,
                width,
                height,
                max_bytes,
                viewport_x: parse_optional_usize_param(target, "x", 0, usize::MAX)
                    .or_else(|| parse_optional_usize_param(target, "viewport_x", 0, usize::MAX))
                    .unwrap_or(0),
                viewport_y: parse_optional_usize_param(target, "y", 0, usize::MAX)
                    .or_else(|| parse_optional_usize_param(target, "viewport_y", 0, usize::MAX))
                    .unwrap_or(0),
                back_href: back_href.clone(),
                find_query: String::new(),
                find_active_line: None,
                tab_search_query: String::new(),
                resource_report: None,
                action_feedback: None,
                pending_source: None,
                display_source: None,
                pinned: tab.pinned,
                tab_label: tab.label.clone(),
            };
            if !has_explicit_viewport_y {
                reset_viewport_to_fragment(&mut web_session);
            }
            restored_sessions.push(web_session);
        }

        let mut active_id = String::new();
        let mut sessions = self.sessions.lock().await;
        for (index, mut web_session) in restored_sessions.into_iter().enumerate() {
            let id = self.next_session_id();
            web_session.tab_order = browser_session_id_number(&id);
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
        let notifies_in_flight_waiters = browser_action_marks_session_in_flight(&action);
        let current_viewport_jump_requested = matches!(action, BrowserSessionAction::Current)
            && browser_session_target_has_viewport_position(target);

        if let Some(result) = self
            .apply_in_flight_viewport_partial(target, &id, &action)
            .await?
        {
            return Ok(result);
        }

        let mut web_session = match self
            .take_session_for_action(&id, notifies_in_flight_waiters)
            .await
        {
            Ok(web_session) => web_session,
            Err(BrowserRouteError::NotFound(message)) => {
                if let Some(result) = self
                    .recover_missing_session_target(target, &message)
                    .await?
                {
                    return Ok(result);
                }
                return Err(BrowserRouteError::NotFound(message));
            }
            Err(error) => return Err(error),
        };

        web_session.width =
            parse_optional_usize_param(target, "width", 40, 160).unwrap_or(web_session.width);
        web_session.height =
            parse_optional_usize_param(target, "height", 16, 120).unwrap_or(web_session.height);
        web_session.max_bytes =
            parse_optional_usize_param(target, "max_bytes", 64 * 1024, 16 * 1024 * 1024)
                .unwrap_or(web_session.max_bytes);
        if current_viewport_jump_requested {
            normalize_browser_session_viewport(&mut web_session);
        }
        let previous_viewport_x = web_session.viewport_x;
        let previous_viewport_y = web_session.viewport_y;
        web_session.viewport_x =
            browser_session_target_viewport_x(target, &action).unwrap_or(web_session.viewport_x);
        web_session.viewport_y =
            browser_session_target_viewport_y(target, &action).unwrap_or(web_session.viewport_y);
        if current_viewport_jump_requested {
            normalize_browser_session_viewport(&mut web_session);
            set_browser_viewport_jump_feedback(
                &mut web_session,
                previous_viewport_x,
                previous_viewport_y,
            );
        }
        if let Some(return_href) = target.param("from") {
            web_session.back_href = sanitized_search_return_href(Some(&return_href));
        }
        let payload_options = browser_session_payload_options_for_action(&action);

        if let BrowserSessionAction::CloseSession(close_id) = action {
            self.sessions.lock().await.insert(id.clone(), web_session);
            return self.close_target(target, &id, &close_id).await;
        }
        if let BrowserSessionAction::RestoreClosedSession(closed_id) = action {
            self.sessions.lock().await.insert(id.clone(), web_session);
            return self.restore_closed_target(target, &id, &closed_id).await;
        }
        if let BrowserSessionAction::RestoreClosedBackgroundSession(closed_id) = action {
            return self
                .restore_closed_background_target(target, &id, web_session, &closed_id)
                .await;
        }
        if let BrowserSessionAction::RestoreClosedBackgroundSessions = action {
            return self
                .restore_closed_sessions_background_target(target, &id, web_session)
                .await;
        }
        if let BrowserSessionAction::OpenProfileClosedBackgroundSession(index) = action {
            return self
                .open_profile_closed_background_target(&id, web_session, index)
                .await;
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
        if let BrowserSessionAction::JumpSession(query) = action {
            self.sessions.lock().await.insert(id.clone(), web_session);
            return self.switch_matching_session_target(&query).await;
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
        if let BrowserSessionAction::OpenBackgroundSession(url) = action {
            return self
                .open_browser_action_in_background_session_target(
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
        if let BrowserSessionAction::LinkTextBackgroundSession(text) = action {
            return self
                .open_browser_action_in_background_session_target(
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
        if let BrowserSessionAction::LinkSelectorBackgroundSession(selector) = action {
            return self
                .open_browser_action_in_background_session_target(
                    &id,
                    web_session,
                    BrowserSessionAction::LinkSelector(selector),
                )
                .await;
        }
        if let BrowserSessionAction::AnchorNewSession(index) = action {
            return self
                .open_browser_action_in_new_session_target(
                    &id,
                    web_session,
                    BrowserSessionAction::Anchor(index),
                )
                .await;
        }
        if let BrowserSessionAction::AnchorBackgroundSession(index) = action {
            return self
                .open_browser_action_in_background_session_target(
                    &id,
                    web_session,
                    BrowserSessionAction::Anchor(index),
                )
                .await;
        }
        if let BrowserSessionAction::FindMatchNewSession(index) = action {
            return self
                .open_browser_action_in_new_session_target(
                    &id,
                    web_session,
                    BrowserSessionAction::FindMatch(index),
                )
                .await;
        }
        if let BrowserSessionAction::FindMatchBackgroundSession(index) = action {
            return self
                .open_browser_action_in_background_session_target(
                    &id,
                    web_session,
                    BrowserSessionAction::FindMatch(index),
                )
                .await;
        }
        if let BrowserSessionAction::OpenFindMatchesNewSessions { limit } = action {
            return self
                .open_find_matches_new_sessions_target(&id, web_session, limit)
                .await;
        }
        if let BrowserSessionAction::OpenFindMatchesBackgroundSessions { limit } = action {
            return self
                .open_find_matches_background_target(&id, web_session, limit)
                .await;
        }
        if let BrowserSessionAction::ResourceNewSession(index) = action {
            return self
                .open_browser_action_in_new_session_target(
                    &id,
                    web_session,
                    BrowserSessionAction::Resource(index),
                )
                .await;
        }
        if let BrowserSessionAction::LinkBackgroundSession(index) = action {
            return self
                .open_browser_action_in_background_session_target(
                    &id,
                    web_session,
                    BrowserSessionAction::Link(index),
                )
                .await;
        }
        if let BrowserSessionAction::ResourceBackgroundSession(index) = action {
            return self
                .open_browser_action_in_background_session_target(
                    &id,
                    web_session,
                    BrowserSessionAction::Resource(index),
                )
                .await;
        }
        if let BrowserSessionAction::OpenResourcesNewSessions { limit } = action {
            return self
                .open_resources_new_sessions_target(&id, web_session, limit)
                .await;
        }
        if let BrowserSessionAction::OpenResourcesBackgroundSessions { limit } = action {
            return self
                .open_resources_background_target(&id, web_session, limit)
                .await;
        }
        if let BrowserSessionAction::OpenLinksNewSessions { limit } = action {
            return self
                .open_links_new_sessions_target(&id, web_session, limit)
                .await;
        }
        if let BrowserSessionAction::OpenLinksBackgroundSessions { limit } = action {
            return self
                .open_links_background_target(&id, web_session, limit)
                .await;
        }
        if let BrowserSessionAction::BookmarkPageLinks = action {
            return self.bookmark_page_links_target(&id, web_session).await;
        }
        if let BrowserSessionAction::RemovePageLinkBookmarks = action {
            return self
                .remove_page_link_bookmarks_target(&id, web_session)
                .await;
        }
        if let BrowserSessionAction::SubmitNewSession { form_index } = action {
            return self
                .open_browser_action_in_new_session_target(
                    &id,
                    web_session,
                    BrowserSessionAction::Submit { form_index },
                )
                .await;
        }
        if let BrowserSessionAction::SubmitBackgroundSession { form_index } = action {
            return self
                .open_browser_action_in_background_session_target(
                    &id,
                    web_session,
                    BrowserSessionAction::Submit { form_index },
                )
                .await;
        }
        if let BrowserSessionAction::ActivateControlNewSession {
            form_index,
            control_index,
        } = action
        {
            return self
                .open_browser_action_in_new_session_target(
                    &id,
                    web_session,
                    BrowserSessionAction::ActivateControl {
                        form_index,
                        control_index,
                    },
                )
                .await;
        }
        if let BrowserSessionAction::ActivateControlBackgroundSession {
            form_index,
            control_index,
        } = action
        {
            return self
                .open_browser_action_in_background_session_target(
                    &id,
                    web_session,
                    BrowserSessionAction::ActivateControl {
                        form_index,
                        control_index,
                    },
                )
                .await;
        }
        if let BrowserSessionAction::DuplicateSession(duplicate_id) = action {
            self.sessions.lock().await.insert(id.clone(), web_session);
            return self.duplicate_session_target(&duplicate_id).await;
        }
        if let BrowserSessionAction::DuplicateBackgroundSession(duplicate_id) = action {
            return self
                .duplicate_session_background_target(&id, web_session, &duplicate_id)
                .await;
        }
        if let BrowserSessionAction::DuplicateTabSearchResults = action {
            return self
                .duplicate_tab_search_results_target(&id, web_session)
                .await;
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
        if let BrowserSessionAction::CloseUnpinnedSessions = action {
            return self
                .close_scoped_sessions_target(
                    target,
                    &id,
                    web_session,
                    BrowserSessionCloseScope::Unpinned,
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
        if let BrowserSessionAction::ReloadTabSearchResults = action {
            return self
                .reload_tab_search_results_target(&id, web_session)
                .await;
        }
        if let BrowserSessionAction::CloseTabSearchResults = action {
            return self.close_tab_search_results_target(&id, web_session).await;
        }
        if let BrowserSessionAction::CloseTabSearchNonMatches = action {
            return self
                .close_tab_search_nonmatches_target(&id, web_session)
                .await;
        }
        if let BrowserSessionAction::PinTabSearchResults = action {
            return self
                .set_tab_search_results_pinned_target(&id, web_session, true)
                .await;
        }
        if let BrowserSessionAction::UnpinTabSearchResults = action {
            return self
                .set_tab_search_results_pinned_target(&id, web_session, false)
                .await;
        }
        if let BrowserSessionAction::LabelTabSearchResults(label) = action {
            return self
                .set_tab_search_results_label_target(&id, web_session, Some(label))
                .await;
        }
        if let BrowserSessionAction::ClearTabSearchLabels = action {
            return self
                .set_tab_search_results_label_target(&id, web_session, None)
                .await;
        }
        if let BrowserSessionAction::PinSession(pin_id) = action {
            return self
                .set_session_pinned_target(&id, web_session, &pin_id, true)
                .await;
        }
        if let BrowserSessionAction::UnpinSession(unpin_id) = action {
            return self
                .set_session_pinned_target(&id, web_session, &unpin_id, false)
                .await;
        }
        if let BrowserSessionAction::PinAllSessions = action {
            return self
                .set_all_sessions_pinned_target(&id, web_session, true)
                .await;
        }
        if let BrowserSessionAction::UnpinAllSessions = action {
            return self
                .set_all_sessions_pinned_target(&id, web_session, false)
                .await;
        }
        if let BrowserSessionAction::MoveSessionLeft(move_id) = action {
            return self
                .move_session_target(
                    &id,
                    web_session,
                    &move_id,
                    BrowserSessionMoveDirection::Left,
                )
                .await;
        }
        if let BrowserSessionAction::MoveSessionRight(move_id) = action {
            return self
                .move_session_target(
                    &id,
                    web_session,
                    &move_id,
                    BrowserSessionMoveDirection::Right,
                )
                .await;
        }
        if let BrowserSessionAction::MoveTabSearchResultsToFront = action {
            return self
                .move_tab_search_results_target(&id, web_session, true)
                .await;
        }
        if let BrowserSessionAction::MoveTabSearchResultsToBack = action {
            return self
                .move_tab_search_results_target(&id, web_session, false)
                .await;
        }
        if let BrowserSessionAction::LabelSession { session_id, label } = action {
            return self
                .set_session_label_target(&id, web_session, &session_id, Some(label))
                .await;
        }
        if let BrowserSessionAction::ClearSessionLabel(session_id) = action {
            return self
                .set_session_label_target(&id, web_session, &session_id, None)
                .await;
        }
        if let BrowserSessionAction::BookmarkAllTabs = action {
            return self.bookmark_all_tabs_target(&id, web_session).await;
        }
        if let BrowserSessionAction::BookmarkProfileHistory = action {
            return self.bookmark_profile_history_target(&id, web_session).await;
        }
        if let BrowserSessionAction::RemoveProfileHistoryBookmarks = action {
            return self
                .remove_profile_history_bookmarks_target(&id, web_session)
                .await;
        }
        if let BrowserSessionAction::BookmarkTabSearchResults = action {
            return self
                .bookmark_tab_search_results_target(&id, web_session)
                .await;
        }
        if let BrowserSessionAction::RemoveTabSearchBookmarks = action {
            return self
                .remove_tab_search_bookmarks_target(&id, web_session)
                .await;
        }
        if let BrowserSessionAction::OpenBookmarksNewSessions = action {
            return self
                .open_bookmarks_new_sessions_target(&id, web_session)
                .await;
        }
        if let BrowserSessionAction::OpenBookmarksBackgroundSessions = action {
            return self
                .open_bookmarks_background_target(&id, web_session)
                .await;
        }
        if let BrowserSessionAction::OpenProfileHistoryNewSessions { limit } = action {
            return self
                .open_profile_history_new_sessions_target(&id, web_session, limit)
                .await;
        }
        if let BrowserSessionAction::OpenProfileHistoryBackgroundSessions { limit } = action {
            return self
                .open_profile_history_background_target(&id, web_session, limit)
                .await;
        }

        let result = match action {
            BrowserSessionAction::AddBookmark => self
                .add_current_bookmark(&web_session)
                .await
                .and_then(|_| browser_session_payload(&id, &mut web_session)),
            BrowserSessionAction::RenameBookmark { bookmark_id, title } => self
                .rename_bookmark(&bookmark_id, &title)
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
                .and_then(|_| {
                    browser_session_payload_with_options(&id, &mut web_session, payload_options)
                }),
        };
        let mut payload = match result {
            Ok(payload) => payload,
            Err(error) => {
                let sessions = self
                    .return_session_after_action(&id, web_session, notifies_in_flight_waiters)
                    .await;
                drop(sessions);
                return Err(error);
            }
        };
        let back_href = web_session.back_href.clone();
        if should_record_profile_visit {
            self.record_browser_profile_visit(&payload).await;
        }
        let sessions = self
            .return_session_after_action(&id, web_session, notifies_in_flight_waiters)
            .await;
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
        active_session: BrowserWebSession,
        scope: BrowserSessionCloseScope,
    ) -> Result<(BrowserSessionPayload, String), BrowserRouteError> {
        let mut sessions = self.sessions.lock().await;
        sessions.insert(active_id.to_owned(), active_session);
        let ordered_ids = browser_sorted_session_ids(&sessions);
        let active_index = ordered_ids
            .iter()
            .position(|id| id == active_id)
            .ok_or_else(|| {
                BrowserRouteError::NotFound(format!("browser session {active_id} not found"))
            })?;
        let closed_ids = ordered_ids
            .into_iter()
            .enumerate()
            .filter(|(index, id)| {
                if id == active_id {
                    return false;
                }
                match &scope {
                    BrowserSessionCloseScope::Others => true,
                    BrowserSessionCloseScope::Unpinned => true,
                    BrowserSessionCloseScope::LeftOfActive => *index < active_index,
                    BrowserSessionCloseScope::RightOfActive => *index > active_index,
                    BrowserSessionCloseScope::DuplicateSource(active_source) => sessions
                        .get(id)
                        .and_then(|session| session.session.current())
                        .is_some_and(|render| render.source.as_str() == active_source.as_str()),
                }
            })
            .map(|(_, id)| id)
            .filter(|id| !sessions.get(id).is_some_and(|session| session.pinned))
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

        let (mut payload, back_href) = {
            let active = sessions.get_mut(active_id).ok_or_else(|| {
                BrowserRouteError::NotFound(format!("browser session {active_id} not found"))
            })?;
            let payload = browser_session_payload(active_id, active)?;
            (payload, active.back_href.clone())
        };
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

    async fn move_session_target(
        &self,
        active_id: &str,
        active_session: BrowserWebSession,
        target_id: &str,
        direction: BrowserSessionMoveDirection,
    ) -> Result<(BrowserSessionPayload, String), BrowserRouteError> {
        let mut sessions = self.sessions.lock().await;
        sessions.insert(active_id.to_owned(), active_session);
        if !sessions.contains_key(target_id) {
            return Err(BrowserRouteError::NotFound(format!(
                "browser session {target_id} not found"
            )));
        }

        let ordered_ids = browser_sorted_session_ids(&sessions);
        let target_index = ordered_ids
            .iter()
            .position(|id| id == target_id)
            .ok_or_else(|| {
                BrowserRouteError::NotFound(format!("browser session {target_id} not found"))
            })?;
        let neighbor_index = match direction {
            BrowserSessionMoveDirection::Left => target_index.checked_sub(1),
            BrowserSessionMoveDirection::Right => {
                (target_index + 1 < ordered_ids.len()).then_some(target_index + 1)
            }
        };
        if let Some(neighbor_index) = neighbor_index {
            let neighbor_id = ordered_ids[neighbor_index].clone();
            let target_order = sessions
                .get(target_id)
                .map(|session| session.tab_order)
                .unwrap_or_else(|| browser_session_id_number(target_id));
            let neighbor_order = sessions
                .get(&neighbor_id)
                .map(|session| session.tab_order)
                .unwrap_or_else(|| browser_session_id_number(&neighbor_id));
            if let Some(target_session) = sessions.get_mut(target_id) {
                target_session.tab_order = neighbor_order;
            }
            if let Some(neighbor_session) = sessions.get_mut(&neighbor_id) {
                neighbor_session.tab_order = target_order;
            }
        }

        let (mut payload, back_href) = {
            let active = sessions.get_mut(active_id).ok_or_else(|| {
                BrowserRouteError::NotFound(format!("browser session {active_id} not found"))
            })?;
            let payload = browser_session_payload(active_id, active)?;
            (payload, active.back_href.clone())
        };
        self.record_browser_profile_tabs(&sessions, active_id).await;
        let closed_sessions = self.closed_sessions.lock().await;
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

    async fn move_tab_search_results_target(
        &self,
        active_id: &str,
        active_session: BrowserWebSession,
        to_front: bool,
    ) -> Result<(BrowserSessionPayload, String), BrowserRouteError> {
        let query = active_session.tab_search_query.clone();
        let mut sessions = self.sessions.lock().await;
        sessions.insert(active_id.to_owned(), active_session);
        let match_ids = browser_session_tab_search_results(&sessions, active_id, &query)
            .into_iter()
            .map(|result| result.id)
            .collect::<HashSet<_>>();
        let ordered_ids = browser_sorted_session_ids(&sessions);
        let (matches, non_matches): (Vec<_>, Vec<_>) = ordered_ids
            .into_iter()
            .partition(|id| match_ids.contains(id));
        let reordered_ids = if to_front {
            matches.into_iter().chain(non_matches).collect::<Vec<_>>()
        } else {
            non_matches.into_iter().chain(matches).collect::<Vec<_>>()
        };
        for (index, id) in reordered_ids.iter().enumerate() {
            if let Some(session) = sessions.get_mut(id) {
                session.tab_order = (index + 1) as u64;
            }
        }

        let (mut payload, back_href) = {
            let active = sessions.get_mut(active_id).ok_or_else(|| {
                BrowserRouteError::NotFound(format!("browser session {active_id} not found"))
            })?;
            let payload = browser_session_payload(active_id, active)?;
            (payload, active.back_href.clone())
        };
        self.record_browser_profile_tabs(&sessions, active_id).await;
        let closed_sessions = self.closed_sessions.lock().await;
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

    async fn close_tab_search_results_target(
        &self,
        active_id: &str,
        active_session: BrowserWebSession,
    ) -> Result<(BrowserSessionPayload, String), BrowserRouteError> {
        let query = active_session.tab_search_query.clone();
        let mut sessions = self.sessions.lock().await;
        sessions.insert(active_id.to_owned(), active_session);
        let close_ids = browser_session_tab_search_results(&sessions, active_id, &query)
            .into_iter()
            .filter(|result| !result.current && !result.pinned)
            .map(|result| result.id)
            .collect::<HashSet<_>>();

        let mut closed_sessions = self.closed_sessions.lock().await;
        for close_id in browser_sorted_session_ids(&sessions)
            .into_iter()
            .filter(|id| close_ids.contains(id))
        {
            let Some(closed_session) = sessions.remove(&close_id) else {
                continue;
            };
            let profile_closed_session =
                browser_stored_closed_session_from_web_session(&closed_session);
            remember_closed_browser_session(&mut closed_sessions, &close_id, closed_session);
            self.record_browser_profile_closed_session(profile_closed_session)
                .await;
        }

        let (mut payload, back_href) = {
            let active = sessions.get_mut(active_id).ok_or_else(|| {
                BrowserRouteError::NotFound(format!("browser session {active_id} not found"))
            })?;
            let payload = browser_session_payload(active_id, active)?;
            (payload, active.back_href.clone())
        };
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

    async fn close_tab_search_nonmatches_target(
        &self,
        active_id: &str,
        active_session: BrowserWebSession,
    ) -> Result<(BrowserSessionPayload, String), BrowserRouteError> {
        let query = active_session.tab_search_query.clone();
        let mut sessions = self.sessions.lock().await;
        sessions.insert(active_id.to_owned(), active_session);
        let match_ids = browser_session_tab_search_results(&sessions, active_id, &query)
            .into_iter()
            .map(|result| result.id)
            .collect::<HashSet<_>>();
        let close_ids = browser_sorted_session_ids(&sessions)
            .into_iter()
            .filter(|id| id != active_id)
            .filter(|id| !match_ids.contains(id))
            .filter(|id| !sessions.get(id).is_some_and(|session| session.pinned))
            .collect::<Vec<_>>();

        let mut closed_sessions = self.closed_sessions.lock().await;
        for close_id in close_ids {
            let Some(closed_session) = sessions.remove(&close_id) else {
                continue;
            };
            let profile_closed_session =
                browser_stored_closed_session_from_web_session(&closed_session);
            remember_closed_browser_session(&mut closed_sessions, &close_id, closed_session);
            self.record_browser_profile_closed_session(profile_closed_session)
                .await;
        }

        let (mut payload, back_href) = {
            let active = sessions.get_mut(active_id).ok_or_else(|| {
                BrowserRouteError::NotFound(format!("browser session {active_id} not found"))
            })?;
            let payload = browser_session_payload(active_id, active)?;
            (payload, active.back_href.clone())
        };
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

    async fn reload_tab_search_results_target(
        &self,
        active_id: &str,
        active_session: BrowserWebSession,
    ) -> Result<(BrowserSessionPayload, String), BrowserRouteError> {
        let query = active_session.tab_search_query.clone();
        let reload_ids = {
            let mut sessions = self.sessions.lock().await;
            sessions.insert(active_id.to_owned(), active_session);
            let match_ids = browser_session_tab_search_results(&sessions, active_id, &query)
                .into_iter()
                .map(|result| result.id)
                .collect::<HashSet<_>>();
            browser_sorted_session_ids(&sessions)
                .into_iter()
                .filter(|id| match_ids.contains(id))
                .collect::<Vec<_>>()
        };

        for reload_id in reload_ids {
            let Some(mut reload_session) = ({
                let mut sessions = self.sessions.lock().await;
                sessions.remove(&reload_id)
            }) else {
                continue;
            };
            if let Err(error) = reload_session.session.reload().await.map_err(|error| {
                BrowserRouteError::Upstream(format!("browser reload failed: {error:#}"))
            }) {
                self.sessions.lock().await.insert(reload_id, reload_session);
                return Err(error);
            }
            reset_viewport_after_navigation(&mut reload_session);
            clear_browser_find_active_line(&mut reload_session);
            self.sessions.lock().await.insert(reload_id, reload_session);
        }

        let mut sessions = self.sessions.lock().await;
        let (mut payload, back_href) = {
            let active = sessions.get_mut(active_id).ok_or_else(|| {
                BrowserRouteError::NotFound(format!("browser session {active_id} not found"))
            })?;
            let payload = browser_session_payload(active_id, active)?;
            (payload, active.back_href.clone())
        };
        self.record_browser_profile_tabs(&sessions, active_id).await;
        let closed_sessions = self.closed_sessions.lock().await;
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

    async fn set_tab_search_results_pinned_target(
        &self,
        active_id: &str,
        active_session: BrowserWebSession,
        pinned: bool,
    ) -> Result<(BrowserSessionPayload, String), BrowserRouteError> {
        let query = active_session.tab_search_query.clone();
        let mut sessions = self.sessions.lock().await;
        sessions.insert(active_id.to_owned(), active_session);
        let pin_ids = browser_session_tab_search_results(&sessions, active_id, &query)
            .into_iter()
            .filter(|result| result.pinned != pinned)
            .map(|result| result.id)
            .collect::<HashSet<_>>();

        for pin_id in pin_ids {
            if let Some(session) = sessions.get_mut(&pin_id) {
                session.pinned = pinned;
            }
        }

        let (mut payload, back_href) = {
            let active = sessions.get_mut(active_id).ok_or_else(|| {
                BrowserRouteError::NotFound(format!("browser session {active_id} not found"))
            })?;
            let payload = browser_session_payload(active_id, active)?;
            (payload, active.back_href.clone())
        };
        self.record_browser_profile_tabs(&sessions, active_id).await;
        let closed_sessions = self.closed_sessions.lock().await;
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

    async fn set_tab_search_results_label_target(
        &self,
        active_id: &str,
        active_session: BrowserWebSession,
        label: Option<String>,
    ) -> Result<(BrowserSessionPayload, String), BrowserRouteError> {
        let query = active_session.tab_search_query.clone();
        let mut sessions = self.sessions.lock().await;
        sessions.insert(active_id.to_owned(), active_session);
        let label_ids = browser_session_tab_search_results(&sessions, active_id, &query)
            .into_iter()
            .map(|result| result.id)
            .collect::<HashSet<_>>();

        for label_id in label_ids {
            if let Some(session) = sessions.get_mut(&label_id) {
                session.tab_label = label.clone();
            }
        }

        let (mut payload, back_href) = {
            let active = sessions.get_mut(active_id).ok_or_else(|| {
                BrowserRouteError::NotFound(format!("browser session {active_id} not found"))
            })?;
            let payload = browser_session_payload(active_id, active)?;
            (payload, active.back_href.clone())
        };
        self.record_browser_profile_tabs(&sessions, active_id).await;
        let closed_sessions = self.closed_sessions.lock().await;
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

    async fn set_session_pinned_target(
        &self,
        active_id: &str,
        mut active_session: BrowserWebSession,
        target_id: &str,
        pinned: bool,
    ) -> Result<(BrowserSessionPayload, String), BrowserRouteError> {
        let back_href = active_session.back_href.clone();
        let mut sessions = self.sessions.lock().await;
        if target_id == active_id {
            active_session.pinned = pinned;
        } else if let Some(target_session) = sessions.get_mut(target_id) {
            target_session.pinned = pinned;
        } else {
            sessions.insert(active_id.to_owned(), active_session);
            return Err(BrowserRouteError::NotFound(format!(
                "browser session {target_id} not found"
            )));
        }

        let mut payload = browser_session_payload(active_id, &mut active_session)?;
        sessions.insert(active_id.to_owned(), active_session);
        self.record_browser_profile_tabs(&sessions, active_id).await;
        let closed_sessions = self.closed_sessions.lock().await;
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

    async fn set_all_sessions_pinned_target(
        &self,
        active_id: &str,
        mut active_session: BrowserWebSession,
        pinned: bool,
    ) -> Result<(BrowserSessionPayload, String), BrowserRouteError> {
        let back_href = active_session.back_href.clone();
        active_session.pinned = pinned;
        let mut sessions = self.sessions.lock().await;
        for session in sessions.values_mut() {
            session.pinned = pinned;
        }

        let mut payload = browser_session_payload(active_id, &mut active_session)?;
        sessions.insert(active_id.to_owned(), active_session);
        self.record_browser_profile_tabs(&sessions, active_id).await;
        let closed_sessions = self.closed_sessions.lock().await;
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

    async fn set_session_label_target(
        &self,
        active_id: &str,
        mut active_session: BrowserWebSession,
        target_id: &str,
        label: Option<String>,
    ) -> Result<(BrowserSessionPayload, String), BrowserRouteError> {
        let back_href = active_session.back_href.clone();
        let mut sessions = self.sessions.lock().await;
        if target_id == active_id {
            active_session.tab_label = label;
        } else if let Some(target_session) = sessions.get_mut(target_id) {
            target_session.tab_label = label;
        } else {
            sessions.insert(active_id.to_owned(), active_session);
            return Err(BrowserRouteError::NotFound(format!(
                "browser session {target_id} not found"
            )));
        }

        let mut payload = browser_session_payload(active_id, &mut active_session)?;
        sessions.insert(active_id.to_owned(), active_session);
        self.record_browser_profile_tabs(&sessions, active_id).await;
        let closed_sessions = self.closed_sessions.lock().await;
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

    async fn switch_matching_session_target(
        &self,
        query: &str,
    ) -> Result<(BrowserSessionPayload, String), BrowserRouteError> {
        let query = query.trim();
        if query.is_empty() {
            return Err(BrowserRouteError::BadRequest(
                "missing browser session query".to_owned(),
            ));
        }

        let needle = query.to_lowercase();
        let mut sessions = self.sessions.lock().await;
        let selected_id = browser_sorted_session_ids(&sessions)
            .into_iter()
            .find(|id| {
                sessions
                    .get(id)
                    .is_some_and(|session| browser_session_matches_query(id, session, &needle))
            })
            .ok_or_else(|| {
                BrowserRouteError::NotFound(format!("no browser session matches {query}"))
            })?;
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
        new_session.pinned = false;
        new_session.tab_label = None;
        {
            let mut sessions = self.sessions.lock().await;
            sessions.insert(active_id.to_owned(), active_session);
        }

        apply_browser_action(browser_action, &mut new_session).await?;

        let new_id = self.next_session_id();
        new_session.tab_order = browser_session_id_number(&new_id);
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

    async fn open_browser_action_in_background_session_target(
        &self,
        active_id: &str,
        mut active_session: BrowserWebSession,
        browser_action: BrowserSessionAction,
    ) -> Result<(BrowserSessionPayload, String), BrowserRouteError> {
        let mut new_session = active_session.clone();
        new_session.pinned = false;
        new_session.tab_label = None;
        if let Err(error) = apply_browser_action(browser_action, &mut new_session).await {
            self.sessions
                .lock()
                .await
                .insert(active_id.to_owned(), active_session);
            return Err(error);
        }

        let new_id = self.next_session_id();
        new_session.tab_order = browser_session_id_number(&new_id);
        let new_payload = browser_session_payload(&new_id, &mut new_session)?;
        self.record_browser_profile_visit(&new_payload).await;
        let back_href = active_session.back_href.clone();
        let mut payload = browser_session_payload(active_id, &mut active_session)?;
        let mut sessions = self.sessions.lock().await;
        sessions.insert(active_id.to_owned(), active_session);
        sessions.insert(new_id.clone(), new_session);
        self.record_browser_profile_tabs(&sessions, active_id).await;
        let closed_sessions = self.closed_sessions.lock().await;
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

    async fn open_bookmarks_background_target(
        &self,
        active_id: &str,
        mut active_session: BrowserWebSession,
    ) -> Result<(BrowserSessionPayload, String), BrowserRouteError> {
        let bookmarks = self.bookmarks.lock().await.clone();
        let mut open_sources = HashSet::new();
        if let Some(source) = current_session_source(&active_session) {
            if !source.trim().is_empty() {
                open_sources.insert(source);
            }
        }
        {
            let sessions = self.sessions.lock().await;
            for session in sessions.values() {
                if let Some(source) = current_session_source(session) {
                    if !source.trim().is_empty() {
                        open_sources.insert(source);
                    }
                }
            }
        }

        let mut new_sessions = Vec::new();
        for bookmark in bookmarks
            .iter()
            .filter(|bookmark| !bookmark.source.trim().is_empty())
        {
            if !open_sources.insert(bookmark.source.clone()) {
                continue;
            }
            let mut new_session = active_session.clone();
            new_session.pinned = false;
            new_session.tab_label = None;
            if let Err(error) = apply_browser_action(
                BrowserSessionAction::Open(bookmark.source.clone()),
                &mut new_session,
            )
            .await
            {
                self.sessions
                    .lock()
                    .await
                    .insert(active_id.to_owned(), active_session);
                return Err(error);
            }
            let new_id = self.next_session_id();
            new_session.tab_order = browser_session_id_number(&new_id);
            let new_payload = match browser_session_payload(&new_id, &mut new_session) {
                Ok(payload) => payload,
                Err(error) => {
                    self.sessions
                        .lock()
                        .await
                        .insert(active_id.to_owned(), active_session);
                    return Err(error);
                }
            };
            self.record_browser_profile_visit(&new_payload).await;
            new_sessions.push((new_id, new_session));
        }

        let back_href = active_session.back_href.clone();
        let mut payload = match browser_session_payload(active_id, &mut active_session) {
            Ok(payload) => payload,
            Err(error) => {
                self.sessions
                    .lock()
                    .await
                    .insert(active_id.to_owned(), active_session);
                return Err(error);
            }
        };
        let mut sessions = self.sessions.lock().await;
        sessions.insert(active_id.to_owned(), active_session);
        for (new_id, new_session) in new_sessions {
            sessions.insert(new_id, new_session);
        }
        self.record_browser_profile_tabs(&sessions, active_id).await;
        let closed_sessions = self.closed_sessions.lock().await;
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

    async fn open_profile_history_background_target(
        &self,
        active_id: &str,
        mut active_session: BrowserWebSession,
        limit: usize,
    ) -> Result<(BrowserSessionPayload, String), BrowserRouteError> {
        let profile_history = self.profile_history.lock().await.clone();
        let mut open_sources = HashSet::new();
        if let Some(source) = current_session_source(&active_session) {
            if !source.trim().is_empty() {
                open_sources.insert(source);
            }
        }
        {
            let sessions = self.sessions.lock().await;
            for session in sessions.values() {
                if let Some(source) = current_session_source(session)
                    && !source.trim().is_empty()
                {
                    open_sources.insert(source);
                }
            }
        }

        let mut new_sessions = Vec::new();
        for entry in profile_history
            .iter()
            .rev()
            .take(MAX_BULK_BACKGROUND_LINKS)
            .take(limit)
            .filter(|entry| !entry.source.trim().is_empty())
        {
            if !open_sources.insert(entry.source.clone()) {
                continue;
            }
            let mut new_session = active_session.clone();
            new_session.pinned = false;
            new_session.tab_label = None;
            if let Err(error) = apply_browser_action(
                BrowserSessionAction::Open(entry.source.clone()),
                &mut new_session,
            )
            .await
            {
                self.sessions
                    .lock()
                    .await
                    .insert(active_id.to_owned(), active_session);
                return Err(error);
            }
            let new_id = self.next_session_id();
            new_session.tab_order = browser_session_id_number(&new_id);
            let new_payload = match browser_session_payload(&new_id, &mut new_session) {
                Ok(payload) => payload,
                Err(error) => {
                    self.sessions
                        .lock()
                        .await
                        .insert(active_id.to_owned(), active_session);
                    return Err(error);
                }
            };
            self.record_browser_profile_visit(&new_payload).await;
            new_sessions.push((new_id, new_session));
        }

        let back_href = active_session.back_href.clone();
        let mut payload = match browser_session_payload(active_id, &mut active_session) {
            Ok(payload) => payload,
            Err(error) => {
                self.sessions
                    .lock()
                    .await
                    .insert(active_id.to_owned(), active_session);
                return Err(error);
            }
        };
        let mut sessions = self.sessions.lock().await;
        sessions.insert(active_id.to_owned(), active_session);
        for (new_id, new_session) in new_sessions {
            sessions.insert(new_id, new_session);
        }
        self.record_browser_profile_tabs(&sessions, active_id).await;
        let closed_sessions = self.closed_sessions.lock().await;
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

    async fn open_bookmarks_new_sessions_target(
        &self,
        active_id: &str,
        active_session: BrowserWebSession,
    ) -> Result<(BrowserSessionPayload, String), BrowserRouteError> {
        let bookmarks = self.bookmarks.lock().await.clone();
        let mut open_sources = HashSet::new();
        if let Some(source) = current_session_source(&active_session) {
            if !source.trim().is_empty() {
                open_sources.insert(source);
            }
        }
        {
            let sessions = self.sessions.lock().await;
            for session in sessions.values() {
                if let Some(source) = current_session_source(session)
                    && !source.trim().is_empty()
                {
                    open_sources.insert(source);
                }
            }
        }

        let mut new_sessions = Vec::new();
        let mut selected_id = None;
        for bookmark in bookmarks
            .iter()
            .filter(|bookmark| !bookmark.source.trim().is_empty())
        {
            if !open_sources.insert(bookmark.source.clone()) {
                continue;
            }
            let mut new_session = active_session.clone();
            new_session.pinned = false;
            new_session.tab_label = None;
            if let Err(error) = apply_browser_action(
                BrowserSessionAction::Open(bookmark.source.clone()),
                &mut new_session,
            )
            .await
            {
                self.sessions
                    .lock()
                    .await
                    .insert(active_id.to_owned(), active_session);
                return Err(error);
            }
            let new_id = self.next_session_id();
            new_session.tab_order = browser_session_id_number(&new_id);
            let new_payload = match browser_session_payload(&new_id, &mut new_session) {
                Ok(payload) => payload,
                Err(error) => {
                    self.sessions
                        .lock()
                        .await
                        .insert(active_id.to_owned(), active_session);
                    return Err(error);
                }
            };
            self.record_browser_profile_visit(&new_payload).await;
            if selected_id.is_none() {
                selected_id = Some(new_id.clone());
            }
            new_sessions.push((new_id, new_session));
        }

        let selected_id = selected_id.unwrap_or_else(|| active_id.to_owned());
        let mut sessions = self.sessions.lock().await;
        sessions.insert(active_id.to_owned(), active_session);
        for (new_id, new_session) in new_sessions {
            sessions.insert(new_id, new_session);
        }
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

    async fn open_links_background_target(
        &self,
        active_id: &str,
        mut active_session: BrowserWebSession,
        limit: usize,
    ) -> Result<(BrowserSessionPayload, String), BrowserRouteError> {
        let link_count = active_session
            .session
            .current()
            .map(|render| render.links.len())
            .unwrap_or(0)
            .min(MAX_BULK_BACKGROUND_LINKS)
            .min(limit);
        let mut open_sources = HashSet::new();
        if let Some(source) = current_session_source(&active_session) {
            if !source.trim().is_empty() {
                open_sources.insert(source);
            }
        }
        {
            let sessions = self.sessions.lock().await;
            for session in sessions.values() {
                if let Some(source) = current_session_source(session) {
                    if !source.trim().is_empty() {
                        open_sources.insert(source);
                    }
                }
            }
        }

        let mut new_sessions = Vec::new();
        for index in 0..link_count {
            let mut new_session = active_session.clone();
            new_session.pinned = false;
            new_session.tab_label = None;
            if let Err(error) =
                apply_browser_action(BrowserSessionAction::Link(index), &mut new_session).await
            {
                self.sessions
                    .lock()
                    .await
                    .insert(active_id.to_owned(), active_session);
                return Err(error);
            }
            let Some(source) = current_session_source(&new_session) else {
                continue;
            };
            if source.trim().is_empty() || !open_sources.insert(source) {
                continue;
            }
            let new_id = self.next_session_id();
            new_session.tab_order = browser_session_id_number(&new_id);
            let new_payload = match browser_session_payload(&new_id, &mut new_session) {
                Ok(payload) => payload,
                Err(error) => {
                    self.sessions
                        .lock()
                        .await
                        .insert(active_id.to_owned(), active_session);
                    return Err(error);
                }
            };
            self.record_browser_profile_visit(&new_payload).await;
            new_sessions.push((new_id, new_session));
        }

        let back_href = active_session.back_href.clone();
        let mut payload = match browser_session_payload(active_id, &mut active_session) {
            Ok(payload) => payload,
            Err(error) => {
                self.sessions
                    .lock()
                    .await
                    .insert(active_id.to_owned(), active_session);
                return Err(error);
            }
        };
        let mut sessions = self.sessions.lock().await;
        sessions.insert(active_id.to_owned(), active_session);
        for (new_id, new_session) in new_sessions {
            sessions.insert(new_id, new_session);
        }
        self.record_browser_profile_tabs(&sessions, active_id).await;
        let closed_sessions = self.closed_sessions.lock().await;
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

    async fn open_links_new_sessions_target(
        &self,
        active_id: &str,
        active_session: BrowserWebSession,
        limit: usize,
    ) -> Result<(BrowserSessionPayload, String), BrowserRouteError> {
        let link_count = active_session
            .session
            .current()
            .map(|render| render.links.len())
            .unwrap_or(0)
            .min(MAX_BULK_BACKGROUND_LINKS)
            .min(limit);
        let mut open_sources = HashSet::new();
        if let Some(source) = current_session_source(&active_session) {
            if !source.trim().is_empty() {
                open_sources.insert(source);
            }
        }
        {
            let sessions = self.sessions.lock().await;
            for session in sessions.values() {
                if let Some(source) = current_session_source(session)
                    && !source.trim().is_empty()
                {
                    open_sources.insert(source);
                }
            }
        }

        let mut new_sessions = Vec::new();
        let mut selected_id = None;
        for index in 0..link_count {
            let mut new_session = active_session.clone();
            new_session.pinned = false;
            new_session.tab_label = None;
            if let Err(error) =
                apply_browser_action(BrowserSessionAction::Link(index), &mut new_session).await
            {
                self.sessions
                    .lock()
                    .await
                    .insert(active_id.to_owned(), active_session);
                return Err(error);
            }
            let Some(source) = current_session_source(&new_session) else {
                continue;
            };
            if source.trim().is_empty() || !open_sources.insert(source) {
                continue;
            }
            let new_id = self.next_session_id();
            new_session.tab_order = browser_session_id_number(&new_id);
            let new_payload = match browser_session_payload(&new_id, &mut new_session) {
                Ok(payload) => payload,
                Err(error) => {
                    self.sessions
                        .lock()
                        .await
                        .insert(active_id.to_owned(), active_session);
                    return Err(error);
                }
            };
            self.record_browser_profile_visit(&new_payload).await;
            if selected_id.is_none() {
                selected_id = Some(new_id.clone());
            }
            new_sessions.push((new_id, new_session));
        }

        let selected_id = selected_id.unwrap_or_else(|| active_id.to_owned());
        let mut sessions = self.sessions.lock().await;
        sessions.insert(active_id.to_owned(), active_session);
        for (new_id, new_session) in new_sessions {
            sessions.insert(new_id, new_session);
        }
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

    async fn open_resources_background_target(
        &self,
        active_id: &str,
        mut active_session: BrowserWebSession,
        limit: usize,
    ) -> Result<(BrowserSessionPayload, String), BrowserRouteError> {
        let resource_count = active_session
            .session
            .current()
            .map(|render| render.resources.len())
            .unwrap_or(0)
            .min(MAX_BULK_BACKGROUND_LINKS)
            .min(limit);
        let mut open_sources = HashSet::new();
        if let Some(source) = current_session_source(&active_session) {
            if !source.trim().is_empty() {
                open_sources.insert(source);
            }
        }
        {
            let sessions = self.sessions.lock().await;
            for session in sessions.values() {
                if let Some(source) = current_session_source(session)
                    && !source.trim().is_empty()
                {
                    open_sources.insert(source);
                }
            }
        }

        let mut new_sessions = Vec::new();
        for index in 0..resource_count {
            let mut new_session = active_session.clone();
            new_session.pinned = false;
            new_session.tab_label = None;
            if let Err(error) =
                apply_browser_action(BrowserSessionAction::Resource(index), &mut new_session).await
            {
                self.sessions
                    .lock()
                    .await
                    .insert(active_id.to_owned(), active_session);
                return Err(error);
            }
            let Some(source) = current_session_source(&new_session) else {
                continue;
            };
            if source.trim().is_empty() || !open_sources.insert(source) {
                continue;
            }
            let new_id = self.next_session_id();
            new_session.tab_order = browser_session_id_number(&new_id);
            let new_payload = match browser_session_payload(&new_id, &mut new_session) {
                Ok(payload) => payload,
                Err(error) => {
                    self.sessions
                        .lock()
                        .await
                        .insert(active_id.to_owned(), active_session);
                    return Err(error);
                }
            };
            self.record_browser_profile_visit(&new_payload).await;
            new_sessions.push((new_id, new_session));
        }

        let back_href = active_session.back_href.clone();
        let mut payload = match browser_session_payload(active_id, &mut active_session) {
            Ok(payload) => payload,
            Err(error) => {
                self.sessions
                    .lock()
                    .await
                    .insert(active_id.to_owned(), active_session);
                return Err(error);
            }
        };
        let mut sessions = self.sessions.lock().await;
        sessions.insert(active_id.to_owned(), active_session);
        for (new_id, new_session) in new_sessions {
            sessions.insert(new_id, new_session);
        }
        self.record_browser_profile_tabs(&sessions, active_id).await;
        let closed_sessions = self.closed_sessions.lock().await;
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

    async fn open_resources_new_sessions_target(
        &self,
        active_id: &str,
        active_session: BrowserWebSession,
        limit: usize,
    ) -> Result<(BrowserSessionPayload, String), BrowserRouteError> {
        let resource_count = active_session
            .session
            .current()
            .map(|render| render.resources.len())
            .unwrap_or(0)
            .min(MAX_BULK_BACKGROUND_LINKS)
            .min(limit);
        let mut open_sources = HashSet::new();
        if let Some(source) = current_session_source(&active_session) {
            if !source.trim().is_empty() {
                open_sources.insert(source);
            }
        }
        {
            let sessions = self.sessions.lock().await;
            for session in sessions.values() {
                if let Some(source) = current_session_source(session)
                    && !source.trim().is_empty()
                {
                    open_sources.insert(source);
                }
            }
        }

        let mut new_sessions = Vec::new();
        let mut selected_id = None;
        for index in 0..resource_count {
            let mut new_session = active_session.clone();
            new_session.pinned = false;
            new_session.tab_label = None;
            if let Err(error) =
                apply_browser_action(BrowserSessionAction::Resource(index), &mut new_session).await
            {
                self.sessions
                    .lock()
                    .await
                    .insert(active_id.to_owned(), active_session);
                return Err(error);
            }
            let Some(source) = current_session_source(&new_session) else {
                continue;
            };
            if source.trim().is_empty() || !open_sources.insert(source) {
                continue;
            }
            let new_id = self.next_session_id();
            new_session.tab_order = browser_session_id_number(&new_id);
            let new_payload = match browser_session_payload(&new_id, &mut new_session) {
                Ok(payload) => payload,
                Err(error) => {
                    self.sessions
                        .lock()
                        .await
                        .insert(active_id.to_owned(), active_session);
                    return Err(error);
                }
            };
            self.record_browser_profile_visit(&new_payload).await;
            if selected_id.is_none() {
                selected_id = Some(new_id.clone());
            }
            new_sessions.push((new_id, new_session));
        }

        let selected_id = selected_id.unwrap_or_else(|| active_id.to_owned());
        let mut sessions = self.sessions.lock().await;
        sessions.insert(active_id.to_owned(), active_session);
        for (new_id, new_session) in new_sessions {
            sessions.insert(new_id, new_session);
        }
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

    async fn open_find_matches_background_target(
        &self,
        active_id: &str,
        mut active_session: BrowserWebSession,
        limit: usize,
    ) -> Result<(BrowserSessionPayload, String), BrowserRouteError> {
        let match_indices = browser_bulk_find_match_indices(&active_session, limit)?;
        let mut new_sessions = Vec::new();
        for index in match_indices {
            let mut new_session = active_session.clone();
            new_session.pinned = false;
            new_session.tab_label = None;
            if let Err(error) =
                apply_browser_action(BrowserSessionAction::FindMatch(index), &mut new_session).await
            {
                self.sessions
                    .lock()
                    .await
                    .insert(active_id.to_owned(), active_session);
                return Err(error);
            }
            let new_id = self.next_session_id();
            new_session.tab_order = browser_session_id_number(&new_id);
            let new_payload = match browser_session_payload(&new_id, &mut new_session) {
                Ok(payload) => payload,
                Err(error) => {
                    self.sessions
                        .lock()
                        .await
                        .insert(active_id.to_owned(), active_session);
                    return Err(error);
                }
            };
            self.record_browser_profile_visit(&new_payload).await;
            new_sessions.push((new_id, new_session));
        }

        let back_href = active_session.back_href.clone();
        let mut payload = match browser_session_payload(active_id, &mut active_session) {
            Ok(payload) => payload,
            Err(error) => {
                self.sessions
                    .lock()
                    .await
                    .insert(active_id.to_owned(), active_session);
                return Err(error);
            }
        };
        let mut sessions = self.sessions.lock().await;
        sessions.insert(active_id.to_owned(), active_session);
        for (new_id, new_session) in new_sessions {
            sessions.insert(new_id, new_session);
        }
        self.record_browser_profile_tabs(&sessions, active_id).await;
        let closed_sessions = self.closed_sessions.lock().await;
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

    async fn open_find_matches_new_sessions_target(
        &self,
        active_id: &str,
        active_session: BrowserWebSession,
        limit: usize,
    ) -> Result<(BrowserSessionPayload, String), BrowserRouteError> {
        let match_indices = browser_bulk_find_match_indices(&active_session, limit)?;
        let mut new_sessions = Vec::new();
        let mut selected_id = None;
        for index in match_indices {
            let mut new_session = active_session.clone();
            new_session.pinned = false;
            new_session.tab_label = None;
            if let Err(error) =
                apply_browser_action(BrowserSessionAction::FindMatch(index), &mut new_session).await
            {
                self.sessions
                    .lock()
                    .await
                    .insert(active_id.to_owned(), active_session);
                return Err(error);
            }
            let new_id = self.next_session_id();
            new_session.tab_order = browser_session_id_number(&new_id);
            let new_payload = match browser_session_payload(&new_id, &mut new_session) {
                Ok(payload) => payload,
                Err(error) => {
                    self.sessions
                        .lock()
                        .await
                        .insert(active_id.to_owned(), active_session);
                    return Err(error);
                }
            };
            self.record_browser_profile_visit(&new_payload).await;
            if selected_id.is_none() {
                selected_id = Some(new_id.clone());
            }
            new_sessions.push((new_id, new_session));
        }

        let selected_id = selected_id.unwrap_or_else(|| active_id.to_owned());
        let mut sessions = self.sessions.lock().await;
        sessions.insert(active_id.to_owned(), active_session);
        for (new_id, new_session) in new_sessions {
            sessions.insert(new_id, new_session);
        }
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

    async fn open_profile_history_new_sessions_target(
        &self,
        active_id: &str,
        active_session: BrowserWebSession,
        limit: usize,
    ) -> Result<(BrowserSessionPayload, String), BrowserRouteError> {
        let profile_history = self.profile_history.lock().await.clone();
        let mut open_sources = HashSet::new();
        if let Some(source) = current_session_source(&active_session) {
            if !source.trim().is_empty() {
                open_sources.insert(source);
            }
        }
        {
            let sessions = self.sessions.lock().await;
            for session in sessions.values() {
                if let Some(source) = current_session_source(session)
                    && !source.trim().is_empty()
                {
                    open_sources.insert(source);
                }
            }
        }

        let mut new_sessions = Vec::new();
        let mut selected_id = None;
        for entry in profile_history
            .iter()
            .rev()
            .take(MAX_BULK_BACKGROUND_LINKS)
            .take(limit)
            .filter(|entry| !entry.source.trim().is_empty())
        {
            if !open_sources.insert(entry.source.clone()) {
                continue;
            }
            let mut new_session = active_session.clone();
            new_session.pinned = false;
            new_session.tab_label = None;
            if let Err(error) = apply_browser_action(
                BrowserSessionAction::Open(entry.source.clone()),
                &mut new_session,
            )
            .await
            {
                self.sessions
                    .lock()
                    .await
                    .insert(active_id.to_owned(), active_session);
                return Err(error);
            }
            let new_id = self.next_session_id();
            new_session.tab_order = browser_session_id_number(&new_id);
            let new_payload = match browser_session_payload(&new_id, &mut new_session) {
                Ok(payload) => payload,
                Err(error) => {
                    self.sessions
                        .lock()
                        .await
                        .insert(active_id.to_owned(), active_session);
                    return Err(error);
                }
            };
            self.record_browser_profile_visit(&new_payload).await;
            if selected_id.is_none() {
                selected_id = Some(new_id.clone());
            }
            new_sessions.push((new_id, new_session));
        }

        let selected_id = selected_id.unwrap_or_else(|| active_id.to_owned());
        let mut sessions = self.sessions.lock().await;
        sessions.insert(active_id.to_owned(), active_session);
        for (new_id, new_session) in new_sessions {
            sessions.insert(new_id, new_session);
        }
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

    async fn bookmark_page_links_target(
        &self,
        active_id: &str,
        mut active_session: BrowserWebSession,
    ) -> Result<(BrowserSessionPayload, String), BrowserRouteError> {
        let bookmark_entries = {
            let render = active_session.session.current().ok_or_else(|| {
                BrowserRouteError::BadRequest("browser session has no current page".to_owned())
            })?;
            let mut seen_sources = HashSet::new();
            render
                .links
                .iter()
                .take(MAX_BULK_BACKGROUND_LINKS)
                .filter_map(|link| {
                    let source = link.resolved.trim();
                    if source.is_empty() || !seen_sources.insert(source.to_owned()) {
                        return None;
                    }
                    let title = if link.text.trim().is_empty() {
                        source.to_owned()
                    } else {
                        link.text.trim().to_owned()
                    };
                    Some((title, source.to_owned()))
                })
                .collect::<Vec<_>>()
        };

        if !bookmark_entries.is_empty() {
            {
                let mut bookmarks = self.bookmarks.lock().await;
                for (title, source) in bookmark_entries {
                    if let Some(bookmark) = bookmarks
                        .iter_mut()
                        .find(|bookmark| bookmark.source == source)
                    {
                        bookmark.title = title;
                    } else {
                        let bookmark_id =
                            self.next_bookmark_id.fetch_add(1, AtomicOrdering::Relaxed);
                        bookmarks.push(BrowserStoredBookmark {
                            id: format!("b{bookmark_id}"),
                            title,
                            source,
                        });
                    }
                }
            }
            self.save_browser_profile().await;
        }

        let back_href = active_session.back_href.clone();
        let mut payload = match browser_session_payload(active_id, &mut active_session) {
            Ok(payload) => payload,
            Err(error) => {
                self.sessions
                    .lock()
                    .await
                    .insert(active_id.to_owned(), active_session);
                return Err(error);
            }
        };
        let mut sessions = self.sessions.lock().await;
        sessions.insert(active_id.to_owned(), active_session);
        self.record_browser_profile_tabs(&sessions, active_id).await;
        let closed_sessions = self.closed_sessions.lock().await;
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

    async fn remove_page_link_bookmarks_target(
        &self,
        active_id: &str,
        mut active_session: BrowserWebSession,
    ) -> Result<(BrowserSessionPayload, String), BrowserRouteError> {
        let link_sources = {
            let render = active_session.session.current().ok_or_else(|| {
                BrowserRouteError::BadRequest("browser session has no current page".to_owned())
            })?;
            render
                .links
                .iter()
                .take(MAX_BULK_BACKGROUND_LINKS)
                .filter_map(|link| {
                    let source = link.resolved.trim();
                    (!source.is_empty()).then(|| source.to_owned())
                })
                .collect::<HashSet<_>>()
        };

        if !link_sources.is_empty() {
            let removed = {
                let mut bookmarks = self.bookmarks.lock().await;
                let previous_len = bookmarks.len();
                bookmarks.retain(|bookmark| !link_sources.contains(bookmark.source.trim()));
                bookmarks.len() != previous_len
            };
            if removed {
                self.save_browser_profile().await;
            }
        }

        let back_href = active_session.back_href.clone();
        let mut payload = match browser_session_payload(active_id, &mut active_session) {
            Ok(payload) => payload,
            Err(error) => {
                self.sessions
                    .lock()
                    .await
                    .insert(active_id.to_owned(), active_session);
                return Err(error);
            }
        };
        let mut sessions = self.sessions.lock().await;
        sessions.insert(active_id.to_owned(), active_session);
        self.record_browser_profile_tabs(&sessions, active_id).await;
        let closed_sessions = self.closed_sessions.lock().await;
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

    async fn duplicate_session_target(
        &self,
        duplicate_id: &str,
    ) -> Result<(BrowserSessionPayload, String), BrowserRouteError> {
        let mut sessions = self.sessions.lock().await;
        let mut duplicated = sessions.get(duplicate_id).cloned().ok_or_else(|| {
            BrowserRouteError::NotFound(format!("browser session {duplicate_id} not found"))
        })?;
        duplicated.pinned = false;
        duplicated.tab_label = None;
        let new_id = self.next_session_id();
        duplicated.tab_order = browser_session_id_number(&new_id);
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

    async fn duplicate_session_background_target(
        &self,
        active_id: &str,
        active_session: BrowserWebSession,
        duplicate_id: &str,
    ) -> Result<(BrowserSessionPayload, String), BrowserRouteError> {
        let mut sessions = self.sessions.lock().await;
        sessions.insert(active_id.to_owned(), active_session);
        let mut duplicated = sessions.get(duplicate_id).cloned().ok_or_else(|| {
            BrowserRouteError::NotFound(format!("browser session {duplicate_id} not found"))
        })?;
        duplicated.pinned = false;
        duplicated.tab_label = None;
        let new_id = self.next_session_id();
        duplicated.tab_order = browser_session_id_number(&new_id);
        let visit_payload = browser_session_payload(&new_id, &mut duplicated)?;
        sessions.insert(new_id, duplicated);
        let (mut payload, back_href) = {
            let active = sessions.get_mut(active_id).ok_or_else(|| {
                BrowserRouteError::NotFound(format!("browser session {active_id} not found"))
            })?;
            let payload = browser_session_payload(active_id, active)?;
            (payload, active.back_href.clone())
        };
        self.record_browser_profile_visit(&visit_payload).await;
        self.record_browser_profile_tabs(&sessions, active_id).await;
        let closed_sessions = self.closed_sessions.lock().await;
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

    async fn duplicate_tab_search_results_target(
        &self,
        active_id: &str,
        active_session: BrowserWebSession,
    ) -> Result<(BrowserSessionPayload, String), BrowserRouteError> {
        let query = active_session.tab_search_query.clone();
        let mut sessions = self.sessions.lock().await;
        sessions.insert(active_id.to_owned(), active_session);
        let duplicate_ids = browser_session_tab_search_results(&sessions, active_id, &query)
            .into_iter()
            .map(|result| result.id)
            .collect::<HashSet<_>>();
        let ordered_duplicate_ids = browser_sorted_session_ids(&sessions)
            .into_iter()
            .filter(|id| duplicate_ids.contains(id))
            .collect::<Vec<_>>();
        let mut visit_payloads = Vec::new();
        for duplicate_id in ordered_duplicate_ids {
            let Some(mut duplicated) = sessions.get(&duplicate_id).cloned() else {
                continue;
            };
            duplicated.pinned = false;
            duplicated.tab_label = None;
            let new_id = self.next_session_id();
            duplicated.tab_order = browser_session_id_number(&new_id);
            let visit_payload = browser_session_payload(&new_id, &mut duplicated)?;
            visit_payloads.push(visit_payload);
            sessions.insert(new_id, duplicated);
        }

        let (mut payload, back_href) = {
            let active = sessions.get_mut(active_id).ok_or_else(|| {
                BrowserRouteError::NotFound(format!("browser session {active_id} not found"))
            })?;
            let payload = browser_session_payload(active_id, active)?;
            (payload, active.back_href.clone())
        };
        for visit_payload in &visit_payloads {
            self.record_browser_profile_visit(visit_payload).await;
        }
        self.record_browser_profile_tabs(&sessions, active_id).await;
        let closed_sessions = self.closed_sessions.lock().await;
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
            selected.viewport_y = parse_optional_usize_param(target, "y", 0, usize::MAX)
                .or_else(|| parse_optional_usize_param(target, "viewport_y", 0, usize::MAX))
                .unwrap_or(selected.viewport_y);
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
        restored.tab_order = browser_session_id_number(&restored_id);
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

    async fn restore_closed_background_target(
        &self,
        target: &RequestTarget,
        active_id: &str,
        mut active_session: BrowserWebSession,
        closed_id: &str,
    ) -> Result<(BrowserSessionPayload, String), BrowserRouteError> {
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
        restored.tab_order = browser_session_id_number(&restored_id);
        let restored_payload = browser_session_payload(&restored_id, &mut restored)?;
        self.record_browser_profile_visit(&restored_payload).await;
        let back_href = active_session.back_href.clone();
        let mut payload = browser_session_payload(active_id, &mut active_session)?;
        let mut sessions = self.sessions.lock().await;
        sessions.insert(active_id.to_owned(), active_session);
        sessions.insert(restored_id, restored);
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

    async fn restore_closed_sessions_background_target(
        &self,
        target: &RequestTarget,
        active_id: &str,
        mut active_session: BrowserWebSession,
    ) -> Result<(BrowserSessionPayload, String), BrowserRouteError> {
        let width = parse_optional_usize_param(target, "width", 40, 160);
        let height = parse_optional_usize_param(target, "height", 16, 120);
        let max_bytes =
            parse_optional_usize_param(target, "max_bytes", 64 * 1024, 16 * 1024 * 1024);
        let return_href = target
            .param("from")
            .map(|href| sanitized_search_return_href(Some(&href)));
        let live_closed_sessions = {
            let closed_sessions = self.closed_sessions.lock().await;
            closed_sessions
                .iter()
                .map(|closed| (closed.source.clone(), closed.session.clone()))
                .collect::<Vec<_>>()
        };
        let profile_closed_sessions = self.profile_closed_sessions.lock().await.clone();

        let mut restored_sources = HashSet::new();
        let mut restored_sessions = Vec::new();
        let mut visit_payloads = Vec::new();
        for (source, mut restored) in live_closed_sessions {
            restored.width = width.unwrap_or(restored.width);
            restored.height = height.unwrap_or(restored.height);
            restored.max_bytes = max_bytes.unwrap_or(restored.max_bytes);
            if let Some(return_href) = return_href.as_ref() {
                restored.back_href = return_href.clone();
            }

            let restored_id = self.next_session_id();
            restored.tab_order = browser_session_id_number(&restored_id);
            let visit_payload = match browser_session_payload(&restored_id, &mut restored) {
                Ok(payload) => payload,
                Err(error) => {
                    self.sessions
                        .lock()
                        .await
                        .insert(active_id.to_owned(), active_session);
                    return Err(error);
                }
            };
            let source = source.trim();
            if !source.is_empty() {
                restored_sources.insert(source.to_owned());
            } else if let Some(source) = current_session_source(&restored)
                && !source.trim().is_empty()
            {
                restored_sources.insert(source);
            }
            visit_payloads.push(visit_payload);
            restored_sessions.push((restored_id, restored));
        }

        for closed in profile_closed_sessions {
            let source = closed.source.trim();
            if source.is_empty() || !restored_sources.insert(source.to_owned()) {
                continue;
            }
            let mut restored = active_session.clone();
            restored.pinned = false;
            restored.tab_label = None;
            if let Some(return_href) = return_href.as_ref() {
                restored.back_href = return_href.clone();
            }
            if let Err(error) =
                apply_browser_action(BrowserSessionAction::Open(source.to_owned()), &mut restored)
                    .await
            {
                self.sessions
                    .lock()
                    .await
                    .insert(active_id.to_owned(), active_session);
                return Err(error);
            }

            let restored_id = self.next_session_id();
            restored.tab_order = browser_session_id_number(&restored_id);
            let visit_payload = match browser_session_payload(&restored_id, &mut restored) {
                Ok(payload) => payload,
                Err(error) => {
                    self.sessions
                        .lock()
                        .await
                        .insert(active_id.to_owned(), active_session);
                    return Err(error);
                }
            };
            visit_payloads.push(visit_payload);
            restored_sessions.push((restored_id, restored));
        }

        if restored_sessions.is_empty() {
            self.sessions
                .lock()
                .await
                .insert(active_id.to_owned(), active_session);
            return Err(BrowserRouteError::BadRequest(
                "no closed browser sessions to restore".to_owned(),
            ));
        }

        for visit_payload in &visit_payloads {
            self.record_browser_profile_visit(visit_payload).await;
        }
        {
            let mut closed_sessions = self.closed_sessions.lock().await;
            closed_sessions.clear();
        }
        {
            let mut profile_closed_sessions = self.profile_closed_sessions.lock().await;
            profile_closed_sessions
                .retain(|closed| !restored_sources.contains(closed.source.trim()));
        }
        self.save_browser_profile().await;

        let back_href = active_session.back_href.clone();
        let mut payload = match browser_session_payload(active_id, &mut active_session) {
            Ok(payload) => payload,
            Err(error) => {
                self.sessions
                    .lock()
                    .await
                    .insert(active_id.to_owned(), active_session);
                return Err(error);
            }
        };
        let mut sessions = self.sessions.lock().await;
        sessions.insert(active_id.to_owned(), active_session);
        for (restored_id, restored) in restored_sessions {
            sessions.insert(restored_id, restored);
        }
        self.record_browser_profile_tabs(&sessions, active_id).await;
        let closed_sessions = self.closed_sessions.lock().await;
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

    async fn open_profile_closed_background_target(
        &self,
        active_id: &str,
        mut active_session: BrowserWebSession,
        index: usize,
    ) -> Result<(BrowserSessionPayload, String), BrowserRouteError> {
        let source = self.profile_closed_source(index).await?;
        let mut new_session = active_session.clone();
        new_session.pinned = false;
        new_session.tab_label = None;
        if let Err(error) =
            apply_browser_action(BrowserSessionAction::Open(source), &mut new_session).await
        {
            self.sessions
                .lock()
                .await
                .insert(active_id.to_owned(), active_session);
            return Err(error);
        }
        self.remove_profile_closed(index).await?;

        let new_id = self.next_session_id();
        new_session.tab_order = browser_session_id_number(&new_id);
        let new_payload = browser_session_payload(&new_id, &mut new_session)?;
        self.record_browser_profile_visit(&new_payload).await;
        let back_href = active_session.back_href.clone();
        let mut payload = browser_session_payload(active_id, &mut active_session)?;
        let mut sessions = self.sessions.lock().await;
        sessions.insert(active_id.to_owned(), active_session);
        sessions.insert(new_id, new_session);
        self.record_browser_profile_tabs(&sessions, active_id).await;
        let closed_sessions = self.closed_sessions.lock().await;
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

    async fn bookmark_all_tabs_target(
        &self,
        active_id: &str,
        active_session: BrowserWebSession,
    ) -> Result<(BrowserSessionPayload, String), BrowserRouteError> {
        let bookmark_entries = {
            let mut sessions = self.sessions.lock().await;
            sessions.insert(active_id.to_owned(), active_session);
            let mut seen_sources = HashSet::new();
            browser_sorted_session_ids(&sessions)
                .into_iter()
                .filter_map(|id| {
                    let render = sessions.get(&id)?.session.current()?;
                    let source = render.source.trim();
                    if source.is_empty() || !seen_sources.insert(source.to_owned()) {
                        return None;
                    }
                    Some((browser_session_title(render), source.to_owned()))
                })
                .collect::<Vec<_>>()
        };

        if !bookmark_entries.is_empty() {
            {
                let mut bookmarks = self.bookmarks.lock().await;
                for (title, source) in bookmark_entries {
                    if let Some(bookmark) = bookmarks
                        .iter_mut()
                        .find(|bookmark| bookmark.source == source)
                    {
                        bookmark.title = title;
                    } else {
                        let bookmark_id =
                            self.next_bookmark_id.fetch_add(1, AtomicOrdering::Relaxed);
                        bookmarks.push(BrowserStoredBookmark {
                            id: format!("b{bookmark_id}"),
                            title,
                            source,
                        });
                    }
                }
            }
            self.save_browser_profile().await;
        }

        let mut sessions = self.sessions.lock().await;
        let (mut payload, back_href) = {
            let active = sessions.get_mut(active_id).ok_or_else(|| {
                BrowserRouteError::NotFound(format!("browser session {active_id} not found"))
            })?;
            let payload = browser_session_payload(active_id, active)?;
            (payload, active.back_href.clone())
        };
        self.record_browser_profile_tabs(&sessions, active_id).await;
        let closed_sessions = self.closed_sessions.lock().await;
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

    async fn bookmark_profile_history_target(
        &self,
        active_id: &str,
        active_session: BrowserWebSession,
    ) -> Result<(BrowserSessionPayload, String), BrowserRouteError> {
        let bookmark_entries = {
            let profile_history = self.profile_history.lock().await;
            let mut seen_sources = HashSet::new();
            profile_history
                .iter()
                .rev()
                .take(MAX_VISIBLE_BROWSER_PROFILE_HISTORY)
                .filter_map(|entry| {
                    let source = entry.source.trim();
                    if source.is_empty() || !seen_sources.insert(source.to_owned()) {
                        return None;
                    }
                    Some((entry.title.clone(), source.to_owned()))
                })
                .collect::<Vec<_>>()
        };

        if !bookmark_entries.is_empty() {
            {
                let mut bookmarks = self.bookmarks.lock().await;
                for (title, source) in bookmark_entries {
                    if let Some(bookmark) = bookmarks
                        .iter_mut()
                        .find(|bookmark| bookmark.source == source)
                    {
                        bookmark.title = title;
                    } else {
                        let bookmark_id =
                            self.next_bookmark_id.fetch_add(1, AtomicOrdering::Relaxed);
                        bookmarks.push(BrowserStoredBookmark {
                            id: format!("b{bookmark_id}"),
                            title,
                            source,
                        });
                    }
                }
            }
            self.save_browser_profile().await;
        }

        let mut sessions = self.sessions.lock().await;
        sessions.insert(active_id.to_owned(), active_session);
        let (mut payload, back_href) = {
            let active = sessions.get_mut(active_id).ok_or_else(|| {
                BrowserRouteError::NotFound(format!("browser session {active_id} not found"))
            })?;
            let payload = browser_session_payload(active_id, active)?;
            (payload, active.back_href.clone())
        };
        self.record_browser_profile_tabs(&sessions, active_id).await;
        let closed_sessions = self.closed_sessions.lock().await;
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

    async fn remove_profile_history_bookmarks_target(
        &self,
        active_id: &str,
        active_session: BrowserWebSession,
    ) -> Result<(BrowserSessionPayload, String), BrowserRouteError> {
        let history_sources = {
            let profile_history = self.profile_history.lock().await;
            profile_history
                .iter()
                .rev()
                .take(MAX_VISIBLE_BROWSER_PROFILE_HISTORY)
                .filter_map(|entry| {
                    let source = entry.source.trim();
                    (!source.is_empty()).then(|| source.to_owned())
                })
                .collect::<HashSet<_>>()
        };

        if !history_sources.is_empty() {
            let removed = {
                let mut bookmarks = self.bookmarks.lock().await;
                let previous_len = bookmarks.len();
                bookmarks.retain(|bookmark| !history_sources.contains(bookmark.source.trim()));
                bookmarks.len() != previous_len
            };
            if removed {
                self.save_browser_profile().await;
            }
        }

        let mut sessions = self.sessions.lock().await;
        sessions.insert(active_id.to_owned(), active_session);
        let (mut payload, back_href) = {
            let active = sessions.get_mut(active_id).ok_or_else(|| {
                BrowserRouteError::NotFound(format!("browser session {active_id} not found"))
            })?;
            let payload = browser_session_payload(active_id, active)?;
            (payload, active.back_href.clone())
        };
        self.record_browser_profile_tabs(&sessions, active_id).await;
        let closed_sessions = self.closed_sessions.lock().await;
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

    async fn bookmark_tab_search_results_target(
        &self,
        active_id: &str,
        active_session: BrowserWebSession,
    ) -> Result<(BrowserSessionPayload, String), BrowserRouteError> {
        let query = active_session.tab_search_query.clone();
        let bookmark_entries = {
            let mut sessions = self.sessions.lock().await;
            sessions.insert(active_id.to_owned(), active_session);
            let match_ids = browser_session_tab_search_results(&sessions, active_id, &query)
                .into_iter()
                .map(|result| result.id)
                .collect::<HashSet<_>>();
            let mut seen_sources = HashSet::new();
            browser_sorted_session_ids(&sessions)
                .into_iter()
                .filter(|id| match_ids.contains(id))
                .filter_map(|id| {
                    let render = sessions.get(&id)?.session.current()?;
                    let source = render.source.trim();
                    if source.is_empty() || !seen_sources.insert(source.to_owned()) {
                        return None;
                    }
                    Some((browser_session_title(render), source.to_owned()))
                })
                .collect::<Vec<_>>()
        };

        if !bookmark_entries.is_empty() {
            {
                let mut bookmarks = self.bookmarks.lock().await;
                for (title, source) in bookmark_entries {
                    if let Some(bookmark) = bookmarks
                        .iter_mut()
                        .find(|bookmark| bookmark.source == source)
                    {
                        bookmark.title = title;
                    } else {
                        let bookmark_id =
                            self.next_bookmark_id.fetch_add(1, AtomicOrdering::Relaxed);
                        bookmarks.push(BrowserStoredBookmark {
                            id: format!("b{bookmark_id}"),
                            title,
                            source,
                        });
                    }
                }
            }
            self.save_browser_profile().await;
        }

        let mut sessions = self.sessions.lock().await;
        let (mut payload, back_href) = {
            let active = sessions.get_mut(active_id).ok_or_else(|| {
                BrowserRouteError::NotFound(format!("browser session {active_id} not found"))
            })?;
            let payload = browser_session_payload(active_id, active)?;
            (payload, active.back_href.clone())
        };
        self.record_browser_profile_tabs(&sessions, active_id).await;
        let closed_sessions = self.closed_sessions.lock().await;
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

    async fn remove_tab_search_bookmarks_target(
        &self,
        active_id: &str,
        active_session: BrowserWebSession,
    ) -> Result<(BrowserSessionPayload, String), BrowserRouteError> {
        let query = active_session.tab_search_query.clone();
        let bookmark_sources = {
            let mut sessions = self.sessions.lock().await;
            sessions.insert(active_id.to_owned(), active_session);
            let match_ids = browser_session_tab_search_results(&sessions, active_id, &query)
                .into_iter()
                .map(|result| result.id)
                .collect::<HashSet<_>>();
            browser_sorted_session_ids(&sessions)
                .into_iter()
                .filter(|id| match_ids.contains(id))
                .filter_map(|id| {
                    let source = sessions
                        .get(&id)?
                        .session
                        .current()?
                        .source
                        .trim()
                        .to_owned();
                    (!source.is_empty()).then_some(source)
                })
                .collect::<HashSet<_>>()
        };

        if !bookmark_sources.is_empty() {
            {
                let mut bookmarks = self.bookmarks.lock().await;
                bookmarks.retain(|bookmark| !bookmark_sources.contains(&bookmark.source));
            }
            self.save_browser_profile().await;
        }

        let mut sessions = self.sessions.lock().await;
        let (mut payload, back_href) = {
            let active = sessions.get_mut(active_id).ok_or_else(|| {
                BrowserRouteError::NotFound(format!("browser session {active_id} not found"))
            })?;
            let payload = browser_session_payload(active_id, active)?;
            (payload, active.back_href.clone())
        };
        self.record_browser_profile_tabs(&sessions, active_id).await;
        let closed_sessions = self.closed_sessions.lock().await;
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

    async fn rename_bookmark(
        &self,
        bookmark_id: &str,
        title: &str,
    ) -> Result<(), BrowserRouteError> {
        let Some(title) = normalize_browser_tab_label_option(Some(title)) else {
            return Err(BrowserRouteError::BadRequest(
                "missing browser bookmark title".to_owned(),
            ));
        };
        {
            let mut bookmarks = self.bookmarks.lock().await;
            let bookmark = bookmarks
                .iter_mut()
                .find(|bookmark| bookmark.id == bookmark_id)
                .ok_or_else(|| {
                    BrowserRouteError::NotFound(format!("browser bookmark {bookmark_id} not found"))
                })?;
            bookmark.title = title;
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
                    pinned: session.pinned,
                    label: session.tab_label.clone(),
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
        let profile_tabs = self.profile_tabs.lock().await;
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
            &profile_tabs,
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
    profile_tabs: &[BrowserStoredProfileTab],
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
    payload.bookmarks_background_url = (!bookmarks.is_empty()).then(|| {
        browser_session_action_href(
            &payload.id,
            "open-bookmarks-background-sessions",
            &[],
            payload,
        )
    });
    payload.bookmarks = browser_session_bookmarks(bookmarks, payload);
    payload.profile_tabs_clear_url = (!profile_tabs.is_empty())
        .then(|| browser_session_action_href(&payload.id, "clear-profile-tabs", &[], payload));
    payload.profile_history_clear_url = (!profile_history.is_empty())
        .then(|| browser_session_action_href(&payload.id, "clear-profile-history", &[], payload));
    payload.profile_history = browser_session_profile_history(profile_history, payload);
    payload.tab_search_results =
        browser_session_tab_search_results(sessions, current_id, &payload.tab_search_query);
}

fn browser_session_summaries(
    sessions: &HashMap<String, BrowserWebSession>,
    current_id: &str,
) -> Vec<BrowserSessionSummaryPayload> {
    let can_close = sessions.len() > 1;
    let close_href_source = sessions.get(current_id);
    let ordered_ids = browser_sorted_session_ids(sessions);
    ordered_ids
        .iter()
        .enumerate()
        .filter_map(|(index, id)| {
            let session = sessions.get(id)?;
            let (page_title, source) = session
                .session
                .current()
                .map(|render| {
                    (
                        browser_session_display_title(render, session.display_source.as_deref()),
                        render.source.clone(),
                    )
                })
                .unwrap_or_else(|| ("Untitled".to_owned(), String::new()));
            let (page_title, source) = if let Some(pending_source) = session.pending_source.as_ref()
            {
                (
                    format!(
                        "Loading {}",
                        browser_session_feedback_excerpt(pending_source)
                    ),
                    pending_source.clone(),
                )
            } else {
                (page_title, session.display_source.clone().unwrap_or(source))
            };
            let title = session
                .tab_label
                .clone()
                .unwrap_or_else(|| page_title.clone());
            let href_source = close_href_source.unwrap_or(session);
            Some(BrowserSessionSummaryPayload {
                id: id.clone(),
                position: index + 1,
                order: session.tab_order,
                title,
                page_title,
                label: session.tab_label.clone(),
                source: source.clone(),
                action_url: browser_session_action_href(id, "current", &[], session),
                reload_url: browser_session_action_href(id, "reload", &[], session),
                duplicate_url: browser_session_action_href(
                    current_id,
                    "duplicate-session",
                    &[("session", id.clone())],
                    href_source,
                ),
                duplicate_background_url: browser_session_action_href(
                    current_id,
                    "duplicate-background-session",
                    &[("session", id.clone())],
                    href_source,
                ),
                label_url: browser_session_action_href(
                    current_id,
                    "label-tab",
                    &[("session", id.clone())],
                    href_source,
                ),
                clear_label_url: session.tab_label.as_ref().map_or_else(String::new, |_| {
                    browser_session_action_href(
                        current_id,
                        "clear-tab-label",
                        &[("session", id.clone())],
                        href_source,
                    )
                }),
                move_left_url: (index > 0)
                    .then(|| {
                        browser_session_action_href(
                            current_id,
                            "move-tab-left",
                            &[("session", id.clone())],
                            href_source,
                        )
                    })
                    .unwrap_or_default(),
                move_right_url: (index + 1 < ordered_ids.len())
                    .then(|| {
                        browser_session_action_href(
                            current_id,
                            "move-tab-right",
                            &[("session", id.clone())],
                            href_source,
                        )
                    })
                    .unwrap_or_default(),
                pin_url: browser_session_action_href(
                    current_id,
                    "pin-tab",
                    &[("session", id.clone())],
                    href_source,
                ),
                unpin_url: browser_session_action_href(
                    current_id,
                    "unpin-tab",
                    &[("session", id.clone())],
                    href_source,
                ),
                close_url: (can_close)
                    .then(|| {
                        close_href_source.map(|source| {
                            browser_session_action_href(
                                current_id,
                                "close-session",
                                &[("close_id", id.clone())],
                                source,
                            )
                        })
                    })
                    .flatten()
                    .unwrap_or_default(),
                current: id == current_id,
                can_close,
                can_move_left: index > 0,
                can_move_right: index + 1 < ordered_ids.len(),
                pinned: session.pinned,
            })
        })
        .collect()
}

fn browser_session_tab_search_results(
    sessions: &HashMap<String, BrowserWebSession>,
    current_id: &str,
    query: &str,
) -> Vec<BrowserSessionTabSearchResultPayload> {
    let query = normalize_browser_search_query(query);
    if query.is_empty() {
        return Vec::new();
    }
    let needle = query.to_lowercase();
    let mut results = Vec::new();
    let can_close = sessions.len() > 1;
    let active_href_source = sessions.get(current_id);
    for id in browser_sorted_session_ids(sessions) {
        let Some(session) = sessions.get(&id) else {
            continue;
        };
        let Some(render) = session.session.current() else {
            continue;
        };
        let page_title = browser_session_title(render);
        let title = session
            .tab_label
            .clone()
            .unwrap_or_else(|| page_title.clone());
        let action_url = browser_session_action_href(&id, "current", &[], session);
        let href_source = active_href_source.unwrap_or(session);
        let current = id == current_id;
        if let Some(label) = session.tab_label.as_ref()
            && label.to_lowercase().contains(&needle)
        {
            push_browser_session_tab_search_result(
                &mut results,
                &id,
                session,
                &title,
                &page_title,
                &render.source,
                &action_url,
                current_id,
                href_source,
                can_close,
                current,
                "label",
                None,
                label.clone(),
            );
        }
        if page_title.to_lowercase().contains(&needle) {
            push_browser_session_tab_search_result(
                &mut results,
                &id,
                session,
                &title,
                &page_title,
                &render.source,
                &action_url,
                current_id,
                href_source,
                can_close,
                current,
                "title",
                None,
                page_title.clone(),
            );
        }
        if render.source.to_lowercase().contains(&needle) {
            push_browser_session_tab_search_result(
                &mut results,
                &id,
                session,
                &title,
                &page_title,
                &render.source,
                &action_url,
                current_id,
                href_source,
                can_close,
                current,
                "source",
                None,
                render.source.clone(),
            );
        }
        for (line_index, line) in render.text.lines().enumerate() {
            if results.len() >= 120 {
                break;
            }
            if line.to_lowercase().contains(&needle) {
                push_browser_session_tab_search_result(
                    &mut results,
                    &id,
                    session,
                    &title,
                    &page_title,
                    &render.source,
                    &action_url,
                    current_id,
                    href_source,
                    can_close,
                    current,
                    "text",
                    Some(line_index),
                    line.trim().to_owned(),
                );
            }
        }
        if results.len() >= 120 {
            break;
        }
    }
    results
}

#[allow(clippy::too_many_arguments)]
fn push_browser_session_tab_search_result(
    results: &mut Vec<BrowserSessionTabSearchResultPayload>,
    id: &str,
    session: &BrowserWebSession,
    title: &str,
    page_title: &str,
    source: &str,
    action_url: &str,
    current_id: &str,
    href_source: &BrowserWebSession,
    can_close: bool,
    current: bool,
    field: &str,
    line: Option<usize>,
    text: String,
) {
    if results.len() >= 120 {
        return;
    }
    results.push(BrowserSessionTabSearchResultPayload {
        id: id.to_owned(),
        title: title.to_owned(),
        page_title: page_title.to_owned(),
        label: session.tab_label.clone(),
        source: source.to_owned(),
        pinned: session.pinned,
        field: field.to_owned(),
        line,
        text,
        action_url: action_url.to_owned(),
        reload_url: browser_session_action_href(id, "reload", &[], session),
        duplicate_url: browser_session_action_href(
            current_id,
            "duplicate-session",
            &[("session", id.to_owned())],
            href_source,
        ),
        duplicate_background_url: browser_session_action_href(
            current_id,
            "duplicate-background-session",
            &[("session", id.to_owned())],
            href_source,
        ),
        pin_url: browser_session_action_href(
            current_id,
            "pin-tab",
            &[("session", id.to_owned())],
            href_source,
        ),
        unpin_url: browser_session_action_href(
            current_id,
            "unpin-tab",
            &[("session", id.to_owned())],
            href_source,
        ),
        close_url: can_close
            .then(|| {
                browser_session_action_href(
                    current_id,
                    "close-session",
                    &[("close_id", id.to_owned())],
                    href_source,
                )
            })
            .unwrap_or_default(),
        current,
    });
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
            background_restore_url: browser_session_action_href(
                &source.id,
                "restore-closed-background-session",
                &[("closed_id", closed.id.clone())],
                source,
            ),
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
            background_restore_url: browser_session_action_href(
                &source.id,
                "open-profile-closed-background-session",
                &[("closed", index.to_string())],
                source,
            ),
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
            background_session_url: browser_session_action_href(
                &source.id,
                "open-background-session",
                &[("url", bookmark.source.clone())],
                source,
            ),
            rename_url: browser_session_action_href(
                &source.id,
                "rename-bookmark",
                &[("bookmark", bookmark.id.clone())],
                source,
            ),
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
        .take(MAX_VISIBLE_BROWSER_PROFILE_HISTORY)
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
                background_session_url: browser_session_action_href(
                    &source.id,
                    "open-background-session",
                    &[("url", entry.source.clone())],
                    source,
                ),
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
        pinned: entry.pinned,
        label: normalize_browser_tab_label_option(entry.label.as_deref()),
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
        pinned: tab.pinned,
        label: tab.label.clone(),
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
        browser_session_sort_order(sessions, left)
            .cmp(&browser_session_sort_order(sessions, right))
            .then_with(|| browser_session_id_number(left).cmp(&browser_session_id_number(right)))
            .then_with(|| left.cmp(right))
    });
    ids
}

fn browser_session_sort_order(sessions: &HashMap<String, BrowserWebSession>, id: &str) -> u64 {
    sessions
        .get(id)
        .map(|session| session.tab_order)
        .unwrap_or_else(|| browser_session_id_number(id))
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

fn browser_session_matches_query(id: &str, session: &BrowserWebSession, needle: &str) -> bool {
    if id.to_lowercase().contains(needle) {
        return true;
    }
    if session
        .tab_label
        .as_ref()
        .is_some_and(|label| label.to_lowercase().contains(needle))
    {
        return true;
    }
    session.session.current().is_some_and(|render| {
        browser_session_title(render)
            .to_lowercase()
            .contains(needle)
            || render.source.to_lowercase().contains(needle)
    })
}

fn browser_session_target_viewport_x(
    target: &RequestTarget,
    action: &BrowserSessionAction,
) -> Option<usize> {
    parse_optional_usize_param(target, "viewport_x", 0, usize::MAX).or_else(|| {
        browser_session_action_allows_xy_viewport_alias(action)
            .then(|| parse_optional_usize_param(target, "x", 0, usize::MAX))
            .flatten()
    })
}

fn browser_session_target_viewport_y(
    target: &RequestTarget,
    action: &BrowserSessionAction,
) -> Option<usize> {
    parse_optional_usize_param(target, "viewport_y", 0, usize::MAX).or_else(|| {
        browser_session_action_allows_xy_viewport_alias(action)
            .then(|| parse_optional_usize_param(target, "y", 0, usize::MAX))
            .flatten()
    })
}

fn browser_session_target_has_viewport_position(target: &RequestTarget) -> bool {
    target.param("x").is_some()
        || target.param("y").is_some()
        || target.param("viewport_x").is_some()
        || target.param("viewport_y").is_some()
}

fn browser_session_action_allows_xy_viewport_alias(action: &BrowserSessionAction) -> bool {
    !matches!(action, BrowserSessionAction::ClickAt { .. })
}

fn scale_browser_raster_click_coordinate(
    coordinate: usize,
    raster_size: usize,
    viewport_size: usize,
    padding: usize,
    cell_size: usize,
) -> usize {
    let viewport_size = viewport_size.max(1);
    let raster_size = raster_size.max(1);
    if cell_size > 0 {
        let expected = viewport_size
            .saturating_mul(cell_size)
            .saturating_add(padding.saturating_mul(2));
        if expected == raster_size {
            return coordinate
                .saturating_sub(padding)
                .saturating_div(cell_size)
                .min(viewport_size.saturating_sub(1));
        }
    }
    let scaled = ((coordinate as u128) * (viewport_size as u128) / (raster_size as u128)) as usize;
    scaled.min(viewport_size.saturating_sub(1))
}

fn browser_session_click_feedback_point(
    raw_x: usize,
    raw_y: usize,
    click_x: usize,
    click_y: usize,
    page_x: usize,
    page_y: usize,
    raster_size: Option<(usize, usize)>,
) -> String {
    if let Some((raster_width, raster_height)) = raster_size {
        return format!(
            "raster x {raw_x}, y {raw_y} ({raster_width}x{raster_height}) mapped to DOM point x {click_x}, y {click_y} (page {page_x}, {page_y})"
        );
    }
    format!("DOM point x {click_x}, y {click_y} (page {page_x}, {page_y})")
}

fn browser_session_click_feedback_label(
    raw_x: usize,
    raw_y: usize,
    click_x: usize,
    click_y: usize,
    page_x: usize,
    page_y: usize,
    raster_size: Option<(usize, usize)>,
) -> String {
    format!(
        "Clicked {}",
        browser_session_click_feedback_point(
            raw_x,
            raw_y,
            click_x,
            click_y,
            page_x,
            page_y,
            raster_size,
        )
    )
}

async fn apply_browser_action(
    action: BrowserSessionAction,
    web_session: &mut BrowserWebSession,
) -> Result<(), BrowserRouteError> {
    if !matches!(
        &action,
        BrowserSessionAction::Current
            | BrowserSessionAction::ClickSelector(_)
            | BrowserSessionAction::ClickAt { .. }
    ) {
        web_session.action_feedback = None;
    }

    match action {
        BrowserSessionAction::Current => {
            normalize_browser_session_viewport(web_session);
        }
        BrowserSessionAction::Open(url) => {
            let target_url = web_session.session.resolve_current_target(&url);
            apply_browser_open_with_pending_shell(web_session, &target_url).await?;
        }
        BrowserSessionAction::Back => {
            web_session
                .session
                .back()
                .map_err(|error| BrowserRouteError::BadRequest(error.to_string()))?;
            reset_viewport_after_navigation(web_session);
            clear_browser_find_active_line(web_session);
            web_session.pending_source = None;
            web_session.display_source = None;
        }
        BrowserSessionAction::Forward => {
            web_session
                .session
                .forward()
                .map_err(|error| BrowserRouteError::BadRequest(error.to_string()))?;
            reset_viewport_after_navigation(web_session);
            clear_browser_find_active_line(web_session);
            web_session.pending_source = None;
            web_session.display_source = None;
        }
        BrowserSessionAction::Reload => {
            if let Some(pending_source) = web_session.pending_source.clone() {
                apply_browser_open_with_pending_shell(web_session, &pending_source).await?;
            } else {
                web_session.session.reload().await.map_err(|error| {
                    BrowserRouteError::Upstream(format!("browser reload failed: {error:#}"))
                })?;
                reset_viewport_after_navigation(web_session);
                clear_browser_find_active_line(web_session);
            }
        }
        BrowserSessionAction::Link(index) => {
            let before = current_session_source(web_session);
            let target_url = web_session.session.link_target(index).ok();
            if let Err(error) = web_session.session.activate_link(index).await {
                if let Some(target_url) = target_url.as_deref() {
                    set_browser_link_pending_navigation_feedback(
                        web_session,
                        format!("Opened link {}", index + 1),
                        target_url,
                        &error.to_string(),
                    );
                    return Ok(());
                }
                return Err(BrowserRouteError::BadRequest(error.to_string()));
            }
            if current_session_source(web_session) != before {
                reset_viewport_after_navigation(web_session);
                clear_browser_find_active_line(web_session);
                web_session.pending_source = None;
                web_session.display_source = None;
            }
            set_browser_navigation_feedback(
                web_session,
                format!("Opened link {}", index + 1),
                before,
            );
        }
        BrowserSessionAction::Anchor(index) => {
            apply_browser_anchor(web_session, index)?;
        }
        BrowserSessionAction::Resource(index) => {
            apply_browser_resource(web_session, index).await?;
            reset_viewport_after_navigation(web_session);
            clear_browser_find_active_line(web_session);
            web_session.pending_source = None;
            web_session.display_source = None;
        }
        BrowserSessionAction::LinkText(text) => {
            let before = current_session_source(web_session);
            let target_url = web_session.session.link_text_target(&text).ok();
            if let Err(error) = web_session.session.activate_link_text(&text).await {
                if let Some(target_url) = target_url.as_deref() {
                    set_browser_link_pending_navigation_feedback(
                        web_session,
                        format!(
                            "Opened link text {}",
                            browser_session_feedback_excerpt(&text)
                        ),
                        target_url,
                        &error.to_string(),
                    );
                    return Ok(());
                }
                return Err(BrowserRouteError::BadRequest(error.to_string()));
            }
            if current_session_source(web_session) != before {
                reset_viewport_after_navigation(web_session);
                clear_browser_find_active_line(web_session);
                web_session.pending_source = None;
                web_session.display_source = None;
            }
            set_browser_navigation_feedback(
                web_session,
                format!(
                    "Opened link text {}",
                    browser_session_feedback_excerpt(&text)
                ),
                before,
            );
        }
        BrowserSessionAction::LinkSelector(selector) => {
            let before = current_session_source(web_session);
            let target_url = web_session.session.link_selector_target(&selector).ok();
            if let Err(error) = web_session.session.activate_link_selector(&selector).await {
                if let Some(target_url) = target_url.as_deref() {
                    set_browser_link_pending_navigation_feedback(
                        web_session,
                        format!(
                            "Opened link selector {}",
                            browser_session_feedback_excerpt(&selector)
                        ),
                        target_url,
                        &error.to_string(),
                    );
                    return Ok(());
                }
                return Err(BrowserRouteError::BadRequest(error.to_string()));
            }
            if current_session_source(web_session) != before {
                reset_viewport_after_navigation(web_session);
                clear_browser_find_active_line(web_session);
                web_session.pending_source = None;
                web_session.display_source = None;
            }
            set_browser_navigation_feedback(
                web_session,
                format!(
                    "Opened link selector {}",
                    browser_session_feedback_excerpt(&selector)
                ),
                before,
            );
        }
        BrowserSessionAction::History(index) => {
            apply_browser_history_entry(web_session, index)?;
            reset_viewport_after_navigation(web_session);
            clear_browser_find_active_line(web_session);
            web_session.pending_source = None;
            web_session.display_source = None;
        }
        BrowserSessionAction::Find(query) => {
            web_session.find_query = query.trim().to_owned();
            if !web_session.find_query.is_empty() {
                apply_browser_find(web_session, BrowserFindDirection::First)?;
            } else {
                clear_browser_find_active_line(web_session);
            }
        }
        BrowserSessionAction::FindMatch(match_index) => {
            apply_browser_find_match(web_session, match_index)?;
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
        BrowserSessionAction::SearchTabs(query) => {
            web_session.tab_search_query = normalize_browser_search_query(&query);
        }
        BrowserSessionAction::ClearTabSearch => {
            web_session.tab_search_query.clear();
        }
        BrowserSessionAction::ClickSelector(selector) => {
            let before = current_session_interaction_snapshot(web_session);
            if let Err(error) = web_session
                .session
                .click_selector_with_default_action(&selector)
                .await
            {
                set_browser_click_error_feedback(
                    web_session,
                    format!(
                        "Clicked selector {}",
                        browser_session_feedback_excerpt(&selector)
                    ),
                    format!(
                        "No click target for selector {}",
                        browser_session_feedback_excerpt(&selector)
                    ),
                    &error.to_string(),
                    "navigation failed",
                );
                return Ok(());
            }
            if browser_interaction_snapshot_navigated(&before, web_session) {
                reset_viewport_after_navigation(web_session);
                clear_browser_find_active_line(web_session);
                web_session.pending_source = None;
                web_session.display_source = None;
            }
            set_browser_click_feedback(
                web_session,
                format!(
                    "Clicked selector {}",
                    browser_session_feedback_excerpt(&selector)
                ),
                before,
            );
        }
        BrowserSessionAction::ClickAt {
            x,
            y,
            raster_width,
            raster_height,
        } => {
            let before = current_session_interaction_snapshot(web_session);
            let raster_click = raster_width.zip(raster_height);
            let raster_options = BrowserRasterOptions::default();
            let (click_x, click_y) = if let Some((raster_width, raster_height)) = raster_click {
                (
                    scale_browser_raster_click_coordinate(
                        x,
                        raster_width,
                        web_session.width,
                        raster_options.padding_x,
                        raster_options.cell_width,
                    ),
                    scale_browser_raster_click_coordinate(
                        y,
                        raster_height,
                        web_session.height,
                        raster_options.padding_y,
                        raster_options.cell_height,
                    ),
                )
            } else {
                (x, y)
            };
            let page_x = web_session.viewport_x.saturating_add(click_x);
            let page_y = web_session.viewport_y.saturating_add(click_y);
            let viewport = BrowserViewportState {
                x: web_session.viewport_x,
                y: web_session.viewport_y,
                width: web_session.width,
                height: web_session.height,
            };
            let click_navigation_target = if raster_click.is_some() {
                web_session
                    .session
                    .link_target_at_viewport(viewport, click_x, click_y)
            } else {
                web_session.session.link_target_at(page_x, page_y)
            };
            let click_result = if raster_click.is_some() {
                web_session
                    .session
                    .click_viewport_at_with_default_action(viewport, click_x, click_y)
                    .await
            } else {
                web_session
                    .session
                    .click_at_with_default_action(page_x, page_y)
                    .await
            };
            if let Err(error) = click_result {
                if let Some(target_url) = click_navigation_target.as_deref()
                    && !browser_click_error_is_target_miss(&error.to_string())
                {
                    set_browser_click_pending_navigation_feedback(
                        web_session,
                        browser_session_click_feedback_label(
                            x,
                            y,
                            click_x,
                            click_y,
                            page_x,
                            page_y,
                            raster_click,
                        ),
                        target_url,
                        &error.to_string(),
                    );
                    return Ok(());
                }
                set_browser_click_error_feedback(
                    web_session,
                    browser_session_click_feedback_label(
                        x,
                        y,
                        click_x,
                        click_y,
                        page_x,
                        page_y,
                        raster_click,
                    ),
                    format!(
                        "No click target at {}",
                        browser_session_click_feedback_point(
                            x,
                            y,
                            click_x,
                            click_y,
                            page_x,
                            page_y,
                            raster_click,
                        )
                    ),
                    &error.to_string(),
                    "navigation failed",
                );
                return Ok(());
            }
            if browser_interaction_snapshot_navigated(&before, web_session) {
                reset_viewport_after_navigation(web_session);
                clear_browser_find_active_line(web_session);
                web_session.pending_source = None;
                web_session.display_source = None;
            }
            set_browser_click_feedback(
                web_session,
                browser_session_click_feedback_label(
                    x,
                    y,
                    click_x,
                    click_y,
                    page_x,
                    page_y,
                    raster_click,
                ),
                before,
            );
        }
        BrowserSessionAction::FocusSelector(selector) => {
            web_session
                .session
                .focus_selector(&selector)
                .map_err(|error| BrowserRouteError::BadRequest(error.to_string()))?;
            web_session.action_feedback = Some(format!(
                "Focused selector {}.",
                browser_session_feedback_excerpt(&selector)
            ));
        }
        BrowserSessionAction::FocusControl {
            form_index,
            control_index,
        } => {
            focus_browser_form_control(web_session, form_index, control_index)?;
            web_session.action_feedback = Some(format!(
                "Focused form {form_index} control {control_index}."
            ));
        }
        BrowserSessionAction::ActivateControl {
            form_index,
            control_index,
        } => {
            let before = current_session_source(web_session);
            focus_browser_form_control(web_session, form_index, control_index)?;
            let form_target = browser_session_form_target(web_session, form_index);
            if let Err(error) = web_session.session.submit_focused_form().await {
                if let Some(target_url) = form_target.as_deref() {
                    set_browser_form_pending_navigation_feedback(
                        web_session,
                        format!("Activated form {form_index} control {control_index}"),
                        target_url,
                        &error.to_string(),
                    );
                    return Ok(());
                }
                return Err(BrowserRouteError::BadRequest(error.to_string()));
            }
            let navigated = current_session_source(web_session) != before;
            if navigated {
                reset_viewport_after_navigation(web_session);
                clear_browser_find_active_line(web_session);
                web_session.pending_source = None;
                web_session.display_source = None;
            }
            set_browser_form_navigation_feedback(
                web_session,
                format!("Activated form {form_index} control {control_index}"),
                navigated,
            );
        }
        BrowserSessionAction::FocusNext => {
            web_session
                .session
                .focus_next_control()
                .map_err(|error| BrowserRouteError::BadRequest(error.to_string()))?;
            set_browser_focused_control_feedback(web_session, "Focused next control");
        }
        BrowserSessionAction::FocusPrevious => {
            web_session
                .session
                .focus_previous_control()
                .map_err(|error| BrowserRouteError::BadRequest(error.to_string()))?;
            set_browser_focused_control_feedback(web_session, "Focused previous control");
        }
        BrowserSessionAction::TypeText(text) => {
            web_session
                .session
                .type_text(&text)
                .map_err(|error| BrowserRouteError::BadRequest(error.to_string()))?;
            web_session.action_feedback = Some(format!(
                "Typed {} into focused control.",
                browser_session_feedback_excerpt(&text)
            ));
        }
        BrowserSessionAction::Backspace(count) => {
            web_session
                .session
                .delete_text_backward(count)
                .map_err(|error| BrowserRouteError::BadRequest(error.to_string()))?;
            web_session.action_feedback = Some(format!(
                "Deleted {count} character(s) from focused control."
            ));
        }
        BrowserSessionAction::ClearInput => {
            web_session
                .session
                .clear_focused_text()
                .map_err(|error| BrowserRouteError::BadRequest(error.to_string()))?;
            web_session.action_feedback = Some("Cleared focused input.".to_owned());
        }
        BrowserSessionAction::Enter => {
            let before = current_session_source(web_session);
            let form_target = web_session
                .session
                .focused_control()
                .and_then(|focused| browser_session_form_target(web_session, focused.form_index));
            if let Err(error) = web_session.session.submit_focused_form().await {
                if let Some(target_url) = form_target.as_deref() {
                    set_browser_form_pending_navigation_feedback(
                        web_session,
                        "Submitted focused form".to_owned(),
                        target_url,
                        &error.to_string(),
                    );
                    return Ok(());
                }
                return Err(BrowserRouteError::BadRequest(error.to_string()));
            }
            let navigated = current_session_source(web_session) != before;
            if navigated {
                reset_viewport_after_navigation(web_session);
                clear_browser_find_active_line(web_session);
                web_session.pending_source = None;
                web_session.display_source = None;
            }
            set_browser_form_navigation_feedback(
                web_session,
                "Submitted focused form".to_owned(),
                navigated,
            );
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
            web_session.action_feedback = Some(format!(
                "Toggled focused form {} control {}.",
                focused.form_index, focused.control_index
            ));
        }
        BrowserSessionAction::Choose(value) => {
            web_session
                .session
                .select_focused_option(&value)
                .map_err(|error| BrowserRouteError::BadRequest(error.to_string()))?;
            web_session.action_feedback = Some(format!(
                "Chose {} in focused select.",
                browser_session_feedback_excerpt(&value)
            ));
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
        BrowserSessionAction::MakeVisual => {
            let stylesheet_report = web_session
                .session
                .render_current_with_stylesheets(web_session.max_bytes)
                .await
                .map_err(|error| {
                    BrowserRouteError::Upstream(format!(
                        "browser visual stylesheet render failed: {error:#}"
                    ))
                })?;
            let image_report = web_session
                .session
                .render_current_with_images(web_session.max_bytes)
                .await
                .map_err(|error| {
                    BrowserRouteError::Upstream(format!(
                        "browser visual image render failed: {error:#}"
                    ))
                })?;
            web_session.resource_report = Some(browser_session_resource_report_from_make_visual(
                browser_session_resource_report_from_stylesheets(stylesheet_report),
                browser_session_resource_report_from_images(image_report),
            ));
            normalize_browser_session_viewport(web_session);
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
            normalize_browser_session_viewport(web_session);
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
            normalize_browser_session_viewport(web_session);
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
            normalize_browser_session_viewport(web_session);
        }
        BrowserSessionAction::ClearResourceReport => {
            web_session.resource_report = None;
        }
        BrowserSessionAction::AddBookmark
        | BrowserSessionAction::BookmarkAllTabs
        | BrowserSessionAction::BookmarkProfileHistory
        | BrowserSessionAction::RemoveProfileHistoryBookmarks
        | BrowserSessionAction::BookmarkTabSearchResults
        | BrowserSessionAction::RemoveTabSearchBookmarks
        | BrowserSessionAction::OpenBookmark(_)
        | BrowserSessionAction::RenameBookmark { .. }
        | BrowserSessionAction::RemoveBookmark(_)
        | BrowserSessionAction::ClearBookmarks
        | BrowserSessionAction::OpenBookmarksNewSessions
        | BrowserSessionAction::OpenBookmarksBackgroundSessions
        | BrowserSessionAction::OpenProfileHistoryNewSessions { .. }
        | BrowserSessionAction::OpenProfileHistoryBackgroundSessions { .. }
        | BrowserSessionAction::OpenProfileClosed(_)
        | BrowserSessionAction::OpenProfileClosedBackgroundSession(_)
        | BrowserSessionAction::RemoveProfileHistory(_)
        | BrowserSessionAction::ClearClosedSessions
        | BrowserSessionAction::ClearProfileTabs
        | BrowserSessionAction::ClearProfileHistory
        | BrowserSessionAction::RestoreClosedSession(_)
        | BrowserSessionAction::RestoreClosedBackgroundSession(_)
        | BrowserSessionAction::RestoreClosedBackgroundSessions
        | BrowserSessionAction::ForgetClosedSession(_)
        | BrowserSessionAction::ForgetProfileClosed(_) => {
            return Err(BrowserRouteError::BadRequest(
                "browser registry actions must be handled by the registry".to_owned(),
            ));
        }
        BrowserSessionAction::OpenNewSession(_)
        | BrowserSessionAction::OpenBackgroundSession(_)
        | BrowserSessionAction::LinkTextNewSession(_)
        | BrowserSessionAction::LinkSelectorNewSession(_)
        | BrowserSessionAction::LinkTextBackgroundSession(_)
        | BrowserSessionAction::LinkSelectorBackgroundSession(_)
        | BrowserSessionAction::AnchorNewSession(_)
        | BrowserSessionAction::AnchorBackgroundSession(_)
        | BrowserSessionAction::FindMatchNewSession(_)
        | BrowserSessionAction::FindMatchBackgroundSession(_)
        | BrowserSessionAction::OpenFindMatchesNewSessions { .. }
        | BrowserSessionAction::OpenFindMatchesBackgroundSessions { .. }
        | BrowserSessionAction::ResourceNewSession(_)
        | BrowserSessionAction::LinkBackgroundSession(_)
        | BrowserSessionAction::OpenLinksNewSessions { .. }
        | BrowserSessionAction::OpenLinksBackgroundSessions { .. }
        | BrowserSessionAction::BookmarkPageLinks
        | BrowserSessionAction::RemovePageLinkBookmarks
        | BrowserSessionAction::ResourceBackgroundSession(_)
        | BrowserSessionAction::OpenResourcesNewSessions { .. }
        | BrowserSessionAction::OpenResourcesBackgroundSessions { .. }
        | BrowserSessionAction::SubmitNewSession { .. }
        | BrowserSessionAction::ActivateControlNewSession { .. }
        | BrowserSessionAction::SubmitBackgroundSession { .. }
        | BrowserSessionAction::ActivateControlBackgroundSession { .. }
        | BrowserSessionAction::DuplicateSession(_)
        | BrowserSessionAction::DuplicateBackgroundSession(_)
        | BrowserSessionAction::DuplicateTabSearchResults
        | BrowserSessionAction::CloseSession(_)
        | BrowserSessionAction::CloseOtherSessions
        | BrowserSessionAction::CloseUnpinnedSessions
        | BrowserSessionAction::CloseSessionsToRight
        | BrowserSessionAction::CloseSessionsToLeft
        | BrowserSessionAction::CloseDuplicateSessions
        | BrowserSessionAction::ReloadTabSearchResults
        | BrowserSessionAction::CloseTabSearchResults
        | BrowserSessionAction::CloseTabSearchNonMatches
        | BrowserSessionAction::PinTabSearchResults
        | BrowserSessionAction::UnpinTabSearchResults
        | BrowserSessionAction::LabelTabSearchResults(_)
        | BrowserSessionAction::ClearTabSearchLabels
        | BrowserSessionAction::PinSession(_)
        | BrowserSessionAction::UnpinSession(_)
        | BrowserSessionAction::PinAllSessions
        | BrowserSessionAction::UnpinAllSessions
        | BrowserSessionAction::MoveSessionLeft(_)
        | BrowserSessionAction::MoveSessionRight(_)
        | BrowserSessionAction::MoveTabSearchResultsToFront
        | BrowserSessionAction::MoveTabSearchResultsToBack
        | BrowserSessionAction::LabelSession { .. }
        | BrowserSessionAction::ClearSessionLabel(_)
        | BrowserSessionAction::SwitchNextSession
        | BrowserSessionAction::SwitchPreviousSession
        | BrowserSessionAction::JumpSession(_) => {
            return Err(BrowserRouteError::BadRequest(
                "browser session registry actions must be handled by the registry".to_owned(),
            ));
        }
        BrowserSessionAction::Scroll { dx, dy } => {
            let before_x = web_session.viewport_x;
            let before_y = web_session.viewport_y;
            web_session.viewport_x = apply_scroll_delta(web_session.viewport_x, dx);
            web_session.viewport_y = apply_scroll_delta(web_session.viewport_y, dy);
            normalize_browser_session_viewport(web_session);
            set_browser_scroll_noop_feedback(web_session, before_x, before_y, dx, dy);
        }
        BrowserSessionAction::Top => {
            let before_y = web_session.viewport_y;
            web_session.viewport_y = 0;
            normalize_browser_session_viewport(web_session);
            if web_session.viewport_y == before_y {
                web_session.action_feedback = Some("Already at top.".to_owned());
            } else {
                set_browser_visual_scroll_moved_feedback(web_session);
            }
        }
        BrowserSessionAction::Bottom => {
            let before_y = web_session.viewport_y;
            web_session.viewport_y = usize::MAX;
            normalize_browser_session_viewport(web_session);
            if web_session.viewport_y == before_y {
                web_session.action_feedback = Some("Already at bottom.".to_owned());
            } else {
                set_browser_visual_scroll_moved_feedback(web_session);
            }
        }
        BrowserSessionAction::PageUp => {
            let before_y = web_session.viewport_y;
            web_session.viewport_y = apply_scroll_delta(
                web_session.viewport_y,
                -(web_session.height.max(1) as isize),
            );
            normalize_browser_session_viewport(web_session);
            if web_session.viewport_y == before_y {
                web_session.action_feedback = Some("Already at top.".to_owned());
            } else {
                set_browser_visual_scroll_moved_feedback(web_session);
            }
        }
        BrowserSessionAction::PageDown => {
            let before_y = web_session.viewport_y;
            web_session.viewport_y =
                apply_scroll_delta(web_session.viewport_y, web_session.height.max(1) as isize);
            normalize_browser_session_viewport(web_session);
            if web_session.viewport_y == before_y {
                web_session.action_feedback = Some("Already at bottom.".to_owned());
            } else {
                set_browser_visual_scroll_moved_feedback(web_session);
            }
        }
        BrowserSessionAction::LineUp => {
            let before_y = web_session.viewport_y;
            web_session.viewport_y = apply_scroll_delta(web_session.viewport_y, -1);
            normalize_browser_session_viewport(web_session);
            if web_session.viewport_y == before_y {
                web_session.action_feedback = Some("Already at top.".to_owned());
            } else {
                set_browser_visual_scroll_moved_feedback(web_session);
            }
        }
        BrowserSessionAction::LineDown => {
            let before_y = web_session.viewport_y;
            web_session.viewport_y = apply_scroll_delta(web_session.viewport_y, 1);
            normalize_browser_session_viewport(web_session);
            if web_session.viewport_y == before_y {
                web_session.action_feedback = Some("Already at bottom.".to_owned());
            } else {
                set_browser_visual_scroll_moved_feedback(web_session);
            }
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
            web_session.action_feedback = Some(format!(
                "Set form {form_index} field {}.",
                browser_session_feedback_excerpt(&name)
            ));
        }
        BrowserSessionAction::FillControl {
            form_index,
            control_index,
            value,
        } => {
            fill_browser_form_control(web_session, form_index, control_index, &value)?;
            web_session.action_feedback =
                Some(format!("Set form {form_index} control {control_index}."));
        }
        BrowserSessionAction::TypeControl {
            form_index,
            control_index,
            value,
        } => {
            type_browser_form_control(web_session, form_index, control_index, &value)?;
            web_session.action_feedback =
                Some(format!("Typed form {form_index} control {control_index}."));
        }
        BrowserSessionAction::ClearControl {
            form_index,
            control_index,
        } => {
            type_browser_form_control(web_session, form_index, control_index, "")?;
            web_session.action_feedback = Some(format!(
                "Cleared form {form_index} control {control_index}."
            ));
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
            web_session.action_feedback = Some(format!(
                "Selected {} for form {form_index} control {control_index}.",
                browser_session_feedback_excerpt(&value)
            ));
        }
        BrowserSessionAction::Toggle {
            form_index,
            control_index,
        } => {
            web_session
                .session
                .toggle_form_control(form_index, control_index)
                .map_err(|error| BrowserRouteError::BadRequest(error.to_string()))?;
            web_session.action_feedback = Some(format!(
                "Toggled form {form_index} control {control_index}."
            ));
        }
        BrowserSessionAction::Submit { form_index } => {
            let before = current_session_source(web_session);
            let form_target = browser_session_form_target(web_session, form_index);
            if let Err(error) = web_session.session.submit_form(form_index, &[]).await {
                if let Some(target_url) = form_target.as_deref() {
                    set_browser_form_pending_navigation_feedback(
                        web_session,
                        format!("Submitted form {form_index}"),
                        target_url,
                        &error.to_string(),
                    );
                    return Ok(());
                }
                return Err(BrowserRouteError::BadRequest(error.to_string()));
            }
            let navigated = current_session_source(web_session) != before;
            if navigated {
                reset_viewport_after_navigation(web_session);
                clear_browser_find_active_line(web_session);
            }
            set_browser_form_navigation_feedback(
                web_session,
                format!("Submitted form {form_index}"),
                navigated,
            );
        }
    }
    Ok(())
}

fn focus_browser_form_control(
    web_session: &mut BrowserWebSession,
    form_index: usize,
    control_index: usize,
) -> Result<(), BrowserRouteError> {
    let control_count = {
        let forms = web_session.session.current_forms();
        let form = forms
            .iter()
            .find(|form| form.index == form_index)
            .ok_or_else(|| {
                BrowserRouteError::BadRequest(format!("form {form_index} is not available"))
            })?;
        if control_index >= form.controls.len() {
            return Err(BrowserRouteError::BadRequest(format!(
                "form {form_index} control {control_index} is not available"
            )));
        }
        forms.iter().map(|form| form.controls.len()).sum::<usize>()
    };

    for _ in 0..=control_count {
        if web_session
            .session
            .focused_control()
            .is_some_and(|focused| {
                focused.form_index == form_index && focused.control_index == control_index
            })
        {
            return Ok(());
        }
        web_session
            .session
            .focus_next_control()
            .map_err(|error| BrowserRouteError::BadRequest(error.to_string()))?;
    }

    Err(BrowserRouteError::BadRequest(format!(
        "form {form_index} control {control_index} cannot be focused"
    )))
}

fn fill_browser_form_control(
    web_session: &mut BrowserWebSession,
    form_index: usize,
    control_index: usize,
    value: &str,
) -> Result<(), BrowserRouteError> {
    let name = {
        let forms = web_session.session.current_forms();
        let form = forms
            .iter()
            .find(|form| form.index == form_index)
            .ok_or_else(|| {
                BrowserRouteError::BadRequest(format!("form {form_index} is not available"))
            })?;
        let control = form.controls.get(control_index).ok_or_else(|| {
            BrowserRouteError::BadRequest(format!(
                "form {form_index} control {control_index} is not available"
            ))
        })?;
        if control.disabled
            || control.name.is_empty()
            || !browser_form_control_name_is_unique(form, &control.name)
            || !form_control_is_text_editable(&control.kind)
        {
            return Err(BrowserRouteError::BadRequest(format!(
                "form {form_index} control {control_index} is not an editable text control"
            )));
        }
        control.name.clone()
    };

    web_session
        .session
        .set_form_field(form_index, &name, value)
        .map_err(|error| BrowserRouteError::BadRequest(error.to_string()))?;
    Ok(())
}

fn type_browser_form_control(
    web_session: &mut BrowserWebSession,
    form_index: usize,
    control_index: usize,
    value: &str,
) -> Result<(), BrowserRouteError> {
    {
        let forms = web_session.session.current_forms();
        let form = forms
            .iter()
            .find(|form| form.index == form_index)
            .ok_or_else(|| {
                BrowserRouteError::BadRequest(format!("form {form_index} is not available"))
            })?;
        let control = form.controls.get(control_index).ok_or_else(|| {
            BrowserRouteError::BadRequest(format!(
                "form {form_index} control {control_index} is not available"
            ))
        })?;
        if control.disabled
            || control.name.is_empty()
            || !browser_form_control_name_is_unique(form, &control.name)
            || !form_control_is_text_editable(&control.kind)
        {
            return Err(BrowserRouteError::BadRequest(format!(
                "form {form_index} control {control_index} is not a typeable text control"
            )));
        }
    }

    focus_browser_form_control(web_session, form_index, control_index)?;
    web_session
        .session
        .clear_focused_text()
        .map_err(|error| BrowserRouteError::BadRequest(error.to_string()))?;
    if !value.is_empty() {
        web_session
            .session
            .type_text(value)
            .map_err(|error| BrowserRouteError::BadRequest(error.to_string()))?;
    }
    Ok(())
}

async fn apply_browser_resource(
    web_session: &mut BrowserWebSession,
    resource_index: usize,
) -> Result<(), BrowserRouteError> {
    let target = {
        let render = web_session.session.current().ok_or_else(|| {
            BrowserRouteError::BadRequest(
                "cannot open resource: session has no current page".to_owned(),
            )
        })?;
        let Some(resource) = render.resources.get(resource_index) else {
            return Err(BrowserRouteError::BadRequest(format!(
                "resource index {} not found; current page has {} resource(s)",
                resource_index,
                render.resources.len()
            )));
        };
        resource.resolved.clone()
    };
    web_session
        .session
        .navigate(&target)
        .await
        .map_err(|error| {
            BrowserRouteError::Upstream(format!("browser resource navigation failed: {error:#}"))
        })?;
    Ok(())
}

fn apply_browser_anchor(
    web_session: &mut BrowserWebSession,
    anchor_index: usize,
) -> Result<(), BrowserRouteError> {
    let target_y = {
        let render = web_session.session.current().ok_or_else(|| {
            BrowserRouteError::BadRequest(
                "cannot jump to anchor: session has no current page".to_owned(),
            )
        })?;
        let Some(target) = render.fragment_targets.get(anchor_index) else {
            return Err(BrowserRouteError::BadRequest(format!(
                "anchor index {} not found; current page has {} anchor target(s)",
                anchor_index + 1,
                render.fragment_targets.len()
            )));
        };
        target.y
    };
    web_session.viewport_x = 0;
    web_session.viewport_y = target_y;
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
        "open-background-session"
        | "open_background_session"
        | "open-background-tab"
        | "open_background_tab"
        | "background-session"
        | "background_session"
        | "background-tab"
        | "background_tab" => {
            browser_action_url(target).map(BrowserSessionAction::OpenBackgroundSession)
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
        "link-background-session"
        | "link_background_session"
        | "link-background-tab"
        | "link_background_tab"
        | "open-link-background-session"
        | "open_link_background_session"
        | "open-link-background-tab"
        | "open_link_background_tab" => {
            let index = target
                .param("link")
                .or_else(|| target.param("index"))
                .ok_or_else(|| BrowserRouteError::BadRequest("missing link index".to_owned()))?
                .parse::<usize>()
                .map_err(|_| BrowserRouteError::BadRequest("invalid link index".to_owned()))?;
            Ok(BrowserSessionAction::LinkBackgroundSession(index))
        }
        "open-links-new-sessions"
        | "open_links_new_sessions"
        | "open-links-new-tabs"
        | "open_links_new_tabs"
        | "open-all-links-new-sessions"
        | "open_all_links_new_sessions"
        | "open-all-links-new-tabs"
        | "open_all_links_new_tabs"
        | "open-links-tabs"
        | "open_links_tabs" => Ok(BrowserSessionAction::OpenLinksNewSessions {
            limit: parse_optional_usize_param(target, "limit", 1, MAX_BULK_BACKGROUND_LINKS)
                .or_else(|| {
                    parse_optional_usize_param(target, "count", 1, MAX_BULK_BACKGROUND_LINKS)
                })
                .unwrap_or(DEFAULT_BULK_BACKGROUND_LINKS),
        }),
        "open-links-background"
        | "open_links_background"
        | "open-links-background-sessions"
        | "open_links_background_sessions"
        | "open-all-links-background"
        | "open_all_links_background"
        | "open-all-links-background-sessions"
        | "open_all_links_background_sessions" => {
            Ok(BrowserSessionAction::OpenLinksBackgroundSessions {
                limit: parse_optional_usize_param(target, "limit", 1, MAX_BULK_BACKGROUND_LINKS)
                    .or_else(|| {
                        parse_optional_usize_param(target, "count", 1, MAX_BULK_BACKGROUND_LINKS)
                    })
                    .unwrap_or(DEFAULT_BULK_BACKGROUND_LINKS),
            })
        }
        "bookmark-page-links"
        | "bookmark_page_links"
        | "bookmark-current-page-links"
        | "bookmark_current_page_links"
        | "bookmark-links"
        | "bookmark_links"
        | "bookmark-visible-links"
        | "bookmark_visible_links"
        | "save-page-links"
        | "save_page_links"
        | "save-links"
        | "save_links" => Ok(BrowserSessionAction::BookmarkPageLinks),
        "remove-page-link-bookmarks"
        | "remove_page_link_bookmarks"
        | "remove-page-links-bookmarks"
        | "remove_page_links_bookmarks"
        | "remove-link-bookmarks"
        | "remove_link_bookmarks"
        | "unbookmark-page-links"
        | "unbookmark_page_links"
        | "unbookmark-links"
        | "unbookmark_links" => Ok(BrowserSessionAction::RemovePageLinkBookmarks),
        "anchor" | "fragment" | "jump-anchor" | "jump_anchor" | "jump-fragment"
        | "jump_fragment" => browser_anchor_index(target).map(BrowserSessionAction::Anchor),
        "anchor-new-session"
        | "anchor_new_session"
        | "anchor-new-tab"
        | "anchor_new_tab"
        | "fragment-new-session"
        | "fragment_new_session"
        | "fragment-new-tab"
        | "fragment_new_tab"
        | "open-anchor-new-session"
        | "open_anchor_new_session"
        | "open-anchor-new-tab"
        | "open_anchor_new_tab" => {
            browser_anchor_index(target).map(BrowserSessionAction::AnchorNewSession)
        }
        "anchor-background-session"
        | "anchor_background_session"
        | "anchor-background-tab"
        | "anchor_background_tab"
        | "fragment-background-session"
        | "fragment_background_session"
        | "fragment-background-tab"
        | "fragment_background_tab"
        | "open-anchor-background-session"
        | "open_anchor_background_session"
        | "open-anchor-background-tab"
        | "open_anchor_background_tab" => {
            browser_anchor_index(target).map(BrowserSessionAction::AnchorBackgroundSession)
        }
        "resource" | "open-resource" | "open_resource" => Ok(BrowserSessionAction::Resource(
            browser_resource_index(target)?,
        )),
        "resource-new-session"
        | "resource_new_session"
        | "resource-new-tab"
        | "resource_new_tab"
        | "open-resource-new-session"
        | "open_resource_new_session"
        | "open-resource-new-tab"
        | "open_resource_new_tab" => Ok(BrowserSessionAction::ResourceNewSession(
            browser_resource_index(target)?,
        )),
        "resource-background-session"
        | "resource_background_session"
        | "resource-background-tab"
        | "resource_background_tab"
        | "open-resource-background-session"
        | "open_resource_background_session"
        | "open-resource-background-tab"
        | "open_resource_background_tab" => Ok(BrowserSessionAction::ResourceBackgroundSession(
            browser_resource_index(target)?,
        )),
        "open-resources-new-sessions"
        | "open_resources_new_sessions"
        | "open-resources-new-tabs"
        | "open_resources_new_tabs"
        | "open-all-resources-new-sessions"
        | "open_all_resources_new_sessions"
        | "open-all-resources-new-tabs"
        | "open_all_resources_new_tabs"
        | "open-resources-tabs"
        | "open_resources_tabs" => Ok(BrowserSessionAction::OpenResourcesNewSessions {
            limit: parse_optional_usize_param(target, "limit", 1, MAX_BULK_BACKGROUND_LINKS)
                .or_else(|| {
                    parse_optional_usize_param(target, "count", 1, MAX_BULK_BACKGROUND_LINKS)
                })
                .unwrap_or(DEFAULT_BULK_BACKGROUND_LINKS),
        }),
        "open-resources-background"
        | "open_resources_background"
        | "open-resources-background-sessions"
        | "open_resources_background_sessions"
        | "open-all-resources-background"
        | "open_all_resources_background"
        | "open-all-resources-background-sessions"
        | "open_all_resources_background_sessions" => {
            Ok(BrowserSessionAction::OpenResourcesBackgroundSessions {
                limit: parse_optional_usize_param(target, "limit", 1, MAX_BULK_BACKGROUND_LINKS)
                    .or_else(|| {
                        parse_optional_usize_param(target, "count", 1, MAX_BULK_BACKGROUND_LINKS)
                    })
                    .unwrap_or(DEFAULT_BULK_BACKGROUND_LINKS),
            })
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
        "link-text-background-session"
        | "link_text_background_session"
        | "link-text-background-tab"
        | "link_text_background_tab"
        | "open-link-text-background-session"
        | "open_link_text_background_session"
        | "open-link-text-background-tab"
        | "open_link_text_background_tab" => {
            browser_action_link_text(target).map(BrowserSessionAction::LinkTextBackgroundSession)
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
        "link-selector-background-session"
        | "link_selector_background_session"
        | "link-selector-background-tab"
        | "link_selector_background_tab"
        | "open-link-selector-background-session"
        | "open_link_selector_background_session"
        | "open-link-selector-background-tab"
        | "open_link_selector_background_tab" => browser_action_link_selector(target)
            .map(BrowserSessionAction::LinkSelectorBackgroundSession),
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
        "find-match" | "find_match" | "jump-find" | "jump_find" => {
            browser_find_match_index(target).map(BrowserSessionAction::FindMatch)
        }
        "find-match-new-session"
        | "find_match_new_session"
        | "find-match-new-tab"
        | "find_match_new_tab"
        | "open-find-match-new-session"
        | "open_find_match_new_session"
        | "open-find-match-new-tab"
        | "open_find_match_new_tab" => {
            browser_find_match_index(target).map(BrowserSessionAction::FindMatchNewSession)
        }
        "find-match-background-session"
        | "find_match_background_session"
        | "find-match-background-tab"
        | "find_match_background_tab"
        | "open-find-match-background-session"
        | "open_find_match_background_session"
        | "open-find-match-background-tab"
        | "open_find_match_background_tab" => {
            browser_find_match_index(target).map(BrowserSessionAction::FindMatchBackgroundSession)
        }
        "open-find-matches-new-sessions"
        | "open_find_matches_new_sessions"
        | "open-find-matches-new-tabs"
        | "open_find_matches_new_tabs"
        | "open-all-find-matches-new-sessions"
        | "open_all_find_matches_new_sessions"
        | "open-all-find-matches-new-tabs"
        | "open_all_find_matches_new_tabs"
        | "open-find-matches-tabs"
        | "open_find_matches_tabs" => Ok(BrowserSessionAction::OpenFindMatchesNewSessions {
            limit: parse_optional_usize_param(target, "limit", 1, MAX_BULK_BACKGROUND_LINKS)
                .or_else(|| {
                    parse_optional_usize_param(target, "count", 1, MAX_BULK_BACKGROUND_LINKS)
                })
                .unwrap_or(DEFAULT_BULK_BACKGROUND_LINKS),
        }),
        "open-find-matches-background"
        | "open_find_matches_background"
        | "open-find-matches-background-sessions"
        | "open_find_matches_background_sessions"
        | "open-all-find-matches-background"
        | "open_all_find_matches_background"
        | "open-all-find-matches-background-sessions"
        | "open_all_find_matches_background_sessions" => {
            Ok(BrowserSessionAction::OpenFindMatchesBackgroundSessions {
                limit: parse_optional_usize_param(target, "limit", 1, MAX_BULK_BACKGROUND_LINKS)
                    .or_else(|| {
                        parse_optional_usize_param(target, "count", 1, MAX_BULK_BACKGROUND_LINKS)
                    })
                    .unwrap_or(DEFAULT_BULK_BACKGROUND_LINKS),
            })
        }
        "find-next" | "find_next" => Ok(BrowserSessionAction::FindNext),
        "find-prev" | "find_previous" | "find-previous" => Ok(BrowserSessionAction::FindPrevious),
        "clear-find" | "clear_find" => Ok(BrowserSessionAction::ClearFind),
        "search-tabs" | "search_tabs" | "find-tabs" | "find_tabs" | "tab-search" | "tab_search" => {
            Ok(BrowserSessionAction::SearchTabs(browser_search_query(
                target,
            )))
        }
        "clear-tab-search" | "clear_tab_search" | "clear-tabs-search" | "clear_tabs_search" => {
            Ok(BrowserSessionAction::ClearTabSearch)
        }
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
            raster_width: parse_optional_usize_param(target, "raster_width", 1, usize::MAX),
            raster_height: parse_optional_usize_param(target, "raster_height", 1, usize::MAX),
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
        "focus-control" | "focus_control" | "focus-form-control" | "focus_form_control" => {
            Ok(BrowserSessionAction::FocusControl {
                form_index: browser_action_index(target, "form", "form index")?,
                control_index: browser_action_index(target, "control", "control index")?,
            })
        }
        "activate-control"
        | "activate_control"
        | "activate-form-control"
        | "activate_form_control" => Ok(BrowserSessionAction::ActivateControl {
            form_index: browser_action_index(target, "form", "form index")?,
            control_index: browser_action_index(target, "control", "control index")?,
        }),
        "activate-control-new-session"
        | "activate_control_new_session"
        | "activate-control-new-tab"
        | "activate_control_new_tab"
        | "activate-form-control-new-session"
        | "activate_form_control_new_session"
        | "activate-form-control-new-tab"
        | "activate_form_control_new_tab" => Ok(BrowserSessionAction::ActivateControlNewSession {
            form_index: browser_action_index(target, "form", "form index")?,
            control_index: browser_action_index(target, "control", "control index")?,
        }),
        "activate-control-background-session"
        | "activate_control_background_session"
        | "activate-control-background-tab"
        | "activate_control_background_tab"
        | "activate-form-control-background-session"
        | "activate_form_control_background_session"
        | "activate-form-control-background-tab"
        | "activate_form_control_background_tab" => {
            Ok(BrowserSessionAction::ActivateControlBackgroundSession {
                form_index: browser_action_index(target, "form", "form index")?,
                control_index: browser_action_index(target, "control", "control index")?,
            })
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
        "bookmark-all-tabs"
        | "bookmark_all_tabs"
        | "bookmark-all-sessions"
        | "bookmark_all_sessions"
        | "bookmark-open-tabs"
        | "bookmark_open_tabs"
        | "bookmark-open-sessions"
        | "bookmark_open_sessions"
        | "bookmark-tabs"
        | "bookmark_tabs" => Ok(BrowserSessionAction::BookmarkAllTabs),
        "bookmark-profile-history"
        | "bookmark_profile_history"
        | "bookmark-profile-history-entries"
        | "bookmark_profile_history_entries"
        | "bookmark-history"
        | "bookmark_history"
        | "bookmark-history-entries"
        | "bookmark_history_entries" => Ok(BrowserSessionAction::BookmarkProfileHistory),
        "remove-profile-history-bookmarks"
        | "remove_profile_history_bookmarks"
        | "remove-profile-history-bookmark"
        | "remove_profile_history_bookmark"
        | "remove-history-bookmarks"
        | "remove_history_bookmarks"
        | "unbookmark-profile-history"
        | "unbookmark_profile_history"
        | "unbookmark-history"
        | "unbookmark_history" => Ok(BrowserSessionAction::RemoveProfileHistoryBookmarks),
        "bookmark-tab-search-results"
        | "bookmark_tab_search_results"
        | "bookmark-tab-search-matches"
        | "bookmark_tab_search_matches"
        | "bookmark-search-tabs"
        | "bookmark_search_tabs"
        | "bookmark-matching-tabs"
        | "bookmark_matching_tabs" => Ok(BrowserSessionAction::BookmarkTabSearchResults),
        "remove-tab-search-bookmarks"
        | "remove_tab_search_bookmarks"
        | "remove-tab-search-bookmark"
        | "remove_tab_search_bookmark"
        | "remove-tab-search-match-bookmarks"
        | "remove_tab_search_match_bookmarks"
        | "remove-bookmarked-search-tabs"
        | "remove_bookmarked_search_tabs"
        | "unbookmark-tab-search-results"
        | "unbookmark_tab_search_results"
        | "unbookmark-search-tabs"
        | "unbookmark_search_tabs" => Ok(BrowserSessionAction::RemoveTabSearchBookmarks),
        "open-bookmark" | "open_bookmark" => Ok(BrowserSessionAction::OpenBookmark(
            browser_bookmark_id(target)?,
        )),
        "rename-bookmark" | "rename_bookmark" | "label-bookmark" | "label_bookmark" => {
            Ok(BrowserSessionAction::RenameBookmark {
                bookmark_id: browser_bookmark_id(target)?,
                title: browser_bookmark_title(target)?,
            })
        }
        "remove-bookmark" | "remove_bookmark" | "delete-bookmark" | "delete_bookmark" => Ok(
            BrowserSessionAction::RemoveBookmark(browser_bookmark_id(target)?),
        ),
        "clear-bookmarks" | "clear_bookmarks" | "remove-bookmarks" | "remove_bookmarks"
        | "delete-bookmarks" | "delete_bookmarks" => Ok(BrowserSessionAction::ClearBookmarks),
        "open-bookmarks-new-sessions"
        | "open_bookmarks_new_sessions"
        | "open-bookmarks-new-tabs"
        | "open_bookmarks_new_tabs"
        | "open-all-bookmarks-new-sessions"
        | "open_all_bookmarks_new_sessions"
        | "open-all-bookmarks-new-tabs"
        | "open_all_bookmarks_new_tabs"
        | "open-bookmarks-tabs"
        | "open_bookmarks_tabs" => Ok(BrowserSessionAction::OpenBookmarksNewSessions),
        "open-bookmarks-background"
        | "open_bookmarks_background"
        | "open-bookmarks-background-sessions"
        | "open_bookmarks_background_sessions"
        | "open-all-bookmarks-background"
        | "open_all_bookmarks_background"
        | "open-all-bookmarks-background-sessions"
        | "open_all_bookmarks_background_sessions" => {
            Ok(BrowserSessionAction::OpenBookmarksBackgroundSessions)
        }
        "open-profile-history-new-sessions"
        | "open_profile_history_new_sessions"
        | "open-profile-history-new-tabs"
        | "open_profile_history_new_tabs"
        | "open-all-profile-history-new-sessions"
        | "open_all_profile_history_new_sessions"
        | "open-all-profile-history-new-tabs"
        | "open_all_profile_history_new_tabs"
        | "open-profile-history-tabs"
        | "open_profile_history_tabs"
        | "restore-profile-history-new-sessions"
        | "restore_profile_history_new_sessions"
        | "restore-profile-history-new-tabs"
        | "restore_profile_history_new_tabs" => {
            Ok(BrowserSessionAction::OpenProfileHistoryNewSessions {
                limit: parse_optional_usize_param(target, "limit", 1, MAX_BULK_BACKGROUND_LINKS)
                    .or_else(|| {
                        parse_optional_usize_param(target, "count", 1, MAX_BULK_BACKGROUND_LINKS)
                    })
                    .unwrap_or(DEFAULT_BULK_BACKGROUND_LINKS),
            })
        }
        "open-profile-history-background"
        | "open_profile_history_background"
        | "open-profile-history-background-sessions"
        | "open_profile_history_background_sessions"
        | "open-all-profile-history-background"
        | "open_all_profile_history_background"
        | "open-all-profile-history-background-sessions"
        | "open_all_profile_history_background_sessions"
        | "restore-profile-history-background"
        | "restore_profile_history_background"
        | "restore-profile-history-background-sessions"
        | "restore_profile_history_background_sessions" => {
            Ok(BrowserSessionAction::OpenProfileHistoryBackgroundSessions {
                limit: parse_optional_usize_param(target, "limit", 1, MAX_BULK_BACKGROUND_LINKS)
                    .or_else(|| {
                        parse_optional_usize_param(target, "count", 1, MAX_BULK_BACKGROUND_LINKS)
                    })
                    .unwrap_or(DEFAULT_BULK_BACKGROUND_LINKS),
            })
        }
        "open-profile-closed"
        | "open_profile_closed"
        | "restore-profile-closed"
        | "restore_profile_closed" => Ok(BrowserSessionAction::OpenProfileClosed(
            browser_profile_closed_index(target)?,
        )),
        "open-profile-closed-background-session"
        | "open_profile_closed_background_session"
        | "open-profile-closed-background-tab"
        | "open_profile_closed_background_tab"
        | "restore-profile-closed-background-session"
        | "restore_profile_closed_background_session"
        | "restore-profile-closed-background-tab"
        | "restore_profile_closed_background_tab" => {
            Ok(BrowserSessionAction::OpenProfileClosedBackgroundSession(
                browser_profile_closed_index(target)?,
            ))
        }
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
        "restore-closed-background-session"
        | "restore_closed_background_session"
        | "restore-closed-background-tab"
        | "restore_closed_background_tab"
        | "restore-session-background"
        | "restore_session_background" => Ok(BrowserSessionAction::RestoreClosedBackgroundSession(
            browser_closed_session_id(target)?,
        )),
        "restore-all-closed-background"
        | "restore_all_closed_background"
        | "restore-closed-background-sessions"
        | "restore_closed_background_sessions"
        | "restore-closed-background-tabs"
        | "restore_closed_background_tabs"
        | "restore-all-closed-background-sessions"
        | "restore_all_closed_background_sessions"
        | "restore-all-closed-background-tabs"
        | "restore_all_closed_background_tabs"
        | "restore-closed-sessions-background"
        | "restore_closed_sessions_background"
        | "restore-closed-tabs-background"
        | "restore_closed_tabs_background" => {
            Ok(BrowserSessionAction::RestoreClosedBackgroundSessions)
        }
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
        "make-visual" | "make_visual" | "visual" | "visual-render" | "visual_render" => {
            Ok(BrowserSessionAction::MakeVisual)
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
        "duplicate-background"
        | "duplicate_background"
        | "duplicate-background-tab"
        | "duplicate_background_tab"
        | "duplicate-background-session"
        | "duplicate_background_session"
        | "duplicate-tab-background"
        | "duplicate_tab_background"
        | "duplicate-session-background"
        | "duplicate_session_background" => {
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
                Ok(BrowserSessionAction::DuplicateBackgroundSession(
                    duplicate_id,
                ))
            }
        }
        "duplicate-tab-search-results"
        | "duplicate_tab_search_results"
        | "duplicate-tab-search-matches"
        | "duplicate_tab_search_matches"
        | "duplicate-search-tabs"
        | "duplicate_search_tabs"
        | "duplicate-matching-tabs"
        | "duplicate_matching_tabs" => Ok(BrowserSessionAction::DuplicateTabSearchResults),
        "close-other-tabs"
        | "close_other_tabs"
        | "close-other-sessions"
        | "close_other_sessions" => Ok(BrowserSessionAction::CloseOtherSessions),
        "close-unpinned-tabs"
        | "close_unpinned_tabs"
        | "close-unpinned"
        | "close_unpinned"
        | "close-unpinned-sessions"
        | "close_unpinned_sessions" => Ok(BrowserSessionAction::CloseUnpinnedSessions),
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
        "close-tab-search-results"
        | "close_tab_search_results"
        | "close-tab-search-matches"
        | "close_tab_search_matches"
        | "close-search-tabs"
        | "close_search_tabs"
        | "close-matching-tabs"
        | "close_matching_tabs" => Ok(BrowserSessionAction::CloseTabSearchResults),
        "close-tab-search-nonmatches"
        | "close_tab_search_nonmatches"
        | "close-tab-search-non-matches"
        | "close_tab_search_non_matches"
        | "close-tabs-not-matching-search"
        | "close_tabs_not_matching_search"
        | "close-nonmatching-tabs"
        | "close_nonmatching_tabs"
        | "close-non-matching-tabs"
        | "close_non_matching_tabs" => Ok(BrowserSessionAction::CloseTabSearchNonMatches),
        "reload-tab-search-results"
        | "reload_tab_search_results"
        | "reload-tab-search-matches"
        | "reload_tab_search_matches"
        | "reload-search-tabs"
        | "reload_search_tabs"
        | "reload-matching-tabs"
        | "reload_matching_tabs" => Ok(BrowserSessionAction::ReloadTabSearchResults),
        "pin-tab-search-results"
        | "pin_tab_search_results"
        | "pin-tab-search-matches"
        | "pin_tab_search_matches"
        | "pin-search-tabs"
        | "pin_search_tabs"
        | "pin-matching-tabs"
        | "pin_matching_tabs" => Ok(BrowserSessionAction::PinTabSearchResults),
        "unpin-tab-search-results"
        | "unpin_tab_search_results"
        | "unpin-tab-search-matches"
        | "unpin_tab_search_matches"
        | "unpin-search-tabs"
        | "unpin_search_tabs"
        | "unpin-matching-tabs"
        | "unpin_matching_tabs" => Ok(BrowserSessionAction::UnpinTabSearchResults),
        "label-tab-search-results"
        | "label_tab_search_results"
        | "label-tab-search-matches"
        | "label_tab_search_matches"
        | "label-search-tabs"
        | "label_search_tabs"
        | "label-matching-tabs"
        | "label_matching_tabs" => Ok(BrowserSessionAction::LabelTabSearchResults(
            browser_tab_label(target)?,
        )),
        "clear-tab-search-labels"
        | "clear_tab_search_labels"
        | "clear-tab-search-label"
        | "clear_tab_search_label"
        | "clear-search-tab-labels"
        | "clear_search_tab_labels"
        | "clear-matching-tab-labels"
        | "clear_matching_tab_labels" => Ok(BrowserSessionAction::ClearTabSearchLabels),
        "pin-tab" | "pin_tab" | "pin-session" | "pin_session" => {
            browser_target_session_id(target, "pin").map(BrowserSessionAction::PinSession)
        }
        "unpin-tab" | "unpin_tab" | "unpin-session" | "unpin_session" => {
            browser_target_session_id(target, "unpin").map(BrowserSessionAction::UnpinSession)
        }
        "pin-all-tabs" | "pin_all_tabs" | "pin-tabs" | "pin_tabs" | "pin-all-sessions"
        | "pin_all_sessions" => Ok(BrowserSessionAction::PinAllSessions),
        "unpin-all-tabs" | "unpin_all_tabs" | "unpin-tabs" | "unpin_tabs"
        | "unpin-all-sessions" | "unpin_all_sessions" => Ok(BrowserSessionAction::UnpinAllSessions),
        "move-tab-left" | "move_tab_left" | "move-session-left" | "move_session_left"
        | "tab-left" | "tab_left" => browser_target_session_id(target, "move left")
            .map(BrowserSessionAction::MoveSessionLeft),
        "move-tab-right" | "move_tab_right" | "move-session-right" | "move_session_right"
        | "tab-right" | "tab_right" => browser_target_session_id(target, "move right")
            .map(BrowserSessionAction::MoveSessionRight),
        "move-tab-search-results-front"
        | "move_tab_search_results_front"
        | "move-tab-search-matches-front"
        | "move_tab_search_matches_front"
        | "move-search-tabs-front"
        | "move_search_tabs_front"
        | "move-matching-tabs-front"
        | "move_matching_tabs_front"
        | "tab-search-results-front"
        | "tab_search_results_front" => Ok(BrowserSessionAction::MoveTabSearchResultsToFront),
        "move-tab-search-results-back"
        | "move_tab_search_results_back"
        | "move-tab-search-results-end"
        | "move_tab_search_results_end"
        | "move-tab-search-matches-back"
        | "move_tab_search_matches_back"
        | "move-tab-search-matches-end"
        | "move_tab_search_matches_end"
        | "move-search-tabs-back"
        | "move_search_tabs_back"
        | "move-search-tabs-end"
        | "move_search_tabs_end"
        | "move-matching-tabs-back"
        | "move_matching_tabs_back"
        | "move-matching-tabs-end"
        | "move_matching_tabs_end"
        | "tab-search-results-back"
        | "tab_search_results_back"
        | "tab-search-results-end"
        | "tab_search_results_end" => Ok(BrowserSessionAction::MoveTabSearchResultsToBack),
        "label-tab" | "label_tab" | "rename-tab" | "rename_tab" | "label-session"
        | "label_session" | "rename-session" | "rename_session" => {
            Ok(BrowserSessionAction::LabelSession {
                session_id: browser_target_session_id(target, "label")?,
                label: browser_tab_label(target)?,
            })
        }
        "clear-tab-label" | "clear_tab_label" | "clear-session-label" | "clear_session_label" => {
            browser_target_session_id(target, "clear label")
                .map(BrowserSessionAction::ClearSessionLabel)
        }
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
        "jump-tab" | "jump_tab" | "jump-session" | "jump_session" | "switch-tab" | "switch_tab"
        | "switch-session" | "switch_session" => {
            browser_session_query(target).map(BrowserSessionAction::JumpSession)
        }
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
        "page-up" | "page_up" | "pageup" => Ok(BrowserSessionAction::PageUp),
        "page-down" | "page_down" | "pagedown" => Ok(BrowserSessionAction::PageDown),
        "line-up" | "line_up" | "up-one" | "up_one" => Ok(BrowserSessionAction::LineUp),
        "line-down" | "line_down" | "down-one" | "down_one" => Ok(BrowserSessionAction::LineDown),
        "fill" => Ok(BrowserSessionAction::Fill {
            form_index: browser_action_index(target, "form", "form index")?,
            name: target
                .param("name")
                .ok_or_else(|| BrowserRouteError::BadRequest("missing field name".to_owned()))?,
            value: target.param("value").unwrap_or_default(),
        }),
        "fill-control" | "fill_control" | "fill-form-control" | "fill_form_control" => {
            Ok(BrowserSessionAction::FillControl {
                form_index: browser_action_index(target, "form", "form index")?,
                control_index: browser_action_index(target, "control", "control index")?,
                value: target.param("value").unwrap_or_default(),
            })
        }
        "type-control" | "type_control" | "type-form-control" | "type_form_control"
        | "edit-control" | "edit_control" | "edit-form-control" | "edit_form_control" => {
            Ok(BrowserSessionAction::TypeControl {
                form_index: browser_action_index(target, "form", "form index")?,
                control_index: browser_action_index(target, "control", "control index")?,
                value: target.param("value").unwrap_or_default(),
            })
        }
        "clear-control" | "clear_control" | "clear-form-control" | "clear_form_control"
        | "clear-field" | "clear_field" => Ok(BrowserSessionAction::ClearControl {
            form_index: browser_action_index(target, "form", "form index")?,
            control_index: browser_action_index(target, "control", "control index")?,
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
        "submit-new-session"
        | "submit_new_session"
        | "submit-new-tab"
        | "submit_new_tab"
        | "submit-form-new-session"
        | "submit_form_new_session"
        | "submit-form-new-tab"
        | "submit_form_new_tab" => Ok(BrowserSessionAction::SubmitNewSession {
            form_index: browser_action_index(target, "form", "form index")?,
        }),
        "submit-background-session"
        | "submit_background_session"
        | "submit-background-tab"
        | "submit_background_tab"
        | "submit-form-background-session"
        | "submit_form_background_session"
        | "submit-form-background-tab"
        | "submit_form_background_tab" => Ok(BrowserSessionAction::SubmitBackgroundSession {
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

#[derive(Debug, Clone, Copy)]
struct BrowserFindMatch {
    line: usize,
    column: usize,
}

#[derive(Debug, Clone)]
enum BrowserSessionCloseScope {
    Others,
    Unpinned,
    LeftOfActive,
    RightOfActive,
    DuplicateSource(String),
}

#[derive(Debug, Clone, Copy)]
enum BrowserSessionSwitchDirection {
    Next,
    Previous,
}

#[derive(Debug, Clone, Copy)]
enum BrowserSessionMoveDirection {
    Left,
    Right,
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
        checked_browser_address_url(&url)
    }
}

struct BrowserSessionNavigationTarget {
    target: String,
    display_source: Option<String>,
}

fn browser_session_navigation_target(
    target: &str,
    max_bytes: usize,
) -> Result<BrowserSessionNavigationTarget, BrowserRouteError> {
    if !target.trim_start().starts_with("data:") {
        return Ok(BrowserSessionNavigationTarget {
            target: target.to_owned(),
            display_source: None,
        });
    }

    let (content_type, bytes) = browser_session_decode_data_url(target, max_bytes)?;
    let extension = if content_type
        .split(';')
        .next()
        .unwrap_or_default()
        .eq_ignore_ascii_case("image/svg+xml")
    {
        "svg"
    } else {
        "html"
    };
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    let path = std::env::temp_dir().join(format!(
        "brutal-browser-data-url-{}-{now}.{extension}",
        std::process::id()
    ));
    fs::write(&path, bytes).map_err(|error| {
        BrowserRouteError::Upstream(format!("browser data URL materialization failed: {error}"))
    })?;

    Ok(BrowserSessionNavigationTarget {
        target: path.display().to_string(),
        display_source: Some(target.to_owned()),
    })
}

fn browser_session_decode_data_url(
    target: &str,
    max_bytes: usize,
) -> Result<(String, Vec<u8>), BrowserRouteError> {
    let payload = target
        .strip_prefix("data:")
        .ok_or_else(|| BrowserRouteError::BadRequest("invalid data URL".to_owned()))?;
    let (metadata, data) = payload.split_once(',').ok_or_else(|| {
        BrowserRouteError::BadRequest("invalid data URL: missing payload".to_owned())
    })?;
    let mut content_type = "text/plain".to_owned();
    let mut base64 = false;
    for (index, part) in metadata.split(';').enumerate() {
        if index == 0 && !part.is_empty() {
            content_type = part.to_owned();
        } else if part.eq_ignore_ascii_case("base64") {
            base64 = true;
        }
    }
    let bytes = if base64 {
        browser_session_decode_base64_data_url_payload(data)?
    } else {
        browser_session_percent_decode_data_url_payload(data)?
    };
    if bytes.len() > max_bytes {
        return Err(BrowserRouteError::BadRequest(format!(
            "data URL exceeds byte cap: {} > {}",
            bytes.len(),
            max_bytes
        )));
    }
    Ok((content_type, bytes))
}

fn browser_session_decode_base64_data_url_payload(
    input: &str,
) -> Result<Vec<u8>, BrowserRouteError> {
    let mut out = Vec::with_capacity(input.len().saturating_mul(3) / 4);
    let mut block = [0u8; 4];
    let mut block_len = 0usize;
    let mut padding = 0usize;

    for byte in input.bytes().filter(|byte| !byte.is_ascii_whitespace()) {
        let value = match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            b'=' => {
                padding += 1;
                0
            }
            _ => {
                return Err(BrowserRouteError::BadRequest(format!(
                    "invalid base64 data URL byte: 0x{byte:02x}"
                )));
            }
        };
        if padding > 0 && byte != b'=' {
            return Err(BrowserRouteError::BadRequest(
                "invalid base64 data URL padding".to_owned(),
            ));
        }
        block[block_len] = value;
        block_len += 1;
        if block_len == 4 {
            out.push((block[0] << 2) | (block[1] >> 4));
            if padding < 2 {
                out.push((block[1] << 4) | (block[2] >> 2));
            }
            if padding == 0 {
                out.push((block[2] << 6) | block[3]);
            }
            block_len = 0;
            padding = 0;
        }
    }

    if block_len != 0 {
        return Err(BrowserRouteError::BadRequest(
            "truncated base64 data URL payload".to_owned(),
        ));
    }
    Ok(out)
}

fn browser_session_percent_decode_data_url_payload(
    input: &str,
) -> Result<Vec<u8>, BrowserRouteError> {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0usize;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            let high =
                browser_session_data_url_hex_value(*bytes.get(index + 1).ok_or_else(|| {
                    BrowserRouteError::BadRequest("truncated percent escape in data URL".to_owned())
                })?)?;
            let low =
                browser_session_data_url_hex_value(*bytes.get(index + 2).ok_or_else(|| {
                    BrowserRouteError::BadRequest("truncated percent escape in data URL".to_owned())
                })?)?;
            out.push((high << 4) | low);
            index += 3;
        } else {
            out.push(bytes[index]);
            index += 1;
        }
    }
    Ok(out)
}

fn browser_session_data_url_hex_value(byte: u8) -> Result<u8, BrowserRouteError> {
    match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        b'A'..=b'F' => Ok(byte - b'A' + 10),
        _ => Err(BrowserRouteError::BadRequest(
            "invalid percent escape in data URL".to_owned(),
        )),
    }
}

fn normalize_browser_address_url(input: &str) -> String {
    let trimmed = input.trim();
    if trimmed.is_empty()
        || browser_address_has_scheme(trimmed)
        || browser_address_is_file_path(trimmed)
    {
        return trimmed.to_owned();
    }

    let host = browser_address_host_prefix(trimmed);
    if host.eq_ignore_ascii_case("localhost")
        || host.starts_with("localhost:")
        || host.starts_with("127.")
        || host.starts_with("[::1]")
    {
        return format!("http://{trimmed}");
    }
    if browser_address_looks_like_host(host) {
        return format!("https://{trimmed}");
    }

    trimmed.to_owned()
}

fn checked_browser_address_url(input: &str) -> Result<String, BrowserRouteError> {
    let normalized = normalize_browser_address_url(input);
    if browser_address_has_unsafe_pseudo_scheme(&normalized) {
        let scheme = normalized
            .split_once(':')
            .map(|(scheme, _)| scheme)
            .unwrap_or("unknown");
        return Err(BrowserRouteError::BadRequest(format!(
            "unsupported browser URL scheme: {scheme}"
        )));
    }
    Ok(normalized)
}

fn browser_safe_source_param(source: &str) -> Option<&str> {
    let clean = source.trim();
    if clean.is_empty() || browser_address_has_unsafe_pseudo_scheme(clean) {
        None
    } else {
        Some(source)
    }
}

fn browser_address_has_unsafe_pseudo_scheme(value: &str) -> bool {
    let clean = value.trim_start();
    if !browser_address_has_scheme(clean) {
        return false;
    }
    let scheme = clean
        .split_once(':')
        .map(|(scheme, _)| scheme)
        .unwrap_or("");
    matches!(
        scheme.to_ascii_lowercase().as_str(),
        "javascript" | "vbscript" | "livescript"
    )
}

fn browser_address_has_scheme(value: &str) -> bool {
    let Some(colon) = value.find(':') else {
        return false;
    };
    let first_delimiter = value
        .find(|ch| matches!(ch, '/' | '?' | '#'))
        .unwrap_or(value.len());
    if colon > first_delimiter {
        return false;
    }
    let scheme = &value[..colon];
    if scheme.contains('.') || scheme.eq_ignore_ascii_case("localhost") {
        return false;
    }
    let after_colon = &value[colon + 1..first_delimiter];
    if !after_colon.is_empty() && after_colon.chars().all(|ch| ch.is_ascii_digit()) {
        return false;
    }
    let mut chars = scheme.chars();
    chars.next().is_some_and(|ch| ch.is_ascii_alphabetic())
        && chars.all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '+' | '-' | '.'))
}

fn browser_address_is_file_path(value: &str) -> bool {
    value.starts_with('/')
        || value.starts_with("./")
        || value.starts_with("../")
        || value.starts_with("~/")
        || Path::new(value).exists()
}

fn browser_address_host_prefix(value: &str) -> &str {
    value
        .split(|ch| matches!(ch, '/' | '?' | '#'))
        .next()
        .unwrap_or(value)
}

fn browser_address_looks_like_host(host: &str) -> bool {
    let host = host.trim_matches(|ch| matches!(ch, '[' | ']'));
    !host.is_empty() && (host.contains('.') || host.contains(':'))
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

fn browser_resource_index(target: &RequestTarget) -> Result<usize, BrowserRouteError> {
    target
        .param("resource")
        .or_else(|| target.param("index"))
        .ok_or_else(|| BrowserRouteError::BadRequest("missing resource index".to_owned()))?
        .parse::<usize>()
        .map_err(|_| BrowserRouteError::BadRequest("invalid resource index".to_owned()))
}

fn browser_anchor_index(target: &RequestTarget) -> Result<usize, BrowserRouteError> {
    let value = target
        .param("anchor")
        .or_else(|| target.param("fragment"))
        .or_else(|| target.param("index"))
        .ok_or_else(|| BrowserRouteError::BadRequest("missing anchor index".to_owned()))?;
    let anchor_index = value
        .parse::<usize>()
        .map_err(|_| BrowserRouteError::BadRequest("invalid anchor index".to_owned()))?;
    if anchor_index == 0 {
        return Err(BrowserRouteError::BadRequest(
            "anchor index must be 1 or greater".to_owned(),
        ));
    }
    Ok(anchor_index - 1)
}

fn browser_action_records_profile_visit(action: &BrowserSessionAction) -> bool {
    matches!(
        action,
        BrowserSessionAction::Open(_)
            | BrowserSessionAction::Back
            | BrowserSessionAction::Forward
            | BrowserSessionAction::Link(_)
            | BrowserSessionAction::Resource(_)
            | BrowserSessionAction::LinkText(_)
            | BrowserSessionAction::LinkSelector(_)
            | BrowserSessionAction::History(_)
            | BrowserSessionAction::ClickSelector(_)
            | BrowserSessionAction::ClickAt { .. }
            | BrowserSessionAction::Enter
            | BrowserSessionAction::ActivateControl { .. }
            | BrowserSessionAction::OpenBookmark(_)
            | BrowserSessionAction::OpenProfileClosed(_)
            | BrowserSessionAction::Submit { .. }
    )
}

fn browser_action_records_profile_tabs(action: &BrowserSessionAction) -> bool {
    !matches!(
        action,
        BrowserSessionAction::ClearProfileTabs
            | BrowserSessionAction::SearchTabs(_)
            | BrowserSessionAction::ClearTabSearch
    )
}

fn browser_session_payload_options_for_action(
    _action: &BrowserSessionAction,
) -> BrowserSessionPayloadOptions {
    BrowserSessionPayloadOptions::default()
}

fn browser_action_marks_session_in_flight(action: &BrowserSessionAction) -> bool {
    matches!(
        action,
        BrowserSessionAction::FetchResources
            | BrowserSessionAction::MakeVisual
            | BrowserSessionAction::ApplyStylesheets
            | BrowserSessionAction::RunScripts
            | BrowserSessionAction::LoadImages
    )
}

fn browser_action_can_apply_in_flight_viewport_partial(action: &BrowserSessionAction) -> bool {
    matches!(
        action,
        BrowserSessionAction::Scroll { .. }
            | BrowserSessionAction::Top
            | BrowserSessionAction::Bottom
            | BrowserSessionAction::PageUp
            | BrowserSessionAction::PageDown
            | BrowserSessionAction::LineUp
            | BrowserSessionAction::LineDown
    )
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

fn browser_bookmark_title(target: &RequestTarget) -> Result<String, BrowserRouteError> {
    let title = target
        .param("title")
        .or_else(|| target.param("label"))
        .or_else(|| target.param("name"))
        .unwrap_or_default();
    normalize_browser_tab_label_option(Some(&title))
        .ok_or_else(|| BrowserRouteError::BadRequest("missing browser bookmark title".to_owned()))
}

fn browser_session_query(target: &RequestTarget) -> Result<String, BrowserRouteError> {
    let query = target
        .param("q")
        .or_else(|| target.param("query"))
        .or_else(|| target.param("tab"))
        .or_else(|| target.param("session"))
        .unwrap_or_default();
    let query = query.trim();
    if query.is_empty() {
        Err(BrowserRouteError::BadRequest(
            "missing browser session query".to_owned(),
        ))
    } else {
        Ok(query.to_owned())
    }
}

fn browser_target_session_id(
    target: &RequestTarget,
    label: &str,
) -> Result<String, BrowserRouteError> {
    let session_id = target
        .param("session")
        .or_else(|| target.param("target_session"))
        .or_else(|| target.param("tab"))
        .or_else(|| target.param("id"))
        .unwrap_or_default();
    if session_id.trim().is_empty() {
        Err(BrowserRouteError::BadRequest(format!(
            "missing browser session to {label}"
        )))
    } else {
        Ok(session_id)
    }
}

fn browser_tab_label(target: &RequestTarget) -> Result<String, BrowserRouteError> {
    let label = target
        .param("label")
        .or_else(|| target.param("title"))
        .or_else(|| target.param("name"))
        .unwrap_or_default();
    normalize_browser_tab_label_option(Some(&label))
        .ok_or_else(|| BrowserRouteError::BadRequest("missing browser tab label".to_owned()))
}

fn browser_search_query(target: &RequestTarget) -> String {
    let query = target
        .param("q")
        .or_else(|| target.param("query"))
        .or_else(|| target.param("text"))
        .unwrap_or_default();
    normalize_browser_search_query(&query)
}

fn normalize_browser_tab_label_option(label: Option<&str>) -> Option<String> {
    let label = label?.trim();
    if label.is_empty() {
        return None;
    }
    let mut normalized = String::new();
    let mut previous_was_whitespace = false;
    for character in label.chars() {
        if character.is_whitespace() {
            if !previous_was_whitespace && !normalized.is_empty() {
                normalized.push(' ');
                previous_was_whitespace = true;
            }
        } else {
            normalized.push(character);
            previous_was_whitespace = false;
        }
        if normalized.chars().count() >= 80 {
            break;
        }
    }
    let normalized = normalized.trim();
    (!normalized.is_empty()).then(|| normalized.to_owned())
}

fn normalize_browser_search_query(query: &str) -> String {
    query
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(160)
        .collect()
}

fn browser_find_match_index(target: &RequestTarget) -> Result<usize, BrowserRouteError> {
    let value = target
        .param("match")
        .or_else(|| target.param("match_index"))
        .or_else(|| target.param("index"))
        .ok_or_else(|| BrowserRouteError::BadRequest("missing find match index".to_owned()))?;
    let match_index = value
        .parse::<usize>()
        .map_err(|_| BrowserRouteError::BadRequest("invalid find match index".to_owned()))?;
    if match_index == 0 {
        return Err(BrowserRouteError::BadRequest(
            "find match index must be 1 or greater".to_owned(),
        ));
    }
    Ok(match_index - 1)
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
    let Some(target_match) = browser_find_target_match(&matches, current_line, direction) else {
        clear_browser_find_active_line(web_session);
        return Ok(());
    };
    web_session.viewport_x = target_match.column;
    web_session.viewport_y = target_match.line;
    web_session.find_active_line = Some(target_match.line);
    Ok(())
}

fn apply_browser_find_match(
    web_session: &mut BrowserWebSession,
    match_index: usize,
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
    let Some(target_match) = matches.get(match_index).copied() else {
        return Err(BrowserRouteError::BadRequest(format!(
            "find match {} is not available",
            match_index + 1
        )));
    };
    web_session.viewport_x = target_match.column;
    web_session.viewport_y = target_match.line;
    web_session.find_active_line = Some(target_match.line);
    Ok(())
}

fn browser_find_target_match(
    matches: &[BrowserFindMatch],
    viewport_y: usize,
    direction: BrowserFindDirection,
) -> Option<BrowserFindMatch> {
    match direction {
        BrowserFindDirection::First => matches.first().copied(),
        BrowserFindDirection::Next => matches
            .iter()
            .copied()
            .find(|match_| match_.line > viewport_y)
            .or_else(|| matches.first().copied()),
        BrowserFindDirection::Previous => matches
            .iter()
            .rev()
            .copied()
            .find(|match_| match_.line < viewport_y)
            .or_else(|| matches.last().copied()),
    }
}

fn browser_find_matches(text: &str, query: &str) -> Vec<BrowserFindMatch> {
    let needle = query.trim();
    if needle.is_empty() {
        return Vec::new();
    }
    text.lines()
        .enumerate()
        .filter_map(|(line, text)| {
            let byte_index = find_ascii_case_insensitive(text, needle)?;
            Some(BrowserFindMatch {
                line,
                column: text[..byte_index].chars().count(),
            })
        })
        .collect()
}

fn browser_bulk_find_match_indices(
    web_session: &BrowserWebSession,
    limit: usize,
) -> Result<Vec<usize>, BrowserRouteError> {
    let query = web_session.find_query.trim();
    if query.is_empty() {
        return Err(BrowserRouteError::BadRequest(
            "missing browser find query".to_owned(),
        ));
    }
    let render = web_session.session.current().ok_or_else(|| {
        BrowserRouteError::BadRequest("browser session has no current page".to_owned())
    })?;
    let active_line = web_session.find_active_line;
    Ok(browser_find_matches(&render.text, query)
        .into_iter()
        .enumerate()
        .filter(|(_, match_)| Some(match_.line) != active_line)
        .map(|(index, _)| index)
        .take(MAX_BULK_BACKGROUND_LINKS)
        .take(limit)
        .collect())
}

fn browser_session_find_match_payloads(
    id: &str,
    web_session: &BrowserWebSession,
    text: &str,
    matches: &[BrowserFindMatch],
    current_index: Option<usize>,
) -> Vec<BrowserSessionFindMatchPayload> {
    let lines = text.lines().collect::<Vec<_>>();
    matches
        .iter()
        .copied()
        .enumerate()
        .map(|(index, match_)| BrowserSessionFindMatchPayload {
            index,
            line: match_.line,
            column: match_.column,
            current: current_index == Some(index),
            text: lines
                .get(match_.line)
                .copied()
                .unwrap_or_default()
                .to_owned(),
            action_url: browser_session_action_href(
                id,
                "find-match",
                &[("match", (index + 1).to_string())],
                web_session,
            ),
            new_session_url: browser_session_action_href(
                id,
                "find-match-new-session",
                &[("match", (index + 1).to_string())],
                web_session,
            ),
            background_session_url: browser_session_action_href(
                id,
                "find-match-background-session",
                &[("match", (index + 1).to_string())],
                web_session,
            ),
        })
        .collect()
}

fn browser_find_visible_match(
    matches: &[BrowserFindMatch],
    viewport_y: usize,
    viewport_height: usize,
) -> Option<(usize, BrowserFindMatch)> {
    let viewport_end = viewport_y.saturating_add(viewport_height.max(1));
    matches
        .iter()
        .enumerate()
        .find(|(_, match_)| match_.line >= viewport_y && match_.line < viewport_end)
        .map(|(index, match_)| (index, *match_))
}

fn browser_find_active_match(
    matches: &[BrowserFindMatch],
    active_line: Option<usize>,
) -> Option<(usize, BrowserFindMatch)> {
    let active_line = active_line?;
    matches
        .iter()
        .enumerate()
        .find(|(_, match_)| match_.line == active_line)
        .map(|(index, match_)| (index, *match_))
}

fn clear_browser_find_active_line(web_session: &mut BrowserWebSession) {
    web_session.find_active_line = None;
}

async fn apply_browser_open_with_pending_shell(
    web_session: &mut BrowserWebSession,
    target_url: &str,
) -> Result<(), BrowserRouteError> {
    let navigation_target = browser_session_navigation_target(target_url, web_session.max_bytes)?;
    match timeout(
        BROWSER_CREATE_TARGET_TIMEOUT,
        web_session.session.navigate(&navigation_target.target),
    )
    .await
    {
        Ok(Ok(_)) => {
            reset_viewport_after_navigation(web_session);
            clear_browser_find_active_line(web_session);
            web_session.pending_source = None;
            web_session.display_source = navigation_target.display_source;
            web_session.action_feedback = Some(format!(
                "Opened {}.",
                browser_session_feedback_excerpt(target_url)
            ));
        }
        Ok(Err(error)) => {
            set_browser_pending_open_feedback(
                web_session,
                target_url,
                format!(
                    "Still opening {}; renderer reported: {}",
                    browser_session_feedback_excerpt(target_url),
                    browser_session_feedback_excerpt(&error.to_string())
                ),
            )
            .await?;
        }
        Err(_) => {
            set_browser_pending_open_feedback(
                web_session,
                target_url,
                format!(
                    "Still opening {}; retry stayed in this tab after {}ms.",
                    browser_session_feedback_excerpt(target_url),
                    BROWSER_CREATE_TARGET_TIMEOUT.as_millis()
                ),
            )
            .await?;
        }
    }
    Ok(())
}

async fn set_browser_pending_open_feedback(
    web_session: &mut BrowserWebSession,
    target_url: &str,
    feedback: String,
) -> Result<(), BrowserRouteError> {
    let already_showing_pending_shell = current_session_source(web_session)
        .as_deref()
        .is_some_and(|source| source == BROWSER_ABOUT_BLANK_TARGET)
        && web_session.pending_source.as_deref() == Some(target_url);
    if !already_showing_pending_shell {
        web_session
            .session
            .navigate(BROWSER_ABOUT_BLANK_TARGET)
            .await
            .map_err(|blank_error| {
                BrowserRouteError::Upstream(format!(
                    "browser fallback shell failed after {target_url} stayed pending: {blank_error:#}"
                ))
            })?;
    }
    web_session.pending_source = Some(target_url.to_owned());
    web_session.display_source = None;
    web_session.resource_report = None;
    web_session.action_feedback = Some(feedback);
    clear_browser_find_active_line(web_session);
    Ok(())
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

fn browser_session_resource_report_from_make_visual(
    stylesheets: BrowserSessionResourceReportPayload,
    images: BrowserSessionResourceReportPayload,
) -> BrowserSessionResourceReportPayload {
    let applied = stylesheets.applied.unwrap_or(0);
    let decoded = images.decoded.unwrap_or(0);
    let mut resources = stylesheets.resources;
    resources.extend(images.resources);
    BrowserSessionResourceReportPayload {
        action: "Make visual".to_owned(),
        page_source: images.page_source,
        total: stylesheets.total + images.total,
        fetched: stylesheets.fetched + images.fetched,
        cached: stylesheets.cached + images.cached,
        failed: stylesheets.failed + images.failed,
        skipped: stylesheets.skipped + images.skipped,
        applied: Some(applied),
        decoded: Some(decoded),
        resources,
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
    browser_session_payload_with_options(id, web_session, BrowserSessionPayloadOptions::default())
}

fn browser_session_payload_with_options(
    id: &str,
    web_session: &mut BrowserWebSession,
    options: BrowserSessionPayloadOptions,
) -> Result<BrowserSessionPayload, BrowserRouteError> {
    if web_session.session.current().is_none() {
        return Err(BrowserRouteError::BadRequest(
            "browser session has no current page".to_owned(),
        ));
    }
    let pending_viewport = web_session
        .pending_source
        .as_ref()
        .map(|_| (web_session.viewport_x, web_session.viewport_y));
    normalize_browser_session_viewport(web_session);
    if let Some((viewport_x, viewport_y)) = pending_viewport {
        web_session.viewport_x = viewport_x;
        web_session.viewport_y = viewport_y;
    }

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
        let (viewport_image, viewport_image_error) = if options.render_viewport_image {
            match browser_session_viewport_image(
                render,
                viewport.x,
                viewport.y,
                viewport.width,
                viewport.height,
            ) {
                Ok(image) => (Some(image), None),
                Err(error) => (None, Some(error)),
            }
        } else {
            (None, None)
        };
        let history = web_session.session.snapshot();
        let find_matches = browser_find_matches(&render.text, &web_session.find_query);
        let find_current = browser_find_active_match(&find_matches, web_session.find_active_line)
            .or_else(|| browser_find_visible_match(&find_matches, viewport.y, viewport.height));
        let find_current_index = find_current.map(|(index, _)| index);
        let find_match_payloads = browser_session_find_match_payloads(
            id,
            web_session,
            &render.text,
            &find_matches,
            find_current_index,
        );
        let can_back = history.current_index.is_some_and(|index| index > 0);
        let can_forward = history
            .current_index
            .is_some_and(|index| index + 1 < history.entries.len());
        let display_source = web_session.display_source.as_deref();
        let current_render_source = render.source.as_str();
        let history_entries = history
            .entries
            .iter()
            .enumerate()
            .map(|(index, entry)| {
                let source = if history.current_index == Some(index)
                    && entry.source == current_render_source
                    && let Some(display_source) = display_source
                {
                    display_source
                } else {
                    entry.source.as_str()
                };
                BrowserSessionHistoryEntryPayload {
                    index,
                    title: if entry.title.trim().is_empty() {
                        browser_session_feedback_excerpt(source)
                    } else {
                        entry.title.clone()
                    },
                    source: source.to_owned(),
                    target: if history.current_index == Some(index)
                        && entry.target == current_render_source
                        && display_source.is_some()
                    {
                        source.to_owned()
                    } else {
                        entry.target.clone()
                    },
                    action_url: browser_session_action_href(
                        id,
                        "history",
                        &[("history", index.to_string())],
                        web_session,
                    ),
                    new_session_url: browser_session_new_session_href(source, web_session),
                    background_session_url: browser_session_action_href(
                        id,
                        "open-background-session",
                        &[("url", source.to_owned())],
                        web_session,
                    ),
                    current: history.current_index == Some(index),
                }
            })
            .collect::<Vec<_>>();
        let anchors = render
            .fragment_targets
            .iter()
            .take(120)
            .enumerate()
            .map(|(index, target)| BrowserSessionAnchorPayload {
                index,
                name: target.name.clone(),
                y: target.y,
                action_url: browser_session_action_href(
                    id,
                    "anchor",
                    &[("anchor", (index + 1).to_string())],
                    web_session,
                ),
                new_session_url: browser_session_action_href(
                    id,
                    "anchor-new-session",
                    &[("anchor", (index + 1).to_string())],
                    web_session,
                ),
                background_session_url: browser_session_action_href(
                    id,
                    "anchor-background-session",
                    &[("anchor", (index + 1).to_string())],
                    web_session,
                ),
            })
            .collect();
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
                    background_session_url: browser_session_action_href(
                        id,
                        "link-background-session",
                        &[("link", index.to_string())],
                        web_session,
                    ),
                }
            })
            .collect();
        let links_background_url = (!render.links.is_empty()).then(|| {
            browser_session_action_href(
                id,
                "open-links-background-sessions",
                &[("limit", DEFAULT_BULK_BACKGROUND_LINKS.to_string())],
                web_session,
            )
        });
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
                                    select_url: (!control.disabled && !option.disabled).then(
                                        || {
                                            browser_session_action_href(
                                                id,
                                                "select",
                                                &[
                                                    ("form", form.index.to_string()),
                                                    ("control", control_index.to_string()),
                                                    ("value", option.value.clone()),
                                                ],
                                                web_session,
                                            )
                                        },
                                    ),
                                })
                                .collect(),
                            fill_url: (!control.disabled
                                && !control.name.is_empty()
                                && browser_form_control_name_is_unique(form, &control.name)
                                && form_control_is_text_editable(&control.kind))
                            .then(|| {
                                browser_session_action_href(
                                    id,
                                    "fill-control",
                                    &[
                                        ("form", form.index.to_string()),
                                        ("control", control_index.to_string()),
                                    ],
                                    web_session,
                                )
                            }),
                            type_url: (!control.disabled
                                && !control.name.is_empty()
                                && browser_form_control_name_is_unique(form, &control.name)
                                && form_control_is_text_editable(&control.kind))
                            .then(|| {
                                browser_session_action_href(
                                    id,
                                    "type-control",
                                    &[
                                        ("form", form.index.to_string()),
                                        ("control", control_index.to_string()),
                                    ],
                                    web_session,
                                )
                            }),
                            clear_url: (!control.disabled
                                && !control.name.is_empty()
                                && browser_form_control_name_is_unique(form, &control.name)
                                && form_control_is_text_editable(&control.kind))
                            .then(|| {
                                browser_session_action_href(
                                    id,
                                    "clear-control",
                                    &[
                                        ("form", form.index.to_string()),
                                        ("control", control_index.to_string()),
                                    ],
                                    web_session,
                                )
                            }),
                            focus_url: (!control.disabled
                                && form_control_is_focusable(
                                    &control.kind,
                                    !control.options.is_empty(),
                                    !control.name.is_empty(),
                                ))
                            .then(|| {
                                browser_session_action_href(
                                    id,
                                    "focus-control",
                                    &[
                                        ("form", form.index.to_string()),
                                        ("control", control_index.to_string()),
                                    ],
                                    web_session,
                                )
                            }),
                            activate_url: (!control.disabled
                                && form_control_is_activatable(&control.kind))
                            .then(|| {
                                browser_session_action_href(
                                    id,
                                    "activate-control",
                                    &[
                                        ("form", form.index.to_string()),
                                        ("control", control_index.to_string()),
                                    ],
                                    web_session,
                                )
                            }),
                            activate_new_session_url: (!control.disabled
                                && form_control_is_submit(&control.kind))
                            .then(|| {
                                browser_session_action_href(
                                    id,
                                    "activate-control-new-session",
                                    &[
                                        ("form", form.index.to_string()),
                                        ("control", control_index.to_string()),
                                    ],
                                    web_session,
                                )
                            }),
                            activate_background_session_url: (!control.disabled
                                && form_control_is_submit(&control.kind))
                            .then(|| {
                                browser_session_action_href(
                                    id,
                                    "activate-control-background-session",
                                    &[
                                        ("form", form.index.to_string()),
                                        ("control", control_index.to_string()),
                                    ],
                                    web_session,
                                )
                            }),
                            toggle_url: if !control.disabled
                                && form_control_is_checkable(&control.kind)
                            {
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
                submit_new_session_url: browser_session_action_href(
                    id,
                    "submit-new-session",
                    &[("form", form.index.to_string())],
                    web_session,
                ),
                submit_background_session_url: browser_session_action_href(
                    id,
                    "submit-background-session",
                    &[("form", form.index.to_string())],
                    web_session,
                ),
            })
            .collect::<Vec<_>>();
        let resource_kind_counts = browser_session_resource_kind_counts(&render.resources);
        let resources = browser_session_visible_resources(&render.resources)
            .into_iter()
            .map(|(index, resource)| {
                let resolved = resource.resolved.clone();
                BrowserSessionResourcePayload {
                    index,
                    kind: resource.kind.clone(),
                    initiator: resource.initiator.clone(),
                    url: resource.url.clone(),
                    resolved: resolved.clone(),
                    rel: resource.rel.clone(),
                    media: resource.media.clone(),
                    alt: resource.alt.clone(),
                    type_hint: resource.type_hint.clone(),
                    details: browser_resource_detail(
                        resource.rel.as_deref(),
                        resource.media.as_deref(),
                        resource.alt.as_deref(),
                        resource.type_hint.as_deref(),
                    ),
                    open_url: browser_session_action_href(
                        id,
                        "resource",
                        &[("resource", index.to_string())],
                        web_session,
                    ),
                    new_session_url: browser_session_action_href(
                        id,
                        "resource-new-session",
                        &[("resource", index.to_string())],
                        web_session,
                    ),
                    background_session_url: browser_session_action_href(
                        id,
                        "resource-background-session",
                        &[("resource", index.to_string())],
                        web_session,
                    ),
                }
            })
            .collect::<Vec<_>>();

        let mut payload = BrowserSessionPayload {
            id: id.to_owned(),
            back_href: web_session.back_href.clone(),
            title: browser_session_title(render),
            source: render.source.clone(),
            rendered_source: render.source.clone(),
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
            anchor_count: render.fragment_targets.len(),
            can_back,
            can_forward,
            history_len: history.entries.len(),
            current_history_index: history.current_index,
            profile_enabled: false,
            profile_error: None,
            current_bookmarked: false,
            bookmarks_clear_url: None,
            bookmarks_background_url: None,
            links_background_url,
            closed_sessions_clear_url: None,
            profile_tabs_clear_url: None,
            profile_history_clear_url: None,
            find_query: web_session.find_query.clone(),
            find_match_count: find_matches.len(),
            find_current_index,
            find_current_line: find_current.map(|(_, match_)| match_.line),
            find_current_column: find_current.map(|(_, match_)| match_.column),
            find_matches: find_match_payloads,
            tab_search_query: web_session.tab_search_query.clone(),
            tab_search_results: Vec::new(),
            sessions: Vec::new(),
            closed_sessions: Vec::new(),
            bookmarks: Vec::new(),
            profile_history: Vec::new(),
            history: history_entries,
            viewport: viewport.lines.join("\n"),
            viewport_image,
            viewport_image_error,
            page_text: render.text.clone(),
            focused: web_session.session.focused_control(),
            anchors,
            links,
            form_count: render.forms.len(),
            forms,
            cookies: web_session.session.cookies_snapshot(),
            local_storage: web_session.session.local_storage_entries(),
            session_storage: web_session.session.session_storage_entries(),
            resource_count: render.resources.len(),
            resource_image_count: resource_kind_counts.images,
            resource_stylesheet_count: resource_kind_counts.stylesheets,
            resource_script_count: resource_kind_counts.scripts,
            resources,
            resource_report: web_session.resource_report.clone(),
            action_feedback: web_session.action_feedback.clone(),
            pending_source: web_session.pending_source.clone(),
            fast_scroll: options.fast_scroll,
        };
        if let Some(pending_source) = web_session.pending_source.as_ref() {
            payload.title = format!(
                "Loading {}",
                browser_session_feedback_excerpt(pending_source)
            );
            payload.source = pending_source.clone();
            payload.viewport_x = web_session.viewport_x;
            payload.viewport_y = web_session.viewport_y;
        } else if let Some(display_source) = web_session.display_source.as_ref() {
            payload.title = browser_session_display_title(render, Some(display_source));
            payload.source = display_source.clone();
        }
        payload
    };
    web_session.viewport_x = payload.viewport_x;
    web_session.viewport_y = payload.viewport_y;
    Ok(payload)
}

#[derive(Debug, Clone, Copy, Default)]
struct BrowserSessionResourceKindCounts {
    images: usize,
    stylesheets: usize,
    scripts: usize,
}

fn browser_session_resource_kind_counts(
    resources: &[BrowserResource],
) -> BrowserSessionResourceKindCounts {
    let mut counts = BrowserSessionResourceKindCounts::default();
    for resource in resources {
        match resource.kind.as_str() {
            "image" => counts.images += 1,
            "stylesheet" => counts.stylesheets += 1,
            "script" => counts.scripts += 1,
            _ => {}
        }
    }
    counts
}

fn browser_session_visible_resources(
    resources: &[BrowserResource],
) -> Vec<(usize, &BrowserResource)> {
    let mut visible = resources.iter().enumerate().collect::<Vec<_>>();
    visible.sort_by_key(|(index, resource)| (browser_session_resource_priority(resource), *index));
    visible.truncate(MAX_BROWSER_SESSION_RESOURCES);
    visible
}

fn browser_session_resource_priority(resource: &BrowserResource) -> u8 {
    match resource.kind.as_str() {
        "image" => 0,
        "stylesheet" => 1,
        "script" => 2,
        _ => 3,
    }
}

fn browser_session_viewport_image(
    render: &BrowserRender,
    x: usize,
    y: usize,
    width: usize,
    height: usize,
) -> Result<BrowserSessionViewportImagePayload, String> {
    let raster = rasterize_render_rgba(
        render,
        BrowserRasterOptions {
            viewport_x: Some(x),
            viewport_y: Some(y),
            viewport_width: Some(width.max(1)),
            viewport_height: Some(height.max(1)),
            ..BrowserRasterOptions::default()
        },
    )
    .map_err(|error| error.to_string())?;
    let png = raster.encode_png().map_err(|error| error.to_string())?;
    Ok(BrowserSessionViewportImagePayload {
        data_url: format!("data:image/png;base64,{}", browser_base64_encode(&png)),
        width: raster.width,
        height: raster.height,
    })
}

fn browser_base64_encode(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut encoded = String::with_capacity(bytes.len().saturating_add(2) / 3 * 4);
    for chunk in bytes.chunks(3) {
        let first = chunk[0];
        let second = chunk.get(1).copied().unwrap_or(0);
        let third = chunk.get(2).copied().unwrap_or(0);

        encoded.push(TABLE[(first >> 2) as usize] as char);
        encoded.push(TABLE[(((first & 0b0000_0011) << 4) | (second >> 4)) as usize] as char);
        if chunk.len() > 1 {
            encoded.push(TABLE[(((second & 0b0000_1111) << 2) | (third >> 6)) as usize] as char);
        } else {
            encoded.push('=');
        }
        if chunk.len() > 2 {
            encoded.push(TABLE[(third & 0b0011_1111) as usize] as char);
        } else {
            encoded.push('=');
        }
    }
    encoded
}

fn render_browser_session_page(payload: &BrowserSessionPayload, back_href: &str) -> String {
    render_browser_session_page_with_diagnostics(payload, back_href, true)
}

fn render_browser_session_page_with_diagnostics(
    payload: &BrowserSessionPayload,
    back_href: &str,
    show_diagnostics: bool,
) -> String {
    let mut link_rows = String::new();
    if show_diagnostics {
        for link in &payload.links {
            let _ = write!(
                link_rows,
                r#"<li><span>{index}</span><div class="link-body"><a href="{href}">{label}</a><div class="link-target">{url}</div><div class="link-actions"><a href="{href}">Open</a><a href="{new_href}">New session</a><a href="{background_href}">Background</a></div></div></li>"#,
                index = link.index + 1,
                href = html_escape::encode_double_quoted_attribute(&link.action_url),
                new_href = html_escape::encode_double_quoted_attribute(&link.new_session_url),
                background_href =
                    html_escape::encode_double_quoted_attribute(&link.background_session_url),
                label = html_escape::encode_text(&link.label),
                url = html_escape::encode_text(&link.url),
            );
        }
    }
    if show_diagnostics && payload.link_count > payload.links.len() {
        let _ = write!(
            link_rows,
            r#"<li><span></span><div>{count} more links omitted</div></li>"#,
            count = payload.link_count - payload.links.len(),
        );
    }
    if show_diagnostics && link_rows.is_empty() {
        link_rows
            .push_str(r#"<li><span></span><div>No links found in this session page.</div></li>"#);
    }
    let link_controls = if show_diagnostics {
        render_browser_session_link_controls(payload)
    } else {
        String::new()
    };
    let form_rows = if show_diagnostics {
        render_browser_session_forms(payload)
    } else {
        String::new()
    };
    let click_controls = if show_diagnostics {
        render_browser_session_click_controls(payload)
    } else {
        String::new()
    };
    let keyboard_controls = if show_diagnostics {
        render_browser_session_keyboard_controls(payload)
    } else {
        String::new()
    };
    let inspector = if show_diagnostics {
        render_browser_session_inspector(payload)
    } else {
        String::new()
    };
    let session_tabs = if show_diagnostics {
        render_browser_session_tabs(payload)
    } else {
        String::new()
    };
    let primary_tab_strip = render_browser_session_primary_tab_strip(payload);
    let closed_sessions = if show_diagnostics {
        render_browser_session_closed_sessions(payload)
    } else {
        String::new()
    };
    let bookmarks = if show_diagnostics {
        render_browser_session_bookmarks(payload)
    } else {
        String::new()
    };
    let profile_history = if show_diagnostics {
        render_browser_session_profile_history(payload)
    } else {
        String::new()
    };
    let find_controls = render_browser_session_find_controls(payload);
    let viewport = render_browser_session_viewport(payload);
    let viewport_image = render_browser_session_viewport_image(payload);
    let primary_input_controls = render_browser_session_primary_input_controls(payload);
    let viewport_status = render_browser_session_viewport_status(payload);
    let viewport_scroll_controls = render_browser_session_viewport_scroll_controls(payload);
    let primary_page_state = render_browser_session_primary_page_state(payload);
    let pending_primary_page_state = browser_session_pending_without_ready_viewport(payload)
        .then_some(primary_page_state.as_str());
    let settled_primary_page_state = (!browser_session_pending_without_ready_viewport(payload))
        .then_some(primary_page_state.as_str());
    let auto_visual_bootstrap = render_browser_session_auto_visual_bootstrap(payload);
    let pending_load_retry = render_browser_session_pending_load_retry_script(payload);
    let viewport_command_strip = render_browser_session_viewport_command_strip(payload);
    let viewport_text = if show_diagnostics {
        render_browser_session_viewport_text(payload, &viewport)
    } else {
        String::new()
    };
    let navigation_state = render_browser_session_navigation_state(payload, back_href);
    let page_summary = format!(
        r#"<details class="browser-page-summary" data-browser-page-details><summary>Page details</summary><div class="browser-page-summary-content"><div class="meta">rust browser session {id} · history {history_index}/{history_len} · viewport {width}x{height} at x={viewport_x} y={viewport_y} · max scroll {max_scroll_x}x{max_scroll_y} · document {doc_width}x{doc_height} · {nodes} DOM nodes · {links} links · {anchors} anchors · {forms} forms</div>{navigation_state}</div></details>"#,
        id = html_escape::encode_text(&payload.id),
        history_index = payload.current_history_index.map_or(0, |index| index + 1),
        history_len = payload.history_len,
        width = payload.width,
        height = payload.height,
        viewport_x = payload.viewport_x,
        viewport_y = payload.viewport_y,
        max_scroll_x = payload.max_scroll_x,
        max_scroll_y = payload.max_scroll_y,
        doc_width = payload.document_width,
        doc_height = payload.document_height,
        nodes = payload.dom_node_count,
        links = payload.link_count,
        anchors = payload.anchor_count,
        forms = payload.form_count,
        navigation_state = navigation_state,
    );
    let resource_quick_actions = render_browser_session_resource_quick_actions(payload);
    let viewport_interaction_controls =
        render_browser_session_viewport_interaction_controls(payload);
    let forms_json_href = browser_session_api_href(&payload.id, "forms-json", payload);
    let forms_csv_href = browser_session_api_href(&payload.id, "forms-csv", payload);
    let links_csv_href = browser_session_api_href(&payload.id, "links-csv", payload);
    let links_background_control = payload
        .links_background_url
        .as_ref()
        .map_or_else(String::new, |href| {
            nav_control(!payload.links.is_empty(), "Open links bg", href)
        });
    let links_new_sessions_href = browser_session_action_href(
        &payload.id,
        "open-links-new-sessions",
        &[("limit", DEFAULT_BULK_BACKGROUND_LINKS.to_string())],
        payload,
    );
    let links_new_sessions_control = nav_control(
        !payload.links.is_empty(),
        "Open links tabs",
        &links_new_sessions_href,
    );
    let bookmark_links_href =
        browser_session_action_href(&payload.id, "bookmark-page-links", &[], payload);
    let bookmark_links_control = nav_control(
        browser_has_unbookmarked_page_links(payload),
        "Bookmark links",
        &bookmark_links_href,
    );
    let remove_link_bookmarks_href =
        browser_session_action_href(&payload.id, "remove-page-link-bookmarks", &[], payload);
    let remove_link_bookmarks_control = nav_control(
        browser_has_bookmarked_page_links(payload),
        "Remove link bookmarks",
        &remove_link_bookmarks_href,
    );

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
    let keyboard_controls_script = render_browser_session_keyboard_controls_script(&reload_href);
    let duplicate_href = browser_session_action_href(
        &payload.id,
        "duplicate-session",
        &[("session", payload.id.clone())],
        payload,
    );
    let current_session_summary = payload
        .sessions
        .iter()
        .find(|session| session.id == payload.id);
    let current_session_pinned = current_session_summary.is_some_and(|session| session.pinned);
    let pin_current_action = if current_session_pinned {
        "unpin-tab"
    } else {
        "pin-tab"
    };
    let pin_current_label = if current_session_pinned {
        "Unpin current"
    } else {
        "Pin current"
    };
    let pin_current_href = browser_session_action_href(
        &payload.id,
        pin_current_action,
        &[("session", payload.id.clone())],
        payload,
    );
    let pin_all_href = browser_session_action_href(&payload.id, "pin-all-tabs", &[], payload);
    let pin_all_control = nav_control(
        payload.sessions.iter().any(|session| !session.pinned),
        "Pin all",
        &pin_all_href,
    );
    let unpin_all_href = browser_session_action_href(&payload.id, "unpin-all-tabs", &[], payload);
    let unpin_all_control = nav_control(
        payload.sessions.iter().any(|session| session.pinned),
        "Unpin all",
        &unpin_all_href,
    );
    let move_left_href = current_session_summary
        .map(|session| session.move_left_url.as_str())
        .unwrap_or("");
    let move_left_control = nav_control(
        current_session_summary
            .map(|session| session.can_move_left)
            .unwrap_or(false),
        "Move left",
        move_left_href,
    );
    let move_right_href = current_session_summary
        .map(|session| session.move_right_url.as_str())
        .unwrap_or("");
    let move_right_control = nav_control(
        current_session_summary
            .map(|session| session.can_move_right)
            .unwrap_or(false),
        "Move right",
        move_right_href,
    );
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
    let has_unpinned_sessions_to_close = payload
        .sessions
        .iter()
        .any(|session| !session.current && !session.pinned);
    let close_unpinned_href =
        browser_session_action_href(&payload.id, "close-unpinned-tabs", &[], payload);
    let close_unpinned_control = nav_control(
        has_unpinned_sessions_to_close,
        "Close unpinned",
        &close_unpinned_href,
    );
    let has_left_sessions = current_session_summary
        .map(|session| session.can_move_left)
        .unwrap_or(false);
    let has_right_sessions = current_session_summary
        .map(|session| session.can_move_right)
        .unwrap_or(false);
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
    let diagnostics_href = browser_session_action_href(
        &payload.id,
        "current",
        &[("debug", "1".to_owned())],
        payload,
    );
    let diagnostics_section = if show_diagnostics {
        format!(
            r#"<details class="debug-stack browser-tools-menu" data-browser-tools-tray>
<summary>Diagnostics</summary>
<div class="debug-stack-content">
<details class="debug-section"><summary>Tabs and saved state</summary><div class="debug-section-content"><div class="toolbar secondary-toolbar">{move_left_control}{move_right_control}<a href="{duplicate_href}">Duplicate current</a><a href="{pin_current_href}">{pin_current_label}</a>{pin_all_control}{unpin_all_control}{close_current_control}{close_others_control}{close_unpinned_control}{close_left_control}{close_right_control}{close_duplicates_control}{restore_tab_control}</div>{session_tabs}{closed_sessions}{bookmarks}{profile_history}</div></details>
<details class="debug-section"><summary>Input tools and forms</summary><div class="debug-section-content"><h2>Click</h2><div class="browser-actions">{click_controls}</div><h2>Keyboard</h2><div class="keyboard-actions">{keyboard_controls}</div><div class="session-title"><h2>Forms</h2><div class="resource-actions"><span class="meta">{forms} found</span><a class="clear-link" href="{forms_json_href}">Forms JSON</a><a class="clear-link" href="{forms_csv_href}">Forms CSV</a></div></div><div class="browser-forms">{form_rows}</div></div></details>
<details class="debug-section"><summary>Inspector and resources</summary><div class="debug-section-content"><h2>Inspector</h2><div class="browser-inspector">{inspector}</div></div></details>
<details class="debug-section"><summary>Links</summary><div class="debug-section-content"><div class="session-title"><h2>Links</h2><div class="resource-actions"><span class="meta">{links} found</span><a class="clear-link" href="{links_csv_href}">Links CSV</a>{links_new_sessions_control}{links_background_control}{bookmark_links_control}{remove_link_bookmarks_control}</div></div><div class="browser-actions">{link_controls}</div><ol>{link_rows}</ol></div></details>
</div>
</details>"#,
            move_left_control = move_left_control,
            move_right_control = move_right_control,
            duplicate_href = html_escape::encode_double_quoted_attribute(&duplicate_href),
            pin_current_href = html_escape::encode_double_quoted_attribute(&pin_current_href),
            pin_current_label = pin_current_label,
            pin_all_control = pin_all_control,
            unpin_all_control = unpin_all_control,
            close_current_control = close_current_control,
            close_others_control = close_others_control,
            close_unpinned_control = close_unpinned_control,
            close_left_control = close_left_control,
            close_right_control = close_right_control,
            close_duplicates_control = close_duplicates_control,
            restore_tab_control = restore_tab_control,
            session_tabs = session_tabs,
            closed_sessions = closed_sessions,
            bookmarks = bookmarks,
            profile_history = profile_history,
            click_controls = click_controls,
            keyboard_controls = keyboard_controls,
            forms = payload.form_count,
            forms_json_href = html_escape::encode_double_quoted_attribute(&forms_json_href),
            forms_csv_href = html_escape::encode_double_quoted_attribute(&forms_csv_href),
            form_rows = form_rows,
            inspector = inspector,
            links = payload.link_count,
            links_csv_href = html_escape::encode_double_quoted_attribute(&links_csv_href),
            links_new_sessions_control = links_new_sessions_control,
            links_background_control = links_background_control,
            bookmark_links_control = bookmark_links_control,
            remove_link_bookmarks_control = remove_link_bookmarks_control,
            link_controls = link_controls,
            link_rows = link_rows,
        )
    } else {
        format!(
            r#"<div class="debug-stack browser-tools-menu browser-diagnostics-compact" data-browser-tools-tray><span class="meta">Diagnostics are available when needed.</span><a class="clear-link" href="{diagnostics_href}">Open diagnostics</a></div>"#,
            diagnostics_href = html_escape::encode_double_quoted_attribute(&diagnostics_href),
        )
    };
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
main {{ max-width: none; margin: 0; padding: 18px 18px 56px; }}
a {{ color: #123fae; text-decoration: none; font-weight: 700; overflow-wrap: anywhere; }}
a:hover {{ text-decoration: underline; }}
h1 {{ margin: 14px 0 6px; font-size: 24px; letter-spacing: 0; }}
h2 {{ margin: 24px 0 10px; font-size: 16px; letter-spacing: 0; }}
.browser-topbar {{ position: sticky; top: 0; z-index: 20; display: grid; gap: 3px; margin: -18px -18px 8px; padding: 4px 14px; background: rgba(247, 247, 245, 0.97); border-bottom: 1px solid #dfe2e6; backdrop-filter: blur(8px); }}
.browser-chrome-row {{ display: grid; grid-template-columns: auto minmax(0, 1fr) auto; gap: 5px; align-items: center; }}
.browser-primary-nav {{ margin-bottom: 0; flex-wrap: nowrap; }}
.browser-primary-nav a, .browser-primary-nav span {{ min-width: 28px; min-height: 26px; justify-content: center; padding: 0 6px; font-size: 11px; white-space: nowrap; }}
.browser-chrome-status {{ display: flex; flex-wrap: nowrap; justify-content: flex-end; gap: 4px; align-items: center; min-width: 0; color: #5d636b; font-size: 11px; font-weight: 800; overflow: hidden; white-space: nowrap; }}
.browser-chrome-status .viewport-state-chip {{ min-height: 20px; max-width: 96px; padding: 0 5px; font-size: 10px; overflow: hidden; text-overflow: ellipsis; }}
.browser-chrome-status a {{ min-height: 22px; display: inline-flex; align-items: center; border: 1px solid #c6cbd2; border-radius: 6px; padding: 0 7px; background: #fff; color: #20242a; font-size: 11px; font-weight: 800; white-space: nowrap; }}
.browser-chrome-status a.primary-action {{ background: #2457d6; border-color: #2457d6; color: #fff; }}
.browser-chrome-status[data-resource-pending="true"] a[href^="/browser"], .browser-chrome-status[data-visual-pending="true"] .primary-action {{ cursor: wait; opacity: 0.72; }}
.browser-tab-strip {{ margin: 0; }}
.browser-tab-strip > summary {{ min-height: 28px; display: inline-flex; max-width: 100%; align-items: center; gap: 8px; cursor: pointer; border: 1px solid #dfe2e6; border-radius: 6px; padding: 0 9px; background: #fff; color: #20242a; font-size: 12px; font-weight: 800; }}
.browser-tab-strip > summary span {{ min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }}
.browser-tab-strip > summary strong {{ white-space: nowrap; }}
.browser-tab-list {{ display: flex; gap: 6px; overflow-x: auto; margin: 6px 0 2px; padding: 2px 0 4px; scrollbar-gutter: stable; }}
.browser-tab-pill {{ flex: 0 0 clamp(150px, 22vw, 230px); min-width: 0; display: grid; gap: 2px; border: 1px solid #c6cbd2; border-radius: 6px; padding: 7px 9px; background: #fff; color: #20242a; text-decoration: none; }}
.browser-tab-pill:hover {{ text-decoration: none; border-color: #8f98a3; }}
.browser-tab-pill.current {{ background: #191a1c; color: #fff; border-color: #191a1c; }}
.browser-tab-pill.pinned {{ border-left: 4px solid #2457d6; padding-left: 6px; }}
.browser-tab-pill strong, .browser-tab-pill span {{ min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }}
.browser-tab-pill strong {{ font-size: 13px; font-weight: 800; }}
.browser-tab-pill span {{ color: inherit; opacity: 0.72; font-size: 11px; font-weight: 600; }}
.browser-page-head {{ margin: 6px 0 8px; }}
.browser-page-title {{ display: flex; flex-wrap: wrap; gap: 8px; align-items: baseline; margin: 4px 0 6px; }}
.browser-page-title h1 {{ margin: 0; font-size: 15px; font-weight: 900; }}
.browser-page-title .meta {{ min-width: 0; max-width: 100%; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }}
.browser-page-summary {{ margin: 4px 0 8px; color: #3a3f45; }}
.browser-page-summary > summary {{ cursor: pointer; color: #5d636b; font-size: 12px; font-weight: 800; }}
.browser-page-summary-content {{ display: grid; gap: 6px; padding-top: 8px; }}
.browser-navigation-state {{ display: flex; flex-wrap: wrap; gap: 8px; align-items: center; margin: 10px 0 4px; }}
.browser-navigation-state a, .browser-navigation-state span {{ min-height: 28px; display: inline-flex; align-items: center; border: 1px solid #dfe2e6; border-radius: 6px; padding: 0 8px; background: #fff; color: #3a3f45; font-size: 12px; font-weight: 800; overflow-wrap: anywhere; }}
.browser-navigation-state a {{ color: #123fae; border-color: #c6cbd2; }}
.toolbar {{ display: flex; align-items: center; flex-wrap: nowrap; gap: 5px; margin-bottom: 8px; min-width: 0; }}
.toolbar a, .toolbar span, .toolbar button {{ min-height: 26px; display: inline-flex; align-items: center; border: 1px solid #c6cbd2; border-radius: 6px; padding: 0 7px; background: #fff; color: #20242a; font-size: 11px; font-weight: 700; white-space: nowrap; }}
.toolbar span {{ color: #8a929d; background: #eef0f3; }}
.toolbar form {{ display: flex; flex: 1 1 auto; min-width: 0; gap: 5px; }}
.toolbar input[name="url"] {{ flex: 1; min-width: 0; height: 26px; border: 1px solid #b7bdc5; border-radius: 6px; padding: 0 8px; font-size: 12px; background: #fff; }}
.toolbar button {{ cursor: pointer; background: #2457d6; color: #fff; border-color: #2457d6; }}
[data-browser-auto-visual-control][aria-busy="true"] a[href^="/browser"], [data-browser-auto-visual-control][aria-busy="true"] button {{ cursor: wait; opacity: 0.62; }}
.address-bar {{ margin-bottom: 0; flex-wrap: nowrap; }}
.address-bar input[name="url"] {{ flex: 1 1 auto; }}
.address-bar button.browser-background-tab {{ display: none; }}
.secondary-toolbar {{ margin: 0 0 12px; }}
.viewport-jump {{ display: flex; flex-wrap: wrap; gap: 8px; align-items: center; margin: 8px 0 12px; }}
.viewport-jump label {{ color: #3a3f45; font-size: 13px; font-weight: 700; }}
.viewport-jump input[type="number"] {{ width: 96px; height: 32px; border: 1px solid #b7bdc5; border-radius: 6px; padding: 0 9px; font-size: 13px; background: #fff; }}
.viewport-jump button {{ min-height: 32px; border: 1px solid #2457d6; border-radius: 6px; padding: 0 10px; background: #2457d6; color: #fff; font-size: 13px; font-weight: 700; cursor: pointer; }}
.viewport-jump-range {{ color: #5d636b; font-size: 12px; font-weight: 700; }}
.viewport-status {{ display: grid; gap: 5px; margin: 4px 0 8px; }}
.viewport-status-text {{ display: flex; flex-wrap: wrap; gap: 6px; align-items: center; color: #3a3f45; font-size: 12px; font-weight: 700; }}
.viewport-status-text span {{ min-height: 22px; display: inline-flex; align-items: center; border: 1px solid #dfe2e6; border-radius: 6px; padding: 0 6px; background: #fff; }}
.viewport-scroll-feedback {{ max-width: 100%; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }}
.viewport-scroll-meter {{ height: 6px; border-radius: 999px; background: #dfe2e6; overflow: hidden; }}
.viewport-scroll-meter span {{ display: block; height: 100%; background: #2457d6; }}
.browser-surface-state {{ display: flex; flex-wrap: wrap; gap: 8px; align-items: center; margin: 6px 0 8px; }}
.browser-surface-state.compact {{ gap: 10px; color: #5d636b; font-size: 12px; font-weight: 800; }}
.browser-surface-state.compact [data-browser-primary-raster] {{ color: #20242a; }}
.browser-surface-state .primary-action {{ min-height: 28px; display: inline-flex; align-items: center; border: 1px solid #2457d6; border-radius: 6px; padding: 0 9px; background: #2457d6; color: #fff; font-size: 12px; font-weight: 800; white-space: nowrap; }}
.viewport-scroll-controls {{ display: flex; flex-wrap: wrap; gap: 8px; align-items: center; margin: 8px 0 10px; }}
.viewport-scroll-controls a, .viewport-scroll-controls span {{ min-height: 32px; display: inline-flex; align-items: center; border: 1px solid #c6cbd2; border-radius: 6px; padding: 0 10px; background: #fff; color: #20242a; font-size: 13px; font-weight: 700; }}
.viewport-scroll-controls span {{ color: #8a929d; background: #eef0f3; }}
.viewport-scroll-controls[data-scroll-pending="true"] a {{ cursor: wait; opacity: 0.72; }}
.viewport-scroll-feedback {{ color: #5d636b; border-color: transparent !important; background: transparent !important; }}
.viewport-command-strip {{ display: grid; gap: 8px; border: 1px solid #d3d8df; border-radius: 6px; padding: 10px 12px; margin: 10px 0 12px; background: #fff; }}
.viewport-command-row {{ display: flex; flex-wrap: wrap; gap: 8px; align-items: center; }}
.viewport-command-group {{ display: inline-flex; flex-wrap: wrap; gap: 6px; align-items: center; min-width: 0; }}
.viewport-command-label {{ min-height: 28px; display: inline-flex; align-items: center; color: #5d636b; font-size: 11px; font-weight: 900; text-transform: uppercase; }}
.viewport-command-strip .resource-actions {{ flex: 1 1 auto; }}
.viewport-page-state {{ color: #5d636b; font-size: 12px; font-weight: 700; }}
.viewport-state-chip {{ min-height: 28px; display: inline-flex; align-items: center; border: 1px solid #dfe2e6; border-radius: 6px; padding: 0 8px; background: #f7f7f5; color: #3a3f45; font-size: 12px; font-weight: 800; white-space: nowrap; }}
.viewport-state-chip.report {{ background: #eef4ff; border-color: #c7d7ff; color: #1d3f91; line-height: 1.3; white-space: normal; overflow-wrap: anywhere; }}
.viewport-state-chip.warning {{ background: #fff7e8; border-color: #f0c16b; color: #6b4300; line-height: 1.3; white-space: normal; overflow-wrap: anywhere; }}
.viewport-command-jump {{ display: flex; flex-wrap: wrap; gap: 6px; align-items: center; }}
.viewport-command-jump label {{ color: #3a3f45; font-size: 12px; font-weight: 800; }}
.viewport-command-jump input[type="number"] {{ width: 82px; height: 28px; border: 1px solid #b7bdc5; border-radius: 6px; padding: 0 8px; font-size: 12px; background: #fff; }}
.viewport-command-jump button {{ min-height: 28px; border: 1px solid #2457d6; border-radius: 6px; padding: 0 9px; background: #2457d6; color: #fff; font-size: 12px; font-weight: 800; cursor: pointer; }}
.viewport-interaction-row {{ display: flex; flex-wrap: wrap; gap: 8px; align-items: center; margin: 8px 0 10px; }}
.viewport-interaction-row.compact {{ margin: 6px 0 8px; }}
.viewport-click-status-row {{ flex: 1 1 280px; min-width: 0; display: inline-flex; flex-wrap: wrap; gap: 6px; align-items: center; }}
.viewport-click-details {{ border: 1px solid #dfe2e6; border-radius: 6px; background: #fff; }}
.viewport-click-details > summary {{ min-height: 28px; display: inline-flex; align-items: center; cursor: pointer; padding: 0 9px; color: #20242a; font-size: 12px; font-weight: 800; }}
.viewport-click-form {{ display: inline-flex; flex-wrap: wrap; gap: 6px; align-items: center; min-width: 0; }}
.viewport-click-details .viewport-click-form {{ padding: 0 9px 9px; }}
.viewport-click-form label {{ color: #3a3f45; font-size: 12px; font-weight: 800; }}
.viewport-click-form input[type="number"] {{ width: 76px; height: 28px; border: 1px solid #b7bdc5; border-radius: 6px; padding: 0 8px; font-size: 12px; background: #fff; }}
.viewport-click-form button {{ min-height: 28px; border: 1px solid #2457d6; border-radius: 6px; padding: 0 9px; background: #2457d6; color: #fff; font-size: 12px; font-weight: 800; cursor: pointer; }}
.viewport-link-strip {{ min-width: 0; max-width: min(100%, 340px); border: 1px solid #dfe2e6; border-radius: 6px; background: #fff; }}
.viewport-link-strip > summary {{ min-height: 28px; display: inline-flex; align-items: center; cursor: pointer; padding: 0 9px; color: #20242a; font-size: 12px; font-weight: 800; }}
.viewport-link-list {{ display: flex; flex-wrap: wrap; gap: 6px; padding: 0 9px 9px; }}
.viewport-link-list a {{ min-height: 28px; display: inline-flex; align-items: center; max-width: 220px; border: 1px solid #c6cbd2; border-radius: 6px; padding: 0 9px; background: #fff; color: #20242a; font-size: 12px; font-weight: 700; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }}
.viewport-command-strip[data-resource-pending="true"] a[href^="/browser"], .viewport-command-strip[data-visual-pending="true"] .primary-action, .viewport-command-strip[data-scroll-pending="true"] a[href^="/browser"], .viewport-command-strip[data-scroll-pending="true"] button {{ cursor: wait; opacity: 0.72; }}
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
.session-tab-card.pinned {{ border-left: 4px solid #2457d6; padding-left: 7px; }}
.session-tab-card.current {{ background: #191a1c; color: #fff; border-color: #191a1c; }}
.session-tab {{ min-width: 0; display: grid; gap: 3px; color: inherit; }}
.session-tab strong {{ display: block; min-width: 0; font-size: 13px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }}
.session-tab span {{ min-width: 0; color: inherit; opacity: 0.72; font-size: 11px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }}
.session-actions {{ display: grid; gap: 6px; justify-items: end; }}
.session-action {{ min-height: 24px; display: inline-flex; align-items: center; border: 1px solid #c6cbd2; border-radius: 6px; padding: 0 7px; background: #fff; color: #20242a; font-size: 12px; font-weight: 700; }}
.session-tab-card.current .session-action {{ border-color: #fff; }}
.session-new {{ display: grid; grid-template-columns: minmax(0, 1fr) auto; gap: 8px; border: 1px dashed #b7bdc5; border-radius: 6px; padding: 8px; background: #fff; }}
.session-new input[type="search"], .session-new input[type="text"] {{ min-width: 0; height: 32px; border: 1px solid #b7bdc5; border-radius: 6px; padding: 0 9px; font-size: 13px; background: #fff; }}
.session-new button {{ min-height: 32px; border: 1px solid #2457d6; border-radius: 6px; padding: 0 10px; background: #2457d6; color: #fff; font-size: 13px; font-weight: 700; cursor: pointer; }}
.tab-search {{ grid-column: 1 / -1; display: grid; gap: 8px; }}
.tab-search table {{ width: 100%; border-collapse: collapse; table-layout: fixed; background: #fff; border: 1px solid #dfe2e6; border-radius: 6px; overflow: hidden; }}
.tab-search th, .tab-search td {{ border-top: 1px solid #eef0f3; padding: 7px 6px; color: #3a3f45; font-size: 12px; text-align: left; vertical-align: top; overflow-wrap: anywhere; }}
.tab-search th {{ color: #5d636b; font-weight: 700; }}
.meta {{ color: #5d636b; font-size: 13px; overflow-wrap: anywhere; line-height: 1.45; }}
pre {{ white-space: pre-wrap; background: #fff; border: 1px solid #dfe2e6; border-radius: 6px; padding: 16px; line-height: 1.35; overflow: auto; font: 13px ui-monospace, SFMono-Regular, Menlo, Consolas, monospace; }}
pre mark {{ background: #ffe08a; color: inherit; border-radius: 2px; padding: 0 1px; }}
.auto-visual-status {{ margin: 12px 0 8px; color: #5d636b; font-size: 13px; font-weight: 700; }}
.resource-quick-actions {{ display: grid; gap: 8px; border: 1px solid #dfe2e6; border-radius: 6px; padding: 10px 12px; margin: 10px 0 12px; background: #fff; }}
.resource-quick-actions > summary {{ cursor: pointer; display: flex; flex-wrap: wrap; align-items: center; justify-content: space-between; gap: 10px; }}
.resource-quick-summary {{ min-width: 220px; display: grid; gap: 2px; }}
.resource-quick-summary strong {{ color: #20242a; font-size: 13px; }}
.resource-quick-summary span {{ color: #5d636b; font-size: 12px; font-weight: 700; }}
.resource-quick-actions .resource-actions {{ justify-content: flex-start; }}
.resource-quick-actions[data-visual-pending="true"] .primary-action {{ opacity: 0.72; }}
.resource-quick-actions[data-resource-pending="true"] a[href^="/browser"], .browser-inspector section[data-resource-pending="true"] a[href^="/browser"] {{ cursor: wait; opacity: 0.72; }}
.resource-action-status, .resource-visual-status {{ min-height: 28px; display: inline-flex; align-items: center; color: #5d636b; font-size: 12px; font-weight: 700; }}
.browser-raster-shell {{ position: relative; width: 100%; background: #fff; border: 1px solid #dfe2e6; border-radius: 6px; margin: 12px 0; overflow: auto; overscroll-behavior: contain; touch-action: pan-x pan-y; scrollbar-gutter: stable; cursor: crosshair; }}
.browser-raster-shell:focus {{ outline: 2px solid #2457d6; outline-offset: 2px; }}
.browser-raster-shell[data-viewport-pending="true"] {{ cursor: wait; }}
.browser-raster-shell[data-browser-pending-viewport="true"] {{ cursor: wait; }}
.browser-raster {{ display: block; max-width: none; width: auto; height: auto; }}
.browser-click-marker {{ position: absolute; z-index: 2; width: 16px; height: 16px; margin: -8px 0 0 -8px; border: 2px solid #2457d6; border-radius: 999px; background: rgba(36, 87, 214, 0.12); box-shadow: 0 0 0 2px rgba(255,255,255,0.88); pointer-events: none; }}
.browser-click-marker::after {{ content: ""; position: absolute; left: 50%; top: 50%; width: 4px; height: 4px; margin: -2px 0 0 -2px; border-radius: 999px; background: #2457d6; }}
.browser-raster-placeholder {{ min-height: 96px; display: grid; align-content: center; gap: 6px; padding: 14px; background: #f6f8fb; color: #3a3f45; font-size: 13px; cursor: default; }}
.browser-raster-placeholder strong {{ color: #20242a; }}
.browser-raster-error {{ margin: 12px 0; border: 1px solid #d7a8a8; border-radius: 6px; padding: 10px 12px; background: #fff5f5; color: #7a2020; font-size: 13px; }}
.browser-viewport-primary {{ margin: 10px 0 18px; scroll-margin-top: 76px; }}
.browser-controls-tray {{ border: 1px solid #dfe2e6; border-radius: 6px; background: #fff; margin: 12px 0 16px; }}
.browser-controls-tray > summary {{ cursor: pointer; padding: 10px 12px; color: #20242a; font-size: 13px; font-weight: 900; }}
.browser-controls-content {{ display: grid; gap: 10px; padding: 0 12px 12px; border-top: 1px solid #eef0f3; }}
.viewport-input {{ display: grid; gap: 8px; margin: 8px 0 12px; }}
.viewport-input form {{ display: grid; grid-template-columns: minmax(0, 1fr) auto auto auto auto; gap: 8px; align-items: center; }}
.viewport-input input[type="text"] {{ min-width: 0; height: 32px; border: 1px solid #b7bdc5; border-radius: 6px; padding: 0 9px; font-size: 13px; background: #fff; }}
.viewport-input button, .viewport-input a {{ min-height: 32px; display: inline-flex; align-items: center; border: 1px solid #c6cbd2; border-radius: 6px; padding: 0 10px; background: #fff; color: #20242a; font-size: 13px; font-weight: 700; cursor: pointer; }}
.viewport-input button {{ background: #2457d6; color: #fff; border-color: #2457d6; }}
.viewport-text {{ margin-top: 10px; }}
.viewport-text summary {{ cursor: pointer; color: #3a3f45; font-size: 13px; font-weight: 700; }}
.viewport-text pre {{ margin-top: 8px; }}
.debug-stack {{ margin-top: 18px; }}
.browser-tools-menu {{ border: 1px solid #dfe2e6; border-radius: 6px; background: #fff; }}
.browser-tools-menu > summary {{ cursor: pointer; padding: 11px 12px; color: #20242a; font-size: 14px; font-weight: 900; }}
.browser-tools-menu > summary::marker {{ color: #5d636b; }}
.browser-diagnostics-compact {{ display: flex; align-items: center; justify-content: space-between; gap: 10px; padding: 10px 12px; }}
.browser-diagnostics-compact .meta {{ min-width: 0; }}
.debug-stack-content {{ display: grid; gap: 10px; padding: 0 12px 12px; border-top: 1px solid #eef0f3; }}
.debug-stack-content > :first-child {{ margin-top: 12px; }}
.debug-section {{ border: 1px solid #dfe2e6; border-radius: 6px; background: #fff; }}
.debug-section > summary {{ cursor: pointer; padding: 11px 12px; color: #20242a; font-size: 14px; font-weight: 800; }}
.debug-section > summary::marker {{ color: #5d636b; }}
.debug-section-content {{ padding: 0 12px 12px; border-top: 1px solid #eef0f3; }}
.debug-section-content > :first-child {{ margin-top: 12px; }}
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
.clear-link {{ min-height: 28px; display: inline-flex; align-items: center; border: 1px solid #c6cbd2; border-radius: 6px; padding: 0 9px; background: #fff; color: #20242a; font-size: 12px; font-weight: 700; white-space: nowrap; }}
.clear-link.primary-action {{ background: #2457d6; border-color: #2457d6; color: #fff; }}
.resource-actions {{ display: flex; flex-wrap: wrap; gap: 6px; align-items: center; }}
.resource-report {{ display: grid; gap: 6px; margin: 8px 0 10px; color: #3a3f45; font-size: 12px; }}
.resource-report-summary {{ color: #5d636b; overflow-wrap: anywhere; }}
.browser-inspector table {{ width: 100%; border-collapse: collapse; table-layout: fixed; }}
.browser-inspector th, .browser-inspector td {{ border-top: 1px solid #eef0f3; padding: 7px 6px; color: #3a3f45; font-size: 12px; text-align: left; vertical-align: top; overflow-wrap: anywhere; }}
.browser-inspector th {{ color: #5d636b; font-weight: 700; }}
.browser-inspector .current-row td {{ background: #eef4ff; }}
@media (max-width: 720px) {{ .browser-chrome-row {{ grid-template-columns: 1fr; }} .browser-primary-nav, .address-bar {{ overflow-x: auto; }} .browser-actions {{ grid-template-columns: 1fr; }} .browser-action {{ grid-template-columns: 1fr; }} .viewport-input form {{ grid-template-columns: 1fr 1fr; }} .viewport-input input[type="text"] {{ grid-column: 1 / -1; }} }}
</style>
</head>
<body>
<main>
<header class="browser-topbar">
<div class="browser-chrome-row" data-browser-chrome>
<nav class="toolbar browser-primary-nav" data-browser-auto-visual-control><a href="{back_href}">Search</a>{back_control}{forward_control}<a href="{reload_href}">Reload</a></nav>
<form class="toolbar address-bar" action="/browser" method="get" data-browser-auto-visual-control>
<input type="hidden" name="id" value="{id}">
<input type="hidden" name="from" value="{back_href}">
<input type="hidden" name="width" value="{width}">
<input type="hidden" name="height" value="{height}">
<input type="hidden" name="viewport_x" value="{viewport_x}">
<input type="hidden" name="viewport_y" value="{viewport_y}">
<input type="hidden" name="max_bytes" value="{max_bytes}">
<input data-browser-address type="text" inputmode="url" autocapitalize="none" spellcheck="false" name="url" value="{source_attr}" title="{source_attr}" aria-label="Address">
<button type="submit" name="action" value="open">Go</button><button class="browser-new-tab" type="submit" name="action" value="open-new-session">New tab</button><button class="browser-background-tab" type="submit" name="action" value="open-background-session">Background</button>
</form>
<div class="browser-chrome-status" data-browser-chrome-status data-browser-resource-actions data-browser-auto-visual-control>{browser_chrome_status}<a class="browser-chrome-tool" href="{tools_href}">Tools</a></div>
</div>
{primary_tab_strip}
</header>
<section class="browser-page-head">
<div class="browser-page-title"><h1>{heading}</h1><div class="meta" title="{source_attr}">{source}</div></div>
</section>
{auto_visual_bootstrap}
{pending_load_retry}
<section class="browser-viewport-primary" data-browser-primary-surface>
{pending_primary_page_state}
{viewport_image}
{settled_primary_page_state}
{viewport_status}
{viewport_interaction_controls}
{primary_input_controls}
<details id="browser-controls-tray" class="browser-controls-tray" data-browser-controls-tray><summary>More browser tools</summary><div class="browser-controls-content">{viewport_scroll_controls}{page_summary}{find_controls}{viewport_command_strip}{resource_quick_actions}{viewport_text}</div></details>
</section>
{diagnostics_section}
{keyboard_controls_script}
</main>
</body>
</html>"#,
        title = html_escape::encode_text(&payload.title),
        heading = html_escape::encode_text(&payload.title),
        source = html_escape::encode_text(&browser_session_feedback_excerpt(&payload.source)),
        source_attr = html_escape::encode_double_quoted_attribute(&payload.source),
        id = html_escape::encode_double_quoted_attribute(&payload.id),
        back_href = html_escape::encode_double_quoted_attribute(back_href),
        back_control = back_control,
        forward_control = forward_control,
        reload_href = html_escape::encode_double_quoted_attribute(&reload_href),
        keyboard_controls_script = keyboard_controls_script,
        width = payload.width,
        height = payload.height,
        max_bytes = payload.max_bytes,
        viewport_x = payload.viewport_x,
        viewport_y = payload.viewport_y,
        viewport_status = viewport_status,
        viewport_scroll_controls = viewport_scroll_controls,
        pending_primary_page_state = pending_primary_page_state.unwrap_or_default(),
        settled_primary_page_state = settled_primary_page_state.unwrap_or_default(),
        auto_visual_bootstrap = auto_visual_bootstrap,
        pending_load_retry = pending_load_retry,
        browser_chrome_status = render_browser_session_chrome_status(payload),
        tools_href = html_escape::encode_double_quoted_attribute("#browser-controls-tray"),
        viewport_command_strip = viewport_command_strip,
        page_summary = page_summary,
        resource_quick_actions = resource_quick_actions,
        viewport_image = viewport_image,
        viewport_interaction_controls = viewport_interaction_controls,
        primary_input_controls = primary_input_controls,
        viewport_text = viewport_text,
        find_controls = find_controls,
        primary_tab_strip = primary_tab_strip,
        diagnostics_section = diagnostics_section,
    )
}

fn render_browser_session_viewport_partial(payload: &BrowserSessionPayload) -> String {
    format!(
        r#"<div data-browser-partial-viewport data-viewport-x="{viewport_x}" data-viewport-y="{viewport_y}" data-max-scroll-x="{max_scroll_x}" data-max-scroll-y="{max_scroll_y}"><div data-browser-partial-raster>{raster}</div><div data-browser-partial-status>{status}</div><div data-browser-partial-interactions>{interactions}</div><div data-browser-partial-scroll-controls>{scroll_controls}</div><div data-browser-partial-command-strip>{command_strip}</div></div>"#,
        viewport_x = payload.viewport_x,
        viewport_y = payload.viewport_y,
        max_scroll_x = payload.max_scroll_x,
        max_scroll_y = payload.max_scroll_y,
        raster = render_browser_session_viewport_image_shell(payload),
        status = render_browser_session_viewport_status(payload),
        interactions = render_browser_session_viewport_interaction_controls(payload),
        scroll_controls = render_browser_session_viewport_scroll_controls(payload),
        command_strip = render_browser_session_viewport_command_strip(payload),
    )
}

fn render_browser_session_auto_visual_bootstrap(payload: &BrowserSessionPayload) -> String {
    if payload.resource_report.is_some() {
        return String::new();
    }

    let action_urls = browser_session_state_action_urls(payload);
    if action_urls.make_visual.is_none()
        && action_urls.apply_stylesheets.is_none()
        && action_urls.load_images.is_none()
    {
        return String::new();
    }

    let make_visual = action_urls.make_visual.unwrap_or_default();
    let apply_stylesheets = action_urls.apply_stylesheets.unwrap_or_default();
    let load_images = action_urls.load_images.unwrap_or_default();
    let refresh_url = browser_session_action_href(&payload.id, "current", &[], payload);
    let block_browser_controls = !browser_session_has_ready_raster(payload);
    let status_label = if block_browser_controls {
        "Preparing visual render..."
    } else {
        "Loading visual resources..."
    };
    let state_key = format!(
        "brutal:auto-visual:{}:{}:{}:{}",
        payload.id, payload.source, payload.viewport_x, payload.viewport_y
    );

    format!(
        r#"<div class="auto-visual-status" data-auto-visual-status>{status_label}</div>
<script>
(() => {{
  const makeVisualUrl = {make_visual};
  const applyStylesheetsUrl = {apply_stylesheets};
  const loadImagesUrl = {load_images};
  const refreshUrl = {refresh_url};
  const stateKey = {state_key};
  const blockBrowserControls = {block_browser_controls};
  const runningRefreshDelayMs = 5000;
  const runningStaleAfterMs = 45000;
  const failedRetryCooldownMs = 60000;
  const requestTimeoutMs = 12000;
  const timeoutSeconds = Math.round(requestTimeoutMs / 1000);
  const status = document.querySelector("[data-auto-visual-status]");
  const setStatus = (message) => {{
    if (status) {{
      status.textContent = message;
    }}
  }};
  const autoVisualControls = () => Array.from(document.querySelectorAll("[data-browser-auto-visual-control]"));
  let autoVisualControlsBlocked = blockBrowserControls;
  const setAutoVisualControlsBusy = (busy) => {{
    autoVisualControlsBlocked = busy;
    for (const control of autoVisualControls()) {{
      if (busy) {{
        control.dataset.autoVisualPending = "true";
        control.setAttribute("aria-busy", "true");
      }} else {{
        delete control.dataset.autoVisualPending;
        control.removeAttribute("aria-busy");
      }}
    }}
  }};
  const refreshUrlForCurrentViewport = () => {{
    if (!refreshUrl) {{
      return "";
    }}
    try {{
      const url = new URL(refreshUrl, window.location.href);
      const shell = document.querySelector("[data-browser-viewport-scroll]");
      if (shell) {{
        const viewportX = Number(shell.dataset.viewportX);
        const viewportY = Number(shell.dataset.viewportY);
        if (Number.isFinite(viewportX)) {{
          url.searchParams.set("viewport_x", String(viewportX));
        }}
        if (Number.isFinite(viewportY)) {{
          url.searchParams.set("viewport_y", String(viewportY));
        }}
      }}
      return url.toString();
    }} catch (_) {{
      return refreshUrl;
    }}
  }};
  const isBrowserActionLink = (target) => {{
    if (!target || target.tagName !== "A") {{
      return true;
    }}
    const href = target.getAttribute("href") || "";
    if (!href) {{
      return false;
    }}
    try {{
      return new URL(target.href, window.location.href).pathname === "/browser";
    }} catch (_) {{
      return href.startsWith("/browser");
    }}
  }};
  const showAutoVisualControlStatus = () => setStatus("Visual render is still running. Please wait...");
  const guardAutoVisualControls = () => {{
    setAutoVisualControlsBusy(blockBrowserControls);
    document.addEventListener("click", (event) => {{
      if (!autoVisualControlsBlocked) {{
        return;
      }}
      const eventTarget = event.target instanceof Element ? event.target : event.target && event.target.parentElement;
      const target = eventTarget && typeof eventTarget.closest === "function" ? eventTarget.closest("[data-browser-auto-visual-control] a, [data-browser-auto-visual-control] button") : null;
      if (!target || !isBrowserActionLink(target)) {{
        return;
      }}
      event.preventDefault();
      showAutoVisualControlStatus();
    }});
    document.addEventListener("submit", (event) => {{
      if (!autoVisualControlsBlocked) {{
        return;
      }}
      const target = event.target instanceof Element ? event.target : null;
      if (!target || !target.closest("[data-browser-auto-visual-control]")) {{
        return;
      }}
      event.preventDefault();
      showAutoVisualControlStatus();
    }});
  }};
  guardAutoVisualControls();
  const scheduleRefresh = (message, delayMs) => {{
    setStatus(message);
    if (refreshUrl) {{
      window.setTimeout(() => window.location.replace(refreshUrlForCurrentViewport()), delayMs);
    }}
  }};
  const currentState = sessionStorage.getItem(stateKey) || "";
  if (currentState === "done") {{
    sessionStorage.removeItem(stateKey);
  }} else if (currentState.startsWith("running:")) {{
    const startedAt = Number(currentState.slice("running:".length));
    if (Number.isFinite(startedAt) && Date.now() - startedAt < runningStaleAfterMs) {{
      scheduleRefresh("Visual render is still running. Refreshing soon...", runningRefreshDelayMs);
      return;
    }}
  }} else if (currentState.startsWith("failed:")) {{
    const failedAt = Number(currentState.slice("failed:".length));
    if (Number.isFinite(failedAt) && Date.now() - failedAt < failedRetryCooldownMs) {{
      setAutoVisualControlsBusy(false);
      setStatus("Visual render failed or timed out. Use Tools to retry.");
      return;
    }}
    sessionStorage.removeItem(stateKey);
  }}
  sessionStorage.setItem(stateKey, `running:${{Date.now()}}`);
  const request = async (label, url) => {{
    if (!url) {{
      return;
    }}
    const startedAt = Date.now();
    const updateProgress = () => {{
      const elapsedSeconds = Math.max(0, Math.floor((Date.now() - startedAt) / 1000));
      setStatus(`${{label}} ${{elapsedSeconds}}s elapsed, timeout ${{timeoutSeconds}}s...`);
    }};
    updateProgress();
    const progress = window.setInterval(updateProgress, 1000);
    const controller = new AbortController();
    const timeout = window.setTimeout(() => controller.abort(), requestTimeoutMs);
    try {{
      const response = await fetch(url, {{
        cache: "no-store",
        credentials: "same-origin",
        signal: controller.signal,
      }});
      if (!response.ok) {{
        throw new Error(`${{label}} failed (${{response.status}})`);
      }}
    }} catch (error) {{
      if (error && error.name === "AbortError") {{
        throw new Error(`${{label}} timed out`);
      }}
      throw error;
    }} finally {{
      window.clearTimeout(timeout);
      window.clearInterval(progress);
    }}
  }};
  (async () => {{
    if (makeVisualUrl) {{
      await request("Making visual...", makeVisualUrl);
    }} else {{
      await request("Applying styles...", applyStylesheetsUrl);
      await request("Loading images...", loadImagesUrl);
    }}
    sessionStorage.setItem(stateKey, "done");
    if (refreshUrl) {{
      setStatus("Visual render complete. Opening page...");
      window.location.replace(refreshUrlForCurrentViewport());
    }}
  }})().catch((error) => {{
    sessionStorage.setItem(stateKey, `failed:${{Date.now()}}`);
    setAutoVisualControlsBusy(false);
    setStatus(error && error.message ? error.message : "Visual render failed");
  }});
}})();
</script>"#,
        make_visual = browser_json_script_string(&make_visual),
        apply_stylesheets = browser_json_script_string(&apply_stylesheets),
        load_images = browser_json_script_string(&load_images),
        refresh_url = browser_json_script_string(&refresh_url),
        state_key = browser_json_script_string(&state_key),
        status_label = html_escape::encode_text(status_label),
        block_browser_controls = if block_browser_controls {
            "true"
        } else {
            "false"
        },
    )
}

fn render_browser_session_pending_load_retry_script(payload: &BrowserSessionPayload) -> String {
    if !browser_session_pending_without_ready_viewport(payload) {
        return String::new();
    }
    let Some(pending_source) = payload.pending_source.as_ref() else {
        return String::new();
    };
    let continue_href = browser_session_action_href(
        &payload.id,
        "open",
        &[("url", pending_source.clone())],
        payload,
    );
    let state_key = format!(
        "brutal:pending-open:{}:{}:{}:{}:{}:{}",
        payload.id,
        pending_source,
        payload.width,
        payload.height,
        payload.viewport_x,
        payload.viewport_y
    );
    format!(
        r#"<script data-browser-pending-load-retry>
(() => {{
  const retryUrl = {retry_url};
  const stateKey = {state_key};
  const status = document.querySelector("[data-browser-pending-auto-retry]");
  const continueLink = document.querySelector("[data-browser-continue-load]");
  if (!retryUrl || !stateKey) {{
    return;
  }}
  const setStatus = (message) => {{
    if (status) {{
      status.textContent = message;
    }}
  }};
  try {{
    if (sessionStorage.getItem(stateKey) === "tried") {{
      setStatus("Still opening; use Continue loading to retry in this tab.");
      if (continueLink) {{
        continueLink.dataset.pendingAutoRetry = "used";
      }}
      return;
    }}
    sessionStorage.setItem(stateKey, "tried");
  }} catch (_) {{
    return;
  }}
  setStatus("Still opening; retrying once in this tab...");
  if (continueLink) {{
    continueLink.dataset.pendingAutoRetry = "scheduled";
    continueLink.setAttribute("aria-busy", "true");
  }}
  window.setTimeout(() => window.location.replace(retryUrl), 900);
}})();
</script>"#,
        retry_url = browser_json_script_string(&continue_href),
        state_key = browser_json_script_string(&state_key),
    )
}

fn browser_json_script_string(value: &str) -> String {
    let json = serde_json::to_string(value).unwrap_or_else(|_| "\"\"".to_owned());
    json.replace('<', "\\u003c")
        .replace('>', "\\u003e")
        .replace('&', "\\u0026")
}

fn browser_session_has_ready_raster(payload: &BrowserSessionPayload) -> bool {
    payload.viewport_image.is_some()
}

fn render_browser_session_keyboard_controls_script(reload_href: &str) -> String {
    format!(
        r#"<script data-browser-keyboard-controls>
(() => {{
  const addressInput = document.querySelector("[data-browser-address]");
  const findInput = document.querySelector("[data-browser-find]");
  const reloadUrl = {reload_url};
  const showPendingAutoVisualStatus = () => {{
    const pendingAutoVisual = document.querySelector("[data-auto-visual-" + "status]");
    if (!pendingAutoVisual) {{
      return false;
    }}
    pendingAutoVisual.textContent = "Visual render is still running. Please wait...";
    return true;
  }};
  const focusAndSelect = (element) => {{
    if (!element) {{
      return false;
    }}
    element.focus();
    if (typeof element.select === "function") {{
      element.select();
    }}
    return true;
  }};
  document.addEventListener("keydown", (event) => {{
    if (event.defaultPrevented || event.altKey || !(event.metaKey || event.ctrlKey)) {{
      return;
    }}
    const key = event.key.toLowerCase();
    if (key === "l") {{
      if (focusAndSelect(addressInput)) {{
        event.preventDefault();
      }}
    }} else if (key === "f") {{
      if (focusAndSelect(findInput)) {{
        event.preventDefault();
    }}
  }} else if (key === "r" && reloadUrl) {{
    event.preventDefault();
    if (showPendingAutoVisualStatus()) {{
      return;
    }}
    window.location.href = reloadUrl;
  }}
  }});
}})();
</script>"#,
        reload_url = browser_json_script_string(reload_href),
    )
}

fn render_browser_session_viewport_image(payload: &BrowserSessionPayload) -> String {
    format!(
        r#"<div data-browser-partial-raster>{shell}</div>{script}"#,
        shell = render_browser_session_viewport_image_shell(payload),
        script = render_browser_session_viewport_scroll_script(),
    )
}

fn render_browser_session_viewport_image_shell(payload: &BrowserSessionPayload) -> String {
    let scroll_url = browser_session_action_href(&payload.id, "scroll", &[], payload);
    let click_url = browser_session_action_href(&payload.id, "click-at", &[], payload);
    let viewport_accessibility_label = "Rendered browser viewport; click links and buttons in this image, or use wheel, arrows, Page Up, Page Down, Home, and End to scroll";
    if browser_session_pending_without_ready_viewport(payload)
        && let Some(pending_source) = payload.pending_source.as_ref()
    {
        let continue_href = browser_session_action_href(
            &payload.id,
            "open",
            &[("url", pending_source.clone())],
            payload,
        );
        let pending_status =
            payload
                .action_feedback
                .as_deref()
                .map_or_else(String::new, |feedback| {
                    format!(
                        r#"<span data-browser-pending-render-status>Status: {feedback}</span>"#,
                        feedback = html_escape::encode_text(feedback),
                    )
                });
        return format!(
            r#"<div class="browser-raster-shell" data-browser-pending-viewport="true" data-page-source="{source_attr}" data-viewport-state="loading" data-viewport-x="{viewport_x}" data-viewport-y="{viewport_y}" data-viewport-width="{viewport_width}" data-viewport-height="{viewport_height}" data-max-scroll-x="{max_scroll_x}" data-max-scroll-y="{max_scroll_y}" tabindex="0" role="region" aria-busy="true" aria-label="Browser viewport is loading {source_attr}" title="Browser viewport is loading {source_attr}"><div class="browser-raster-placeholder"><strong>No rendered viewport yet</strong><span>Opening {source}. The renderer has not produced a page image yet; the tab is retained and the browser controls remain usable.</span>{pending_status}<a class="clear-link primary-action" href="{continue_href}" data-browser-continue-load>Continue loading</a></div></div>"#,
            viewport_x = payload.viewport_x,
            viewport_y = payload.viewport_y,
            viewport_width = payload.width,
            viewport_height = payload.height,
            max_scroll_x = payload.max_scroll_x,
            max_scroll_y = payload.max_scroll_y,
            source = html_escape::encode_text(&browser_session_feedback_excerpt(pending_source)),
            source_attr = html_escape::encode_double_quoted_attribute(pending_source),
            pending_status = pending_status,
            continue_href = html_escape::encode_double_quoted_attribute(&continue_href),
        );
    }
    if let Some(image) = &payload.viewport_image {
        return format!(
            r#"<div class="browser-raster-shell" data-browser-viewport-scroll data-browser-dom-click data-click-coordinate-space="raster-pixels" data-page-source="{source}" data-scroll-url="{scroll_url}" data-click-url="{click_url}" data-viewport-state="settled" data-viewport-x="{viewport_x}" data-viewport-y="{viewport_y}" data-viewport-width="{viewport_width}" data-viewport-height="{viewport_height}" data-raster-width="{width}" data-raster-height="{height}" data-max-scroll-x="{max_scroll_x}" data-max-scroll-y="{max_scroll_y}" data-settled-viewport-x="{viewport_x}" data-settled-viewport-y="{viewport_y}" tabindex="0" role="region" aria-label="{viewport_accessibility_label}" title="{viewport_accessibility_label}"><img class="browser-raster" src="{src}" width="{width}" height="{height}" alt="Rendered browser viewport; click links and buttons in the image to activate DOM elements"><span class="browser-click-marker" data-browser-click-marker hidden></span></div>"#,
            scroll_url = html_escape::encode_double_quoted_attribute(&scroll_url),
            click_url = html_escape::encode_double_quoted_attribute(&click_url),
            source = html_escape::encode_double_quoted_attribute(&payload.source),
            viewport_accessibility_label =
                html_escape::encode_double_quoted_attribute(viewport_accessibility_label),
            viewport_x = payload.viewport_x,
            viewport_y = payload.viewport_y,
            viewport_width = payload.width,
            viewport_height = payload.height,
            max_scroll_x = payload.max_scroll_x,
            max_scroll_y = payload.max_scroll_y,
            src = html_escape::encode_double_quoted_attribute(&image.data_url),
            width = image.width,
            height = image.height,
        );
    }
    if payload.fast_scroll {
        return format!(
            r#"<div class="browser-raster-shell" data-browser-viewport-scroll data-browser-dom-click data-browser-fast-scroll data-page-source="{source}" data-scroll-url="{scroll_url}" data-click-url="{click_url}" data-viewport-state="settled" data-viewport-x="{viewport_x}" data-viewport-y="{viewport_y}" data-viewport-width="{viewport_width}" data-viewport-height="{viewport_height}" data-max-scroll-x="{max_scroll_x}" data-max-scroll-y="{max_scroll_y}" data-settled-viewport-x="{viewport_x}" data-settled-viewport-y="{viewport_y}" tabindex="0" role="region" aria-label="{viewport_accessibility_label}" title="{viewport_accessibility_label}"><div class="browser-raster-placeholder"><strong>Fast text scroll</strong><span>Skipped visual raster generation for this scroll response. Use Refresh viewport or Make page readable to render the visual view.</span></div></div>"#,
            scroll_url = html_escape::encode_double_quoted_attribute(&scroll_url),
            click_url = html_escape::encode_double_quoted_attribute(&click_url),
            source = html_escape::encode_double_quoted_attribute(&payload.source),
            viewport_accessibility_label =
                html_escape::encode_double_quoted_attribute(viewport_accessibility_label),
            viewport_x = payload.viewport_x,
            viewport_y = payload.viewport_y,
            viewport_width = payload.width,
            viewport_height = payload.height,
            max_scroll_x = payload.max_scroll_x,
            max_scroll_y = payload.max_scroll_y,
        );
    }
    if let Some(error) = &payload.viewport_image_error {
        return format!(
            r#"<div class="browser-raster-error">Viewport image unavailable: {error}</div>"#,
            error = html_escape::encode_text(error),
        );
    }
    String::new()
}

fn render_browser_session_viewport_scroll_script() -> &'static str {
    r#"<script>
(() => {
  const shell = document.querySelector("[data-browser-viewport-scroll]");
  if (!shell) {
    return;
  }
  let raster = shell.querySelector(".browser-raster");
  let clickMarker = shell.querySelector("[data-browser-click-marker]");
  const pendingAutoVisual = document.querySelector("[data-auto-visual-" + "status]");
  if (pendingAutoVisual && !raster) {
    shell.dataset.pendingAutoVisual = "true";
    shell.setAttribute("aria-busy", "true");
  } else if (pendingAutoVisual) {
    pendingAutoVisual.dataset.visualReadyRaster = "true";
  }
  try {
    if (sessionStorage.getItem("browserViewportAnchor") === "1") {
      sessionStorage.removeItem("browserViewportAnchor");
      requestAnimationFrame(() => shell.scrollIntoView({ block: "start", inline: "nearest" }));
    }
  } catch (_) {}
  let lastClickPagePoint = null;
  const viewportControls = () => Array.from(document.querySelectorAll("[data-browser-viewport-controls], [data-browser-viewport-command-strip]"));
  const viewportFeedbackTargets = () => Array.from(document.querySelectorAll("[data-browser-viewport-feedback]"));
  const clickStatusTargets = () => Array.from(document.querySelectorAll("[data-browser-click-status]"));
  const setViewportFeedback = (message) => {
    for (const feedback of viewportFeedbackTargets()) {
      feedback.textContent = message;
    }
  };
  const setClickStatus = (message) => {
    for (const status of clickStatusTargets()) {
      status.textContent = message;
    }
  };
  const viewportStatus = () => document.querySelector("[data-browser-viewport-status]");
  const setPendingViewportTarget = (target) => {
    if (!target) {
      return;
    }
    shell.dataset.pendingViewportX = String(target.x);
    shell.dataset.pendingViewportY = String(target.y);
    const status = viewportStatus();
    if (status) {
      status.dataset.pendingViewportX = String(target.x);
      status.dataset.pendingViewportY = String(target.y);
    }
  };
  const setViewportPending = (message, target) => {
    shell.dataset.viewportPending = "true";
    shell.dataset.viewportState = "pending";
    shell.removeAttribute("data-viewport-page-error");
    shell.setAttribute("aria-busy", "true");
    setPendingViewportTarget(target);
    for (const control of viewportControls()) {
      control.dataset.scrollPending = "true";
      control.setAttribute("aria-busy", "true");
    }
    const status = viewportStatus();
    if (status) {
      status.dataset.viewportPending = "true";
      status.setAttribute("aria-busy", "true");
      status.setAttribute("aria-label", message);
    }
    setViewportFeedback(message);
  };
  const clearViewportPending = () => {
    shell.removeAttribute("data-viewport-pending");
    shell.removeAttribute("data-viewport-request");
    shell.removeAttribute("data-viewport-partial-error");
    shell.removeAttribute("data-viewport-page-error");
    shell.removeAttribute("data-viewport-page-timeout");
    shell.removeAttribute("data-viewport-recovery");
    shell.removeAttribute("data-pending-viewport-x");
    shell.removeAttribute("data-pending-viewport-y");
    shell.removeAttribute("data-queued-scroll-dx");
    shell.removeAttribute("data-queued-scroll-dy");
    shell.removeAttribute("data-queued-scroll-target-x");
    shell.removeAttribute("data-queued-scroll-target-y");
    shell.removeAttribute("data-scroll-queued-during-request");
    shell.removeAttribute("data-stale-viewport-response");
    shell.removeAttribute("data-viewport-request-aborted");
    shell.dataset.viewportState = "settled";
    shell.dataset.settledViewportX = String(numberData("viewportX"));
    shell.dataset.settledViewportY = String(numberData("viewportY"));
    shell.removeAttribute("aria-busy");
    for (const control of viewportControls()) {
      control.removeAttribute("data-scroll-pending");
      control.removeAttribute("aria-busy");
    }
    const status = viewportStatus();
    if (status) {
      status.removeAttribute("data-viewport-pending");
      status.removeAttribute("data-pending-viewport-x");
      status.removeAttribute("data-pending-viewport-y");
      status.removeAttribute("aria-busy");
      status.removeAttribute("aria-label");
    }
  };
  const markStaleViewportResponse = (message) => {
    if (pendingScrollAfterRequest && (pendingScrollDx || pendingScrollDy)) {
      shell.dataset.viewportState = "pending";
    } else {
      shell.dataset.viewportState = "stale-response";
    }
    shell.dataset.staleViewportResponse = "true";
    setViewportFeedback(message);
  };
  const numberData = (name) => {
    const value = Number(shell.dataset[name]);
    return Number.isFinite(value) ? value : 0;
  };
  const clamp = (value, min, max) => Math.min(Math.max(value, min), max);
  const rasterSize = () => {
    const fallbackWidth = Math.max(1, numberData("viewportWidth"));
    const fallbackHeight = Math.max(1, numberData("viewportHeight"));
    if (!raster) {
      return { width: fallbackWidth, height: fallbackHeight };
    }
    const width = Number(raster.naturalWidth) || Number(raster.getAttribute("width")) || numberData("rasterWidth") || fallbackWidth;
    const height = Number(raster.naturalHeight) || Number(raster.getAttribute("height")) || numberData("rasterHeight") || fallbackHeight;
    return {
      width: Math.max(1, Math.round(width)),
      height: Math.max(1, Math.round(height))
    };
  };
  const viewportPointFromEvent = (event) => {
    if (!raster) {
      return null;
    }
    const rect = raster.getBoundingClientRect();
    if (!rect.width || !rect.height) {
      return null;
    }
    const relativeX = event.clientX - rect.left;
    const relativeY = event.clientY - rect.top;
    if (relativeX < 0 || relativeY < 0 || relativeX > rect.width || relativeY > rect.height) {
      return null;
    }
    const size = rasterSize();
    const x = clamp(Math.floor(relativeX / rect.width * size.width), 0, size.width - 1);
    const y = clamp(Math.floor(relativeY / rect.height * size.height), 0, size.height - 1);
    return {
      x,
      y,
      pageX: numberData("viewportX") + x,
      pageY: numberData("viewportY") + y
    };
  };
  const pointMessage = (point) => `DOM point x ${point.x}, y ${point.y} (page ${point.pageX}, ${point.pageY})`;
  const viewportPointFromPagePoint = (pagePoint) => {
    const size = rasterSize();
    const x = pagePoint.pageX - numberData("viewportX");
    const y = pagePoint.pageY - numberData("viewportY");
    if (x < 0 || y < 0 || x >= size.width || y >= size.height) {
      return null;
    }
    return { x, y, pageX: pagePoint.pageX, pageY: pagePoint.pageY };
  };
  const updateClickInputs = (point) => {
    const idPrefix = String.fromCharCode(35);
    const xInput = document.querySelector(idPrefix + "browser-viewport-click-x");
    const yInput = document.querySelector(idPrefix + "browser-viewport-click-y");
    if (xInput) {
      xInput.value = String(point.x);
    }
    if (yInput) {
      yInput.value = String(point.y);
    }
  };
  const hideClickMarker = () => {
    if (clickMarker) {
      clickMarker.hidden = true;
    }
  };
  const clearClickMarkerPoint = () => {
    lastClickPagePoint = null;
    hideClickMarker();
  };
  const moveClickMarker = (point) => {
    if (!clickMarker || !raster) {
      return;
    }
    const rasterRect = raster.getBoundingClientRect();
    const shellRect = shell.getBoundingClientRect();
    const size = rasterSize();
    const left = rasterRect.left - shellRect.left + ((point.x + 0.5) / size.width) * rasterRect.width;
    const top = rasterRect.top - shellRect.top + ((point.y + 0.5) / size.height) * rasterRect.height;
    clickMarker.style.left = `${left}px`;
    clickMarker.style.top = `${top}px`;
    clickMarker.hidden = false;
    lastClickPagePoint = { pageX: point.pageX, pageY: point.pageY };
    updateClickInputs(point);
  };
  const restoreClickMarkerAfterPartial = () => {
    if (!lastClickPagePoint) {
      return;
    }
    const point = viewportPointFromPagePoint(lastClickPagePoint);
    if (!point) {
      clearClickMarkerPoint();
      setClickStatus("Ready for page click.");
      return;
    }
    moveClickMarker(point);
    setClickStatus(`${pointMessage(point)}. Click.`);
  };
  const clearDeferredClick = () => {
    shell.removeAttribute("data-deferred-click-x");
    shell.removeAttribute("data-deferred-click-y");
    shell.removeAttribute("data-deferred-click-page-x");
    shell.removeAttribute("data-deferred-click-page-y");
  };
  const stampCurrentViewportUrl = (url) => {
    url.searchParams.set("viewport_x", String(numberData("viewportX")));
    url.searchParams.set("viewport_y", String(numberData("viewportY")));
    url.searchParams.set("width", String(numberData("viewportWidth")));
    url.searchParams.set("height", String(numberData("viewportHeight")));
    if (shell.dataset.pageSource) {
      url.searchParams.set("source", shell.dataset.pageSource);
    }
    return url;
  };
  const submitViewportClick = (point, messagePrefix) => {
    const url = stampCurrentViewportUrl(new URL(shell.dataset.clickUrl, window.location.href));
    const size = rasterSize();
    url.searchParams.set("x", String(point.x));
    url.searchParams.set("y", String(point.y));
    url.searchParams.set("raster_width", String(size.width));
    url.searchParams.set("raster_height", String(size.height));
    shell.dataset.lastClickX = String(point.x);
    shell.dataset.lastClickY = String(point.y);
    shell.dataset.lastClickPageX = String(point.pageX);
    shell.dataset.lastClickPageY = String(point.pageY);
    const message = `${messagePrefix} ${pointMessage(point)}...`;
    setClickStatus(message);
    replaceViewportPartial(url, message, {
      samePageOnly: true,
      fallback: () => replaceViewportPage(url, message)
    });
  };
  const replayDeferredClickAfterPartial = () => {
    const pageX = Number(shell.dataset.deferredClickPageX);
    const pageY = Number(shell.dataset.deferredClickPageY);
    if (!Number.isFinite(pageX) || !Number.isFinite(pageY)) {
      return false;
    }
    const point = viewportPointFromPagePoint({ pageX, pageY });
    clearDeferredClick();
    if (!point) {
      clearClickMarkerPoint();
      setClickStatus("Saved click is outside the settled viewport.");
      setViewportFeedback("Saved click target moved outside the settled viewport; click again.");
      return false;
    }
    moveClickMarker(point);
    submitViewportClick(point, "Clicking saved");
    return true;
  };
  const scrollMessage = (dx, dy) => {
    if (dx < 0) {
      return "Moving visual viewport left...";
    }
    if (dx > 0) {
      return "Moving visual viewport right...";
    }
    if (dy < 0) {
      return "Moving visual viewport up...";
    }
    if (dy > 0) {
      return "Moving visual viewport down...";
    }
    return "Refreshing visual viewport...";
  };
  const queuedViewportTarget = (dx, dy) => {
    const baseX = Number(shell.dataset.pendingViewportX);
    const baseY = Number(shell.dataset.pendingViewportY);
    const x = Number.isFinite(baseX) ? baseX : numberData("viewportX");
    const y = Number.isFinite(baseY) ? baseY : numberData("viewportY");
    const maxX = Math.max(0, numberData("maxScrollX"));
    const maxY = Math.max(0, numberData("maxScrollY"));
    const nextX = clamp(x + dx, 0, maxX);
    const nextY = clamp(y + dy, 0, maxY);
    return { x: nextX, y: nextY, dx: nextX - x, dy: nextY - y };
  };
  const replaceElementFromPartial = (doc, selector) => {
    const current = document.querySelector(selector);
    const next = doc.querySelector(selector);
    if (!current || !next) {
      return false;
    }
    current.replaceWith(next.cloneNode(true));
    return true;
  };
  const applyViewportPartial = (html, options = {}) => {
    if (typeof DOMParser !== "function") {
      return false;
    }
    const doc = new DOMParser().parseFromString(html, "text/html");
    const partial = doc.querySelector("[data-browser-partial-viewport]");
    const nextShell = doc.querySelector("[data-browser-viewport-scroll]");
    if (!partial || !nextShell) {
      return false;
    }
    if (options.samePageOnly && shell.dataset.pageSource && nextShell.dataset.pageSource && shell.dataset.pageSource !== nextShell.dataset.pageSource) {
      return false;
    }
    const keepFocus = document.activeElement === shell;
    const shellTopBefore = shell.getBoundingClientRect().top;
    for (const attribute of Array.from(shell.attributes)) {
      shell.removeAttribute(attribute.name);
    }
    for (const attribute of Array.from(nextShell.attributes)) {
      shell.setAttribute(attribute.name, attribute.value);
    }
    shell.innerHTML = nextShell.innerHTML;
    raster = shell.querySelector(".browser-raster");
    clickMarker = shell.querySelector("[data-browser-click-marker]");
    replaceElementFromPartial(doc, "[data-browser-viewport-status]");
    replaceElementFromPartial(doc, "[data-browser-viewport-interactions]");
    replaceElementFromPartial(doc, "[data-browser-viewport-controls]");
    replaceElementFromPartial(doc, "[data-browser-viewport-command-strip]");
    shell.dataset.viewportPartial = "true";
    clearViewportPending();
    const shellTopAfter = shell.getBoundingClientRect().top;
    const shellShift = shellTopAfter - shellTopBefore;
    if (Number.isFinite(shellShift) && shellShift) {
      window.scrollBy(0, shellShift);
    }
    if (keepFocus) {
      shell.focus({ preventScroll: true });
    }
    restoreClickMarkerAfterPartial();
    replayDeferredClickAfterPartial();
    return true;
  };
  const replaceViewportPage = (url, message) => {
    setViewportPending(message);
    if (typeof fetch !== "function" || !window.history || typeof window.history.pushState !== "function") {
      window.location.href = url.toString();
      return;
    }
    shell.dataset.viewportRequest = "true";
    try {
      sessionStorage.setItem("browserViewportAnchor", "1");
    } catch (_) {}
    const pageRequestTimeoutMs = 5000;
    const controller = typeof AbortController === "function" ? new AbortController() : null;
    const fetchOptions = {
      headers: { "X-Requested-With": "browser-viewport-scroll" }
    };
    if (controller) {
      fetchOptions.signal = controller.signal;
    }
    const timeout = controller ? window.setTimeout(() => controller.abort(), pageRequestTimeoutMs) : null;
    const clearPageTimeout = () => {
      if (timeout) {
        window.clearTimeout(timeout);
      }
    };
    fetch(url.toString(), fetchOptions).then((response) => {
      if (!response.ok) {
        throw new Error("viewport request failed");
      }
      return response.text();
    }).then((html) => {
      clearPageTimeout();
      window.history.pushState(null, "", url.toString());
      document.open();
      document.write(html);
      document.close();
    }).catch(() => {
      clearPageTimeout();
      settleViewportPageFailure(
        "Browser navigation request timed out or failed; current raster retained. Try again."
      );
    });
  };
  const settleViewportPageFailure = (message) => {
    clearViewportPending();
    shell.dataset.viewportPageError = "true";
    shell.dataset.viewportPageTimeout = "true";
    shell.dataset.viewportRecovery = "retained-shell";
    setViewportFeedback(message);
    setClickStatus(message);
    shell.focus({ preventScroll: true });
    replayDeferredClickAfterPartial();
  };
  const settleViewportPartialFailure = (message) => {
    clearViewportPending();
    shell.dataset.viewportPartialError = "true";
    shell.dataset.viewportRecovery = "retained-shell";
    setViewportFeedback(message);
    shell.focus({ preventScroll: true });
    replayDeferredClickAfterPartial();
  };
  const syncViewportHistory = () => {
    if (!window.history || typeof window.history.replaceState !== "function") {
      return;
    }
    try {
      const currentUrl = new URL(shell.dataset.scrollUrl || window.location.href, window.location.href);
      currentUrl.searchParams.set("action", "current");
      currentUrl.searchParams.set("viewport_x", String(numberData("viewportX")));
      currentUrl.searchParams.set("viewport_y", String(numberData("viewportY")));
      currentUrl.searchParams.delete("dx");
      currentUrl.searchParams.delete("dy");
      currentUrl.searchParams.delete("partial");
      window.history.replaceState(null, "", currentUrl.toString());
    } catch (_) {}
  };
  const replaceViewportPartial = (url, message, options = {}) => {
    setViewportPending(message);
    if (typeof fetch !== "function" || !window.history || typeof window.history.replaceState !== "function") {
      window.location.href = url.toString();
      return;
    }
    const partialUrl = new URL(url.toString());
    partialUrl.searchParams.set("partial", "viewport");
    const requestSeq = ++viewportRequestSeq;
    partialRequestInFlight = true;
    shell.dataset.viewportRequest = "partial";
    const partialRequestTimeoutMs = 2500;
    const controller = typeof AbortController === "function" ? new AbortController() : null;
    partialRequestController = controller;
    const fetchOptions = {
      headers: { "X-Requested-With": "browser-viewport-partial" }
    };
    if (controller) {
      fetchOptions.signal = controller.signal;
    }
    const timeout = controller ? window.setTimeout(() => controller.abort(), partialRequestTimeoutMs) : null;
    const clearPartialTimeout = () => {
      if (timeout) {
        window.clearTimeout(timeout);
      }
    };
    fetch(partialUrl.toString(), fetchOptions).then((response) => {
      if (!response.ok) {
        throw new Error("viewport partial request failed");
      }
      return response.text();
    }).then((html) => {
      if (requestSeq !== viewportRequestSeq) {
        markStaleViewportResponse("Ignored stale visual viewport update; newer scroll is pending.");
        return;
      }
      if (!applyViewportPartial(html, options)) {
        throw new Error("viewport partial response missing required fragments");
      }
      syncViewportHistory();
      setViewportFeedback("Viewport settled.");
    }).catch(() => {
      if (requestSeq !== viewportRequestSeq) {
        markStaleViewportResponse("Ignored stale visual viewport error; newer scroll is pending.");
        return;
      }
      if (typeof options.fallback === "function") {
        options.fallback();
        return;
      }
      if (pendingScrollAfterRequest && (pendingScrollDx || pendingScrollDy)) {
        setViewportFeedback("Applying latest queued scroll...");
        return;
      }
      settleViewportPartialFailure("Visual viewport update failed; current viewport retained. Scroll again to retry.");
    }).then(() => {
      clearPartialTimeout();
      if (requestSeq !== viewportRequestSeq) {
        return;
      }
      partialRequestController = null;
      partialRequestInFlight = false;
      if (pendingScrollAfterRequest && (pendingScrollDx || pendingScrollDy)) {
        pendingScrollAfterRequest = false;
        pendingScrollTimer = setTimeout(flushPendingScroll, 0);
      } else {
        pendingScrollAfterRequest = false;
        pendingScrollDx = 0;
        pendingScrollDy = 0;
      }
    });
  };
  let viewportRequestSeq = 0;
  let partialRequestInFlight = false;
  let partialRequestController = null;
  let pendingScrollAfterRequest = false;
  let pendingScrollDx = 0;
  let pendingScrollDy = 0;
  let pendingScrollTimer = null;
  const scrollFlushDelayMs = 18;
  const buildScrollUrl = (dx, dy) => {
    const x = numberData("viewportX");
    const y = numberData("viewportY");
    const maxX = Math.max(0, numberData("maxScrollX"));
    const maxY = Math.max(0, numberData("maxScrollY"));
    const nextX = clamp(x + dx, 0, maxX);
    const nextY = clamp(y + dy, 0, maxY);
    const appliedDx = nextX - x;
    const appliedDy = nextY - y;
    if (appliedDx === 0 && appliedDy === 0) {
      if (dy < 0 && y <= 0) {
        setViewportFeedback("Already at top.");
      } else if (dy > 0 && y >= maxY) {
        setViewportFeedback("Already at bottom.");
      } else if (dx < 0 && x <= 0) {
        setViewportFeedback("Already at left edge.");
      } else if (dx > 0 && x >= maxX) {
        setViewportFeedback("Already at right edge.");
      } else {
        setViewportFeedback("Viewport is already at that position.");
      }
      return null;
    }
    const url = stampCurrentViewportUrl(new URL(shell.dataset.scrollUrl, window.location.href));
    url.searchParams.set("dx", String(appliedDx));
    url.searchParams.set("dy", String(appliedDy));
    return { url, dx: appliedDx, dy: appliedDy, x: nextX, y: nextY };
  };
  const scrollDeltaFromUrl = (url) => {
    if (!url || url.searchParams.get("action") !== "scroll") {
      return null;
    }
    const dx = Number(url.searchParams.get("dx") || "0");
    const dy = Number(url.searchParams.get("dy") || "0");
    if (!Number.isFinite(dx) || !Number.isFinite(dy) || (!dx && !dy)) {
      return null;
    }
    return { dx, dy };
  };
  const queuedScrollDelta = () => {
    const targetX = Number(shell.dataset.queuedScrollTargetX);
    const targetY = Number(shell.dataset.queuedScrollTargetY);
    if (!Number.isFinite(targetX) || !Number.isFinite(targetY)) {
      return null;
    }
    return {
      dx: targetX - numberData("viewportX"),
      dy: targetY - numberData("viewportY")
    };
  };
  const viewportWorkPending = () => Boolean(pendingScrollTimer || shell.dataset.viewportPending === "true" || shell.dataset.viewportRequest);
  const cancelPendingScrollTimerForClick = () => {
    if (!pendingScrollTimer || partialRequestInFlight || shell.dataset.viewportRequest) {
      return false;
    }
    clearTimeout(pendingScrollTimer);
    pendingScrollTimer = null;
    pendingScrollAfterRequest = false;
    pendingScrollDx = 0;
    pendingScrollDy = 0;
    shell.dataset.clickCanceledPendingScroll = "true";
    clearViewportPending();
    setViewportFeedback("Scroll paused for click.");
    return true;
  };
  const abortPartialViewportRequest = () => {
    if (partialRequestController && typeof partialRequestController.abort === "function") {
      shell.dataset.viewportRequestAborted = "true";
      partialRequestController.abort();
      return true;
    }
    return false;
  };
  const flushPendingScroll = () => {
    pendingScrollTimer = null;
    if (partialRequestInFlight) {
      pendingScrollAfterRequest = true;
      const queued = queuedViewportTarget(pendingScrollDx, pendingScrollDy);
      shell.dataset.scrollQueuedDuringRequest = "true";
      shell.dataset.queuedScrollTargetX = String(queued.x);
      shell.dataset.queuedScrollTargetY = String(queued.y);
      setViewportPending(
        `Scroll queued; visual viewport target x ${queued.x}, y ${queued.y}...`,
        queued
      );
      abortPartialViewportRequest();
      return true;
    }
    const latestQueued = queuedScrollDelta();
    const scroll = latestQueued
      ? buildScrollUrl(latestQueued.dx, latestQueued.dy)
      : buildScrollUrl(pendingScrollDx, pendingScrollDy);
    pendingScrollDx = 0;
    pendingScrollDy = 0;
    if (!scroll) {
      return false;
    }
    shell.dataset.pendingViewportX = String(scroll.x);
    shell.dataset.pendingViewportY = String(scroll.y);
    shell.dataset.queuedScrollDx = String(scroll.dx);
    shell.dataset.queuedScrollDy = String(scroll.dy);
    setPendingViewportTarget(scroll);
    replaceViewportPartial(scroll.url, scrollMessage(scroll.dx, scroll.dy));
    return true;
  };
  const queueViewportScroll = (dx, dy) => {
    pendingScrollDx += dx;
    pendingScrollDy += dy;
    if (partialRequestInFlight) {
      pendingScrollAfterRequest = true;
      const queued = queuedViewportTarget(pendingScrollDx, pendingScrollDy);
      shell.dataset.scrollQueuedDuringRequest = "true";
      shell.dataset.queuedScrollDx = String(pendingScrollDx);
      shell.dataset.queuedScrollDy = String(pendingScrollDy);
      shell.dataset.queuedScrollTargetX = String(queued.x);
      shell.dataset.queuedScrollTargetY = String(queued.y);
      setViewportPending(
        `Scroll queued; visual viewport target x ${queued.x}, y ${queued.y}...`,
        queued
      );
      abortPartialViewportRequest();
      return true;
    }
    const pending = buildScrollUrl(pendingScrollDx, pendingScrollDy);
    if (!pending) {
      pendingScrollDx = 0;
      pendingScrollDy = 0;
      if (pendingScrollTimer) {
        clearTimeout(pendingScrollTimer);
        pendingScrollTimer = null;
      }
      return false;
    }
    shell.dataset.pendingViewportX = String(pending.x);
    shell.dataset.pendingViewportY = String(pending.y);
    shell.dataset.queuedScrollDx = String(pending.dx);
    shell.dataset.queuedScrollDy = String(pending.dy);
    setPendingViewportTarget(pending);
    setViewportPending(`Scrolling visual viewport to x ${pending.x}, y ${pending.y}...`, pending);
    if (pendingScrollTimer) {
      clearTimeout(pendingScrollTimer);
    }
    pendingScrollTimer = setTimeout(flushPendingScroll, scrollFlushDelayMs);
    return true;
  };
  const wheelCells = (delta, deltaMode, viewportSize) => {
    if (!delta) {
      return 0;
    }
    let units = delta / 16;
    if (deltaMode === WheelEvent.DOM_DELTA_LINE) {
      units = delta;
    } else if (deltaMode === WheelEvent.DOM_DELTA_PAGE) {
      units = delta * Math.max(1, viewportSize);
    }
    const limit = Math.max(1, Math.max(1, viewportSize));
    const magnitude = clamp(Math.round(Math.abs(units)) || 1, 1, limit);
    return Math.sign(delta) * magnitude;
  };
  shell.addEventListener("wheel", (event) => {
    const dx = wheelCells(event.deltaX, event.deltaMode, numberData("viewportWidth"));
    const dy = wheelCells(event.deltaY, event.deltaMode, numberData("viewportHeight"));
    if (dx || dy) {
      event.preventDefault();
      queueViewportScroll(dx, dy);
    }
  }, { passive: false });
  let hoverPointTimer = null;
  shell.addEventListener("mousemove", (event) => {
    const point = viewportPointFromEvent(event);
    if (!point) {
      hideClickMarker();
      setClickStatus("Click inside the rendered page image.");
      if (!viewportWorkPending()) {
        setViewportFeedback("Click missed the rendered page image.");
      }
      return;
    }
    moveClickMarker(point);
    if (hoverPointTimer) {
      return;
    }
    hoverPointTimer = setTimeout(() => {
      hoverPointTimer = null;
    }, 120);
    setClickStatus(`${pointMessage(point)}. Click.`);
    if (!viewportWorkPending()) {
      setViewportFeedback(`${pointMessage(point)}. Click.`);
    }
  });
  shell.addEventListener("mouseleave", () => {
    hideClickMarker();
    setClickStatus("Ready for page click.");
    if (!viewportWorkPending()) {
      setViewportFeedback("Ready to scroll.");
    }
  });
  shell.addEventListener("click", (event) => {
    if (event.button !== 0 || event.defaultPrevented || !raster) {
      return;
    }
    event.preventDefault();
    const point = viewportPointFromEvent(event);
    if (!point) {
      clearClickMarkerPoint();
      clearDeferredClick();
      shell.dataset.clickMissClearedDeferred = "true";
      setClickStatus("Click missed the rendered page image; move pointer inside the raster or retry with an exact point.");
      setViewportFeedback("Click missed the rendered page image; retry on a visible link/button.");
      return;
    }
    moveClickMarker(point);
    if (viewportWorkPending()) {
      if (cancelPendingScrollTimerForClick()) {
        submitViewportClick(point, "Clicking");
        return;
      }
      shell.dataset.deferredClickX = String(point.x);
      shell.dataset.deferredClickY = String(point.y);
      shell.dataset.deferredClickPageX = String(point.pageX);
      shell.dataset.deferredClickPageY = String(point.pageY);
      setClickStatus(`Saved ${pointMessage(point)} while viewport updates; clicking after it settles.`);
      setViewportFeedback(`Saved click target while viewport updates.`);
      return;
    }
    submitViewportClick(point, "Clicking");
  });
  document.addEventListener("click", (event) => {
    const eventTarget = event.target instanceof Element ? event.target : event.target && event.target.parentElement;
    const target = eventTarget && typeof eventTarget.closest === "function" ? eventTarget.closest("[data-browser-viewport-controls] a[href]") : null;
    if (!target) {
      return;
    }
    event.preventDefault();
    const targetUrl = new URL(target.href, window.location.href);
    const scrollDelta = scrollDeltaFromUrl(targetUrl);
    if (scrollDelta) {
      queueViewportScroll(scrollDelta.dx, scrollDelta.dy);
      return;
    }
    if (partialRequestInFlight || shell.dataset.viewportPending === "true") {
      setViewportFeedback("Viewport is updating; scroll after it settles.");
      return;
    }
    replaceViewportPartial(targetUrl, "Moving visual viewport...");
  });
  const keyboardDelta = (event) => {
    if (event.altKey || event.ctrlKey || event.metaKey) {
      return null;
    }
    const lineStep = Math.max(1, Math.floor(numberData("viewportHeight") / 6));
    const pageY = Math.max(1, Math.floor(numberData("viewportHeight") / 2));
    let dx = 0;
    let dy = 0;
    if (event.key === "ArrowDown") {
      dy = lineStep;
    } else if (event.key === "ArrowUp") {
      dy = -lineStep;
    } else if (event.key === "ArrowRight") {
      dx = lineStep;
    } else if (event.key === "ArrowLeft") {
      dx = -lineStep;
    } else if (event.key === "PageDown" || event.key === " ") {
      dy = event.key === " " && event.shiftKey ? -pageY : pageY;
    } else if (event.key === "PageUp") {
      dy = -pageY;
    } else if (event.key === "Home") {
      dy = numberData("viewportY") > 0 ? -numberData("viewportY") : -1;
    } else if (event.key === "End") {
      const remainingY = numberData("maxScrollY") - numberData("viewportY");
      dy = remainingY > 0 ? remainingY : 1;
    } else {
      return null;
    }
    return { dx, dy };
  };
  const handleKeyboardScroll = (event) => {
    const delta = keyboardDelta(event);
    if (!delta) {
      return false;
    }
    event.preventDefault();
    queueViewportScroll(delta.dx, delta.dy);
    return true;
  };
  const isInteractiveTarget = (target) => {
    if (!target || typeof target.closest !== "function") {
      return false;
    }
    return Boolean(target.closest("input, textarea, select, button, a, summary, [contenteditable='true']"));
  };
  shell.addEventListener("keydown", (event) => {
    handleKeyboardScroll(event);
  });
  document.addEventListener("keydown", (event) => {
    if (event.defaultPrevented || shell.contains(event.target) || isInteractiveTarget(event.target)) {
      return;
    }
    handleKeyboardScroll(event);
  });
})();
</script>"#
}

fn render_browser_session_primary_input_controls(payload: &BrowserSessionPayload) -> String {
    let Some(focused) = payload
        .focused
        .as_ref()
        .filter(|focused| form_control_is_text_editable(&focused.kind))
    else {
        return String::new();
    };
    let focused_name = if focused.name.trim().is_empty() {
        focused.kind.as_str()
    } else {
        focused.name.as_str()
    };
    let backspace_href = browser_session_action_href(
        &payload.id,
        "backspace",
        &[("count", "1".to_owned())],
        payload,
    );
    let clear_href = browser_session_action_href(&payload.id, "clear-input", &[], payload);
    let enter_href = browser_session_action_href(&payload.id, "enter", &[], payload);

    format!(
        r#"<section class="viewport-input" data-browser-primary-input><div class="meta">Focused {focused_kind} name={focused_name} value={focused_value}</div><form action="/browser" method="get">{common}<input type="hidden" name="action" value="type-text"><input id="browser-primary-type-text" type="text" name="text" placeholder="text" aria-label="Type into focused control" autofocus><button type="submit">Type</button><a href="{enter_href}">Enter</a><a href="{backspace_href}">Backspace</a><a href="{clear_href}">Clear</a></form></section>"#,
        focused_kind = html_escape::encode_text(&focused.kind),
        focused_name = html_escape::encode_text(focused_name),
        focused_value = html_escape::encode_text(&focused.value),
        common = browser_session_common_hidden_inputs(payload),
        enter_href = html_escape::encode_double_quoted_attribute(&enter_href),
        backspace_href = html_escape::encode_double_quoted_attribute(&backspace_href),
        clear_href = html_escape::encode_double_quoted_attribute(&clear_href),
    )
}

fn browser_session_state_export_payload(
    payload: &BrowserSessionPayload,
) -> BrowserSessionStateExportPayload<'_> {
    BrowserSessionStateExportPayload {
        format: "browser-session-state",
        id: &payload.id,
        title: &payload.title,
        source: &payload.source,
        viewport: BrowserSessionStateExportViewport {
            width: payload.width,
            height: payload.height,
            x: payload.viewport_x,
            y: payload.viewport_y,
            document_width: payload.document_width,
            document_height: payload.document_height,
            max_scroll_x: payload.max_scroll_x,
            max_scroll_y: payload.max_scroll_y,
            max_bytes: payload.max_bytes,
        },
        history: BrowserSessionStateExportHistory {
            len: payload.history_len,
            current_index: payload.current_history_index,
            can_back: payload.can_back,
            can_forward: payload.can_forward,
        },
        history_entries: &payload.history,
        tabs: &payload.sessions,
        closed_sessions: &payload.closed_sessions,
        bookmarks: &payload.bookmarks,
        profile_history: &payload.profile_history,
        anchors: &payload.anchors,
        links: &payload.links,
        forms: &payload.forms,
        resources: &payload.resources,
        focused: payload.focused.as_ref(),
        find: BrowserSessionStateExportFind {
            query: &payload.find_query,
            match_count: payload.find_match_count,
            current_index: payload.find_current_index,
            current_line: payload.find_current_line,
            current_column: payload.find_current_column,
            matches: &payload.find_matches,
        },
        tab_search: BrowserSessionStateExportTabSearch {
            query: &payload.tab_search_query,
            result_count: payload.tab_search_results.len(),
            results: &payload.tab_search_results,
        },
        resource_report: payload.resource_report.as_ref().map(|report| {
            BrowserSessionStateExportResourceReport {
                action: &report.action,
                page_source: &report.page_source,
                total: report.total,
                fetched: report.fetched,
                cached: report.cached,
                failed: report.failed,
                skipped: report.skipped,
                applied: report.applied,
                decoded: report.decoded,
                resources: report.resources.len(),
                fetches: &report.resources,
                csv_url: browser_session_api_href(&payload.id, "resource-report-csv", payload),
                clear_url: browser_session_action_href(
                    &payload.id,
                    "clear-resource-report",
                    &[],
                    payload,
                ),
            }
        }),
        profile: BrowserSessionStateExportProfile {
            enabled: payload.profile_enabled,
            error: payload.profile_error.as_deref(),
            current_bookmarked: payload.current_bookmarked,
        },
        counts: BrowserSessionStateExportCounts {
            open_sessions: payload.sessions.len(),
            pinned_tabs: payload
                .sessions
                .iter()
                .filter(|session| session.pinned)
                .count(),
            closed_sessions: payload.closed_sessions.len(),
            bookmarks: payload.bookmarks.len(),
            profile_history: payload.profile_history.len(),
            history: payload.history_len,
            anchors: payload.anchor_count,
            links: payload.link_count,
            forms: payload.form_count,
            find_matches: payload.find_match_count,
            tab_search_results: payload.tab_search_results.len(),
            dom_nodes: payload.dom_node_count,
            resources: payload.resource_count,
            resource_images: payload.resource_image_count,
            resource_stylesheets: payload.resource_stylesheet_count,
            resource_scripts: payload.resource_script_count,
            resource_others: payload.resource_count.saturating_sub(
                payload.resource_image_count
                    + payload.resource_stylesheet_count
                    + payload.resource_script_count,
            ),
            cookies: payload.cookies.len(),
            local_storage: payload.local_storage.len(),
            session_storage: payload.session_storage.len(),
        },
        clear_urls: BrowserSessionStateExportClearUrls {
            cookies: (!payload.cookies.is_empty())
                .then(|| browser_session_action_href(&payload.id, "clear-cookies", &[], payload)),
            local_storage: (!payload.local_storage.is_empty()).then(|| {
                browser_session_action_href(&payload.id, "clear-local-storage", &[], payload)
            }),
            session_storage: (!payload.session_storage.is_empty()).then(|| {
                browser_session_action_href(&payload.id, "clear-session-storage", &[], payload)
            }),
            bookmarks: payload.bookmarks_clear_url.as_deref(),
            closed_sessions: payload.closed_sessions_clear_url.as_deref(),
            profile_tabs: payload.profile_tabs_clear_url.as_deref(),
            profile_history: payload.profile_history_clear_url.as_deref(),
        },
        export_urls: BrowserSessionStateExportUrls {
            payload_json: browser_session_api_href(&payload.id, "json", payload),
            session_state_json: browser_session_api_href(&payload.id, "session-state", payload),
            session_state_csv: browser_session_api_href(&payload.id, "session-state-csv", payload),
            tabs_csv: browser_session_api_href(&payload.id, "tabs-csv", payload),
            closed_sessions_csv: browser_session_api_href(
                &payload.id,
                "closed-sessions-csv",
                payload,
            ),
            bookmarks_csv: browser_session_api_href(&payload.id, "bookmarks-csv", payload),
            anchors_csv: browser_session_api_href(&payload.id, "anchors-csv", payload),
            links_csv: browser_session_api_href(&payload.id, "links-csv", payload),
            forms_json: browser_session_api_href(&payload.id, "forms-json", payload),
            forms_csv: browser_session_api_href(&payload.id, "forms-csv", payload),
            history_csv: browser_session_api_href(&payload.id, "history-csv", payload),
            profile_history_csv: browser_session_api_href(
                &payload.id,
                "profile-history-csv",
                payload,
            ),
            resources_json: browser_session_api_href(&payload.id, "resources-json", payload),
            resources_csv: browser_session_api_href(&payload.id, "resources-csv", payload),
            resource_report_json: browser_session_api_href(
                &payload.id,
                "resource-report-json",
                payload,
            ),
            resource_report_csv: browser_session_api_href(
                &payload.id,
                "resource-report-csv",
                payload,
            ),
            find_json: browser_session_api_href(&payload.id, "find-json", payload),
            find_csv: browser_session_api_href(&payload.id, "find-csv", payload),
            tab_search_json: browser_session_api_href(&payload.id, "tab-search-json", payload),
            tab_search_csv: browser_session_api_href(&payload.id, "tab-search-csv", payload),
            viewport_text: browser_session_api_href(&payload.id, "viewport-text", payload),
            page_text: browser_session_api_href(&payload.id, "page-text", payload),
        },
        action_urls: browser_session_state_action_urls(payload),
        cookies: &payload.cookies,
        local_storage: &payload.local_storage,
        session_storage: &payload.session_storage,
    }
}

fn browser_session_resource_report_export_payload(
    payload: &BrowserSessionPayload,
) -> BrowserSessionResourceReportExportPayload<'_> {
    BrowserSessionResourceReportExportPayload {
        format: "browser-resource-report",
        id: &payload.id,
        title: &payload.title,
        source: &payload.source,
        resource_report: payload.resource_report.as_ref(),
        csv_url: browser_session_api_href(&payload.id, "resource-report-csv", payload),
        clear_url: payload.resource_report.as_ref().map(|_| {
            browser_session_action_href(&payload.id, "clear-resource-report", &[], payload)
        }),
    }
}

fn browser_session_resources_export_payload(
    payload: &BrowserSessionPayload,
) -> BrowserSessionResourcesExportPayload<'_> {
    BrowserSessionResourcesExportPayload {
        format: "browser-resources",
        id: &payload.id,
        title: &payload.title,
        source: &payload.source,
        resource_count: payload.resource_count,
        displayed_resource_count: payload.resources.len(),
        image_count: payload.resource_image_count,
        stylesheet_count: payload.resource_stylesheet_count,
        script_count: payload.resource_script_count,
        other_count: payload.resource_count.saturating_sub(
            payload.resource_image_count
                + payload.resource_stylesheet_count
                + payload.resource_script_count,
        ),
        resources: &payload.resources,
        action_urls: browser_session_resource_action_urls(payload),
        csv_url: browser_session_api_href(&payload.id, "resources-csv", payload),
        session_state_url: browser_session_api_href(&payload.id, "session-state", payload),
    }
}

fn browser_session_resource_action_urls(
    payload: &BrowserSessionPayload,
) -> BrowserSessionResourceActionUrls {
    let can_load_images = browser_session_should_offer_load_images(payload);
    let can_make_visual = payload.resource_stylesheet_count > 0 || can_load_images;
    BrowserSessionResourceActionUrls {
        fetch_resources: (payload.resource_count > 0)
            .then(|| browser_session_action_href(&payload.id, "fetch-resources", &[], payload)),
        make_visual: can_make_visual
            .then(|| browser_session_action_href(&payload.id, "make-visual", &[], payload)),
        apply_stylesheets: (payload.resource_stylesheet_count > 0)
            .then(|| browser_session_action_href(&payload.id, "apply-styles", &[], payload)),
        run_scripts: (payload.resource_script_count > 0)
            .then(|| browser_session_action_href(&payload.id, "run-scripts", &[], payload)),
        load_images: can_load_images
            .then(|| browser_session_action_href(&payload.id, "load-images", &[], payload)),
        clear_resource_report: payload.resource_report.as_ref().map(|_| {
            browser_session_action_href(&payload.id, "clear-resource-report", &[], payload)
        }),
    }
}

fn browser_session_should_offer_load_images(payload: &BrowserSessionPayload) -> bool {
    if payload.resource_image_count == 0 {
        return false;
    }
    let Some(report) = payload.resource_report.as_ref() else {
        return true;
    };
    match report.decoded {
        Some(0) => false,
        Some(decoded) => decoded < payload.resource_image_count,
        None => true,
    }
}

fn browser_session_forms_export_payload(
    payload: &BrowserSessionPayload,
) -> BrowserSessionFormsExportPayload<'_> {
    BrowserSessionFormsExportPayload {
        format: "browser-forms",
        id: &payload.id,
        title: &payload.title,
        source: &payload.source,
        form_count: payload.form_count,
        forms: &payload.forms,
        csv_url: browser_session_api_href(&payload.id, "forms-csv", payload),
        session_state_url: browser_session_api_href(&payload.id, "session-state", payload),
    }
}

fn browser_session_find_export_payload(
    payload: &BrowserSessionPayload,
) -> BrowserSessionFindExportPayload<'_> {
    BrowserSessionFindExportPayload {
        format: "browser-find",
        id: &payload.id,
        title: &payload.title,
        source: &payload.source,
        query: &payload.find_query,
        match_count: payload.find_match_count,
        current_index: payload.find_current_index,
        current_line: payload.find_current_line,
        current_column: payload.find_current_column,
        matches: &payload.find_matches,
        csv_url: browser_session_api_href(&payload.id, "find-csv", payload),
        session_state_url: browser_session_api_href(&payload.id, "session-state", payload),
    }
}

fn browser_session_tab_search_export_payload(
    payload: &BrowserSessionPayload,
) -> BrowserSessionTabSearchExportPayload<'_> {
    BrowserSessionTabSearchExportPayload {
        format: "browser-tab-search",
        id: &payload.id,
        title: &payload.title,
        source: &payload.source,
        query: &payload.tab_search_query,
        result_count: payload.tab_search_results.len(),
        results: &payload.tab_search_results,
        action_urls: browser_session_tab_search_export_action_urls(payload),
        csv_url: browser_session_api_href(&payload.id, "tab-search-csv", payload),
        session_state_url: browser_session_api_href(&payload.id, "session-state", payload),
    }
}

fn browser_session_tab_search_export_action_urls(
    payload: &BrowserSessionPayload,
) -> BrowserSessionTabSearchExportActionUrls {
    let action_urls = browser_session_state_action_urls(payload);
    BrowserSessionTabSearchExportActionUrls {
        move_tab_search_results_front: action_urls.move_tab_search_results_front,
        move_tab_search_results_back: action_urls.move_tab_search_results_back,
        duplicate_tab_search_results: action_urls.duplicate_tab_search_results,
        bookmark_tab_search_results: action_urls.bookmark_tab_search_results,
        remove_tab_search_bookmarks: action_urls.remove_tab_search_bookmarks,
        clear_tab_search: action_urls.clear_tab_search,
        reload_tab_search_results: action_urls.reload_tab_search_results,
        close_tab_search_results: action_urls.close_tab_search_results,
        close_tab_search_nonmatches: action_urls.close_tab_search_nonmatches,
        pin_tab_search_results: action_urls.pin_tab_search_results,
        unpin_tab_search_results: action_urls.unpin_tab_search_results,
        label_tab_search_results: action_urls.label_tab_search_results,
        clear_tab_search_labels: action_urls.clear_tab_search_labels,
    }
}

fn browser_session_state_action_urls(
    payload: &BrowserSessionPayload,
) -> BrowserSessionStateExportActionUrls {
    let current_tab = payload
        .sessions
        .iter()
        .find(|session| session.id == payload.id);
    let tab_search_label = (!payload.tab_search_results.is_empty())
        .then(|| normalize_browser_tab_label_option(Some(&payload.tab_search_query)))
        .flatten();
    let resource_action_urls = browser_session_resource_action_urls(payload);
    BrowserSessionStateExportActionUrls {
        back: payload
            .can_back
            .then(|| browser_session_action_href(&payload.id, "back", &[], payload)),
        forward: payload
            .can_forward
            .then(|| browser_session_action_href(&payload.id, "forward", &[], payload)),
        reload: browser_session_action_href(&payload.id, "reload", &[], payload),
        top: (payload.viewport_y > 0)
            .then(|| browser_session_action_href(&payload.id, "top", &[], payload)),
        bottom: (payload.viewport_y < payload.max_scroll_y)
            .then(|| browser_session_action_href(&payload.id, "bottom", &[], payload)),
        page_up: (payload.viewport_y > 0)
            .then(|| browser_session_action_href(&payload.id, "page-up", &[], payload)),
        page_down: (payload.viewport_y < payload.max_scroll_y)
            .then(|| browser_session_action_href(&payload.id, "page-down", &[], payload)),
        line_up: (payload.viewport_y > 0)
            .then(|| browser_session_action_href(&payload.id, "line-up", &[], payload)),
        line_down: (payload.viewport_y < payload.max_scroll_y)
            .then(|| browser_session_action_href(&payload.id, "line-down", &[], payload)),
        scroll_up: (payload.viewport_y > 0).then(|| {
            browser_session_action_href(
                &payload.id,
                "scroll",
                &[("dy", format!("-{}", payload.height.max(1) / 2))],
                payload,
            )
        }),
        scroll_down: (payload.viewport_y < payload.max_scroll_y).then(|| {
            browser_session_action_href(
                &payload.id,
                "scroll",
                &[("dy", (payload.height.max(1) / 2).to_string())],
                payload,
            )
        }),
        scroll_left: (payload.viewport_x > 0).then(|| {
            browser_session_action_href(
                &payload.id,
                "scroll",
                &[("dx", format!("-{}", payload.width.max(1) / 2))],
                payload,
            )
        }),
        scroll_right: (payload.viewport_x < payload.max_scroll_x).then(|| {
            browser_session_action_href(
                &payload.id,
                "scroll",
                &[("dx", (payload.width.max(1) / 2).to_string())],
                payload,
            )
        }),
        previous_tab: (payload.sessions.len() > 1)
            .then(|| browser_session_action_href(&payload.id, "previous-tab", &[], payload)),
        next_tab: (payload.sessions.len() > 1)
            .then(|| browser_session_action_href(&payload.id, "next-tab", &[], payload)),
        move_tab_left: current_tab
            .filter(|session| session.can_move_left)
            .map(|session| session.move_left_url.clone()),
        move_tab_right: current_tab
            .filter(|session| session.can_move_right)
            .map(|session| session.move_right_url.clone()),
        move_tab_search_results_front: browser_tab_search_results_can_move(payload, true).then(
            || {
                browser_session_action_href(
                    &payload.id,
                    "move-tab-search-results-front",
                    &[],
                    payload,
                )
            },
        ),
        move_tab_search_results_back: browser_tab_search_results_can_move(payload, false).then(
            || {
                browser_session_action_href(
                    &payload.id,
                    "move-tab-search-results-back",
                    &[],
                    payload,
                )
            },
        ),
        duplicate_tab: browser_session_action_href(
            &payload.id,
            "duplicate-session",
            &[("session", payload.id.clone())],
            payload,
        ),
        duplicate_tab_background: browser_session_action_href(
            &payload.id,
            "duplicate-background-session",
            &[("session", payload.id.clone())],
            payload,
        ),
        duplicate_tab_search_results: (!payload.tab_search_results.is_empty()).then(|| {
            browser_session_action_href(&payload.id, "duplicate-tab-search-results", &[], payload)
        }),
        close_tab: (payload.sessions.len() > 1).then(|| {
            browser_session_action_href(
                &payload.id,
                "close-session",
                &[("close_id", payload.id.clone())],
                payload,
            )
        }),
        close_other_tabs: (payload.sessions.len() > 1)
            .then(|| browser_session_action_href(&payload.id, "close-other-tabs", &[], payload)),
        close_unpinned_tabs: payload
            .sessions
            .iter()
            .any(|session| !session.current && !session.pinned)
            .then(|| browser_session_action_href(&payload.id, "close-unpinned-tabs", &[], payload)),
        pin_all_tabs: payload
            .sessions
            .iter()
            .any(|session| !session.pinned)
            .then(|| browser_session_action_href(&payload.id, "pin-all-tabs", &[], payload)),
        unpin_all_tabs: payload
            .sessions
            .iter()
            .any(|session| session.pinned)
            .then(|| browser_session_action_href(&payload.id, "unpin-all-tabs", &[], payload)),
        add_bookmark: (!payload.current_bookmarked)
            .then(|| browser_session_action_href(&payload.id, "add-bookmark", &[], payload)),
        bookmark_all_tabs: browser_has_unbookmarked_open_tabs(payload)
            .then(|| browser_session_action_href(&payload.id, "bookmark-all-tabs", &[], payload)),
        bookmark_profile_history: browser_has_unbookmarked_profile_history(payload).then(|| {
            browser_session_action_href(&payload.id, "bookmark-profile-history", &[], payload)
        }),
        remove_profile_history_bookmarks: browser_has_bookmarked_profile_history(payload).then(
            || {
                browser_session_action_href(
                    &payload.id,
                    "remove-profile-history-bookmarks",
                    &[],
                    payload,
                )
            },
        ),
        bookmark_tab_search_results: payload
            .tab_search_results
            .iter()
            .any(|result| {
                !result.source.trim().is_empty()
                    && !payload
                        .bookmarks
                        .iter()
                        .any(|bookmark| bookmark.source == result.source)
            })
            .then(|| {
                browser_session_action_href(
                    &payload.id,
                    "bookmark-tab-search-results",
                    &[],
                    payload,
                )
            }),
        remove_tab_search_bookmarks: browser_tab_search_has_bookmarked_results(payload).then(
            || {
                browser_session_action_href(
                    &payload.id,
                    "remove-tab-search-bookmarks",
                    &[],
                    payload,
                )
            },
        ),
        open_bookmarks_new_sessions: (!payload.bookmarks.is_empty()).then(|| {
            browser_session_action_href(&payload.id, "open-bookmarks-new-sessions", &[], payload)
        }),
        open_bookmarks_background: payload.bookmarks_background_url.clone(),
        open_links_new_sessions: (!payload.links.is_empty()).then(|| {
            browser_session_action_href(
                &payload.id,
                "open-links-new-sessions",
                &[("limit", DEFAULT_BULK_BACKGROUND_LINKS.to_string())],
                payload,
            )
        }),
        open_links_background: payload.links_background_url.clone(),
        open_resources_new_sessions: (!payload.resources.is_empty()).then(|| {
            browser_session_action_href(
                &payload.id,
                "open-resources-new-sessions",
                &[("limit", DEFAULT_BULK_BACKGROUND_LINKS.to_string())],
                payload,
            )
        }),
        open_resources_background: (!payload.resources.is_empty()).then(|| {
            browser_session_action_href(
                &payload.id,
                "open-resources-background-sessions",
                &[("limit", DEFAULT_BULK_BACKGROUND_LINKS.to_string())],
                payload,
            )
        }),
        open_find_matches_new_sessions: payload
            .find_matches
            .iter()
            .any(|match_| !match_.current)
            .then(|| {
                browser_session_action_href(
                    &payload.id,
                    "open-find-matches-new-sessions",
                    &[("limit", DEFAULT_BULK_BACKGROUND_LINKS.to_string())],
                    payload,
                )
            }),
        open_find_matches_background: payload
            .find_matches
            .iter()
            .any(|match_| !match_.current)
            .then(|| {
                browser_session_action_href(
                    &payload.id,
                    "open-find-matches-background-sessions",
                    &[("limit", DEFAULT_BULK_BACKGROUND_LINKS.to_string())],
                    payload,
                )
            }),
        open_profile_history_new_sessions: (!payload.profile_history.is_empty()).then(|| {
            browser_session_action_href(
                &payload.id,
                "open-profile-history-new-sessions",
                &[("limit", DEFAULT_BULK_BACKGROUND_LINKS.to_string())],
                payload,
            )
        }),
        open_profile_history_background: (!payload.profile_history.is_empty()).then(|| {
            browser_session_action_href(
                &payload.id,
                "open-profile-history-background-sessions",
                &[("limit", DEFAULT_BULK_BACKGROUND_LINKS.to_string())],
                payload,
            )
        }),
        bookmark_page_links: browser_has_unbookmarked_page_links(payload)
            .then(|| browser_session_action_href(&payload.id, "bookmark-page-links", &[], payload)),
        remove_page_link_bookmarks: browser_has_bookmarked_page_links(payload).then(|| {
            browser_session_action_href(&payload.id, "remove-page-link-bookmarks", &[], payload)
        }),
        restore_closed_background_sessions: (!payload.closed_sessions.is_empty()).then(|| {
            browser_session_action_href(&payload.id, "restore-all-closed-background", &[], payload)
        }),
        clear_find: (!payload.find_query.trim().is_empty() || payload.find_match_count > 0)
            .then(|| browser_session_action_href(&payload.id, "clear-find", &[], payload)),
        clear_tab_search: (!payload.tab_search_query.trim().is_empty()
            || !payload.tab_search_results.is_empty())
        .then(|| browser_session_action_href(&payload.id, "clear-tab-search", &[], payload)),
        reload_tab_search_results: (!payload.tab_search_results.is_empty()).then(|| {
            browser_session_action_href(&payload.id, "reload-tab-search-results", &[], payload)
        }),
        close_tab_search_results: payload
            .tab_search_results
            .iter()
            .any(|result| !result.current && !result.pinned)
            .then(|| {
                browser_session_action_href(&payload.id, "close-tab-search-results", &[], payload)
            }),
        close_tab_search_nonmatches: browser_tab_search_has_closeable_nonmatches(payload).then(
            || {
                browser_session_action_href(
                    &payload.id,
                    "close-tab-search-nonmatches",
                    &[],
                    payload,
                )
            },
        ),
        pin_tab_search_results: payload
            .tab_search_results
            .iter()
            .any(|result| !result.pinned)
            .then(|| {
                browser_session_action_href(&payload.id, "pin-tab-search-results", &[], payload)
            }),
        unpin_tab_search_results: payload
            .tab_search_results
            .iter()
            .any(|result| result.pinned)
            .then(|| {
                browser_session_action_href(&payload.id, "unpin-tab-search-results", &[], payload)
            }),
        label_tab_search_results: tab_search_label.map(|label| {
            browser_session_action_href(
                &payload.id,
                "label-tab-search-results",
                &[("label", label)],
                payload,
            )
        }),
        clear_tab_search_labels: payload
            .tab_search_results
            .iter()
            .any(|result| result.label.is_some())
            .then(|| {
                browser_session_action_href(&payload.id, "clear-tab-search-labels", &[], payload)
            }),
        fetch_resources: resource_action_urls.fetch_resources,
        make_visual: resource_action_urls.make_visual,
        apply_stylesheets: resource_action_urls.apply_stylesheets,
        run_scripts: resource_action_urls.run_scripts,
        load_images: resource_action_urls.load_images,
        clear_resource_report: resource_action_urls.clear_resource_report,
    }
}

fn browser_tab_search_results_can_move(payload: &BrowserSessionPayload, to_front: bool) -> bool {
    if payload.tab_search_results.is_empty() {
        return false;
    }
    let match_ids = payload
        .tab_search_results
        .iter()
        .map(|result| result.id.as_str())
        .collect::<HashSet<_>>();
    if match_ids.is_empty() {
        return false;
    }
    let mut seen_match = false;
    let mut seen_non_match = false;
    for session in &payload.sessions {
        let is_match = match_ids.contains(session.id.as_str());
        if to_front {
            if is_match && seen_non_match {
                return true;
            }
            seen_non_match |= !is_match;
        } else {
            if !is_match && seen_match {
                return true;
            }
            seen_match |= is_match;
        }
    }
    false
}

fn browser_tab_search_has_closeable_nonmatches(payload: &BrowserSessionPayload) -> bool {
    if payload.tab_search_results.is_empty() {
        return false;
    }
    let match_ids = payload
        .tab_search_results
        .iter()
        .map(|result| result.id.as_str())
        .collect::<HashSet<_>>();
    payload.sessions.iter().any(|session| {
        !session.current && !session.pinned && !match_ids.contains(session.id.as_str())
    })
}

fn browser_tab_search_has_bookmarked_results(payload: &BrowserSessionPayload) -> bool {
    payload.tab_search_results.iter().any(|result| {
        !result.source.trim().is_empty()
            && payload
                .bookmarks
                .iter()
                .any(|bookmark| bookmark.source == result.source)
    })
}

fn browser_has_unbookmarked_open_tabs(payload: &BrowserSessionPayload) -> bool {
    payload.sessions.iter().any(|session| {
        !session.source.trim().is_empty()
            && !payload
                .bookmarks
                .iter()
                .any(|bookmark| bookmark.source == session.source)
    })
}

fn browser_has_unbookmarked_profile_history(payload: &BrowserSessionPayload) -> bool {
    payload.profile_history.iter().any(|entry| {
        !entry.source.trim().is_empty()
            && !payload
                .bookmarks
                .iter()
                .any(|bookmark| bookmark.source == entry.source)
    })
}

fn browser_has_bookmarked_profile_history(payload: &BrowserSessionPayload) -> bool {
    payload.profile_history.iter().any(|entry| {
        !entry.source.trim().is_empty()
            && payload
                .bookmarks
                .iter()
                .any(|bookmark| bookmark.source == entry.source)
    })
}

fn browser_has_unbookmarked_page_links(payload: &BrowserSessionPayload) -> bool {
    payload.links.iter().any(|link| {
        !link.url.trim().is_empty()
            && !payload
                .bookmarks
                .iter()
                .any(|bookmark| bookmark.source == link.url)
    })
}

fn browser_has_bookmarked_page_links(payload: &BrowserSessionPayload) -> bool {
    payload.links.iter().any(|link| {
        !link.url.trim().is_empty()
            && payload
                .bookmarks
                .iter()
                .any(|bookmark| bookmark.source == link.url)
    })
}

fn browser_session_state_csv_response(payload: &BrowserSessionPayload) -> HttpResponse {
    HttpResponse {
        status: 200,
        reason: "OK",
        content_type: "text/csv; charset=utf-8",
        body: browser_session_state_csv(payload),
    }
}

fn browser_session_state_csv(payload: &BrowserSessionPayload) -> String {
    let mut csv = String::new();
    browser_session_push_csv_row(
        &mut csv,
        &[
            "kind",
            "origin",
            "name",
            "key",
            "value",
            "domain",
            "path",
            "flags",
            "clear_url",
            "session_id",
            "source",
        ],
    );
    if let Some(focused) = payload.focused.as_ref() {
        let form_index = focused.form_index.to_string();
        let control_index = focused.control_index.to_string();
        let key = format!("form={form_index}; control={control_index}");
        let clear_focused = browser_session_action_href(&payload.id, "clear-input", &[], payload);
        browser_session_push_csv_row(
            &mut csv,
            &[
                "focused",
                "",
                &focused.name,
                &key,
                &focused.value,
                &focused.kind,
                "",
                "",
                &clear_focused,
                &payload.id,
                &payload.source,
            ],
        );
    }
    if !payload.find_query.trim().is_empty() || payload.find_match_count > 0 {
        let match_count = payload.find_match_count.to_string();
        let current_index = payload
            .find_current_index
            .map(|index| (index + 1).to_string())
            .unwrap_or_default();
        let current_line = payload
            .find_current_line
            .map(|line| (line + 1).to_string())
            .unwrap_or_default();
        let current_column = payload
            .find_current_column
            .map(|column| (column + 1).to_string())
            .unwrap_or_default();
        let clear_find = browser_session_action_href(&payload.id, "clear-find", &[], payload);
        browser_session_push_csv_row(
            &mut csv,
            &[
                "find",
                "",
                &payload.find_query,
                "match_count",
                &match_count,
                &current_index,
                &current_line,
                &current_column,
                &clear_find,
                &payload.id,
                &payload.source,
            ],
        );
    }
    if !payload.tab_search_query.trim().is_empty() || !payload.tab_search_results.is_empty() {
        let result_count = payload.tab_search_results.len().to_string();
        let clear_tab_search =
            browser_session_action_href(&payload.id, "clear-tab-search", &[], payload);
        browser_session_push_csv_row(
            &mut csv,
            &[
                "tab-search",
                "",
                &payload.tab_search_query,
                "result_count",
                &result_count,
                "",
                "",
                "",
                &clear_tab_search,
                &payload.id,
                &payload.source,
            ],
        );
    }
    let clear_cookies = browser_session_action_href(&payload.id, "clear-cookies", &[], payload);
    for cookie in &payload.cookies {
        browser_session_push_csv_row(
            &mut csv,
            &[
                "cookie",
                "",
                &cookie.name,
                "",
                &cookie.value,
                &cookie.domain,
                &cookie.path,
                &browser_cookie_flags(cookie),
                &clear_cookies,
                &payload.id,
                &payload.source,
            ],
        );
    }

    let clear_local_storage =
        browser_session_action_href(&payload.id, "clear-local-storage", &[], payload);
    for entry in &payload.local_storage {
        browser_session_push_csv_row(
            &mut csv,
            &[
                "localStorage",
                &entry.origin,
                "",
                &entry.key,
                &entry.value,
                "",
                "",
                "",
                &clear_local_storage,
                &payload.id,
                &payload.source,
            ],
        );
    }

    let clear_session_storage =
        browser_session_action_href(&payload.id, "clear-session-storage", &[], payload);
    for entry in &payload.session_storage {
        browser_session_push_csv_row(
            &mut csv,
            &[
                "sessionStorage",
                &entry.origin,
                "",
                &entry.key,
                &entry.value,
                "",
                "",
                "",
                &clear_session_storage,
                &payload.id,
                &payload.source,
            ],
        );
    }
    csv
}

fn browser_session_push_csv_row(csv: &mut String, fields: &[&str]) {
    for (index, field) in fields.iter().enumerate() {
        if index > 0 {
            csv.push(',');
        }
        browser_session_push_csv_field(csv, field);
    }
    csv.push('\n');
}

fn browser_session_push_csv_field(csv: &mut String, field: &str) {
    if field.contains(',') || field.contains('"') || field.contains('\n') || field.contains('\r') {
        csv.push('"');
        for character in field.chars() {
            if character == '"' {
                csv.push('"');
            }
            csv.push(character);
        }
        csv.push('"');
    } else {
        csv.push_str(field);
    }
}

fn browser_session_tabs_csv_response(payload: &BrowserSessionPayload) -> HttpResponse {
    HttpResponse {
        status: 200,
        reason: "OK",
        content_type: "text/csv; charset=utf-8",
        body: browser_session_tabs_csv(payload),
    }
}

fn browser_session_tabs_csv(payload: &BrowserSessionPayload) -> String {
    let mut csv = String::new();
    browser_session_push_csv_row(
        &mut csv,
        &[
            "id",
            "position",
            "order",
            "title",
            "page_title",
            "label",
            "source",
            "current",
            "pinned",
            "can_close",
            "can_move_left",
            "can_move_right",
            "action_url",
            "reload_url",
            "move_left_url",
            "move_right_url",
            "duplicate_url",
            "duplicate_background_url",
            "label_url",
            "clear_label_url",
            "pin_url",
            "unpin_url",
            "close_url",
            "active_session_id",
            "back_href",
        ],
    );
    for session in &payload.sessions {
        let position = session.position.to_string();
        let order = session.order.to_string();
        let current = if session.current { "true" } else { "false" };
        let pinned = if session.pinned { "true" } else { "false" };
        let can_close = if session.can_close { "true" } else { "false" };
        let can_move_left = if session.can_move_left {
            "true"
        } else {
            "false"
        };
        let can_move_right = if session.can_move_right {
            "true"
        } else {
            "false"
        };
        browser_session_push_csv_row(
            &mut csv,
            &[
                &session.id,
                &position,
                &order,
                &session.title,
                &session.page_title,
                session.label.as_deref().unwrap_or(""),
                &session.source,
                current,
                pinned,
                can_close,
                can_move_left,
                can_move_right,
                &session.action_url,
                &session.reload_url,
                &session.move_left_url,
                &session.move_right_url,
                &session.duplicate_url,
                &session.duplicate_background_url,
                &session.label_url,
                &session.clear_label_url,
                &session.pin_url,
                &session.unpin_url,
                &session.close_url,
                &payload.id,
                &payload.back_href,
            ],
        );
    }
    csv
}

fn browser_session_tab_search_csv_response(payload: &BrowserSessionPayload) -> HttpResponse {
    HttpResponse {
        status: 200,
        reason: "OK",
        content_type: "text/csv; charset=utf-8",
        body: browser_session_tab_search_csv(payload),
    }
}

fn browser_session_tab_search_json_response(payload: &BrowserSessionPayload) -> HttpResponse {
    json_response(
        200,
        "OK",
        &browser_session_tab_search_export_payload(payload),
    )
}

fn browser_session_tab_search_csv(payload: &BrowserSessionPayload) -> String {
    let mut csv = String::new();
    browser_session_push_csv_row(
        &mut csv,
        &[
            "session_id",
            "title",
            "page_title",
            "label",
            "source",
            "current",
            "pinned",
            "field",
            "line",
            "text",
            "action_url",
            "reload_url",
            "duplicate_url",
            "duplicate_background_url",
            "pin_url",
            "unpin_url",
            "close_url",
            "active_session_id",
            "query",
            "result_count",
        ],
    );
    let result_count = payload.tab_search_results.len().to_string();
    for result in &payload.tab_search_results {
        let current = if result.current { "true" } else { "false" };
        let pinned = if result.pinned { "true" } else { "false" };
        let line = result
            .line
            .map(|line| (line + 1).to_string())
            .unwrap_or_default();
        browser_session_push_csv_row(
            &mut csv,
            &[
                &result.id,
                &result.title,
                &result.page_title,
                result.label.as_deref().unwrap_or(""),
                &result.source,
                current,
                pinned,
                &result.field,
                &line,
                &result.text,
                &result.action_url,
                &result.reload_url,
                &result.duplicate_url,
                &result.duplicate_background_url,
                &result.pin_url,
                &result.unpin_url,
                &result.close_url,
                &payload.id,
                &payload.tab_search_query,
                &result_count,
            ],
        );
    }
    csv
}

fn browser_session_closed_sessions_csv_response(payload: &BrowserSessionPayload) -> HttpResponse {
    HttpResponse {
        status: 200,
        reason: "OK",
        content_type: "text/csv; charset=utf-8",
        body: browser_session_closed_sessions_csv(payload),
    }
}

fn browser_session_closed_sessions_csv(payload: &BrowserSessionPayload) -> String {
    let mut csv = String::new();
    browser_session_push_csv_row(
        &mut csv,
        &[
            "id",
            "title",
            "source",
            "persisted",
            "closed_at_unix_secs",
            "closed_at",
            "restore_url",
            "new_session_url",
            "background_restore_url",
            "forget_url",
            "session_id",
            "active_source",
            "closed_count",
        ],
    );
    let closed_count = payload.closed_sessions.len().to_string();
    for closed in &payload.closed_sessions {
        let persisted = if closed.persisted { "true" } else { "false" };
        let closed_at_unix_secs = closed.closed_at_unix_secs.to_string();
        browser_session_push_csv_row(
            &mut csv,
            &[
                &closed.id,
                &closed.title,
                &closed.source,
                persisted,
                &closed_at_unix_secs,
                &closed.closed_at,
                &closed.restore_url,
                &closed.new_session_url,
                &closed.background_restore_url,
                &closed.forget_url,
                &payload.id,
                &payload.source,
                &closed_count,
            ],
        );
    }
    csv
}

fn browser_session_bookmarks_csv_response(payload: &BrowserSessionPayload) -> HttpResponse {
    HttpResponse {
        status: 200,
        reason: "OK",
        content_type: "text/csv; charset=utf-8",
        body: browser_session_bookmarks_csv(payload),
    }
}

fn browser_session_bookmarks_csv(payload: &BrowserSessionPayload) -> String {
    let mut csv = String::new();
    browser_session_push_csv_row(
        &mut csv,
        &[
            "id",
            "title",
            "source",
            "current",
            "action_url",
            "new_session_url",
            "background_session_url",
            "rename_url",
            "remove_url",
            "session_id",
            "active_source",
            "bookmark_count",
        ],
    );
    let bookmark_count = payload.bookmarks.len().to_string();
    for bookmark in &payload.bookmarks {
        let current = if bookmark.current { "true" } else { "false" };
        browser_session_push_csv_row(
            &mut csv,
            &[
                &bookmark.id,
                &bookmark.title,
                &bookmark.source,
                current,
                &bookmark.action_url,
                &bookmark.new_session_url,
                &bookmark.background_session_url,
                &bookmark.rename_url,
                &bookmark.remove_url,
                &payload.id,
                &payload.source,
                &bookmark_count,
            ],
        );
    }
    csv
}

fn browser_session_profile_history_csv_response(payload: &BrowserSessionPayload) -> HttpResponse {
    HttpResponse {
        status: 200,
        reason: "OK",
        content_type: "text/csv; charset=utf-8",
        body: browser_session_profile_history_csv(payload),
    }
}

fn browser_session_profile_history_csv(payload: &BrowserSessionPayload) -> String {
    let mut csv = String::new();
    browser_session_push_csv_row(
        &mut csv,
        &[
            "index",
            "title",
            "source",
            "visited_at_unix_secs",
            "visited_at",
            "action_url",
            "new_session_url",
            "background_session_url",
            "remove_url",
            "session_id",
            "active_source",
            "profile_history_count",
        ],
    );
    let profile_history_count = payload.profile_history.len().to_string();
    for entry in &payload.profile_history {
        let index = (entry.index + 1).to_string();
        let visited_at_unix_secs = entry.visited_at_unix_secs.to_string();
        browser_session_push_csv_row(
            &mut csv,
            &[
                &index,
                &entry.title,
                &entry.source,
                &visited_at_unix_secs,
                &entry.visited_at,
                &entry.action_url,
                &entry.new_session_url,
                &entry.background_session_url,
                &entry.remove_url,
                &payload.id,
                &payload.source,
                &profile_history_count,
            ],
        );
    }
    csv
}

fn browser_session_anchors_csv_response(payload: &BrowserSessionPayload) -> HttpResponse {
    HttpResponse {
        status: 200,
        reason: "OK",
        content_type: "text/csv; charset=utf-8",
        body: browser_session_anchors_csv(payload),
    }
}

fn browser_session_anchors_csv(payload: &BrowserSessionPayload) -> String {
    let mut csv = String::new();
    browser_session_push_csv_row(
        &mut csv,
        &[
            "index",
            "name",
            "y",
            "action_url",
            "new_session_url",
            "background_session_url",
            "session_id",
            "source",
            "total_anchor_count",
        ],
    );
    let total_anchor_count = payload.anchor_count.to_string();
    for anchor in &payload.anchors {
        let index = (anchor.index + 1).to_string();
        let y = anchor.y.to_string();
        browser_session_push_csv_row(
            &mut csv,
            &[
                &index,
                &anchor.name,
                &y,
                &anchor.action_url,
                &anchor.new_session_url,
                &anchor.background_session_url,
                &payload.id,
                &payload.source,
                &total_anchor_count,
            ],
        );
    }
    csv
}

fn browser_session_links_csv_response(payload: &BrowserSessionPayload) -> HttpResponse {
    HttpResponse {
        status: 200,
        reason: "OK",
        content_type: "text/csv; charset=utf-8",
        body: browser_session_links_csv(payload),
    }
}

fn browser_session_links_csv(payload: &BrowserSessionPayload) -> String {
    let mut csv = String::new();
    browser_session_push_csv_row(
        &mut csv,
        &[
            "index",
            "label",
            "url",
            "action_url",
            "new_session_url",
            "background_session_url",
            "session_id",
            "source",
            "total_link_count",
        ],
    );
    let total_link_count = payload.link_count.to_string();
    for link in &payload.links {
        let index = (link.index + 1).to_string();
        browser_session_push_csv_row(
            &mut csv,
            &[
                &index,
                &link.label,
                &link.url,
                &link.action_url,
                &link.new_session_url,
                &link.background_session_url,
                &payload.id,
                &payload.source,
                &total_link_count,
            ],
        );
    }
    csv
}

fn browser_session_history_csv_response(payload: &BrowserSessionPayload) -> HttpResponse {
    HttpResponse {
        status: 200,
        reason: "OK",
        content_type: "text/csv; charset=utf-8",
        body: browser_session_history_csv(payload),
    }
}

fn browser_session_history_csv(payload: &BrowserSessionPayload) -> String {
    let mut csv = String::new();
    browser_session_push_csv_row(
        &mut csv,
        &[
            "index",
            "title",
            "source",
            "target",
            "current",
            "action_url",
            "new_session_url",
            "background_session_url",
            "session_id",
            "active_source",
            "history_len",
        ],
    );
    let history_len = payload.history_len.to_string();
    for entry in &payload.history {
        let index = (entry.index + 1).to_string();
        let current = if entry.current { "true" } else { "false" };
        browser_session_push_csv_row(
            &mut csv,
            &[
                &index,
                &entry.title,
                &entry.source,
                &entry.target,
                current,
                &entry.action_url,
                &entry.new_session_url,
                &entry.background_session_url,
                &payload.id,
                &payload.source,
                &history_len,
            ],
        );
    }
    csv
}

fn browser_session_forms_json_response(payload: &BrowserSessionPayload) -> HttpResponse {
    json_response(200, "OK", &browser_session_forms_export_payload(payload))
}

fn browser_session_forms_csv_response(payload: &BrowserSessionPayload) -> HttpResponse {
    HttpResponse {
        status: 200,
        reason: "OK",
        content_type: "text/csv; charset=utf-8",
        body: browser_session_forms_csv(payload),
    }
}

fn browser_session_forms_csv(payload: &BrowserSessionPayload) -> String {
    let mut csv = String::new();
    browser_session_push_csv_row(
        &mut csv,
        &[
            "form_index",
            "control_index",
            "method",
            "action",
            "resolved_action",
            "control_name",
            "control_kind",
            "value",
            "disabled",
            "required",
            "checked",
            "options",
            "option_select_urls",
            "fill_url",
            "type_url",
            "clear_url",
            "focus_url",
            "activate_url",
            "activate_new_session_url",
            "activate_background_session_url",
            "toggle_url",
            "submit_url",
            "submit_new_session_url",
            "submit_background_session_url",
            "session_id",
            "source",
        ],
    );
    for form in &payload.forms {
        for control in &form.controls {
            let form_index = form.index.to_string();
            let control_index = control.index.to_string();
            let disabled = if control.disabled { "true" } else { "false" };
            let required = if control.required { "true" } else { "false" };
            let checked = if control.checked { "true" } else { "false" };
            let options = browser_session_form_options_summary(&control.options);
            let option_select_urls = browser_session_form_option_select_urls(&control.options);
            let fill_url = control.fill_url.as_deref().unwrap_or("");
            let type_url = control.type_url.as_deref().unwrap_or("");
            let clear_url = control.clear_url.as_deref().unwrap_or("");
            let focus_url = control.focus_url.as_deref().unwrap_or("");
            let activate_url = control.activate_url.as_deref().unwrap_or("");
            let activate_new_session_url =
                control.activate_new_session_url.as_deref().unwrap_or("");
            let activate_background_session_url = control
                .activate_background_session_url
                .as_deref()
                .unwrap_or("");
            let toggle_url = control.toggle_url.as_deref().unwrap_or("");
            browser_session_push_csv_row(
                &mut csv,
                &[
                    &form_index,
                    &control_index,
                    &form.method,
                    &form.action,
                    &form.resolved_action,
                    &control.name,
                    &control.kind,
                    &control.value,
                    disabled,
                    required,
                    checked,
                    &options,
                    &option_select_urls,
                    fill_url,
                    type_url,
                    clear_url,
                    focus_url,
                    activate_url,
                    activate_new_session_url,
                    activate_background_session_url,
                    toggle_url,
                    &form.submit_url,
                    &form.submit_new_session_url,
                    &form.submit_background_session_url,
                    &payload.id,
                    &payload.source,
                ],
            );
        }
    }
    csv
}

fn browser_session_resources_csv_response(payload: &BrowserSessionPayload) -> HttpResponse {
    HttpResponse {
        status: 200,
        reason: "OK",
        content_type: "text/csv; charset=utf-8",
        body: browser_session_resources_csv(payload),
    }
}

fn browser_session_resources_csv(payload: &BrowserSessionPayload) -> String {
    let mut csv = String::new();
    browser_session_push_csv_row(
        &mut csv,
        &[
            "index",
            "kind",
            "initiator",
            "url",
            "resolved",
            "details",
            "open_url",
            "new_session_url",
            "background_session_url",
            "session_id",
            "source",
            "total_resource_count",
        ],
    );
    let total_resource_count = payload.resource_count.to_string();
    for (index, resource) in payload.resources.iter().enumerate() {
        let row_index = (index + 1).to_string();
        browser_session_push_csv_row(
            &mut csv,
            &[
                &row_index,
                &resource.kind,
                &resource.initiator,
                &resource.url,
                &resource.resolved,
                &resource.details,
                &resource.open_url,
                &resource.new_session_url,
                &resource.background_session_url,
                &payload.id,
                &payload.source,
                &total_resource_count,
            ],
        );
    }
    csv
}

fn browser_session_resources_json_response(payload: &BrowserSessionPayload) -> HttpResponse {
    json_response(
        200,
        "OK",
        &browser_session_resources_export_payload(payload),
    )
}

fn browser_session_resource_report_json_response(payload: &BrowserSessionPayload) -> HttpResponse {
    json_response(
        200,
        "OK",
        &browser_session_resource_report_export_payload(payload),
    )
}

fn browser_session_resource_report_csv_response(payload: &BrowserSessionPayload) -> HttpResponse {
    HttpResponse {
        status: 200,
        reason: "OK",
        content_type: "text/csv; charset=utf-8",
        body: browser_session_resource_report_csv(payload),
    }
}

fn browser_session_resource_report_csv(payload: &BrowserSessionPayload) -> String {
    let mut csv = String::new();
    browser_session_push_csv_row(
        &mut csv,
        &[
            "action",
            "page_source",
            "total",
            "fetched",
            "cached",
            "failed",
            "skipped",
            "applied",
            "decoded",
            "index",
            "status",
            "kind",
            "url",
            "resolved",
            "source",
            "bytes",
            "content_type",
            "error",
            "session_id",
            "active_source",
        ],
    );
    let Some(report) = payload.resource_report.as_ref() else {
        return csv;
    };
    let total = report.total.to_string();
    let fetched = report.fetched.to_string();
    let cached = report.cached.to_string();
    let failed = report.failed.to_string();
    let skipped = report.skipped.to_string();
    let applied = report
        .applied
        .map(|value| value.to_string())
        .unwrap_or_default();
    let decoded = report
        .decoded
        .map(|value| value.to_string())
        .unwrap_or_default();
    for (index, resource) in report.resources.iter().enumerate() {
        let row_index = (index + 1).to_string();
        let source = resource.source.as_deref().unwrap_or("");
        let bytes = resource.bytes.to_string();
        let content_type = resource.content_type.as_deref().unwrap_or("");
        let error = resource.error.as_deref().unwrap_or("");
        browser_session_push_csv_row(
            &mut csv,
            &[
                &report.action,
                &report.page_source,
                &total,
                &fetched,
                &cached,
                &failed,
                &skipped,
                &applied,
                &decoded,
                &row_index,
                &resource.status,
                &resource.kind,
                &resource.url,
                &resource.resolved,
                source,
                &bytes,
                content_type,
                error,
                &payload.id,
                &payload.source,
            ],
        );
    }
    csv
}

fn browser_session_find_csv_response(payload: &BrowserSessionPayload) -> HttpResponse {
    HttpResponse {
        status: 200,
        reason: "OK",
        content_type: "text/csv; charset=utf-8",
        body: browser_session_find_csv(payload),
    }
}

fn browser_session_find_json_response(payload: &BrowserSessionPayload) -> HttpResponse {
    json_response(200, "OK", &browser_session_find_export_payload(payload))
}

fn browser_session_find_csv(payload: &BrowserSessionPayload) -> String {
    let mut csv = String::new();
    browser_session_push_csv_row(
        &mut csv,
        &[
            "match_index",
            "line",
            "column",
            "current",
            "query",
            "text",
            "action_url",
            "new_session_url",
            "background_session_url",
            "session_id",
            "source",
            "match_count",
            "current_match_index",
            "current_line",
            "current_column",
        ],
    );
    let match_count = payload.find_match_count.to_string();
    let current_match_index = payload
        .find_current_index
        .map(|index| (index + 1).to_string())
        .unwrap_or_default();
    let current_line = payload
        .find_current_line
        .map(|line| (line + 1).to_string())
        .unwrap_or_default();
    let current_column = payload
        .find_current_column
        .map(|column| (column + 1).to_string())
        .unwrap_or_default();
    for find_match in &payload.find_matches {
        let match_index = (find_match.index + 1).to_string();
        let line = (find_match.line + 1).to_string();
        let column = (find_match.column + 1).to_string();
        let current = if find_match.current { "true" } else { "false" };
        browser_session_push_csv_row(
            &mut csv,
            &[
                &match_index,
                &line,
                &column,
                current,
                &payload.find_query,
                &find_match.text,
                &find_match.action_url,
                &find_match.new_session_url,
                &find_match.background_session_url,
                &payload.id,
                &payload.source,
                &match_count,
                &current_match_index,
                &current_line,
                &current_column,
            ],
        );
    }
    csv
}

fn browser_session_form_options_summary(options: &[BrowserSessionFormOptionPayload]) -> String {
    options
        .iter()
        .map(|option| {
            let mut flags = Vec::new();
            if option.selected {
                flags.push("selected");
            }
            if option.disabled {
                flags.push("disabled");
            }
            let flags = if flags.is_empty() {
                String::new()
            } else {
                format!(" [{}]", flags.join(" "))
            };
            format!("{}={}{}", option.value, option.label, flags)
        })
        .collect::<Vec<_>>()
        .join("; ")
}

fn browser_session_form_option_select_urls(options: &[BrowserSessionFormOptionPayload]) -> String {
    options
        .iter()
        .filter_map(|option| {
            option
                .select_url
                .as_deref()
                .map(|href| format!("{}={href}", option.value))
        })
        .collect::<Vec<_>>()
        .join("; ")
}

fn render_browser_session_find_controls(payload: &BrowserSessionPayload) -> String {
    let status = if payload.find_query.trim().is_empty() {
        "Find in page".to_owned()
    } else if payload.find_match_count == 0 {
        format!("0 matches for {}", payload.find_query)
    } else if let (Some(index), Some(line), Some(column)) = (
        payload.find_current_index,
        payload.find_current_line,
        payload.find_current_column,
    ) {
        format!(
            "{} of {} at line {}, col {}",
            index + 1,
            payload.find_match_count,
            line + 1,
            column + 1
        )
    } else {
        format!("{} matches", payload.find_match_count)
    };
    let actions = if payload.find_query.trim().is_empty() {
        String::new()
    } else {
        let clear_href = browser_session_action_href(&payload.id, "clear-find", &[], payload);
        let json_href = browser_session_api_href(&payload.id, "find-json", payload);
        let csv_href = browser_session_api_href(&payload.id, "find-csv", payload);
        let cycle_actions = if payload.find_match_count > 1 {
            let previous_href = browser_session_action_href(&payload.id, "find-prev", &[], payload);
            let next_href = browser_session_action_href(&payload.id, "find-next", &[], payload);
            format!(
                r#"<a href="{previous_href}">Previous</a><a href="{next_href}">Next</a>"#,
                previous_href = html_escape::encode_double_quoted_attribute(&previous_href),
                next_href = html_escape::encode_double_quoted_attribute(&next_href),
            )
        } else {
            String::new()
        };
        let bulk_actions = if payload.find_matches.iter().any(|match_| !match_.current) {
            let open_tabs_href = browser_session_action_href(
                &payload.id,
                "open-find-matches-new-sessions",
                &[("limit", DEFAULT_BULK_BACKGROUND_LINKS.to_string())],
                payload,
            );
            let open_background_href = browser_session_action_href(
                &payload.id,
                "open-find-matches-background-sessions",
                &[("limit", DEFAULT_BULK_BACKGROUND_LINKS.to_string())],
                payload,
            );
            format!(
                r#"<a href="{open_tabs_href}">Open matches tabs</a><a href="{open_background_href}">Open matches bg</a>"#,
                open_tabs_href = html_escape::encode_double_quoted_attribute(&open_tabs_href),
                open_background_href =
                    html_escape::encode_double_quoted_attribute(&open_background_href),
            )
        } else {
            String::new()
        };
        let matches = render_browser_session_find_match_links(payload);
        format!(
            r#"{cycle_actions}<a href="{json_href}">Find JSON</a><a href="{csv_href}">Find CSV</a>{bulk_actions}<a href="{clear_href}">Clear</a>{matches}"#,
            cycle_actions = cycle_actions,
            json_href = html_escape::encode_double_quoted_attribute(&json_href),
            csv_href = html_escape::encode_double_quoted_attribute(&csv_href),
            bulk_actions = bulk_actions,
            clear_href = html_escape::encode_double_quoted_attribute(&clear_href),
            matches = matches,
        )
    };

    format!(
        r#"<div class="find-bar"><form action="/browser" method="get">{common}<input type="hidden" name="action" value="find"><input data-browser-find type="search" name="q" value="{query}" aria-label="Find in page"><button type="submit">Find</button></form><div class="find-actions"><span class="meta">{status}</span>{actions}</div></div>"#,
        common = browser_session_common_hidden_inputs(payload),
        query = html_escape::encode_double_quoted_attribute(&payload.find_query),
        status = html_escape::encode_text(&status),
        actions = actions,
    )
}

fn render_browser_session_find_match_links(payload: &BrowserSessionPayload) -> String {
    if payload.find_matches.is_empty() {
        return String::new();
    }

    let mut links = String::new();
    for find_match in &payload.find_matches {
        let class = if find_match.current {
            "find-match current"
        } else {
            "find-match"
        };
        let _ = write!(
            links,
            r#"<span class="find-match-actions"><a class="{class}" href="{href}">{index} · line {line}, col {column}</a><a class="find-match" href="{new_href}">New</a><a class="find-match" href="{background_href}">Bg</a></span>"#,
            class = class,
            href = html_escape::encode_double_quoted_attribute(&find_match.action_url),
            new_href = html_escape::encode_double_quoted_attribute(&find_match.new_session_url),
            background_href =
                html_escape::encode_double_quoted_attribute(&find_match.background_session_url),
            index = find_match.index + 1,
            line = find_match.line + 1,
            column = find_match.column + 1,
        );
    }

    format!(r#"<span class="find-matches">{links}</span>"#)
}

fn render_browser_session_viewport(payload: &BrowserSessionPayload) -> String {
    render_browser_session_highlighted_text(&payload.viewport, &payload.find_query)
}

fn render_browser_session_viewport_text(payload: &BrowserSessionPayload, viewport: &str) -> String {
    if payload.fast_scroll {
        format!(
            r#"<details class="viewport-text" open data-browser-fast-scroll-text><summary>Text viewport · fast scroll response</summary><pre>{viewport}</pre></details>"#,
            viewport = viewport,
        )
    } else {
        format!(
            r#"<details class="viewport-text"><summary>Text viewport</summary><pre>{viewport}</pre></details>"#,
            viewport = viewport,
        )
    }
}

fn browser_scroll_axis_state(
    value: usize,
    max: usize,
    start_label: &'static str,
    middle_label: &'static str,
    end_label: &'static str,
    none_label: &'static str,
) -> &'static str {
    if max == 0 {
        none_label
    } else if value == 0 {
        start_label
    } else if value >= max {
        end_label
    } else {
        middle_label
    }
}

fn render_browser_session_chrome_status(payload: &BrowserSessionPayload) -> String {
    let mut status = String::new();
    let action_urls = browser_session_resource_action_urls(payload);
    let show_read_action = !browser_session_has_ready_raster(payload);
    let (raster_label, raster_title) = if let Some(image) = &payload.viewport_image {
        let size = format!("{}x{}", image.width, image.height);
        (size.clone(), format!("visual {size}"))
    } else if payload.viewport_image_error.is_some() {
        ("error".to_owned(), "visual error".to_owned())
    } else {
        ("pending".to_owned(), "visual pending".to_owned())
    };
    let image_state =
        browser_session_chrome_image_state(payload, action_urls.load_images.is_some());
    let has_image_state = image_state.is_some();
    let _ = write!(
        status,
        r#"<span class="viewport-state-chip" data-browser-shell-session title="tab {id}">{id}</span><span class="viewport-state-chip" data-browser-shell-viewport title="viewport {width}x{height} at {x},{y}" hidden>{width}x{height}</span><span class="viewport-state-chip" data-browser-shell-render title="{raster_title}">{raster}</span>"#,
        id = html_escape::encode_text(&payload.id),
        width = payload.width,
        height = payload.height,
        x = payload.viewport_x,
        y = payload.viewport_y,
        raster = html_escape::encode_text(&raster_label),
        raster_title = html_escape::encode_double_quoted_attribute(&raster_title),
    );
    if show_read_action {
        if let Some(href) = action_urls.make_visual.as_deref() {
            let _ = write!(
                status,
                r#"<a class="browser-chrome-tool primary-action" href="{href}" data-browser-resource-action data-browser-make-visual-action data-browser-resource-status="Making visual...">Read</a>"#,
                href = html_escape::encode_double_quoted_attribute(href),
            );
        }
    }
    if let Some(image_state) = image_state {
        let _ = write!(
            status,
            r#"<span class="viewport-state-chip" data-browser-shell-images data-browser-resource-status-output aria-live="polite">{image_state}</span>"#,
            image_state = html_escape::encode_text(&image_state),
        );
    }
    if ((show_read_action && action_urls.make_visual.is_some())
        || action_urls.load_images.is_some())
        && !has_image_state
    {
        status.push_str(
            r#"<span class="resource-action-status" data-browser-resource-status-output aria-live="polite"></span>"#,
        );
    }
    status
}

fn browser_session_chrome_image_state(
    payload: &BrowserSessionPayload,
    can_load_images: bool,
) -> Option<String> {
    if let Some(report) = &payload.resource_report {
        if let Some(decoded) = report.decoded {
            if decoded > 0 {
                return Some(format!("images {}", decoded));
            }
            if report.failed > 0 {
                return Some(format!("images failed {}", report.failed));
            }
            if report.fetched > 0 || report.cached > 0 {
                return Some("images not decoded".to_owned());
            }
        }
    }
    if can_load_images && payload.resource_image_count > 0 {
        return Some(format!(
            "{} in Tools",
            browser_resource_count_label(payload.resource_image_count, "image", "images")
        ));
    }
    if payload.resource_image_count > 0 {
        return Some(browser_resource_count_label(
            payload.resource_image_count,
            "image",
            "images",
        ));
    }
    None
}

fn render_browser_session_viewport_status(payload: &BrowserSessionPayload) -> String {
    let vertical_percent = browser_scroll_percent(payload.viewport_y, payload.max_scroll_y);
    let meter_percent = if payload.max_scroll_y == 0 {
        100
    } else {
        vertical_percent
    };
    let horizontal_state = browser_scroll_axis_state(
        payload.viewport_x,
        payload.max_scroll_x,
        "at left edge",
        "horizontal scroll available",
        "at right edge",
        "no horizontal scroll",
    );
    let vertical_state = browser_scroll_axis_state(
        payload.viewport_y,
        payload.max_scroll_y,
        "at top",
        "vertical scroll available",
        "at bottom",
        "no vertical scroll",
    );
    let scroll_summary = if payload.max_scroll_y == 0 {
        format!(
            "Scroll x {}/{} · y {}/{}",
            payload.viewport_x, payload.max_scroll_x, payload.viewport_y, payload.max_scroll_y
        )
    } else {
        format!(
            "Scroll x {}/{} · y {}/{} · {}%",
            payload.viewport_x,
            payload.max_scroll_x,
            payload.viewport_y,
            payload.max_scroll_y,
            vertical_percent
        )
    };
    let input_hint = if payload.max_scroll_x == 0 && payload.max_scroll_y == 0 {
        "No page scroll"
    } else {
        "Wheel / keys scroll"
    };
    let viewport_feedback = render_browser_session_viewport_feedback(payload);
    let click_status = browser_session_click_status(payload);
    let click_hint = browser_session_click_hint(payload);
    format!(
        r#"<div class="viewport-status" data-browser-viewport-status><div class="viewport-status-text"><span class="viewport-scroll-summary" data-browser-scroll-state="summary" data-scroll-x-state="{horizontal_state}" data-scroll-y-state="{vertical_state}">{scroll_summary}</span><span data-browser-scroll-input-hint>{input_hint}</span><span class="viewport-scroll-feedback" data-browser-viewport-feedback aria-live="polite">{viewport_feedback}</span><span class="viewport-state-chip" data-browser-click-status aria-live="polite">{click_status}</span><span data-browser-click-hint>{click_hint}</span></div><div class="viewport-scroll-meter" role="progressbar" aria-label="Vertical scroll position" aria-valuemin="0" aria-valuemax="{max_y}" aria-valuenow="{y}" aria-valuetext="y {y} of {max_y}"><span style="width: {meter_percent}%;"></span></div></div>"#,
        y = payload.viewport_y,
        max_y = payload.max_scroll_y,
        horizontal_state = horizontal_state,
        vertical_state = vertical_state,
        scroll_summary = html_escape::encode_text(&scroll_summary),
        input_hint = input_hint,
        viewport_feedback = viewport_feedback,
        click_status = click_status,
        click_hint = click_hint,
        meter_percent = meter_percent,
    )
}

fn render_browser_session_primary_page_state(payload: &BrowserSessionPayload) -> String {
    let action_feedback = render_browser_session_surface_action_feedback(payload);
    if browser_session_pending_without_ready_viewport(payload)
        && let Some(pending_source) = payload.pending_source.as_ref()
    {
        let continue_href = browser_session_action_href(
            &payload.id,
            "open",
            &[("url", pending_source.clone())],
            payload,
        );
        return format!(
            r#"<div class="browser-surface-state" data-browser-primary-state data-browser-pending-load="true"><span class="viewport-state-chip warning">Loading page</span><span class="viewport-state-chip report">Opening {source}</span><span class="viewport-state-chip report" data-browser-pending-session-retained>same tab retained</span><span class="viewport-state-chip report" data-browser-pending-auto-retry>Retrying once in this tab</span>{action_feedback}<a class="primary-action" href="{continue_href}" data-browser-continue-load>Continue loading</a></div>"#,
            source = html_escape::encode_text(&browser_session_feedback_excerpt(pending_source)),
            action_feedback = action_feedback,
            continue_href = html_escape::encode_double_quoted_attribute(&continue_href),
        );
    }

    let render_label = if let Some(image) = payload.viewport_image.as_ref() {
        format!("Browser view ready: {}x{}", image.width, image.height,)
    } else if payload.viewport_image_error.is_some() {
        "Browser view has a render error".to_owned()
    } else {
        "Browser view waiting for render".to_owned()
    };
    let retained_pending = render_browser_session_retained_pending_status(payload);
    format!(
        r#"<div class="browser-surface-state compact" data-browser-primary-state>{retained_pending}<span data-browser-primary-raster>{render_label}</span>{action_feedback}</div>"#,
        retained_pending = retained_pending,
        render_label = html_escape::encode_text(&render_label),
        action_feedback = action_feedback,
    )
}

fn browser_session_pending_without_ready_viewport(payload: &BrowserSessionPayload) -> bool {
    payload.pending_source.is_some() && payload.viewport_image.is_none()
}

fn render_browser_session_retained_pending_status(payload: &BrowserSessionPayload) -> String {
    let Some(pending_source) = payload.pending_source.as_ref() else {
        return String::new();
    };
    if payload.viewport_image.is_none() {
        return String::new();
    }
    let retry_href = browser_session_action_href(
        &payload.id,
        "open",
        &[("url", pending_source.clone())],
        payload,
    );
    let pending_reason = payload
        .action_feedback
        .as_deref()
        .map(|feedback| {
            format!(
                r#"<span class="viewport-state-chip report" data-browser-retained-pending-reason>{reason}</span>"#,
                reason = html_escape::encode_text(&browser_session_feedback_excerpt(feedback)),
            )
        })
        .unwrap_or_default();
    format!(
        r#"<span class="viewport-state-chip warning" data-browser-retained-pending-target>Opening {target}</span><span class="viewport-state-chip report" data-browser-retained-pending-raster>current raster retained</span>{pending_reason}<a class="primary-action" href="{retry_href}" data-browser-continue-load>Retry load</a>"#,
        target = html_escape::encode_text(&browser_session_feedback_excerpt(pending_source)),
        pending_reason = pending_reason,
        retry_href = html_escape::encode_double_quoted_attribute(&retry_href),
    )
}

fn render_browser_session_surface_action_feedback(payload: &BrowserSessionPayload) -> String {
    if browser_session_action_feedback_text(payload).is_none() {
        return String::new();
    }
    if browser_session_scroll_feedback_text(payload).is_some()
        || browser_session_click_feedback_text(payload).is_some()
        || (payload.pending_source.is_some() && payload.viewport_image.is_some())
    {
        return String::new();
    }
    render_browser_session_action_feedback(payload)
}

fn render_browser_session_viewport_scroll_controls(payload: &BrowserSessionPayload) -> String {
    let top_href = browser_session_action_href(&payload.id, "top", &[], payload);
    let left_href =
        browser_session_action_href(&payload.id, "scroll", &[("dx", "-1".to_owned())], payload);
    let page_up_href = browser_session_action_href(&payload.id, "page-up", &[], payload);
    let line_up_href = browser_session_action_href(&payload.id, "line-up", &[], payload);
    let line_down_href = browser_session_action_href(&payload.id, "line-down", &[], payload);
    let page_down_href = browser_session_action_href(&payload.id, "page-down", &[], payload);
    let right_href =
        browser_session_action_href(&payload.id, "scroll", &[("dx", "1".to_owned())], payload);
    let bottom_href = browser_session_action_href(&payload.id, "bottom", &[], payload);
    let can_scroll_left = payload.viewport_x > 0;
    let can_scroll_right = payload.viewport_x < payload.max_scroll_x;
    let can_scroll_up = payload.viewport_y > 0;
    let can_scroll_down = payload.viewport_y < payload.max_scroll_y;
    let viewport_feedback = render_browser_session_viewport_feedback(payload);
    format!(
        r#"<nav class="viewport-scroll-controls" data-browser-viewport-controls data-browser-viewport-page-controls data-browser-auto-visual-control aria-label="Manual viewport scroll controls; x {x} of {max_x}, y {y} of {max_y}" data-scroll-x="{x}" data-scroll-y="{y}" data-max-scroll-x="{max_x}" data-max-scroll-y="{max_y}" data-can-scroll-left="{can_scroll_left}" data-can-scroll-right="{can_scroll_right}" data-can-scroll-up="{can_scroll_up}" data-can-scroll-down="{can_scroll_down}">{top}{left}{page_up}{line_up}{line_down}{page_down}{right}{bottom}<span class="viewport-scroll-feedback" data-browser-viewport-feedback aria-live="polite">{viewport_feedback}</span></nav>"#,
        x = payload.viewport_x,
        y = payload.viewport_y,
        max_x = payload.max_scroll_x,
        max_y = payload.max_scroll_y,
        can_scroll_left = can_scroll_left,
        can_scroll_right = can_scroll_right,
        can_scroll_up = can_scroll_up,
        can_scroll_down = can_scroll_down,
        top = scroll_nav_control(can_scroll_up, "Top", &top_href, "Already at top"),
        left = scroll_nav_control(can_scroll_left, "Left", &left_href, "Already at left edge"),
        page_up = scroll_nav_control(can_scroll_up, "Page up", &page_up_href, "Already at top"),
        line_up = scroll_nav_control(can_scroll_up, "Line up", &line_up_href, "Already at top"),
        line_down = scroll_nav_control(
            can_scroll_down,
            "Line down",
            &line_down_href,
            "Already at bottom"
        ),
        page_down = scroll_nav_control(
            can_scroll_down,
            "Page down",
            &page_down_href,
            "Already at bottom"
        ),
        right = scroll_nav_control(
            can_scroll_right,
            "Right",
            &right_href,
            "Already at right edge"
        ),
        bottom = scroll_nav_control(can_scroll_down, "Bottom", &bottom_href, "Already at bottom"),
        viewport_feedback = viewport_feedback,
    )
}

fn render_browser_session_viewport_interaction_controls(payload: &BrowserSessionPayload) -> String {
    let _ = payload;
    r#"<div class="viewport-interaction-row compact" data-browser-viewport-interactions hidden></div>"#
        .to_owned()
}

fn render_browser_session_viewport_command_strip(payload: &BrowserSessionPayload) -> String {
    let action_urls = browser_session_resource_action_urls(payload);
    let mut visual_actions = String::new();
    visual_actions.push_str(&browser_session_resource_action_link_with_status(
        action_urls.apply_stylesheets.as_deref(),
        "Apply styles",
        "Applying styles...",
    ));
    visual_actions.push_str(&browser_session_resource_action_link_with_status(
        action_urls.run_scripts.as_deref(),
        "Run scripts",
        "Running scripts...",
    ));
    let current_href = browser_session_action_href(&payload.id, "current", &[], payload);
    let reload_href = browser_session_action_href(&payload.id, "reload", &[], payload);
    let percent = browser_scroll_percent(payload.viewport_y, payload.max_scroll_y);
    let visual_status = if visual_actions.is_empty() {
        render_browser_session_visual_flow_status(payload)
    } else {
        r#"<span class="resource-visual-status resource-action-status" data-browser-visual-status data-browser-resource-status-output aria-live="polite"></span>"#.to_owned()
    };
    let page_state = render_browser_session_viewport_page_state(payload);
    let render_status = render_browser_session_render_status(payload);
    let viewport_feedback = render_browser_session_viewport_feedback(payload);

    format!(
        r#"<section class="viewport-command-strip" data-browser-viewport-command-strip data-browser-resource-actions data-browser-auto-visual-control aria-label="Browser viewport tools"><div class="viewport-command-row viewport-command-state" data-browser-viewport-state-row><div class="viewport-command-group" data-browser-viewport-state-group aria-label="Viewport state"><span class="viewport-command-label">State</span><span class="viewport-state-chip">session {id}</span><span class="viewport-state-chip">viewport {width}x{height}</span><span class="viewport-state-chip">x {x}/{max_x}</span><span class="viewport-state-chip">y {y}/{max_y}</span><span class="viewport-state-chip">{percent}%</span></div><div class="resource-actions viewport-command-group" data-browser-viewport-page-actions aria-label="Page actions"><span class="viewport-command-label">Page</span>{visual_actions}{visual_status}</div><div class="resource-actions viewport-command-group" data-browser-viewport-session-actions aria-label="Session actions"><span class="viewport-command-label">Session</span><a class="clear-link" href="{current_href}">Refresh viewport</a><a class="clear-link" href="{reload_href}">Reload page</a></div></div>{page_state}{render_status}<div class="viewport-command-row"><form class="viewport-command-jump" action="/browser" method="get"><span class="viewport-command-label">Jump</span>{common}<input type="hidden" name="action" value="current"><label for="browser-command-viewport-x">x</label><input id="browser-command-viewport-x" type="number" min="0" max="{max_x}" name="x" value="{x}" aria-label="Viewport x quick jump" aria-describedby="browser-command-viewport-range"><label for="browser-command-viewport-y">y</label><input id="browser-command-viewport-y" type="number" min="0" max="{max_y}" name="y" value="{y}" aria-label="Viewport y quick jump" aria-describedby="browser-command-viewport-range"><span id="browser-command-viewport-range" class="viewport-jump-range">range x 0-{max_x}, y 0-{max_y}</span><button type="submit">Jump</button></form><span class="viewport-scroll-feedback" data-browser-viewport-feedback aria-live="polite">{viewport_feedback}</span></div></section>"#,
        id = html_escape::encode_text(&payload.id),
        width = payload.width,
        height = payload.height,
        x = payload.viewport_x,
        max_x = payload.max_scroll_x,
        y = payload.viewport_y,
        max_y = payload.max_scroll_y,
        percent = percent,
        visual_actions = visual_actions,
        current_href = html_escape::encode_double_quoted_attribute(&current_href),
        reload_href = html_escape::encode_double_quoted_attribute(&reload_href),
        visual_status = visual_status,
        page_state = page_state,
        render_status = render_status,
        common = browser_session_common_hidden_inputs(payload),
        viewport_feedback = viewport_feedback,
    )
}

fn render_browser_session_viewport_page_state(payload: &BrowserSessionPayload) -> String {
    let action_feedback = render_browser_session_surface_action_feedback(payload);
    let visual_flow_status = render_browser_session_visual_flow_status(payload);
    if browser_session_pending_without_ready_viewport(payload)
        && let Some(pending_source) = payload.pending_source.as_ref()
    {
        let continue_href = browser_session_action_href(
            &payload.id,
            "open",
            &[("url", pending_source.clone())],
            payload,
        );
        return format!(
            r#"<div class="viewport-command-row viewport-page-state" data-browser-viewport-page-state data-browser-pending-load="true"><span class="viewport-state-chip warning">Loading page</span><span class="viewport-state-chip report">Waiting for {source}</span><span class="viewport-state-chip report" data-browser-pending-session-retained>same tab retained</span><span class="viewport-state-chip report" data-browser-pending-auto-retry>Retrying once in this tab</span>{action_feedback}<a class="clear-link primary-action" href="{continue_href}" data-browser-continue-load>Continue loading</a></div>"#,
            source = html_escape::encode_text(&browser_session_feedback_excerpt(pending_source)),
            action_feedback = action_feedback,
            continue_href = html_escape::encode_double_quoted_attribute(&continue_href),
        );
    }
    if let Some(report) = payload.resource_report.as_ref() {
        let status = browser_session_resource_report_status(report);
        let report_json_href =
            browser_session_api_href(&payload.id, "resource-report-json", payload);
        let clear_href =
            browser_session_action_href(&payload.id, "clear-resource-report", &[], payload);
        let applied = report
            .applied
            .map(|count| {
                format!(r#"<span class="viewport-state-chip report">applied {count}</span>"#)
            })
            .unwrap_or_default();
        let decoded = report
            .decoded
            .map(|count| {
                format!(r#"<span class="viewport-state-chip report">decoded {count}</span>"#)
            })
            .unwrap_or_default();
        return format!(
            r#"<div class="viewport-command-row viewport-page-state" data-browser-viewport-page-state><span class="viewport-state-chip report">Last action: {action}</span><span class="viewport-state-chip report">{status}</span>{applied}{decoded}{visual_flow_status}{action_feedback}<a class="clear-link" href="{report_json_href}">Report JSON</a><a class="clear-link" href="{clear_href}">Clear report</a></div>"#,
            action = html_escape::encode_text(&report.action),
            status = html_escape::encode_text(&status),
            applied = applied,
            decoded = decoded,
            visual_flow_status = visual_flow_status,
            action_feedback = action_feedback,
            report_json_href = html_escape::encode_double_quoted_attribute(&report_json_href),
            clear_href = html_escape::encode_double_quoted_attribute(&clear_href),
        );
    }

    let mut chips = String::new();
    if payload.resource_stylesheet_count > 0 {
        let _ = write!(
            chips,
            r#"<span class="viewport-state-chip">{}</span>"#,
            html_escape::encode_text(&browser_resource_count_label(
                payload.resource_stylesheet_count,
                "stylesheet",
                "stylesheets",
            )),
        );
    }
    if payload.resource_script_count > 0 {
        let _ = write!(
            chips,
            r#"<span class="viewport-state-chip">{}</span>"#,
            html_escape::encode_text(&browser_resource_count_label(
                payload.resource_script_count,
                "script",
                "scripts",
            )),
        );
    }
    if payload.resource_image_count > 0 {
        let _ = write!(
            chips,
            r#"<span class="viewport-state-chip">{}</span>"#,
            html_escape::encode_text(&browser_resource_count_label(
                payload.resource_image_count,
                "image",
                "images",
            )),
        );
    }
    if chips.is_empty() {
        return format!(
            r#"<div class="viewport-command-row viewport-page-state" data-browser-viewport-page-state><span class="viewport-state-chip">No visual resources</span>{visual_flow_status}{action_feedback}</div>"#,
            visual_flow_status = visual_flow_status,
            action_feedback = action_feedback,
        );
    }
    format!(
        r#"<div class="viewport-command-row viewport-page-state" data-browser-viewport-page-state><span class="viewport-state-chip">Ready</span>{chips}{visual_flow_status}{action_feedback}</div>"#,
        chips = chips,
        visual_flow_status = visual_flow_status,
        action_feedback = action_feedback,
    )
}

fn render_browser_session_visual_flow_status(payload: &BrowserSessionPayload) -> String {
    let Some(report) = payload.resource_report.as_ref() else {
        if payload.resource_stylesheet_count == 0 && payload.resource_image_count == 0 {
            if payload.viewport_image.is_some() {
                return r#"<span class="viewport-state-chip" data-browser-visual-flow-status>visual page ready</span>"#.to_owned();
            }
            return r#"<span class="viewport-state-chip" data-browser-visual-flow-status>visual actions unavailable</span>"#.to_owned();
        }
        return format!(
            r#"<span class="viewport-state-chip" data-browser-visual-flow-status>visual actions ready: {} · {}</span>"#,
            html_escape::encode_text(&browser_resource_count_label(
                payload.resource_stylesheet_count,
                "stylesheet",
                "stylesheets",
            )),
            html_escape::encode_text(&browser_resource_count_label(
                payload.resource_image_count,
                "image",
                "images",
            )),
        );
    };

    let mut chips = String::new();
    if payload.resource_stylesheet_count > 0 {
        let label = match report.applied {
            Some(count) if count > 0 => format!("styles applied: {count}"),
            Some(_) => "styles unchanged".to_owned(),
            None => "styles waiting".to_owned(),
        };
        let _ = write!(
            chips,
            r#"<span class="viewport-state-chip report" data-browser-visual-flow-status>{}</span>"#,
            html_escape::encode_text(&label),
        );
    }
    if payload.resource_image_count > 0 {
        let label = match report.decoded {
            Some(count) if count > 0 => format!("images loaded: {count}"),
            Some(_) => format!(
                "images not decoded: fetched {}, failed {}, skipped {}",
                report.fetched, report.failed, report.skipped
            ),
            None => "images waiting".to_owned(),
        };
        let _ = write!(
            chips,
            r#"<span class="viewport-state-chip report" data-browser-visual-flow-status>{}</span>"#,
            html_escape::encode_text(&label),
        );
    }
    if report.failed > 0 {
        let _ = write!(
            chips,
            r#"<span class="viewport-state-chip warning" data-browser-visual-flow-status>resource failures: {}</span>"#,
            report.failed,
        );
    }
    if chips.is_empty() {
        r#"<span class="viewport-state-chip" data-browser-visual-flow-status>visual actions complete</span>"#.to_owned()
    } else {
        chips
    }
}

fn render_browser_session_action_feedback(payload: &BrowserSessionPayload) -> String {
    browser_session_action_feedback_text(payload)
        .map(|feedback| {
            format!(
                r#"<span class="viewport-state-chip report" data-browser-action-feedback>{}</span>"#,
                html_escape::encode_text(feedback),
            )
        })
        .unwrap_or_default()
}

fn browser_session_action_feedback_text(payload: &BrowserSessionPayload) -> Option<&str> {
    payload
        .action_feedback
        .as_deref()
        .map(str::trim)
        .filter(|feedback| !feedback.is_empty())
}

fn browser_session_scroll_feedback_text(payload: &BrowserSessionPayload) -> Option<&str> {
    browser_session_action_feedback_text(payload).filter(|feedback| {
        feedback.starts_with("Moved visual viewport")
            || feedback.starts_with("Viewport moved")
            || feedback.starts_with("Viewport is already")
            || feedback.starts_with("Already at")
    })
}

fn render_browser_session_viewport_feedback(payload: &BrowserSessionPayload) -> String {
    if browser_session_pending_without_ready_viewport(payload) {
        return "Page is still loading; scroll starts after the first render.".to_owned();
    }
    browser_session_scroll_feedback_text(payload)
        .map(|feedback| html_escape::encode_text(feedback).into_owned())
        .unwrap_or_else(|| {
            if payload.max_scroll_x == 0 && payload.max_scroll_y == 0 {
                "No page scroll.".to_owned()
            } else {
                "Ready to scroll.".to_owned()
            }
        })
}

fn browser_session_click_status(payload: &BrowserSessionPayload) -> String {
    if browser_session_pending_without_ready_viewport(payload) {
        return "Page is still loading; clicks start after the first render.".to_owned();
    }
    browser_session_click_feedback_text(payload)
        .map(|feedback| html_escape::encode_text(feedback).into_owned())
        .unwrap_or_else(|| "Ready for page click.".to_owned())
}

fn browser_session_click_hint(payload: &BrowserSessionPayload) -> &'static str {
    if browser_session_pending_without_ready_viewport(payload) {
        "Clicks start after render"
    } else {
        "Click raster to open links/buttons"
    }
}

fn browser_session_click_feedback_text(payload: &BrowserSessionPayload) -> Option<&str> {
    browser_session_action_feedback_text(payload).filter(|feedback| {
        feedback.starts_with("Clicked ")
            || feedback.starts_with("Click ")
            || feedback.starts_with("No click")
    })
}

fn render_browser_session_render_status(payload: &BrowserSessionPayload) -> String {
    let raster_chip = if let Some(image) = payload.viewport_image.as_ref() {
        format!(
            r#"<span class="viewport-state-chip">raster ready {}x{}</span>"#,
            image.width, image.height,
        )
    } else if payload.fast_scroll {
        r#"<span class="viewport-state-chip">fast text scroll</span>"#.to_owned()
    } else if let Some(error) = payload.viewport_image_error.as_ref() {
        format!(
            r#"<span class="viewport-state-chip warning">raster error: {}</span>"#,
            html_escape::encode_text(error),
        )
    } else {
        r#"<span class="viewport-state-chip warning">raster unavailable</span>"#.to_owned()
    };
    let text_lines = payload.page_text.lines().count();
    let resource_summary = if payload.resource_count == 0 {
        r#"<span class="viewport-state-chip">0 resources</span>"#.to_owned()
    } else {
        format!(
            r#"<span class="viewport-state-chip">{} · {}</span>"#,
            html_escape::encode_text(&browser_resource_count_label(
                payload.resource_count,
                "resource",
                "resources",
            )),
            html_escape::encode_text(&browser_resource_count_label(
                payload.resource_image_count,
                "image",
                "images",
            )),
        )
    };

    format!(
        r#"<div class="viewport-command-row viewport-render-status" data-browser-render-status><span class="viewport-command-label">Render</span>{raster_chip}<span class="viewport-state-chip">text {text_lines} lines</span><span class="viewport-state-chip">document {document_width}x{document_height}</span>{resource_summary}</div>"#,
        raster_chip = raster_chip,
        text_lines = text_lines,
        document_width = payload.document_width,
        document_height = payload.document_height,
        resource_summary = resource_summary,
    )
}

fn browser_scroll_percent(value: usize, max: usize) -> usize {
    if max == 0 {
        return 100;
    }
    ((value.min(max) as u128 * 100) / max as u128) as usize
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

fn render_browser_session_primary_tab_strip(payload: &BrowserSessionPayload) -> String {
    if payload.sessions.len() <= 1 {
        return String::new();
    }

    let current_title = payload
        .sessions
        .iter()
        .find(|session| session.current)
        .map(|session| session.title.as_str())
        .unwrap_or(payload.title.as_str());
    let mut tabs = String::new();
    for session in &payload.sessions {
        let class = match (session.current, session.pinned) {
            (true, true) => "browser-tab-pill current pinned",
            (true, false) => "browser-tab-pill current",
            (false, true) => "browser-tab-pill pinned",
            (false, false) => "browser-tab-pill",
        };
        let aria_current = if session.current {
            r#" aria-current="page""#
        } else {
            ""
        };
        let pinned_marker = if session.pinned { "Pinned · " } else { "" };
        let _ = write!(
            tabs,
            r#"<a class="{class}" href="{href}"{aria_current}><strong>{position} · {pinned_marker}{title}</strong><span>{source}</span></a>"#,
            class = class,
            href = html_escape::encode_double_quoted_attribute(&session.action_url),
            aria_current = aria_current,
            position = session.position,
            pinned_marker = pinned_marker,
            title = html_escape::encode_text(&session.title),
            source = html_escape::encode_text(&session.source),
        );
    }

    format!(
        r#"<details class="browser-tab-strip browser-tab-menu" data-browser-tab-menu aria-label="Open browser tabs"><summary data-browser-tab-summary><strong>{count} tabs</strong><span>{current}</span></summary><div class="browser-tab-list">{tabs}</div></details>"#,
        count = payload.sessions.len(),
        current = html_escape::encode_text(&browser_session_feedback_excerpt(current_title)),
        tabs = tabs,
    )
}

fn render_browser_session_navigation_state(
    payload: &BrowserSessionPayload,
    back_href: &str,
) -> String {
    let current_session = payload
        .sessions
        .iter()
        .find(|session| session.id == payload.id);
    let tab_position = current_session.map_or_else(
        || format!("session {}", payload.id),
        |session| format!("tab {}/{}", session.position, payload.sessions.len()),
    );
    let tab_state = current_session.map_or("active".to_owned(), |session| {
        match (session.current, session.pinned) {
            (true, true) => "active pinned".to_owned(),
            (true, false) => "active".to_owned(),
            (false, true) => "pinned".to_owned(),
            (false, false) => "background".to_owned(),
        }
    });
    let history_position = payload.current_history_index.map_or(0, |index| index + 1);
    let return_target = if back_href.trim().is_empty() {
        r#"<span>no return target</span>"#.to_owned()
    } else {
        format!(
            r#"<a href="{href}" title="Return to {title_target}">Return to results</a><span>from {text_target}</span>"#,
            href = html_escape::encode_double_quoted_attribute(back_href),
            title_target = html_escape::encode_double_quoted_attribute(back_href),
            text_target = html_escape::encode_text(back_href),
        )
    };

    format!(
        r#"<nav class="browser-navigation-state" data-browser-navigation-state aria-label="Browser session state">{return_target}<span>session {id}</span><span>{tab_position}</span><span>{tab_state}</span><span>history {history_position}/{history_len}</span></nav>"#,
        return_target = return_target,
        id = html_escape::encode_text(&payload.id),
        tab_position = html_escape::encode_text(&tab_position),
        tab_state = html_escape::encode_text(&tab_state),
        history_position = history_position,
        history_len = payload.history_len,
    )
}

fn render_browser_session_tabs(payload: &BrowserSessionPayload) -> String {
    let mut tabs = String::new();
    for session in &payload.sessions {
        let class = match (session.current, session.pinned) {
            (true, true) => "session-tab-card current pinned",
            (true, false) => "session-tab-card current",
            (false, true) => "session-tab-card pinned",
            (false, false) => "session-tab-card",
        };
        let pinned_marker = if session.pinned { "Pinned · " } else { "" };
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
        let duplicate_background = format!(
            r#"<a class="session-action" href="{href}" aria-label="Duplicate {id} in background">Duplicate bg</a>"#,
            href = html_escape::encode_double_quoted_attribute(&session.duplicate_background_url),
            id = html_escape::encode_double_quoted_attribute(&session.id),
        );
        let reload = format!(
            r#"<a class="session-action" href="{href}" aria-label="Reload {id}">Reload</a>"#,
            href = html_escape::encode_double_quoted_attribute(&session.reload_url),
            id = html_escape::encode_double_quoted_attribute(&session.id),
        );
        let pin_href = if session.pinned {
            &session.unpin_url
        } else {
            &session.pin_url
        };
        let pin_label = if session.pinned { "Unpin" } else { "Pin" };
        let pin = format!(
            r#"<a class="session-action" href="{href}" aria-label="{label} {id}">{label}</a>"#,
            href = html_escape::encode_double_quoted_attribute(pin_href),
            label = pin_label,
            id = html_escape::encode_double_quoted_attribute(&session.id),
        );
        let clear_label = session.label.as_ref().map_or_else(String::new, |_| {
            format!(
                r#"<a class="session-action" href="{href}" aria-label="Clear label {id}">Clear label</a>"#,
                href = html_escape::encode_double_quoted_attribute(&session.clear_label_url),
                id = html_escape::encode_double_quoted_attribute(&session.id),
            )
        });
        let move_left = if session.can_move_left {
            format!(
                r#"<a class="session-action" href="{href}" aria-label="Move {id} left">Left</a>"#,
                href = html_escape::encode_double_quoted_attribute(&session.move_left_url),
                id = html_escape::encode_double_quoted_attribute(&session.id),
            )
        } else {
            String::new()
        };
        let move_right = if session.can_move_right {
            format!(
                r#"<a class="session-action" href="{href}" aria-label="Move {id} right">Right</a>"#,
                href = html_escape::encode_double_quoted_attribute(&session.move_right_url),
                id = html_escape::encode_double_quoted_attribute(&session.id),
            )
        } else {
            String::new()
        };
        let _ = write!(
            tabs,
            r#"<div class="{class}"><a class="session-tab" href="{href}"><strong>{id} · {pinned_marker}{title}</strong><span>{source}</span></a><div class="session-actions">{reload}{move_left}{move_right}{pin}{duplicate}{duplicate_background}{clear_label}{close}</div></div>"#,
            class = class,
            href = html_escape::encode_double_quoted_attribute(&session.action_url),
            id = html_escape::encode_text(&session.id),
            pinned_marker = pinned_marker,
            title = html_escape::encode_text(&session.title),
            source = html_escape::encode_text(&session.source),
            reload = reload,
            move_left = move_left,
            move_right = move_right,
            pin = pin,
            duplicate = duplicate,
            duplicate_background = duplicate_background,
            clear_label = clear_label,
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
    let tabs_csv_href = browser_session_api_href(&payload.id, "tabs-csv", payload);
    let jump_form = if payload.sessions.len() > 1 {
        format!(
            r#"<form class="session-new" action="/browser" method="get">{common}<input type="hidden" name="action" value="jump-tab"><input type="search" name="q" placeholder="Jump tab" aria-label="Jump tab"><button type="submit">Jump</button></form>"#,
            common = browser_session_common_hidden_inputs(payload),
        )
    } else {
        String::new()
    };
    let current_label = payload
        .sessions
        .iter()
        .find(|session| session.id == payload.id)
        .and_then(|session| session.label.as_deref())
        .unwrap_or_default();
    let label_form = format!(
        r#"<form class="session-new" action="/browser" method="get">{common}<input type="hidden" name="action" value="label-tab"><input type="hidden" name="session" value="{id}"><input type="text" name="label" value="{label}" placeholder="Label current tab" aria-label="Label current tab"><button type="submit">Label</button></form>"#,
        common = browser_session_common_hidden_inputs(payload),
        id = html_escape::encode_double_quoted_attribute(&payload.id),
        label = html_escape::encode_double_quoted_attribute(current_label),
    );
    let tab_search = render_browser_session_tab_search(payload);

    format!(
        r#"<section class="session-shell"><div class="session-title"><h2>Sessions</h2><div class="resource-actions"><span class="meta">{count} open</span><a class="clear-link" href="{tabs_csv_href}">Tabs CSV</a>{forget_saved}</div></div><div class="session-tabs">{tabs}{jump_form}{label_form}{tab_search}<form class="session-new" action="/browser" method="get"><input type="hidden" name="from" value="{back_href}"><input type="hidden" name="width" value="{width}"><input type="hidden" name="height" value="{height}"><input type="hidden" name="viewport_x" value="{viewport_x}"><input type="hidden" name="viewport_y" value="{viewport_y}"><input type="hidden" name="max_bytes" value="{max_bytes}"><input type="text" inputmode="url" autocapitalize="none" spellcheck="false" name="url" placeholder="New session URL" aria-label="New session URL"><button type="submit">New</button></form></div></section>"#,
        count = payload.sessions.len(),
        tabs_csv_href = html_escape::encode_double_quoted_attribute(&tabs_csv_href),
        forget_saved = forget_saved,
        tabs = tabs,
        jump_form = jump_form,
        label_form = label_form,
        tab_search = tab_search,
        back_href = html_escape::encode_double_quoted_attribute(&payload.back_href),
        width = payload.width,
        height = payload.height,
        viewport_x = payload.viewport_x,
        viewport_y = payload.viewport_y,
        max_bytes = payload.max_bytes,
    )
}

fn render_browser_session_tab_search(payload: &BrowserSessionPayload) -> String {
    let search_json_href = browser_session_api_href(&payload.id, "tab-search-json", payload);
    let search_csv_href = browser_session_api_href(&payload.id, "tab-search-csv", payload);
    let clear_href = browser_session_action_href(&payload.id, "clear-tab-search", &[], payload);
    let reload_matches = if payload.tab_search_results.is_empty() {
        String::new()
    } else {
        let href =
            browser_session_action_href(&payload.id, "reload-tab-search-results", &[], payload);
        format!(
            r#"<a class="clear-link" href="{href}">Reload matches</a>"#,
            href = html_escape::encode_double_quoted_attribute(&href),
        )
    };
    let duplicate_matches = if payload.tab_search_results.is_empty() {
        String::new()
    } else {
        let href =
            browser_session_action_href(&payload.id, "duplicate-tab-search-results", &[], payload);
        format!(
            r#"<a class="clear-link" href="{href}">Duplicate matches</a>"#,
            href = html_escape::encode_double_quoted_attribute(&href),
        )
    };
    let move_matches_front = if browser_tab_search_results_can_move(payload, true) {
        let href =
            browser_session_action_href(&payload.id, "move-tab-search-results-front", &[], payload);
        format!(
            r#"<a class="clear-link" href="{href}">Move matches front</a>"#,
            href = html_escape::encode_double_quoted_attribute(&href),
        )
    } else {
        String::new()
    };
    let move_matches_back = if browser_tab_search_results_can_move(payload, false) {
        let href =
            browser_session_action_href(&payload.id, "move-tab-search-results-back", &[], payload);
        format!(
            r#"<a class="clear-link" href="{href}">Move matches end</a>"#,
            href = html_escape::encode_double_quoted_attribute(&href),
        )
    } else {
        String::new()
    };
    let bookmark_matches = if payload.tab_search_results.iter().any(|result| {
        !result.source.trim().is_empty()
            && !payload
                .bookmarks
                .iter()
                .any(|bookmark| bookmark.source == result.source)
    }) {
        let href =
            browser_session_action_href(&payload.id, "bookmark-tab-search-results", &[], payload);
        format!(
            r#"<a class="clear-link" href="{href}">Bookmark matches</a>"#,
            href = html_escape::encode_double_quoted_attribute(&href),
        )
    } else {
        String::new()
    };
    let remove_bookmarks = if browser_tab_search_has_bookmarked_results(payload) {
        let href =
            browser_session_action_href(&payload.id, "remove-tab-search-bookmarks", &[], payload);
        format!(
            r#"<a class="clear-link" href="{href}">Remove bookmarks</a>"#,
            href = html_escape::encode_double_quoted_attribute(&href),
        )
    } else {
        String::new()
    };
    let label_matches = if payload.tab_search_results.is_empty() {
        String::new()
    } else {
        format!(
            r#"<form class="session-new" action="/browser" method="get">{common}<input type="hidden" name="action" value="label-tab-search-results"><input type="text" name="label" placeholder="Label matches" aria-label="Label matches"><button type="submit">Label matches</button></form>"#,
            common = browser_session_common_hidden_inputs(payload),
        )
    };
    let clear_labels = if payload
        .tab_search_results
        .iter()
        .any(|result| result.label.is_some())
    {
        let href =
            browser_session_action_href(&payload.id, "clear-tab-search-labels", &[], payload);
        format!(
            r#"<a class="clear-link" href="{href}">Clear labels</a>"#,
            href = html_escape::encode_double_quoted_attribute(&href),
        )
    } else {
        String::new()
    };
    let pin_matches = if payload
        .tab_search_results
        .iter()
        .any(|result| !result.pinned)
    {
        let href = browser_session_action_href(&payload.id, "pin-tab-search-results", &[], payload);
        format!(
            r#"<a class="clear-link" href="{href}">Pin matches</a>"#,
            href = html_escape::encode_double_quoted_attribute(&href),
        )
    } else {
        String::new()
    };
    let unpin_matches = if payload
        .tab_search_results
        .iter()
        .any(|result| result.pinned)
    {
        let href =
            browser_session_action_href(&payload.id, "unpin-tab-search-results", &[], payload);
        format!(
            r#"<a class="clear-link" href="{href}">Unpin matches</a>"#,
            href = html_escape::encode_double_quoted_attribute(&href),
        )
    } else {
        String::new()
    };
    let close_matches = if payload
        .tab_search_results
        .iter()
        .any(|result| !result.current && !result.pinned)
    {
        let href =
            browser_session_action_href(&payload.id, "close-tab-search-results", &[], payload);
        format!(
            r#"<a class="clear-link" href="{href}">Close matches</a>"#,
            href = html_escape::encode_double_quoted_attribute(&href),
        )
    } else {
        String::new()
    };
    let close_nonmatches = if browser_tab_search_has_closeable_nonmatches(payload) {
        let href =
            browser_session_action_href(&payload.id, "close-tab-search-nonmatches", &[], payload);
        format!(
            r#"<a class="clear-link" href="{href}">Close nonmatches</a>"#,
            href = html_escape::encode_double_quoted_attribute(&href),
        )
    } else {
        String::new()
    };
    let mut rows = String::new();
    for result in &payload.tab_search_results {
        let line = result
            .line
            .map(|line| format!("line {}", line + 1))
            .unwrap_or_else(|| result.field.clone());
        let current = if result.current { " current" } else { "" };
        let pin_href = if result.pinned {
            &result.unpin_url
        } else {
            &result.pin_url
        };
        let pin_label = if result.pinned { "Unpin" } else { "Pin" };
        let close = if result.close_url.trim().is_empty() {
            String::new()
        } else {
            format!(
                r#"<a class="clear-link" href="{href}">Close</a>"#,
                href = html_escape::encode_double_quoted_attribute(&result.close_url),
            )
        };
        let _ = write!(
            rows,
            r#"<tr><td><a class="clear-link" href="{href}">{title}</a></td><td>{field}</td><td>{line}</td><td>{text}</td><td><div class="resource-actions"><a class="clear-link" href="{href}">Open</a><a class="clear-link" href="{reload_href}">Reload</a><a class="clear-link" href="{duplicate_href}">Duplicate</a><a class="clear-link" href="{duplicate_background_href}">Duplicate bg</a><a class="clear-link" href="{pin_href}">{pin_label}</a>{close}</div></td></tr>"#,
            href = html_escape::encode_double_quoted_attribute(&result.action_url),
            reload_href = html_escape::encode_double_quoted_attribute(&result.reload_url),
            duplicate_href = html_escape::encode_double_quoted_attribute(&result.duplicate_url),
            duplicate_background_href =
                html_escape::encode_double_quoted_attribute(&result.duplicate_background_url),
            pin_href = html_escape::encode_double_quoted_attribute(pin_href),
            pin_label = pin_label,
            close = close,
            title = html_escape::encode_text(&format!("{}{}", result.title, current)),
            field = html_escape::encode_text(&result.field),
            line = html_escape::encode_text(&line),
            text = html_escape::encode_text(&result.text),
        );
    }
    if rows.is_empty() && !payload.tab_search_query.trim().is_empty() {
        rows.push_str(r#"<tr><td colspan="5">No open tab matches.</td></tr>"#);
    }
    let results = if payload.tab_search_query.trim().is_empty() {
        String::new()
    } else {
        format!(
            r#"<div class="resource-actions"><span class="meta">{count} matches</span><a class="clear-link" href="{json_href}">Tab Search JSON</a><a class="clear-link" href="{csv_href}">Tab Search CSV</a><a class="clear-link" href="{clear_href}">Clear</a>{reload_matches}{duplicate_matches}{move_matches_front}{move_matches_back}{bookmark_matches}{remove_bookmarks}{pin_matches}{unpin_matches}{close_matches}{close_nonmatches}{clear_labels}</div>{label_matches}<table><thead><tr><th>Tab</th><th>Field</th><th>Line</th><th>Match</th><th>Action</th></tr></thead><tbody>{rows}</tbody></table>"#,
            count = payload.tab_search_results.len(),
            json_href = html_escape::encode_double_quoted_attribute(&search_json_href),
            csv_href = html_escape::encode_double_quoted_attribute(&search_csv_href),
            clear_href = html_escape::encode_double_quoted_attribute(&clear_href),
            reload_matches = reload_matches,
            duplicate_matches = duplicate_matches,
            move_matches_front = move_matches_front,
            move_matches_back = move_matches_back,
            bookmark_matches = bookmark_matches,
            remove_bookmarks = remove_bookmarks,
            pin_matches = pin_matches,
            unpin_matches = unpin_matches,
            close_matches = close_matches,
            close_nonmatches = close_nonmatches,
            clear_labels = clear_labels,
            label_matches = label_matches,
            rows = rows,
        )
    };
    format!(
        r#"<div class="tab-search"><form class="session-new" action="/browser" method="get">{common}<input type="hidden" name="action" value="search-tabs"><input type="search" name="q" value="{query}" placeholder="Search tabs" aria-label="Search tabs"><button type="submit">Search</button></form>{results}</div>"#,
        common = browser_session_common_hidden_inputs(payload),
        query = html_escape::encode_double_quoted_attribute(&payload.tab_search_query),
        results = results,
    )
}

fn render_browser_session_closed_sessions(payload: &BrowserSessionPayload) -> String {
    if payload.closed_sessions.is_empty() {
        return String::new();
    }

    let closed_csv_href = browser_session_api_href(&payload.id, "closed-sessions-csv", payload);
    let restore_background_href =
        browser_session_action_href(&payload.id, "restore-all-closed-background", &[], payload);
    let restore_background_control = nav_control(
        !payload.closed_sessions.is_empty(),
        "Restore all bg",
        &restore_background_href,
    );
    let mut rows = String::new();
    for closed in &payload.closed_sessions {
        let state = if closed.persisted { "saved" } else { "session" };
        let _ = write!(
            rows,
            r#"<div class="session-tab-card"><a class="session-tab" href="{restore_href}"><strong>{id} · {title}</strong><span>{state} · {closed_at} · {source}</span></a><div class="session-actions"><a class="session-action" href="{restore_href}">Restore</a><a class="session-action" href="{new_href}">New session</a><a class="session-action" href="{background_href}">Background</a><a class="session-action" href="{forget_href}">Forget</a></div></div>"#,
            restore_href = html_escape::encode_double_quoted_attribute(&closed.restore_url),
            new_href = html_escape::encode_double_quoted_attribute(&closed.new_session_url),
            background_href =
                html_escape::encode_double_quoted_attribute(&closed.background_restore_url),
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
        r#"<section class="session-shell"><div class="session-title"><h2>Recently closed</h2><div class="resource-actions"><span class="meta">{count} closed</span><a class="clear-link" href="{closed_csv_href}">Closed CSV</a>{restore_background_control}{clear_control}</div></div><div class="session-tabs">{rows}</div></section>"#,
        count = payload.closed_sessions.len(),
        closed_csv_href = html_escape::encode_double_quoted_attribute(&closed_csv_href),
        restore_background_control = restore_background_control,
        clear_control = clear_control,
        rows = rows,
    )
}

fn render_browser_session_bookmarks(payload: &BrowserSessionPayload) -> String {
    let add_href = browser_session_action_href(&payload.id, "add-bookmark", &[], payload);
    let bookmarks_csv_href = browser_session_api_href(&payload.id, "bookmarks-csv", payload);
    let add_label = if payload.current_bookmarked {
        "Bookmarked"
    } else {
        "Add bookmark"
    };
    let add_control = nav_control(!payload.current_bookmarked, add_label, &add_href);
    let add_all_href = browser_session_action_href(&payload.id, "bookmark-all-tabs", &[], payload);
    let add_all_control = nav_control(
        browser_has_unbookmarked_open_tabs(payload),
        "Add all tabs",
        &add_all_href,
    );
    let clear_control = payload
        .bookmarks_clear_url
        .as_ref()
        .map_or_else(String::new, |href| {
            nav_control(!payload.bookmarks.is_empty(), "Clear", href)
        });
    let open_tabs_href =
        browser_session_action_href(&payload.id, "open-bookmarks-new-sessions", &[], payload);
    let open_tabs_control = nav_control(
        !payload.bookmarks.is_empty(),
        "Open all tabs",
        &open_tabs_href,
    );
    let background_control = payload
        .bookmarks_background_url
        .as_ref()
        .map_or_else(String::new, |href| {
            nav_control(!payload.bookmarks.is_empty(), "Open all bg", href)
        });
    let mut rows = String::new();
    for bookmark in &payload.bookmarks {
        let class = if bookmark.current {
            "session-tab-card current"
        } else {
            "session-tab-card"
        };
        let common = browser_session_common_hidden_inputs(payload);
        let _ = write!(
            rows,
            r#"<div class="{class}"><a class="session-tab" href="{href}"><strong>{id} · {title}</strong><span>{source}</span></a><div class="session-actions"><a class="session-action" href="{new_href}">New session</a><a class="session-action" href="{background_href}">Background</a><a class="session-action" href="{remove_href}">Remove</a><form class="session-new" action="/browser" method="get">{common}<input type="hidden" name="action" value="rename-bookmark"><input type="hidden" name="bookmark" value="{bookmark_id}"><input type="text" name="title" value="{title_attr}" aria-label="Bookmark title"><button type="submit">Rename</button></form></div></div>"#,
            class = class,
            href = html_escape::encode_double_quoted_attribute(&bookmark.action_url),
            id = html_escape::encode_text(&bookmark.id),
            title = html_escape::encode_text(&bookmark.title),
            source = html_escape::encode_text(&bookmark.source),
            new_href = html_escape::encode_double_quoted_attribute(&bookmark.new_session_url),
            background_href =
                html_escape::encode_double_quoted_attribute(&bookmark.background_session_url),
            remove_href = html_escape::encode_double_quoted_attribute(&bookmark.remove_url),
            common = common,
            bookmark_id = html_escape::encode_double_quoted_attribute(&bookmark.id),
            title_attr = html_escape::encode_double_quoted_attribute(&bookmark.title),
        );
    }
    if rows.is_empty() {
        rows.push_str(r#"<span class="session-tab-card"><span class="session-tab"><strong>No bookmarks</strong><span>Add the current page to keep it in this browser session.</span></span></span>"#);
    }

    format!(
        r#"<section class="session-shell"><div class="session-title"><h2>Bookmarks</h2><div class="resource-actions"><span class="meta">{count} saved</span><a class="clear-link" href="{bookmarks_csv_href}">Bookmarks CSV</a>{add_control}{add_all_control}{open_tabs_control}{background_control}{clear_control}</div></div><div class="session-tabs">{rows}</div></section>"#,
        count = payload.bookmarks.len(),
        bookmarks_csv_href = html_escape::encode_double_quoted_attribute(&bookmarks_csv_href),
        add_control = add_control,
        add_all_control = add_all_control,
        open_tabs_control = open_tabs_control,
        background_control = background_control,
        clear_control = clear_control,
        rows = rows,
    )
}

fn render_browser_session_profile_history(payload: &BrowserSessionPayload) -> String {
    if !payload.profile_enabled {
        return String::new();
    }

    let profile_history_csv_href =
        browser_session_api_href(&payload.id, "profile-history-csv", payload);
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
            r#"<div class="session-tab-card"><a class="session-tab" href="{href}"><strong>{index} · {title}</strong><span>{visited} · {source}</span></a><div class="session-actions"><a class="session-action" href="{new_href}">New session</a><a class="session-action" href="{background_href}">Background</a><a class="session-action" href="{remove_href}">Remove</a></div></div>"#,
            href = html_escape::encode_double_quoted_attribute(&entry.action_url),
            new_href = html_escape::encode_double_quoted_attribute(&entry.new_session_url),
            background_href =
                html_escape::encode_double_quoted_attribute(&entry.background_session_url),
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
    let bookmark_history_href =
        browser_session_action_href(&payload.id, "bookmark-profile-history", &[], payload);
    let bookmark_history_control = nav_control(
        browser_has_unbookmarked_profile_history(payload),
        "Bookmark history",
        &bookmark_history_href,
    );
    let remove_history_bookmarks_href = browser_session_action_href(
        &payload.id,
        "remove-profile-history-bookmarks",
        &[],
        payload,
    );
    let remove_history_bookmarks_control = nav_control(
        browser_has_bookmarked_profile_history(payload),
        "Remove history bookmarks",
        &remove_history_bookmarks_href,
    );
    let open_tabs_href = browser_session_action_href(
        &payload.id,
        "open-profile-history-new-sessions",
        &[("limit", DEFAULT_BULK_BACKGROUND_LINKS.to_string())],
        payload,
    );
    let open_tabs_control = nav_control(
        !payload.profile_history.is_empty(),
        "Open history tabs",
        &open_tabs_href,
    );
    let open_background_href = browser_session_action_href(
        &payload.id,
        "open-profile-history-background-sessions",
        &[("limit", DEFAULT_BULK_BACKGROUND_LINKS.to_string())],
        payload,
    );
    let open_background_control = nav_control(
        !payload.profile_history.is_empty(),
        "Open history bg",
        &open_background_href,
    );

    format!(
        r#"<section class="session-shell"><div class="session-title"><h2>Profile history</h2><div class="resource-actions"><span class="meta">{count} recent</span><a class="clear-link" href="{profile_history_csv_href}">Profile History CSV</a>{bookmark_history_control}{remove_history_bookmarks_control}{open_tabs_control}{open_background_control}{clear_control}</div></div>{error}<div class="session-tabs">{rows}</div></section>"#,
        count = payload.profile_history.len(),
        profile_history_csv_href =
            html_escape::encode_double_quoted_attribute(&profile_history_csv_href),
        bookmark_history_control = bookmark_history_control,
        remove_history_bookmarks_control = remove_history_bookmarks_control,
        open_tabs_control = open_tabs_control,
        open_background_control = open_background_control,
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
        r##"<form class="browser-action" action="/browser" method="get">{common}<input type="hidden" name="action" value="link-text"><label for="browser-link-text">Text</label><input id="browser-link-text" type="text" name="text" placeholder="Visible text"><button type="submit">Open</button></form><form class="browser-action" action="/browser" method="get">{common}<input type="hidden" name="action" value="link-text-new-session"><label for="browser-link-text-new-session">Text</label><input id="browser-link-text-new-session" type="text" name="text" placeholder="Visible text"><button type="submit">New session</button></form><form class="browser-action" action="/browser" method="get">{common}<input type="hidden" name="action" value="link-text-background-session"><label for="browser-link-text-background-session">Text</label><input id="browser-link-text-background-session" type="text" name="text" placeholder="Visible text"><button type="submit">Background</button></form><form class="browser-action" action="/browser" method="get">{common}<input type="hidden" name="action" value="link-selector"><label for="browser-link-selector">Selector</label><input id="browser-link-selector" type="text" name="selector" placeholder="#link, a.primary"><button type="submit">Open</button></form><form class="browser-action" action="/browser" method="get">{common}<input type="hidden" name="action" value="link-selector-new-session"><label for="browser-link-selector-new-session">Selector</label><input id="browser-link-selector-new-session" type="text" name="selector" placeholder="#link, a.primary"><button type="submit">New session</button></form><form class="browser-action" action="/browser" method="get">{common}<input type="hidden" name="action" value="link-selector-background-session"><label for="browser-link-selector-background-session">Selector</label><input id="browser-link-selector-background-session" type="text" name="selector" placeholder="#link, a.primary"><button type="submit">Background</button></form>"##,
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
    let focus_cycle_actions = if browser_session_has_focusable_controls(payload) {
        let tab_href = browser_session_action_href(&payload.id, "focus-next", &[], payload);
        let shift_tab_href = browser_session_action_href(&payload.id, "focus-prev", &[], payload);
        format!(
            r#"<a href="{tab_href}">Tab</a><a href="{shift_tab_href}">Shift Tab</a>"#,
            tab_href = html_escape::encode_double_quoted_attribute(&tab_href),
            shift_tab_href = html_escape::encode_double_quoted_attribute(&shift_tab_href),
        )
    } else {
        String::new()
    };
    let focused_kind = payload
        .focused
        .as_ref()
        .map(|focused| focused.kind.as_str());
    let text_actions = if focused_kind.is_some_and(form_control_is_text_editable) {
        let backspace_href = browser_session_action_href(
            &payload.id,
            "backspace",
            &[("count", "1".to_owned())],
            payload,
        );
        let clear_href = browser_session_action_href(&payload.id, "clear-input", &[], payload);
        format!(
            r#"<a href="{backspace_href}">Backspace</a><a href="{clear_href}">Clear Input</a>"#,
            backspace_href = html_escape::encode_double_quoted_attribute(&backspace_href),
            clear_href = html_escape::encode_double_quoted_attribute(&clear_href),
        )
    } else {
        String::new()
    };
    let enter_action = if payload.focused.is_some() {
        let enter_href = browser_session_action_href(&payload.id, "enter", &[], payload);
        format!(
            r#"<a href="{enter_href}">Enter</a>"#,
            enter_href = html_escape::encode_double_quoted_attribute(&enter_href),
        )
    } else {
        String::new()
    };
    let space_action = if focused_kind.is_some_and(form_control_is_checkable) {
        let space_href = browser_session_action_href(&payload.id, "space", &[], payload);
        format!(
            r#"<a href="{space_href}">Space</a>"#,
            space_href = html_escape::encode_double_quoted_attribute(&space_href),
        )
    } else {
        String::new()
    };
    let choose_form = if focused_kind.is_some_and(|kind| kind.eq_ignore_ascii_case("select")) {
        format!(
            r#"<form class="browser-action" action="/browser" method="get">{common}<input type="hidden" name="action" value="choose"><label for="browser-choose-value">Choose</label><input id="browser-choose-value" type="text" name="value" placeholder="option value"><button type="submit">Choose</button></form>"#,
            common = browser_session_common_hidden_inputs(payload),
        )
    } else {
        String::new()
    };
    let type_form = if focused_kind.is_some_and(form_control_is_text_editable) {
        format!(
            r#"<form class="browser-action" action="/browser" method="get">{common}<input type="hidden" name="action" value="type-text"><label for="browser-type-text">Type</label><input id="browser-type-text" type="text" name="text" placeholder="text"><button type="submit">Type</button></form>"#,
            common = browser_session_common_hidden_inputs(payload),
        )
    } else {
        String::new()
    };

    format!(
        r##"<div class="meta">{focused}</div><div class="browser-actions"><form class="browser-action" action="/browser" method="get">{common}<input type="hidden" name="action" value="focus-selector"><label for="browser-focus-selector">Focus</label><input id="browser-focus-selector" type="text" name="selector" placeholder="#field, label, button"><button type="submit">Focus</button></form>{type_form}{choose_form}</div><div class="keyboard-action-row">{focus_cycle_actions}{text_actions}{enter_action}{space_action}</div>"##,
        focused = html_escape::encode_text(&focused),
        common = browser_session_common_hidden_inputs(payload),
        type_form = type_form,
        choose_form = choose_form,
        focus_cycle_actions = focus_cycle_actions,
        text_actions = text_actions,
        enter_action = enter_action,
        space_action = space_action,
    )
}

fn browser_session_has_focusable_controls(payload: &BrowserSessionPayload) -> bool {
    payload
        .forms
        .iter()
        .flat_map(|form| form.controls.iter())
        .any(|control| control.focus_url.is_some())
}

fn render_browser_session_inspector(payload: &BrowserSessionPayload) -> String {
    let state = render_browser_session_state_export(payload);
    let history = render_browser_session_history(payload);
    let anchors = render_browser_session_anchors(payload);
    let cookies = render_browser_session_cookies(payload);
    let local_storage_clear_href = (!payload.local_storage.is_empty())
        .then(|| browser_session_action_href(&payload.id, "clear-local-storage", &[], payload));
    let local_storage = render_browser_session_storage(
        "localStorage",
        &payload.local_storage,
        local_storage_clear_href.as_deref(),
    );
    let session_storage_clear_href = (!payload.session_storage.is_empty())
        .then(|| browser_session_action_href(&payload.id, "clear-session-storage", &[], payload));
    let session_storage = render_browser_session_storage(
        "sessionStorage",
        &payload.session_storage,
        session_storage_clear_href.as_deref(),
    );
    let resources = render_browser_session_resources(payload);
    format!("{state}{history}{anchors}{cookies}{local_storage}{session_storage}{resources}")
}

fn render_browser_session_state_export(payload: &BrowserSessionPayload) -> String {
    let state_json_href = browser_session_api_href(&payload.id, "session-state", payload);
    let state_csv_href = browser_session_api_href(&payload.id, "session-state-csv", payload);
    let viewport_text_href = browser_session_api_href(&payload.id, "viewport-text", payload);
    let page_text_href = browser_session_api_href(&payload.id, "page-text", payload);
    format!(
        r#"<section><div class="section-title"><h3>Session State</h3><div class="resource-actions"><a class="clear-link" href="{state_json_href}">State JSON</a><a class="clear-link" href="{state_csv_href}">State CSV</a><a class="clear-link" href="{viewport_text_href}">Viewport Text</a><a class="clear-link" href="{page_text_href}">Page Text</a></div></div><div class="meta">cookies {cookies} · localStorage {local_storage} · sessionStorage {session_storage}</div></section>"#,
        state_json_href = html_escape::encode_double_quoted_attribute(&state_json_href),
        state_csv_href = html_escape::encode_double_quoted_attribute(&state_csv_href),
        viewport_text_href = html_escape::encode_double_quoted_attribute(&viewport_text_href),
        page_text_href = html_escape::encode_double_quoted_attribute(&page_text_href),
        cookies = payload.cookies.len(),
        local_storage = payload.local_storage.len(),
        session_storage = payload.session_storage.len(),
    )
}

fn render_browser_session_history(payload: &BrowserSessionPayload) -> String {
    let history_csv_href = browser_session_api_href(&payload.id, "history-csv", payload);
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
            r#"<tr{row_class}><td>{index}</td><td>{marker}</td><td>{title}</td><td>{source}</td><td>{target}</td><td><div class="resource-actions"><a class="clear-link" href="{href}">Open</a><a class="clear-link" href="{new_href}">New session</a><a class="clear-link" href="{background_href}">Background</a></div></td></tr>"#,
            row_class = row_class,
            index = entry.index + 1,
            marker = marker,
            title = html_escape::encode_text(&entry.title),
            source = html_escape::encode_text(&entry.source),
            target = html_escape::encode_text(&entry.target),
            href = html_escape::encode_double_quoted_attribute(&entry.action_url),
            new_href = html_escape::encode_double_quoted_attribute(&entry.new_session_url),
            background_href =
                html_escape::encode_double_quoted_attribute(&entry.background_session_url),
        );
    }
    if rows.is_empty() {
        rows.push_str(r#"<tr><td colspan="6">No browser session history.</td></tr>"#);
    }
    format!(
        r#"<section><div class="section-title"><h3>History</h3><div class="resource-actions"><span class="meta">{count} entries</span><a class="clear-link" href="{history_csv_href}">History CSV</a></div></div><table><thead><tr><th>#</th><th>State</th><th>Title</th><th>Source</th><th>Target</th><th>Action</th></tr></thead><tbody>{rows}</tbody></table></section>"#,
        count = payload.history.len(),
        history_csv_href = html_escape::encode_double_quoted_attribute(&history_csv_href),
    )
}

fn render_browser_session_anchors(payload: &BrowserSessionPayload) -> String {
    let anchors_csv_href = browser_session_api_href(&payload.id, "anchors-csv", payload);
    let mut rows = String::new();
    for anchor in &payload.anchors {
        let _ = write!(
            rows,
            r#"<tr><td>{index}</td><td>{name}</td><td>{y}</td><td><div class="resource-actions"><a class="clear-link" href="{href}">Jump</a><a class="clear-link" href="{new_href}">New session</a><a class="clear-link" href="{background_href}">Background</a></div></td></tr>"#,
            index = anchor.index + 1,
            name = html_escape::encode_text(&anchor.name),
            y = anchor.y,
            href = html_escape::encode_double_quoted_attribute(&anchor.action_url),
            new_href = html_escape::encode_double_quoted_attribute(&anchor.new_session_url),
            background_href =
                html_escape::encode_double_quoted_attribute(&anchor.background_session_url),
        );
    }
    if payload.anchor_count > payload.anchors.len() {
        let _ = write!(
            rows,
            r#"<tr><td colspan="4">{count} more anchors omitted.</td></tr>"#,
            count = payload.anchor_count - payload.anchors.len(),
        );
    }
    if rows.is_empty() {
        rows.push_str(r#"<tr><td colspan="4">No page anchors discovered.</td></tr>"#);
    }
    format!(
        r#"<section><div class="section-title"><h3>Page Anchors ({count})</h3><div class="resource-actions"><a class="clear-link" href="{anchors_csv_href}">Anchors CSV</a></div></div><table><thead><tr><th>#</th><th>Name</th><th>Y</th><th>Action</th></tr></thead><tbody>{rows}</tbody></table></section>"#,
        count = payload.anchor_count,
        anchors_csv_href = html_escape::encode_double_quoted_attribute(&anchors_csv_href),
        rows = rows,
    )
}

fn render_browser_session_cookies(payload: &BrowserSessionPayload) -> String {
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
    let clear = if payload.cookies.is_empty() {
        String::new()
    } else {
        let clear_href = browser_session_action_href(&payload.id, "clear-cookies", &[], payload);
        format!(
            r#"<a class="clear-link" href="{clear_href}">Clear</a>"#,
            clear_href = html_escape::encode_double_quoted_attribute(&clear_href),
        )
    };
    format!(
        r#"<section><div class="section-title"><h3>Cookies ({count})</h3>{clear}</div><table><thead><tr><th>Name</th><th>Value</th><th>Domain</th><th>Path</th><th>Flags</th></tr></thead><tbody>{rows}</tbody></table></section>"#,
        count = payload.cookies.len(),
        clear = clear,
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

fn render_browser_session_resource_quick_actions(payload: &BrowserSessionPayload) -> String {
    if payload.resource_count == 0 {
        return String::new();
    }

    let action_urls = browser_session_resource_action_urls(payload);
    let mut actions = String::new();
    actions.push_str(
        &browser_session_resource_action_link_with_class_and_attributes(
            action_urls.make_visual.as_deref(),
            "Make visual",
            "clear-link primary-action",
            r#" data-browser-resource-action data-browser-make-visual-action data-browser-resource-status="Making visual...""#,
        ),
    );
    actions.push_str(&browser_session_resource_action_link_with_status(
        action_urls.fetch_resources.as_deref(),
        "Fetch resources",
        "Fetching resources...",
    ));
    actions.push_str(&browser_session_resource_action_link_with_status(
        action_urls.apply_stylesheets.as_deref(),
        "Apply styles",
        "Applying styles...",
    ));
    actions.push_str(&browser_session_resource_action_link_with_status(
        action_urls.run_scripts.as_deref(),
        "Run scripts",
        "Running scripts...",
    ));
    let load_images_label = format!(
        "Load {}",
        browser_resource_count_label(payload.resource_image_count, "image", "images")
    );
    actions.push_str(&browser_session_resource_action_link_with_status(
        action_urls.load_images.as_deref(),
        &load_images_label,
        "Loading images...",
    ));
    let resources_json_href = browser_session_api_href(&payload.id, "resources-json", payload);
    let resources_csv_href = browser_session_api_href(&payload.id, "resources-csv", payload);
    actions.push_str(&browser_session_resource_action_link(
        Some(&resources_json_href),
        "Resources JSON",
    ));
    actions.push_str(&browser_session_resource_action_link(
        Some(&resources_csv_href),
        "Resources CSV",
    ));
    let has_status_action = action_urls.fetch_resources.is_some()
        || action_urls.make_visual.is_some()
        || action_urls.apply_stylesheets.is_some()
        || action_urls.run_scripts.is_some()
        || action_urls.load_images.is_some();
    let visual_status = if has_status_action {
        r#"<span class="resource-visual-status resource-action-status" data-browser-visual-status data-browser-resource-status-output aria-live="polite"></span>"#
                .to_owned()
    } else {
        String::new()
    };
    let visual_status_script = if has_status_action {
        render_browser_session_make_visual_status_script().to_owned()
    } else {
        String::new()
    };

    format!(
        r#"<details class="resource-quick-actions resource-quick-details" data-browser-resource-actions data-browser-auto-visual-control><summary><span class="resource-quick-summary"><strong>Resource actions</strong><span>{summary}</span></span></summary><div class="resource-actions">{actions}{visual_status}</div></details>{visual_status_script}"#,
        summary = html_escape::encode_text(&browser_session_resource_summary(payload)),
        actions = actions,
        visual_status = visual_status,
        visual_status_script = visual_status_script,
    )
}

fn browser_session_resource_action_link(href: Option<&str>, label: &str) -> String {
    browser_session_resource_action_link_with_class(href, label, "clear-link")
}

fn browser_session_resource_action_link_with_status(
    href: Option<&str>,
    label: &str,
    status: &str,
) -> String {
    let attributes = format!(
        r#" data-browser-resource-action data-browser-resource-status="{}""#,
        html_escape::encode_double_quoted_attribute(status),
    );
    browser_session_resource_action_link_with_class_and_attributes(
        href,
        label,
        "clear-link",
        &attributes,
    )
}

fn browser_session_resource_action_link_with_class(
    href: Option<&str>,
    label: &str,
    class: &str,
) -> String {
    browser_session_resource_action_link_with_class_and_attributes(href, label, class, "")
}

fn browser_session_resource_action_link_with_class_and_attributes(
    href: Option<&str>,
    label: &str,
    class: &str,
    attributes: &str,
) -> String {
    href.map_or_else(String::new, |href| {
        format!(
            r#"<a class="{class}" href="{href}"{attributes}>{label}</a>"#,
            class = html_escape::encode_double_quoted_attribute(class),
            href = html_escape::encode_double_quoted_attribute(href),
            attributes = attributes,
            label = html_escape::encode_text(label),
        )
    })
}

fn render_browser_session_make_visual_status_script() -> &'static str {
    r#"<script data-browser-make-visual-status data-browser-resource-action-status>
(() => {
  document.addEventListener("click", (event) => {
    const eventTarget = event.target instanceof Element ? event.target : event.target && event.target.parentElement;
    const target = eventTarget && typeof eventTarget.closest === "function" ? eventTarget.closest("[data-browser-resource-action]") : null;
    if (!target) {
      return;
    }
    const section = target.closest("[data-browser-resource-actions]");
    const status = section ? section.querySelector("[data-browser-resource-status-output], [data-browser-visual-status]") : null;
    const statusOutputs = Array.from(document.querySelectorAll("[data-browser-resource-status-output]"));
    const message = target.dataset.browserResourceStatus || "Working...";
    if (status) {
      status.textContent = message;
    }
    for (const output of statusOutputs) {
      output.textContent = message;
    }
    if (section) {
      if (target.hasAttribute("data-browser-make-visual-action")) {
        section.dataset.visualPending = "true";
      }
      section.dataset.resourcePending = "true";
      section.setAttribute("aria-busy", "true");
    }
    target.setAttribute("aria-disabled", "true");
  });
})();
</script>"#
}

fn browser_session_resource_summary(payload: &BrowserSessionPayload) -> String {
    let image_count_label =
        browser_resource_count_label(payload.resource_image_count, "image", "images");
    let mut resource_summary = vec![image_count_label.clone()];
    if payload.resource_stylesheet_count > 0 {
        resource_summary.push(browser_resource_count_label(
            payload.resource_stylesheet_count,
            "stylesheet",
            "stylesheets",
        ));
    }
    if payload.resource_script_count > 0 {
        resource_summary.push(browser_resource_count_label(
            payload.resource_script_count,
            "script",
            "scripts",
        ));
    }
    let other_count = payload.resource_count.saturating_sub(
        payload.resource_image_count
            + payload.resource_stylesheet_count
            + payload.resource_script_count,
    );
    if other_count > 0 {
        resource_summary.push(browser_resource_count_label(
            other_count,
            "other resource",
            "other resources",
        ));
    }
    resource_summary.join(", ")
}

fn render_browser_session_resources(payload: &BrowserSessionPayload) -> String {
    let image_count_label =
        browser_resource_count_label(payload.resource_image_count, "image", "images");
    let resource_summary = browser_session_resource_summary(payload);
    let action_urls = browser_session_resource_action_urls(payload);
    let fetch_control = if payload.resource_count == 0 {
        String::new()
    } else {
        browser_session_resource_action_link_with_status(
            action_urls.fetch_resources.as_deref(),
            "Fetch",
            "Fetching resources...",
        )
    };
    let make_visual_control = if payload.resource_stylesheet_count == 0
        && payload.resource_image_count == 0
    {
        String::new()
    } else {
        browser_session_resource_action_link_with_class_and_attributes(
            action_urls.make_visual.as_deref(),
            "Make visual",
            "clear-link primary-action",
            r#" data-browser-resource-action data-browser-make-visual-action data-browser-resource-status="Making visual...""#,
        )
    };
    let styles_control = if payload.resource_stylesheet_count == 0 {
        String::new()
    } else {
        browser_session_resource_action_link_with_status(
            action_urls.apply_stylesheets.as_deref(),
            "Apply styles",
            "Applying styles...",
        )
    };
    let scripts_control = if payload.resource_script_count == 0 {
        String::new()
    } else {
        browser_session_resource_action_link_with_status(
            action_urls.run_scripts.as_deref(),
            "Run scripts",
            "Running scripts...",
        )
    };
    let load_images_control = if payload.resource_image_count == 0 {
        String::new()
    } else {
        let load_images_label = format!("Load {image_count_label}");
        browser_session_resource_action_link_with_status(
            action_urls.load_images.as_deref(),
            &load_images_label,
            "Loading images...",
        )
    };
    let open_tabs_href = browser_session_action_href(
        &payload.id,
        "open-resources-new-sessions",
        &[("limit", DEFAULT_BULK_BACKGROUND_LINKS.to_string())],
        payload,
    );
    let open_background_href = browser_session_action_href(
        &payload.id,
        "open-resources-background-sessions",
        &[("limit", DEFAULT_BULK_BACKGROUND_LINKS.to_string())],
        payload,
    );
    let open_resource_controls = (!payload.resources.is_empty())
        .then(|| {
            format!(
                r#"<a class="clear-link" href="{open_tabs_href}">Open resources tabs</a><a class="clear-link" href="{open_background_href}">Open resources bg</a>"#,
                open_tabs_href = html_escape::encode_double_quoted_attribute(&open_tabs_href),
                open_background_href =
                    html_escape::encode_double_quoted_attribute(&open_background_href),
            )
        })
        .unwrap_or_default();
    let resources_json_href = browser_session_api_href(&payload.id, "resources-json", payload);
    let resources_csv_href = browser_session_api_href(&payload.id, "resources-csv", payload);
    let clear_report = payload
        .resource_report
        .as_ref()
        .map_or_else(String::new, |_| {
            let clear_href =
                browser_session_action_href(&payload.id, "clear-resource-report", &[], payload);
            let report_json_href =
                browser_session_api_href(&payload.id, "resource-report-json", payload);
            let report_csv_href =
                browser_session_api_href(&payload.id, "resource-report-csv", payload);
            format!(
                r#"<a class="clear-link" href="{report_json_href}">Report JSON</a><a class="clear-link" href="{report_csv_href}">Report CSV</a><a class="clear-link" href="{clear_href}">Clear report</a>"#,
                report_json_href = html_escape::encode_double_quoted_attribute(&report_json_href),
                report_csv_href = html_escape::encode_double_quoted_attribute(&report_csv_href),
                clear_href = html_escape::encode_double_quoted_attribute(&clear_href),
            )
        });
    let report = render_browser_session_resource_report(payload.resource_report.as_ref());
    let mut rows = String::new();
    for resource in &payload.resources {
        let _ = write!(
            rows,
            r#"<tr><td>{kind}</td><td>{initiator}</td><td>{url}</td><td>{resolved}</td><td>{detail}</td><td><div class="resource-actions"><a class="clear-link" href="{open_href}">Open</a><a class="clear-link" href="{new_href}">New session</a><a class="clear-link" href="{background_href}">Background</a></div></td></tr>"#,
            kind = html_escape::encode_text(&resource.kind),
            initiator = html_escape::encode_text(&resource.initiator),
            url = html_escape::encode_text(&resource.url),
            resolved = html_escape::encode_text(&resource.resolved),
            detail = html_escape::encode_text(&resource.details),
            open_href = html_escape::encode_double_quoted_attribute(&resource.open_url),
            new_href = html_escape::encode_double_quoted_attribute(&resource.new_session_url),
            background_href =
                html_escape::encode_double_quoted_attribute(&resource.background_session_url),
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
    let resource_action_status = if payload.resource_count == 0 {
        String::new()
    } else {
        r#"<span class="resource-action-status" data-browser-resource-status-output aria-live="polite"></span>"#
            .to_owned()
    };
    format!(
        r#"<section data-browser-resource-actions data-browser-auto-visual-control><div class="section-title"><h3>Resources ({count})</h3><div class="resource-actions"><span class="meta">{resource_summary}</span><a class="clear-link" href="{resources_json_href}">Resources JSON</a><a class="clear-link" href="{resources_csv_href}">Resources CSV</a>{open_resource_controls}{fetch_control}{make_visual_control}{styles_control}{scripts_control}{load_images_control}{clear_report}{resource_action_status}</div></div>{report}<table><thead><tr><th>Kind</th><th>Initiator</th><th>URL</th><th>Resolved</th><th>Details</th><th>Action</th></tr></thead><tbody>{rows}</tbody></table></section>"#,
        count = payload.resource_count,
        resource_summary = html_escape::encode_text(&resource_summary),
        resources_json_href = html_escape::encode_double_quoted_attribute(&resources_json_href),
        resources_csv_href = html_escape::encode_double_quoted_attribute(&resources_csv_href),
        open_resource_controls = open_resource_controls,
        fetch_control = fetch_control,
        make_visual_control = make_visual_control,
        styles_control = styles_control,
        scripts_control = scripts_control,
        load_images_control = load_images_control,
        clear_report = clear_report,
        resource_action_status = resource_action_status,
        report = report,
    )
}

fn browser_resource_count_label(count: usize, singular: &str, plural: &str) -> String {
    if count == 1 {
        format!("1 {singular}")
    } else {
        format!("{count} {plural}")
    }
}

fn render_browser_session_resource_report(
    report: Option<&BrowserSessionResourceReportPayload>,
) -> String {
    let Some(report) = report else {
        return String::new();
    };

    let status = browser_session_resource_report_status(report);

    let mut rows = String::new();
    for resource in report.resources.iter().take(20) {
        let source = resource.source.as_deref().unwrap_or("-");
        let content_type = resource.content_type.as_deref().unwrap_or("-");
        let error = resource.error.as_deref().unwrap_or("-");
        let _ = write!(
            rows,
            r#"<tr><td>{status}</td><td>{kind}</td><td>{bytes}</td><td>{source}</td><td>{url}</td><td>{content_type}</td><td>{error}</td></tr>"#,
            status = html_escape::encode_text(&resource.status),
            kind = html_escape::encode_text(&resource.kind),
            bytes = resource.bytes,
            source = html_escape::encode_text(source),
            url = html_escape::encode_text(&resource.resolved),
            content_type = html_escape::encode_text(content_type),
            error = html_escape::encode_text(error),
        );
    }
    if report.resources.len() > 20 {
        let _ = write!(
            rows,
            r#"<tr><td colspan="7">{count} more resource results omitted.</td></tr>"#,
            count = report.resources.len() - 20,
        );
    }
    if rows.is_empty() {
        rows.push_str(r#"<tr><td colspan="7">No resource results.</td></tr>"#);
    }

    format!(
        r#"<div class="resource-report"><div class="resource-report-summary">{summary}</div><table><thead><tr><th>Status</th><th>Kind</th><th>Bytes</th><th>Source</th><th>Resolved</th><th>Content Type</th><th>Error</th></tr></thead><tbody>{rows}</tbody></table></div>"#,
        summary = html_escape::encode_text(&status),
        rows = rows,
    )
}

fn browser_session_resource_report_status(report: &BrowserSessionResourceReportPayload) -> String {
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
    status
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

fn browser_resource_detail(
    rel: Option<&str>,
    media: Option<&str>,
    alt: Option<&str>,
    type_hint: Option<&str>,
) -> String {
    let mut details = Vec::new();
    if let Some(rel) = rel.filter(|value| !value.is_empty()) {
        details.push(format!("rel={rel}"));
    }
    if let Some(media) = media.filter(|value| !value.is_empty()) {
        details.push(format!("media={media}"));
    }
    if let Some(alt) = alt.filter(|value| !value.is_empty()) {
        details.push(format!("alt={alt}"));
    }
    if let Some(type_hint) = type_hint.filter(|value| !value.is_empty()) {
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
            r#"<div class="control"><label>Submit</label><div class="details">{method} {target}</div><div class="resource-actions"><a class="small-action" href="{href}">Submit form</a><a class="small-action" href="{new_href}">New session</a><a class="small-action" href="{background_href}">Background</a></div></div></section>"#,
            method = html_escape::encode_text(&form.method.to_ascii_uppercase()),
            target = html_escape::encode_text(&form.resolved_action),
            href = html_escape::encode_double_quoted_attribute(&form.submit_url),
            new_href = html_escape::encode_double_quoted_attribute(&form.submit_new_session_url),
            background_href =
                html_escape::encode_double_quoted_attribute(&form.submit_background_session_url),
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
        let focus = browser_session_control_focus_link(control);
        let toggle = control.toggle_url.as_deref().map_or_else(
            || r#"<span class="details">read-only</span>"#.to_owned(),
            |href| {
                format!(
                    r#"<a class="small-action" href="{href}">Toggle</a>"#,
                    href = html_escape::encode_double_quoted_attribute(href),
                )
            },
        );
        return format!(
            r#"<div class="control"><label>{label}</label><div class="details">{kind} · {state}{disabled}</div><div class="resource-actions">{focus}{toggle}</div></div>"#,
            label = html_escape::encode_text(&label),
            kind = html_escape::encode_text(&control.kind),
            state = state,
            disabled = disabled,
            focus = focus,
            toggle = toggle,
        );
    }

    if !control.options.is_empty() {
        if control.disabled {
            let value = if control.value.trim().is_empty() {
                "-"
            } else {
                control.value.as_str()
            };
            return format!(
                r#"<div class="control"><label>{label}</label><div class="details">{kind} · {value} disabled</div><div class="resource-actions"><span class="details">read-only</span></div></div>"#,
                label = html_escape::encode_text(&label),
                kind = html_escape::encode_text(&control.kind),
                value = html_escape::encode_text(value),
            );
        }
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
        let focus = browser_session_control_focus_link(control);
        let option_links = browser_session_select_option_links(control);
        return format!(
            r#"<form class="control" action="/browser" method="get">{common}<input type="hidden" name="action" value="select"><input type="hidden" name="form" value="{form_index}"><input type="hidden" name="control" value="{control_index}"><label>{label}</label><select name="value">{options}</select><div class="resource-actions">{focus}<button type="submit">Set</button>{option_links}</div></form>"#,
            common = browser_session_common_hidden_inputs(payload),
            form_index = form.index,
            control_index = control.index,
            label = html_escape::encode_text(&label),
            options = options,
            focus = focus,
            option_links = option_links,
        );
    }

    if control.fill_url.is_some() || control.type_url.is_some() {
        let focus = browser_session_control_focus_link(control);
        let clear = browser_session_control_clear_link(control);
        let action = if control.type_url.is_some() {
            "type-control"
        } else {
            "fill-control"
        };
        let button = if control.type_url.is_some() {
            "Type"
        } else {
            "Set"
        };
        return format!(
            r#"<form class="control" action="/browser" method="get">{common}<input type="hidden" name="action" value="{action}"><input type="hidden" name="form" value="{form_index}"><input type="hidden" name="control" value="{control_index}"><label>{label}</label><input type="text" name="value" value="{value}"><div class="resource-actions">{focus}{clear}<button type="submit">{button}</button></div></form>"#,
            common = browser_session_common_hidden_inputs(payload),
            action = action,
            form_index = form.index,
            control_index = control.index,
            label = html_escape::encode_text(&label),
            value = html_escape::encode_double_quoted_attribute(&control.value),
            focus = focus,
            clear = clear,
            button = button,
        );
    }

    let focus = browser_session_control_focus_link(control);
    let activate = browser_session_control_activate_link(control);
    let activate_new_session = browser_session_control_activate_new_session_link(control);
    let activate_background_session =
        browser_session_control_activate_background_session_link(control);
    format!(
        r#"<div class="control"><label>{label}</label><div class="details">{kind} · {value}</div><div class="resource-actions">{focus}{activate}{activate_new_session}{activate_background_session}<span class="details">read-only</span></div></div>"#,
        label = html_escape::encode_text(&label),
        kind = html_escape::encode_text(&control.kind),
        value = html_escape::encode_text(&control.value),
        focus = focus,
        activate = activate,
        activate_new_session = activate_new_session,
        activate_background_session = activate_background_session,
    )
}

fn browser_session_control_focus_link(control: &BrowserSessionFormControlPayload) -> String {
    control
        .focus_url
        .as_deref()
        .map_or_else(String::new, |href| {
            format!(
                r#"<a class="small-action" href="{href}">Focus</a>"#,
                href = html_escape::encode_double_quoted_attribute(href),
            )
        })
}

fn browser_session_control_clear_link(control: &BrowserSessionFormControlPayload) -> String {
    control
        .clear_url
        .as_deref()
        .map_or_else(String::new, |href| {
            format!(
                r#"<a class="small-action" href="{href}">Clear</a>"#,
                href = html_escape::encode_double_quoted_attribute(href),
            )
        })
}

fn browser_session_control_activate_link(control: &BrowserSessionFormControlPayload) -> String {
    control
        .activate_url
        .as_deref()
        .map_or_else(String::new, |href| {
            format!(
                r#"<a class="small-action" href="{href}">Activate</a>"#,
                href = html_escape::encode_double_quoted_attribute(href),
            )
        })
}

fn browser_session_control_activate_new_session_link(
    control: &BrowserSessionFormControlPayload,
) -> String {
    control
        .activate_new_session_url
        .as_deref()
        .map_or_else(String::new, |href| {
            format!(
                r#"<a class="small-action" href="{href}">New session</a>"#,
                href = html_escape::encode_double_quoted_attribute(href),
            )
        })
}

fn browser_session_control_activate_background_session_link(
    control: &BrowserSessionFormControlPayload,
) -> String {
    control
        .activate_background_session_url
        .as_deref()
        .map_or_else(String::new, |href| {
            format!(
                r#"<a class="small-action" href="{href}">Background</a>"#,
                href = html_escape::encode_double_quoted_attribute(href),
            )
        })
}

fn browser_session_select_option_links(control: &BrowserSessionFormControlPayload) -> String {
    control
        .options
        .iter()
        .filter_map(|option| option.select_url.as_deref().map(|href| (option, href)))
        .map(|(option, href)| {
            format!(
                r#"<a class="small-action" href="{href}">Choose {label}</a>"#,
                href = html_escape::encode_double_quoted_attribute(href),
                label = html_escape::encode_text(&option.label),
            )
        })
        .collect::<String>()
}

fn browser_session_common_hidden_inputs(payload: &BrowserSessionPayload) -> String {
    let source_input =
        browser_safe_source_param(&payload.source).map_or_else(String::new, |source| {
            format!(
                r#"<input type="hidden" name="source" value="{source}">"#,
                source = html_escape::encode_double_quoted_attribute(source),
            )
        });
    format!(
        r#"<input type="hidden" name="id" value="{id}"><input type="hidden" name="from" value="{back_href}"><input type="hidden" name="width" value="{width}"><input type="hidden" name="height" value="{height}"><input type="hidden" name="viewport_x" value="{viewport_x}"><input type="hidden" name="viewport_y" value="{viewport_y}"><input type="hidden" name="max_bytes" value="{max_bytes}">{source_input}"#,
        id = html_escape::encode_double_quoted_attribute(&payload.id),
        back_href = html_escape::encode_double_quoted_attribute(&payload.back_href),
        width = payload.width,
        height = payload.height,
        viewport_x = payload.viewport_x,
        viewport_y = payload.viewport_y,
        max_bytes = payload.max_bytes,
        source_input = source_input,
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

fn form_control_is_activatable(kind: &str) -> bool {
    kind.eq_ignore_ascii_case("submit") || kind.eq_ignore_ascii_case("reset")
}

fn form_control_is_submit(kind: &str) -> bool {
    kind.eq_ignore_ascii_case("submit")
}

fn form_control_is_focusable(kind: &str, has_options: bool, has_name: bool) -> bool {
    (has_name
        && (form_control_is_text_editable(kind) || has_options || form_control_is_checkable(kind)))
        || form_control_is_activatable(kind)
}

fn browser_form_control_name_is_unique(form: &BrowserForm, name: &str) -> bool {
    form.controls
        .iter()
        .filter(|control| control.name == name)
        .take(2)
        .count()
        == 1
}

fn form_control_is_text_editable(kind: &str) -> bool {
    matches!(
        kind.to_ascii_lowercase().as_str(),
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
    query.append_pair("viewport_y", &source.viewport_y().to_string());
    query.append_pair("max_bytes", &source.max_bytes().to_string());
    if let Some(source) = browser_safe_source_param(source.source()) {
        query.append_pair("source", source);
    }
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
    query.append_pair("viewport_y", &source.viewport_y().to_string());
    query.append_pair("max_bytes", &source.max_bytes().to_string());
    format!("/browser?{}", query.finish())
}

fn browser_session_api_href<T: BrowserSessionHrefSource>(
    id: &str,
    format: &str,
    source: &T,
) -> String {
    let mut query = form_urlencoded::Serializer::new(String::new());
    query.append_pair("id", id);
    query.append_pair("format", format);
    query.append_pair("from", source.back_href());
    query.append_pair("width", &source.width().to_string());
    query.append_pair("height", &source.height().to_string());
    query.append_pair("viewport_x", &source.viewport_x().to_string());
    query.append_pair("viewport_y", &source.viewport_y().to_string());
    query.append_pair("max_bytes", &source.max_bytes().to_string());
    format!("/api/browser-session?{}", query.finish())
}

trait BrowserSessionHrefSource {
    fn back_href(&self) -> &str;
    fn source(&self) -> &str;
    fn width(&self) -> usize;
    fn height(&self) -> usize;
    fn viewport_x(&self) -> usize;
    fn viewport_y(&self) -> usize;
    fn max_bytes(&self) -> usize;
}

impl BrowserSessionHrefSource for BrowserWebSession {
    fn back_href(&self) -> &str {
        &self.back_href
    }

    fn source(&self) -> &str {
        self.pending_source
            .as_deref()
            .or(self.display_source.as_deref())
            .unwrap_or_else(|| {
                self.session
                    .current()
                    .map(|render| render.source.as_str())
                    .unwrap_or_default()
            })
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

    fn viewport_y(&self) -> usize {
        self.viewport_y
    }

    fn max_bytes(&self) -> usize {
        self.max_bytes
    }
}

impl BrowserSessionHrefSource for BrowserSessionPayload {
    fn back_href(&self) -> &str {
        &self.back_href
    }

    fn source(&self) -> &str {
        &self.source
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

    fn viewport_y(&self) -> usize {
        self.viewport_y
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

fn scroll_nav_control(enabled: bool, label: &str, href: &str, disabled_reason: &str) -> String {
    if enabled {
        return nav_control(true, label, href);
    }
    format!(
        r#"<span aria-disabled="true" title="{reason}" data-browser-scroll-disabled="{reason}">{label}</span>"#,
        reason = html_escape::encode_double_quoted_attribute(disabled_reason),
        label = html_escape::encode_text(label),
    )
}

fn browser_session_title(render: &crate::browser::BrowserRender) -> String {
    if render.title.trim().is_empty() {
        render.source.clone()
    } else {
        render.title.clone()
    }
}

fn browser_session_display_title(
    render: &crate::browser::BrowserRender,
    display_source: Option<&str>,
) -> String {
    if render.title.trim().is_empty() {
        display_source
            .map(browser_session_feedback_excerpt)
            .unwrap_or_else(|| render.source.clone())
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

fn current_session_interaction_snapshot(
    web_session: &BrowserWebSession,
) -> Option<BrowserInteractionSnapshot> {
    web_session
        .session
        .current()
        .map(|render| BrowserInteractionSnapshot {
            source: render.source.clone(),
            title: render.title.clone(),
            text: render.text.clone(),
            forms: render.forms.clone(),
            link_count: render.links.len(),
        })
}

fn browser_interaction_snapshot_navigated(
    before: &Option<BrowserInteractionSnapshot>,
    web_session: &BrowserWebSession,
) -> bool {
    let before_source = before.as_ref().map(|snapshot| snapshot.source.as_str());
    let after_source = web_session
        .session
        .current()
        .map(|render| render.source.as_str());
    before_source != after_source
}

fn set_browser_click_feedback(
    web_session: &mut BrowserWebSession,
    label: String,
    before: Option<BrowserInteractionSnapshot>,
) {
    let after = current_session_interaction_snapshot(web_session);
    let suffix = if browser_interaction_snapshot_navigated(&before, web_session) {
        let target = after
            .as_ref()
            .map(|snapshot| snapshot.source.as_str())
            .unwrap_or("current page");
        let scope = browser_session_navigation_scope(
            before.as_ref().map(|snapshot| snapshot.source.as_str()),
            target,
        );
        web_session.action_feedback = Some(format!(
            "{label}; opened {scope}: {}",
            browser_session_feedback_excerpt(target)
        ));
        return;
    } else if before != after {
        "; page updated; viewport preserved"
    } else {
        "; no visible change; viewport preserved"
    };
    web_session.action_feedback = Some(format!("{label}{suffix}"));
}

fn set_browser_click_error_feedback(
    web_session: &mut BrowserWebSession,
    click_label: String,
    miss_label: String,
    error: &str,
    failure_label: &str,
) {
    let error_excerpt = browser_session_feedback_excerpt(error);
    let label = if browser_click_error_is_target_miss(error) {
        format!(
            "{miss_label}; {hint}",
            hint = browser_click_miss_retry_hint()
        )
    } else {
        format!("{click_label}; {failure_label}")
    };
    web_session.action_feedback = Some(format!("{label}: {error_excerpt}; viewport preserved"));
}

fn set_browser_click_pending_navigation_feedback(
    web_session: &mut BrowserWebSession,
    click_label: String,
    target_url: &str,
    error: &str,
) {
    web_session.pending_source = Some(target_url.to_owned());
    web_session.display_source = None;
    web_session.resource_report = None;
    clear_browser_find_active_line(web_session);
    web_session.action_feedback = Some(format!(
        "{click_label}; opening {} is pending after navigation failed: {}; viewport preserved",
        browser_session_feedback_excerpt(target_url),
        browser_session_feedback_excerpt(error)
    ));
}

fn browser_click_error_is_target_miss(error: &str) -> bool {
    error.contains("did not hit a DOM target")
        || error.contains("cannot click: session has no current page")
        || error.contains("cannot click coordinates: session has no current page")
}

fn browser_click_miss_retry_hint() -> &'static str {
    "click a visible link/button or retry with an exact point"
}

fn set_browser_navigation_feedback(
    web_session: &mut BrowserWebSession,
    label: String,
    before: Option<String>,
) {
    let after = current_session_source(web_session);
    if after != before {
        let target = after.as_deref().unwrap_or("current page");
        let scope = browser_session_navigation_scope(before.as_deref(), target);
        web_session.action_feedback = Some(format!(
            "{label}; opened {scope}: {}",
            browser_session_feedback_excerpt(target)
        ));
    } else {
        web_session.action_feedback = Some(format!("{label}; no navigation; viewport preserved"));
    }
}

fn set_browser_link_pending_navigation_feedback(
    web_session: &mut BrowserWebSession,
    label: String,
    target_url: &str,
    error: &str,
) {
    web_session.pending_source = Some(target_url.to_owned());
    web_session.display_source = None;
    web_session.resource_report = None;
    clear_browser_find_active_line(web_session);
    web_session.action_feedback = Some(format!(
        "{label}; opening {} is pending after navigation failed: {}; viewport preserved",
        browser_session_feedback_excerpt(target_url),
        browser_session_feedback_excerpt(error)
    ));
}

fn browser_session_navigation_scope(before: Option<&str>, after: &str) -> &'static str {
    let Some(after_url) = Url::parse(after).ok() else {
        return "local page";
    };
    let Some(before_url) = before.and_then(|source| Url::parse(source).ok()) else {
        return if after_url.has_host() {
            "external page"
        } else {
            "local page"
        };
    };
    if before_url.scheme() == after_url.scheme() && before_url.host_str() == after_url.host_str() {
        "internal page"
    } else {
        "external page"
    }
}

fn set_browser_form_navigation_feedback(
    web_session: &mut BrowserWebSession,
    label: String,
    navigated: bool,
) {
    let suffix = if navigated {
        "; navigated"
    } else {
        "; no navigation; viewport preserved"
    };
    web_session.action_feedback = Some(format!("{label}{suffix}"));
}

fn browser_session_form_target(
    web_session: &BrowserWebSession,
    form_index: usize,
) -> Option<String> {
    web_session
        .session
        .current_forms()
        .into_iter()
        .find(|form| form.index == form_index)
        .map(|form| form.resolved_action.trim().to_owned())
        .filter(|target| !target.is_empty())
}

fn set_browser_form_pending_navigation_feedback(
    web_session: &mut BrowserWebSession,
    label: String,
    target_url: &str,
    error: &str,
) {
    web_session.pending_source = Some(target_url.to_owned());
    web_session.display_source = None;
    web_session.resource_report = None;
    clear_browser_find_active_line(web_session);
    web_session.action_feedback = Some(format!(
        "{label}; opening {} is pending after form navigation failed: {}; viewport preserved",
        browser_session_feedback_excerpt(target_url),
        browser_session_feedback_excerpt(error)
    ));
}

fn set_browser_focused_control_feedback(web_session: &mut BrowserWebSession, label: &str) {
    web_session.action_feedback = Some(match web_session.session.focused_control() {
        Some(focused) => format!(
            "{label}: form {} control {}.",
            focused.form_index, focused.control_index
        ),
        None => format!("{label}; no focused control."),
    });
}

fn browser_session_feedback_excerpt(value: &str) -> String {
    let trimmed = value.trim();
    const LIMIT: usize = 80;
    if trimmed.chars().count() <= LIMIT {
        return trimmed.to_owned();
    }
    let mut excerpt = trimmed.chars().take(LIMIT).collect::<String>();
    excerpt.push_str("...");
    excerpt
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

fn set_browser_scroll_noop_feedback(
    web_session: &mut BrowserWebSession,
    before_x: usize,
    before_y: usize,
    dx: isize,
    dy: isize,
) {
    if web_session.viewport_x != before_x || web_session.viewport_y != before_y {
        web_session.action_feedback = Some(format!(
            "Moved visual viewport to x {}, y {}.",
            web_session.viewport_x, web_session.viewport_y
        ));
        return;
    }
    let message = if dx < 0 {
        "Already at left edge."
    } else if dx > 0 {
        "Already at right edge."
    } else if dy < 0 {
        "Already at top."
    } else if dy > 0 {
        "Already at bottom."
    } else {
        "Viewport is already at that position."
    };
    web_session.action_feedback = Some(message.to_owned());
}

fn set_browser_visual_scroll_moved_feedback(web_session: &mut BrowserWebSession) {
    web_session.action_feedback = Some(format!(
        "Moved visual viewport to x {}, y {}.",
        web_session.viewport_x, web_session.viewport_y
    ));
}

fn set_browser_viewport_jump_feedback(
    web_session: &mut BrowserWebSession,
    before_x: usize,
    before_y: usize,
) {
    if web_session.viewport_x == before_x && web_session.viewport_y == before_y {
        web_session.action_feedback = Some("Viewport is already at that position.".to_owned());
    } else {
        web_session.action_feedback = Some(format!(
            "Viewport moved to x {}, y {}.",
            web_session.viewport_x, web_session.viewport_y
        ));
    }
}

fn normalize_browser_session_viewport(web_session: &mut BrowserWebSession) {
    let Some(render) = web_session.session.current() else {
        return;
    };
    let viewport = browser_text_viewport(
        render,
        BrowserTextViewportOptions {
            x: web_session.viewport_x,
            y: web_session.viewport_y,
            width: web_session.width,
            height: web_session.height,
        },
    );
    web_session.viewport_x = viewport.x;
    web_session.viewport_y = viewport.y;
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
