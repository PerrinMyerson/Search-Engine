use std::{
    io::{self, IsTerminal, Write},
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use brutal_search::browser::{
    BrowserApp, BrowserAppAction, BrowserAppOptions, BrowserAppReport, BrowserCookieJar,
    BrowserLocalStorage, BrowserRasterOptions, BrowserRenderOptions,
};
use clap::Args;
use serde::{Deserialize, Serialize};

use crate::brutal_browser_shell::{
    BrowserShellCommand, BrowserShellLinkTarget, parse_browser_shell_command,
};
use crate::{
    load_browser_cookie_jar, load_browser_local_storage, save_browser_cookie_jar,
    save_browser_local_storage,
};

#[derive(Debug, Args)]
pub(crate) struct BrowserAppCli {
    pub(crate) target: String,
    #[arg(long, default_value_t = 100)]
    pub(crate) width: usize,
    #[arg(long, default_value_t = 4 * 1024 * 1024)]
    pub(crate) max_bytes: usize,
    #[arg(long)]
    pub(crate) viewport_width: Option<usize>,
    #[arg(long, default_value_t = 24)]
    pub(crate) viewport_height: usize,
    #[arg(long, alias = "scroll-x", default_value_t = 0)]
    pub(crate) viewport_x: usize,
    #[arg(long, alias = "scroll-y", default_value_t = 0)]
    pub(crate) viewport_y: usize,
    #[arg(long, default_value_t = 8)]
    pub(crate) cell_width: usize,
    #[arg(long, default_value_t = 12)]
    pub(crate) cell_height: usize,
    #[arg(long = "cmd", alias = "command")]
    pub(crate) commands: Vec<String>,
    #[arg(long)]
    pub(crate) stdin: bool,
    #[arg(long)]
    pub(crate) no_interactive: bool,
    #[arg(long)]
    pub(crate) cookie_jar: Option<PathBuf>,
    #[arg(long, alias = "local-storage-file")]
    pub(crate) local_storage: Option<PathBuf>,
    #[arg(long)]
    pub(crate) profile: Option<PathBuf>,
    #[arg(long)]
    pub(crate) output: Option<PathBuf>,
    #[arg(long)]
    pub(crate) window_output: Option<PathBuf>,
    #[arg(long)]
    pub(crate) json: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
struct BrowserAppProfile {
    #[serde(default)]
    history: Vec<BrowserAppProfileEntry>,
    #[serde(default)]
    bookmarks: Vec<BrowserAppProfileEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct BrowserAppProfileEntry {
    title: String,
    source: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum BrowserAppCliCommand {
    Shell(BrowserShellCommand),
    Bookmarks,
    BookmarkAdd,
    BookmarkRemove(usize),
    BookmarkOpen(usize),
    ProfileHistory,
    HistoryOpen(usize),
    ClearProfileHistory,
    WindowClick { x: usize, y: usize },
}

pub(crate) async fn run_browser_app_cli(args: BrowserAppCli) -> Result<()> {
    let options = BrowserAppOptions {
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
    };
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
    let mut profile = args
        .profile
        .as_deref()
        .map(load_browser_app_profile)
        .transpose()?
        .unwrap_or_default();
    record_browser_app_profile_visit(&mut profile, &app)?;

    if args.viewport_x > 0 || args.viewport_y > 0 {
        app.apply_action(BrowserAppAction::SetViewportOrigin {
            x: args.viewport_x,
            y: args.viewport_y,
        })
        .await?;
    }

    if args.stdin {
        run_browser_app_stdin(&mut app, &mut profile, args.output.as_deref(), args.json).await?;
    } else if args.commands.is_empty()
        && !args.json
        && !args.no_interactive
        && io::stdin().is_terminal()
    {
        run_interactive_browser_app_shell(&mut app, &mut profile, args.output.as_deref()).await?;
    } else {
        for command in &args.commands {
            let command = parse_browser_app_cli_command(command)?;
            if !apply_browser_app_cli_command(&mut app, &mut profile, &command).await? {
                break;
            }
            record_browser_app_profile_visit_after_command(&mut profile, &app, &command)?;
        }
    }

    let frame = app.present_frame()?;
    if let Some(path) = args.output.as_deref() {
        write_browser_app_frame(path, &frame.raster.encode_png()?)?;
    }
    if let Some(path) = args.window_output.as_deref() {
        let window_frame = app.window_frame_for_presented_frame(frame.clone())?;
        write_browser_app_frame(path, &window_frame.raster.encode_png()?)?;
    }
    let report = app.report_for_frame(frame.report)?;
    let active_session = app.active_session()?;
    if let Some(path) = args.cookie_jar.as_deref() {
        save_browser_cookie_jar(path, &active_session.cookies_snapshot())?;
    }
    if let Some(path) = args.local_storage.as_deref() {
        save_browser_local_storage(path, &active_session.local_storage_snapshot())?;
    }
    if let Some(path) = args.profile.as_deref() {
        save_browser_app_profile(path, &profile)?;
    }
    if args.json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        print_browser_app_report(&report);
    }
    Ok(())
}

async fn run_interactive_browser_app_shell(
    app: &mut BrowserApp,
    profile: &mut BrowserAppProfile,
    output: Option<&Path>,
) -> Result<()> {
    print_browser_app_help();
    print_browser_app_after_cli_command(
        app,
        profile,
        output,
        false,
        &BrowserAppCliCommand::Shell(BrowserShellCommand::Render),
    )?;
    let stdin = io::stdin();
    loop {
        print!("brutal-browser-app> ");
        io::stdout().flush()?;
        let mut input = String::new();
        if stdin.read_line(&mut input)? == 0 {
            break;
        }
        let command = input.trim();
        if command.is_empty() {
            continue;
        }
        let parsed = parse_browser_app_cli_command(command)?;
        if !apply_browser_app_cli_command(app, profile, &parsed).await? {
            break;
        }
        record_browser_app_profile_visit_after_command(profile, app, &parsed)?;
        print_browser_app_after_cli_command(app, profile, output, false, &parsed)?;
    }
    Ok(())
}

async fn run_browser_app_stdin(
    app: &mut BrowserApp,
    profile: &mut BrowserAppProfile,
    output: Option<&Path>,
    json: bool,
) -> Result<()> {
    let stdin = io::stdin();
    let mut input = String::new();
    loop {
        input.clear();
        if stdin.read_line(&mut input)? == 0 {
            break;
        }
        let command = input.trim();
        if command.is_empty() || command.starts_with('#') {
            continue;
        }
        let parsed = parse_browser_app_cli_command(command)?;
        if !apply_browser_app_cli_command(app, profile, &parsed).await? {
            break;
        }
        record_browser_app_profile_visit_after_command(profile, app, &parsed)?;
        print_browser_app_after_cli_command(app, profile, output, json, &parsed)?;
    }
    Ok(())
}

fn parse_browser_app_cli_command(input: &str) -> Result<BrowserAppCliCommand> {
    let trimmed = input.trim();
    let (name, rest) = trimmed
        .split_once(char::is_whitespace)
        .map_or((trimmed, ""), |(name, rest)| (name, rest.trim()));
    match name.to_ascii_lowercase().as_str() {
        "bookmarks" => {
            ensure_app_no_argument(name, rest)?;
            Ok(BrowserAppCliCommand::Bookmarks)
        }
        "bookmark" | "bookmark-add" | "add-bookmark" => {
            ensure_app_no_argument(name, rest)?;
            Ok(BrowserAppCliCommand::BookmarkAdd)
        }
        "bookmark-remove" | "remove-bookmark" => Ok(BrowserAppCliCommand::BookmarkRemove(
            parse_app_profile_index(name, rest)?,
        )),
        "bookmark-open" | "open-bookmark" => Ok(BrowserAppCliCommand::BookmarkOpen(
            parse_app_profile_index(name, rest)?,
        )),
        "profile-history" | "global-history" => {
            ensure_app_no_argument(name, rest)?;
            Ok(BrowserAppCliCommand::ProfileHistory)
        }
        "history-open" | "open-history" => Ok(BrowserAppCliCommand::HistoryOpen(
            parse_app_profile_index(name, rest)?,
        )),
        "clear-profile-history" => {
            ensure_app_no_argument(name, rest)?;
            Ok(BrowserAppCliCommand::ClearProfileHistory)
        }
        "window-click" | "window-click-at" | "chrome-click" => {
            let (x, y) = parse_app_window_click(name, rest)?;
            Ok(BrowserAppCliCommand::WindowClick { x, y })
        }
        _ => parse_browser_shell_command(input).map(BrowserAppCliCommand::Shell),
    }
}

fn ensure_app_no_argument(command: &str, rest: &str) -> Result<()> {
    if rest.trim().is_empty() {
        Ok(())
    } else {
        bail!("{command} does not take arguments")
    }
}

fn parse_app_profile_index(command: &str, rest: &str) -> Result<usize> {
    let mut parts = rest.split_whitespace();
    let Some(index) = parts.next() else {
        bail!("{command} requires an index");
    };
    if parts.next().is_some() {
        bail!("{command} requires exactly one index");
    }
    index
        .parse::<usize>()
        .with_context(|| format!("{command} requires an unsigned integer index"))
}

fn parse_app_window_click(command: &str, rest: &str) -> Result<(usize, usize)> {
    let mut parts = rest.split_whitespace();
    let Some(x) = parts.next() else {
        bail!("{command} requires x and y pixel coordinates");
    };
    let Some(y) = parts.next() else {
        bail!("{command} requires x and y pixel coordinates");
    };
    if parts.next().is_some() {
        bail!("{command} requires exactly two pixel coordinates");
    }
    let x = x
        .parse::<usize>()
        .with_context(|| format!("{command} requires unsigned integer pixel coordinates"))?;
    let y = y
        .parse::<usize>()
        .with_context(|| format!("{command} requires unsigned integer pixel coordinates"))?;
    Ok((x, y))
}

async fn apply_browser_app_cli_command(
    app: &mut BrowserApp,
    profile: &mut BrowserAppProfile,
    command: &BrowserAppCliCommand,
) -> Result<bool> {
    match command {
        BrowserAppCliCommand::Shell(command) => {
            apply_browser_app_command(app, command.clone()).await
        }
        BrowserAppCliCommand::Bookmarks | BrowserAppCliCommand::ProfileHistory => Ok(true),
        BrowserAppCliCommand::BookmarkAdd => {
            add_current_bookmark(profile, app)?;
            Ok(true)
        }
        BrowserAppCliCommand::BookmarkRemove(index) => {
            remove_profile_entry(&mut profile.bookmarks, *index, "bookmark")?;
            Ok(true)
        }
        BrowserAppCliCommand::BookmarkOpen(index) => {
            let entry = profile_entry(&profile.bookmarks, *index, "bookmark")?.clone();
            app.apply_action(BrowserAppAction::Open(entry.source))
                .await?;
            Ok(true)
        }
        BrowserAppCliCommand::HistoryOpen(index) => {
            let entry = profile_entry(&profile.history, *index, "profile history")?.clone();
            app.apply_action(BrowserAppAction::Open(entry.source))
                .await?;
            Ok(true)
        }
        BrowserAppCliCommand::ClearProfileHistory => {
            profile.history.clear();
            Ok(true)
        }
        BrowserAppCliCommand::WindowClick { x, y } => {
            app.click_window(*x, *y).await?;
            Ok(true)
        }
    }
}

async fn apply_browser_app_command(
    app: &mut BrowserApp,
    command: BrowserShellCommand,
) -> Result<bool> {
    let action = match command {
        BrowserShellCommand::Open(target) => Some(BrowserAppAction::Open(target)),
        BrowserShellCommand::Back => Some(BrowserAppAction::Back),
        BrowserShellCommand::Forward => Some(BrowserAppAction::Forward),
        BrowserShellCommand::Reload => Some(BrowserAppAction::Reload),
        BrowserShellCommand::ClearCookies => Some(BrowserAppAction::ClearCookies),
        BrowserShellCommand::ClearLocalStorage => Some(BrowserAppAction::ClearLocalStorage),
        BrowserShellCommand::ClearSessionStorage => Some(BrowserAppAction::ClearSessionStorage),
        BrowserShellCommand::Find { query, next } => {
            Some(BrowserAppAction::FindText { query, next })
        }
        BrowserShellCommand::Click(selector) => Some(BrowserAppAction::ClickSelector(selector)),
        BrowserShellCommand::ClickAt { x, y } => Some(BrowserAppAction::Click { x, y }),
        BrowserShellCommand::Link(target) => Some(browser_app_link_action(target)),
        BrowserShellCommand::Focus(selector) => Some(BrowserAppAction::Focus(selector)),
        BrowserShellCommand::FocusNext => Some(BrowserAppAction::FocusNext),
        BrowserShellCommand::FocusPrevious => Some(BrowserAppAction::FocusPrevious),
        BrowserShellCommand::TypeText(text) => Some(BrowserAppAction::TypeText(text)),
        BrowserShellCommand::DeleteTextBackward(count) => {
            Some(BrowserAppAction::DeleteTextBackward(count))
        }
        BrowserShellCommand::ClearText => Some(BrowserAppAction::ClearText),
        BrowserShellCommand::SubmitFocused => Some(BrowserAppAction::SubmitFocused),
        BrowserShellCommand::ToggleFocused => Some(BrowserAppAction::ToggleFocused),
        BrowserShellCommand::ToggleControl {
            form_index,
            control_index,
        } => Some(BrowserAppAction::ToggleControl {
            form_index,
            control_index,
        }),
        BrowserShellCommand::SelectFocused(value) => Some(BrowserAppAction::SelectFocused(value)),
        BrowserShellCommand::SelectControl {
            form_index,
            control_index,
            value,
        } => Some(BrowserAppAction::SelectControl {
            form_index,
            control_index,
            value,
        }),
        BrowserShellCommand::NewTab(target) => Some(BrowserAppAction::NewTab(target)),
        BrowserShellCommand::SwitchTab(index) => Some(BrowserAppAction::SwitchTab(index)),
        BrowserShellCommand::CloseTab(index) => Some(BrowserAppAction::CloseTab(index)),
        BrowserShellCommand::Scroll(delta_y) => Some(BrowserAppAction::Scroll {
            delta_x: 0,
            delta_y,
        }),
        BrowserShellCommand::HorizontalScroll(delta_x) => Some(BrowserAppAction::Scroll {
            delta_x,
            delta_y: 0,
        }),
        BrowserShellCommand::Top => {
            let x = app.active_viewport()?.x;
            Some(BrowserAppAction::SetViewportOrigin { x, y: 0 })
        }
        BrowserShellCommand::Bottom => {
            let x = app.active_viewport()?.x;
            let y = app.report()?.frame.viewport.max_scroll_y;
            Some(BrowserAppAction::SetViewportOrigin { x, y })
        }
        BrowserShellCommand::Render
        | BrowserShellCommand::Location
        | BrowserShellCommand::Cookies
        | BrowserShellCommand::LocalStorage
        | BrowserShellCommand::SessionStorage
        | BrowserShellCommand::Links
        | BrowserShellCommand::Forms
        | BrowserShellCommand::Tabs
        | BrowserShellCommand::History
        | BrowserShellCommand::Help => None,
        BrowserShellCommand::Quit => return Ok(false),
        other => bail!("browse app command {other:?} is not supported by BrowserApp yet"),
    };

    if let Some(action) = action {
        app.apply_action(action).await?;
    }
    Ok(true)
}

fn browser_app_link_action(target: BrowserShellLinkTarget) -> BrowserAppAction {
    match target {
        BrowserShellLinkTarget::Index(index) => BrowserAppAction::ActivateLink(index),
        BrowserShellLinkTarget::Text(text) => BrowserAppAction::ActivateLinkText(text),
        BrowserShellLinkTarget::Selector(selector) => {
            BrowserAppAction::ActivateLinkSelector(selector)
        }
    }
}

fn write_browser_app_frame(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create frame directory {}", parent.display()))?;
    }
    std::fs::write(path, bytes)
        .with_context(|| format!("failed to write app frame {}", path.display()))
}

fn load_browser_app_profile(path: &Path) -> Result<BrowserAppProfile> {
    match std::fs::read(path) {
        Ok(bytes) => {
            if bytes.is_empty() {
                return Ok(BrowserAppProfile::default());
            }
            serde_json::from_slice::<BrowserAppProfile>(&bytes)
                .with_context(|| format!("failed to parse app profile {}", path.display()))
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(BrowserAppProfile::default()),
        Err(error) => {
            Err(error).with_context(|| format!("failed to read app profile {}", path.display()))
        }
    }
}

fn save_browser_app_profile(path: &Path, profile: &BrowserAppProfile) -> Result<()> {
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent).with_context(|| {
            format!(
                "failed to create app profile directory {}",
                parent.display()
            )
        })?;
    }
    let bytes = serde_json::to_vec_pretty(profile)
        .with_context(|| format!("failed to encode app profile {}", path.display()))?;
    std::fs::write(path, bytes)
        .with_context(|| format!("failed to write app profile {}", path.display()))
}

fn current_profile_entry(app: &BrowserApp) -> Result<Option<BrowserAppProfileEntry>> {
    let Some(render) = app.active_session()?.current() else {
        return Ok(None);
    };
    Ok(Some(BrowserAppProfileEntry {
        title: if render.title.is_empty() {
            render.source.clone()
        } else {
            render.title.clone()
        },
        source: render.source.clone(),
    }))
}

fn record_browser_app_profile_visit(
    profile: &mut BrowserAppProfile,
    app: &BrowserApp,
) -> Result<()> {
    let Some(entry) = current_profile_entry(app)? else {
        return Ok(());
    };
    if profile
        .history
        .last()
        .is_some_and(|existing| existing.source == entry.source)
    {
        return Ok(());
    }
    profile.history.push(entry);
    Ok(())
}

fn record_browser_app_profile_visit_after_command(
    profile: &mut BrowserAppProfile,
    app: &BrowserApp,
    command: &BrowserAppCliCommand,
) -> Result<()> {
    if browser_app_command_records_profile_visit(command) {
        record_browser_app_profile_visit(profile, app)?;
    }
    Ok(())
}

fn browser_app_command_records_profile_visit(command: &BrowserAppCliCommand) -> bool {
    match command {
        BrowserAppCliCommand::BookmarkOpen(_) | BrowserAppCliCommand::HistoryOpen(_) => true,
        BrowserAppCliCommand::WindowClick { .. } => true,
        BrowserAppCliCommand::Shell(command) => matches!(
            command,
            BrowserShellCommand::Open(_)
                | BrowserShellCommand::Back
                | BrowserShellCommand::Forward
                | BrowserShellCommand::NewTab(_)
                | BrowserShellCommand::Click(_)
                | BrowserShellCommand::ClickAt { .. }
                | BrowserShellCommand::Link(_)
                | BrowserShellCommand::SubmitFocused
        ),
        _ => false,
    }
}

fn add_current_bookmark(profile: &mut BrowserAppProfile, app: &BrowserApp) -> Result<()> {
    let Some(entry) = current_profile_entry(app)? else {
        bail!("cannot bookmark: browser app has no current page");
    };
    if let Some(existing) = profile
        .bookmarks
        .iter_mut()
        .find(|bookmark| bookmark.source == entry.source)
    {
        *existing = entry;
        return Ok(());
    }
    profile.bookmarks.push(entry);
    Ok(())
}

fn remove_profile_entry(
    entries: &mut Vec<BrowserAppProfileEntry>,
    index: usize,
    label: &str,
) -> Result<BrowserAppProfileEntry> {
    if index >= entries.len() {
        bail!(
            "{label} index {index} not found; {} {label}(s) saved",
            entries.len()
        );
    }
    Ok(entries.remove(index))
}

fn profile_entry<'a>(
    entries: &'a [BrowserAppProfileEntry],
    index: usize,
    label: &str,
) -> Result<&'a BrowserAppProfileEntry> {
    entries
        .get(index)
        .ok_or_else(|| anyhow::anyhow!("{label} index {index} not found; {} saved", entries.len()))
}

fn print_browser_app_after_cli_command(
    app: &mut BrowserApp,
    profile: &BrowserAppProfile,
    output: Option<&Path>,
    json: bool,
    command: &BrowserAppCliCommand,
) -> Result<()> {
    match command {
        BrowserAppCliCommand::Shell(command) => {
            print_browser_app_after_command(app, output, json, command)
        }
        BrowserAppCliCommand::Bookmarks => {
            print_browser_app_profile_entries("bookmarks", "bookmark", &profile.bookmarks, json)
        }
        BrowserAppCliCommand::ProfileHistory => {
            print_browser_app_profile_entries("profile_history", "history", &profile.history, json)
        }
        BrowserAppCliCommand::BookmarkAdd => {
            let Some(entry) = current_profile_entry(app)? else {
                println!("bookmark: none");
                return Ok(());
            };
            println!("bookmark: saved {} -> {}", entry.title, entry.source);
            Ok(())
        }
        BrowserAppCliCommand::BookmarkRemove(index) => {
            println!("bookmark: removed index {index}");
            Ok(())
        }
        BrowserAppCliCommand::ClearProfileHistory => {
            println!("profile_history: cleared");
            Ok(())
        }
        BrowserAppCliCommand::BookmarkOpen(_) | BrowserAppCliCommand::HistoryOpen(_) => {
            let frame = app.present_frame()?;
            if let Some(path) = output {
                write_browser_app_frame(path, &frame.raster.encode_png()?)?;
            }
            let report = app.report_for_frame(frame.report)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&report)?);
            } else {
                print_browser_app_report(&report);
            }
            Ok(())
        }
        BrowserAppCliCommand::WindowClick { .. } => {
            let frame = app.present_frame()?;
            if let Some(path) = output {
                write_browser_app_frame(path, &frame.raster.encode_png()?)?;
            }
            let report = app.report_for_frame(frame.report)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&report)?);
            } else {
                print_browser_app_report(&report);
            }
            Ok(())
        }
    }
}

#[derive(Serialize)]
struct BrowserAppProfileEntriesReport<'a> {
    kind: &'a str,
    entries: &'a [BrowserAppProfileEntry],
}

fn print_browser_app_profile_entries(
    kind: &str,
    label: &str,
    entries: &[BrowserAppProfileEntry],
    json: bool,
) -> Result<()> {
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&BrowserAppProfileEntriesReport { kind, entries })?
        );
        return Ok(());
    }
    if entries.is_empty() {
        println!("{kind}: none");
        return Ok(());
    }
    println!("{kind}:");
    for (index, entry) in entries.iter().enumerate() {
        println!("[{index}] {} -> {}", entry.title, entry.source);
    }
    println!("open with: {label}-open <index>");
    Ok(())
}

fn print_browser_app_after_command(
    app: &mut BrowserApp,
    output: Option<&Path>,
    json: bool,
    command: &BrowserShellCommand,
) -> Result<()> {
    let frame = app.present_frame()?;
    if let Some(path) = output {
        write_browser_app_frame(path, &frame.raster.encode_png()?)?;
    }
    let report = app.report_for_frame(frame.report)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&report)?);
        return Ok(());
    }
    match command {
        BrowserShellCommand::Help => {
            print_browser_app_help();
            Ok(())
        }
        BrowserShellCommand::Location => {
            let active = &report.tabs[report.active_tab];
            println!(
                "location: source={} title={} viewport={}x{}+{}+{}",
                active.source,
                active.title,
                report.viewport.width,
                report.viewport.height,
                report.viewport.x,
                report.viewport.y
            );
            Ok(())
        }
        BrowserShellCommand::Cookies => {
            if report.cookies.is_empty() {
                println!("cookies: none");
            } else {
                println!("cookies:");
                for cookie in &report.cookies {
                    println!(
                        "{}={} domain={} path={} secure={} http_only={}",
                        cookie.name,
                        cookie.value,
                        cookie.domain,
                        cookie.path,
                        cookie.secure,
                        cookie.http_only
                    );
                }
            }
            Ok(())
        }
        BrowserShellCommand::LocalStorage => {
            print_browser_app_storage("local_storage", &report.local_storage);
            Ok(())
        }
        BrowserShellCommand::SessionStorage => {
            print_browser_app_storage("session_storage", &report.session_storage);
            Ok(())
        }
        BrowserShellCommand::ClearCookies => {
            println!("cookies: cleared");
            Ok(())
        }
        BrowserShellCommand::ClearLocalStorage => {
            println!("local_storage: cleared");
            Ok(())
        }
        BrowserShellCommand::ClearSessionStorage => {
            println!("session_storage: cleared");
            Ok(())
        }
        BrowserShellCommand::History => {
            print_browser_app_history(&report);
            Ok(())
        }
        BrowserShellCommand::Links => {
            print_browser_app_links(&report);
            Ok(())
        }
        BrowserShellCommand::Forms => {
            print_browser_app_forms(&report);
            Ok(())
        }
        BrowserShellCommand::Tabs => {
            print_browser_app_tabs(&report);
            Ok(())
        }
        _ => {
            print_browser_app_report(&report);
            Ok(())
        }
    }
}

fn print_browser_app_help() {
    println!(
        "commands: open <target>, back, forward, reload, link <n|text|selector>, click <selector>, click-at <x> <y>, window-click <x> <y>, find <text>, find-next <text>, up/down/left/right <n>, top, bottom, new-tab <target>, switch-tab <n>, close-tab [n], tabs, links, forms, history, profile-history, history-open <n>, bookmark, bookmarks, bookmark-open <n>, bookmark-remove <n>, clear-profile-history, location, cookies, local-storage, session-storage, clear-cookies, clear-local-storage, clear-session-storage, render, help, quit"
    );
}

fn print_browser_app_report(report: &BrowserAppReport) {
    let active = &report.tabs[report.active_tab];
    println!("# {}", active.title);
    println!(
        "source={} tabs={} active={} viewport={}x{}+{}+{} max_scroll={}+{} frame={}x{} dirty_pixels={} dirty_pixel_area={} hash={}",
        active.source,
        report.tabs.len(),
        report.active_tab,
        report.viewport.width,
        report.viewport.height,
        report.viewport.x,
        report.viewport.y,
        report.frame.viewport.max_scroll_x,
        report.frame.viewport.max_scroll_y,
        report.frame.frame_width,
        report.frame.frame_height,
        report.frame.dirty_pixel_regions.len(),
        report.frame.dirty_pixel_area,
        report.frame.pixel_hash,
    );
    if let Some(focused) = &report.focused {
        println!(
            "focused: form={} control={} {} name={} value={:?}",
            focused.form_index, focused.control_index, focused.kind, focused.name, focused.value
        );
    }
    if let Some(find) = &report.find {
        println!(
            "find: {}/{} line={} query={}",
            find.active_match_index + 1,
            find.match_count,
            find.line + 1,
            find.query
        );
    }
    println!(
        "links={} forms={} history={}/{}",
        report.links.len(),
        report.forms.len(),
        report
            .history
            .current_index
            .map(|index| index + 1)
            .unwrap_or(0),
        report.history.entries.len()
    );
    if !report.visible_text.is_empty() {
        println!();
        for line in &report.visible_text {
            println!("{line}");
        }
    }
}

fn print_browser_app_tabs(report: &BrowserAppReport) {
    println!("tabs:");
    for tab in &report.tabs {
        let marker = if tab.active { "*" } else { " " };
        println!(
            "{marker}[{}] {} -> {} history={}",
            tab.index, tab.title, tab.source, tab.history_len
        );
    }
}

fn print_browser_app_links(report: &BrowserAppReport) {
    if report.links.is_empty() {
        println!("links: none");
        return;
    }
    println!("links:");
    for (index, link) in report.links.iter().enumerate() {
        let text = if link.text.is_empty() {
            "(empty)"
        } else {
            link.text.as_str()
        };
        println!("[{index}] {text} -> {}", link.resolved);
    }
}

fn print_browser_app_forms(report: &BrowserAppReport) {
    if report.forms.is_empty() {
        println!("forms: none");
        return;
    }
    println!("forms:");
    for form in &report.forms {
        println!(
            "[{}] {} action={} resolved={}",
            form.index, form.method, form.action, form.resolved_action
        );
        for (index, control) in form.controls.iter().enumerate() {
            let name = if control.name.is_empty() {
                "(unnamed)"
            } else {
                control.name.as_str()
            };
            println!(
                "  [{}] {} {} value={:?} checked={} disabled={}",
                index, control.kind, name, control.value, control.checked, control.disabled
            );
        }
    }
}

fn print_browser_app_history(report: &BrowserAppReport) {
    let current_position = report
        .history
        .current_index
        .map(|index| index + 1)
        .unwrap_or(0);
    println!(
        "history: {current_position}/{}",
        report.history.entries.len()
    );
    for (index, entry) in report.history.entries.iter().enumerate() {
        let marker = if Some(index) == report.history.current_index {
            "*"
        } else {
            " "
        };
        let label = if entry.title.is_empty() {
            &entry.source
        } else {
            &entry.title
        };
        println!("{marker} {} -> {}", index + 1, label);
    }
}

fn print_browser_app_storage(
    label: &str,
    entries: &[brutal_search::browser::BrowserLocalStorageEntry],
) {
    if entries.is_empty() {
        println!("{label}: none");
        return;
    }
    println!("{label}:");
    for entry in entries {
        println!("{} {}={:?}", entry.origin, entry.key, entry.value);
    }
}
