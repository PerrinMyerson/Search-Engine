use std::io::{self, IsTerminal, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow};
use brutal_search::browser::{
    BrowserChromiumParityReport, BrowserCookie, BrowserCookieJar, BrowserCoverageGate,
    BrowserCoverageReport, BrowserFeatureState, BrowserHistorySnapshot, BrowserImageRenderReport,
    BrowserLocalStorage, BrowserLocalStorageEntry, BrowserRasterOptions, BrowserRender,
    BrowserRenderOptions, BrowserScriptRenderReport, BrowserSession, BrowserStylesheetRenderReport,
    BrowserTextViewportOptions, BrowserViewportState, BrowserVisualReport, browser_coverage_report,
    browser_document_viewport, browser_text_viewport, browser_viewport_frame, build_get_form_url,
    compare_browser_fixtures_with_chromium, ensure_static_target, hit_test_render,
    layer_tree_render, layout_tree_render, load_accessibility_tree, load_and_render, raster_report,
    rasterize_render, rasterize_render_rgba, render_html, rgba_raster_report,
    unsupported_feature_summary, verify_browser_fixtures, verify_browser_visuals,
};
use brutal_search::browser_compat::{
    BrowserCompatGate, BrowserCompatOptions, BrowserCompatReport, run_browser_compat,
};
use clap::{Parser, Subcommand};
use serde::Serialize;

mod brutal_browser_app;
mod brutal_browser_inspect;
mod brutal_browser_shell;
mod brutal_browser_viewport;
mod brutal_browser_window;
use brutal_browser_app::{BrowserAppCli, run_browser_app_cli};
use brutal_browser_inspect::{
    print_accessibility_tree, print_hit_test, print_layer_tree, print_layout_tree,
};
use brutal_browser_shell::{
    BrowserFormSubmitMode, BrowserShellCommand, BrowserShellLinkTarget, BrowserShellState,
    BrowserShellTabs, browser_shell_forms, browser_shell_links, browser_shell_report,
    clamp_browser_shell_viewport, current_browser_shell_viewport, parse_browser_shell_command,
    reset_browser_shell_viewport_to_current_location, write_browser_shell_screenshot,
};
use brutal_browser_viewport::{
    parse_previous_viewport_state, print_document_viewport, print_viewport_frame,
};
use brutal_browser_window::{BrowserWindowCli, run_browser_window_cli};

#[derive(Debug, Parser)]
#[command(
    version,
    about = "Minimal static browser engine skeleton for the Blackium Starium✴ runtime track."
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    Render {
        target: String,
        #[arg(long, default_value_t = 100)]
        width: usize,
        #[arg(long, default_value_t = 4 * 1024 * 1024)]
        max_bytes: usize,
        #[arg(long)]
        json: bool,
        #[arg(long)]
        display_list: bool,
    },
    RenderFile {
        path: PathBuf,
        #[arg(long, default_value_t = 100)]
        width: usize,
        #[arg(long)]
        json: bool,
        #[arg(long)]
        display_list: bool,
    },
    HitTest {
        target: String,
        #[arg(long)]
        x: usize,
        #[arg(long)]
        y: usize,
        #[arg(long, default_value_t = 100)]
        width: usize,
        #[arg(long, default_value_t = 4 * 1024 * 1024)]
        max_bytes: usize,
        #[arg(long)]
        json: bool,
    },
    LayerTree {
        target: String,
        #[arg(long, default_value_t = 100)]
        width: usize,
        #[arg(long, default_value_t = 4 * 1024 * 1024)]
        max_bytes: usize,
        #[arg(long)]
        json: bool,
    },
    #[command(name = "layout-tree")]
    LayoutTree {
        target: String,
        #[arg(long, default_value_t = 100)]
        width: usize,
        #[arg(long, default_value_t = 4 * 1024 * 1024)]
        max_bytes: usize,
        #[arg(long)]
        json: bool,
    },
    Viewport {
        target: String,
        #[arg(long, default_value_t = 100)]
        width: usize,
        #[arg(long, default_value_t = 4 * 1024 * 1024)]
        max_bytes: usize,
        #[arg(long)]
        viewport_width: Option<usize>,
        #[arg(long, default_value_t = 24)]
        viewport_height: usize,
        #[arg(long, alias = "scroll-x", default_value_t = 0)]
        viewport_x: usize,
        #[arg(long, alias = "scroll-y", default_value_t = 0)]
        viewport_y: usize,
        #[arg(long)]
        previous_x: Option<usize>,
        #[arg(long)]
        previous_y: Option<usize>,
        #[arg(long)]
        previous_width: Option<usize>,
        #[arg(long)]
        previous_height: Option<usize>,
        #[arg(long)]
        json: bool,
    },
    #[command(name = "viewport-frame")]
    ViewportFrame {
        target: String,
        #[arg(long, default_value_t = 100)]
        width: usize,
        #[arg(long, default_value_t = 4 * 1024 * 1024)]
        max_bytes: usize,
        #[arg(long, default_value_t = 8)]
        cell_width: usize,
        #[arg(long, default_value_t = 12)]
        cell_height: usize,
        #[arg(long)]
        viewport_width: Option<usize>,
        #[arg(long, default_value_t = 24)]
        viewport_height: usize,
        #[arg(long, alias = "scroll-x", default_value_t = 0)]
        viewport_x: usize,
        #[arg(long, alias = "scroll-y", default_value_t = 0)]
        viewport_y: usize,
        #[arg(long)]
        previous_x: Option<usize>,
        #[arg(long)]
        previous_y: Option<usize>,
        #[arg(long)]
        previous_width: Option<usize>,
        #[arg(long)]
        previous_height: Option<usize>,
        #[arg(long)]
        output: Option<PathBuf>,
        #[arg(long)]
        json: bool,
    },
    #[command(name = "accessibility-tree", visible_alias = "ax-tree")]
    AccessibilityTree {
        target: String,
        #[arg(long, default_value_t = 100)]
        width: usize,
        #[arg(long, default_value_t = 4 * 1024 * 1024)]
        max_bytes: usize,
        #[arg(long)]
        json: bool,
    },
    Raster {
        target: String,
        #[arg(long, default_value_t = 100)]
        width: usize,
        #[arg(long, default_value_t = 4 * 1024 * 1024)]
        max_bytes: usize,
        #[arg(long, default_value_t = 8)]
        cell_width: usize,
        #[arg(long, default_value_t = 12)]
        cell_height: usize,
        #[arg(long, alias = "scroll-x")]
        viewport_x: Option<usize>,
        #[arg(long, alias = "scroll-y")]
        viewport_y: Option<usize>,
        #[arg(long)]
        viewport_width: Option<usize>,
        #[arg(long)]
        viewport_height: Option<usize>,
        #[arg(long)]
        output: Option<PathBuf>,
        #[arg(long)]
        json: bool,
    },
    RasterFile {
        path: PathBuf,
        #[arg(long, default_value_t = 100)]
        width: usize,
        #[arg(long, default_value_t = 8)]
        cell_width: usize,
        #[arg(long, default_value_t = 12)]
        cell_height: usize,
        #[arg(long, alias = "scroll-x")]
        viewport_x: Option<usize>,
        #[arg(long, alias = "scroll-y")]
        viewport_y: Option<usize>,
        #[arg(long)]
        viewport_width: Option<usize>,
        #[arg(long)]
        viewport_height: Option<usize>,
        #[arg(long)]
        output: Option<PathBuf>,
        #[arg(long)]
        json: bool,
    },
    Screenshot {
        target: String,
        #[arg(long, default_value_t = 100)]
        width: usize,
        #[arg(long, default_value_t = 4 * 1024 * 1024)]
        max_bytes: usize,
        #[arg(long, default_value_t = 8)]
        cell_width: usize,
        #[arg(long, default_value_t = 12)]
        cell_height: usize,
        #[arg(long, alias = "scroll-x")]
        viewport_x: Option<usize>,
        #[arg(long, alias = "scroll-y")]
        viewport_y: Option<usize>,
        #[arg(long)]
        viewport_width: Option<usize>,
        #[arg(long)]
        viewport_height: Option<usize>,
        #[arg(long)]
        output: Option<PathBuf>,
        #[arg(long)]
        json: bool,
    },
    ScreenshotFile {
        path: PathBuf,
        #[arg(long, default_value_t = 100)]
        width: usize,
        #[arg(long, default_value_t = 8)]
        cell_width: usize,
        #[arg(long, default_value_t = 12)]
        cell_height: usize,
        #[arg(long, alias = "scroll-x")]
        viewport_x: Option<usize>,
        #[arg(long, alias = "scroll-y")]
        viewport_y: Option<usize>,
        #[arg(long)]
        viewport_width: Option<usize>,
        #[arg(long)]
        viewport_height: Option<usize>,
        #[arg(long)]
        output: Option<PathBuf>,
        #[arg(long)]
        json: bool,
    },
    RenderStyled {
        target: String,
        #[arg(long, default_value_t = 100)]
        width: usize,
        #[arg(long, default_value_t = 4 * 1024 * 1024)]
        max_bytes: usize,
        #[arg(long, default_value_t = 1024 * 1024)]
        resource_max_bytes: usize,
        #[arg(long)]
        json: bool,
        #[arg(long)]
        display_list: bool,
    },
    RenderScripted {
        target: String,
        #[arg(long, default_value_t = 100)]
        width: usize,
        #[arg(long, default_value_t = 4 * 1024 * 1024)]
        max_bytes: usize,
        #[arg(long, default_value_t = 1024 * 1024)]
        resource_max_bytes: usize,
        #[arg(long)]
        json: bool,
        #[arg(long)]
        display_list: bool,
    },
    RenderImages {
        target: String,
        #[arg(long, default_value_t = 100)]
        width: usize,
        #[arg(long, default_value_t = 4 * 1024 * 1024)]
        max_bytes: usize,
        #[arg(long, default_value_t = 1024 * 1024)]
        resource_max_bytes: usize,
        #[arg(long)]
        json: bool,
        #[arg(long)]
        display_list: bool,
    },
    Click {
        target: String,
        selector: String,
        #[arg(long, default_value_t = 100)]
        width: usize,
        #[arg(long, default_value_t = 4 * 1024 * 1024)]
        max_bytes: usize,
        #[arg(long)]
        json: bool,
        #[arg(long)]
        display_list: bool,
    },
    #[command(name = "click-at", alias = "tap")]
    ClickAt {
        target: String,
        x: usize,
        y: usize,
        #[arg(long, alias = "scroll-x", default_value_t = 0)]
        viewport_x: usize,
        #[arg(long, alias = "scroll-y", default_value_t = 0)]
        viewport_y: usize,
        #[arg(long, default_value_t = 100)]
        width: usize,
        #[arg(long, default_value_t = 4 * 1024 * 1024)]
        max_bytes: usize,
        #[arg(long)]
        json: bool,
        #[arg(long)]
        display_list: bool,
    },
    Session {
        targets: Vec<String>,
        #[arg(long, default_value_t = 100)]
        width: usize,
        #[arg(long, default_value_t = 4 * 1024 * 1024)]
        max_bytes: usize,
        #[arg(long, default_value_t = 0)]
        back: usize,
        #[arg(long, default_value_t = 0)]
        forward: usize,
        #[arg(long)]
        json: bool,
        #[arg(long)]
        display_list: bool,
    },
    Browse {
        target: String,
        #[arg(long, default_value_t = 100)]
        width: usize,
        #[arg(long, default_value_t = 4 * 1024 * 1024)]
        max_bytes: usize,
        #[arg(long, default_value_t = 1024 * 1024)]
        resource_max_bytes: usize,
        #[arg(long)]
        viewport_width: Option<usize>,
        #[arg(long, default_value_t = 24)]
        viewport_height: usize,
        #[arg(long, alias = "scroll-x", default_value_t = 0)]
        viewport_x: usize,
        #[arg(long, alias = "scroll-y", default_value_t = 0)]
        viewport_y: usize,
        #[arg(long = "cmd", alias = "command")]
        commands: Vec<String>,
        #[arg(long)]
        cookie_jar: Option<PathBuf>,
        #[arg(long, alias = "local-storage-file")]
        local_storage: Option<PathBuf>,
        #[arg(long, alias = "screenshot")]
        screenshot_output: Option<PathBuf>,
        #[arg(long)]
        no_interactive: bool,
        #[arg(long)]
        json: bool,
    },
    App(BrowserAppCli),
    Window(BrowserWindowCli),
    #[command(name = "form-url", visible_alias = "get-form-url")]
    FormUrl {
        target: String,
        #[arg(long, default_value_t = 0)]
        form: usize,
        #[arg(long = "field")]
        fields: Vec<String>,
        #[arg(long, default_value_t = 100)]
        width: usize,
        #[arg(long, default_value_t = 4 * 1024 * 1024)]
        max_bytes: usize,
    },
    #[command(name = "submit", visible_alias = "submit-form")]
    Submit {
        target: String,
        #[arg(long, default_value_t = 0)]
        form: usize,
        #[arg(long = "field")]
        fields: Vec<String>,
        #[arg(long, default_value_t = 100)]
        width: usize,
        #[arg(long, default_value_t = 4 * 1024 * 1024)]
        max_bytes: usize,
        #[arg(long)]
        json: bool,
        #[arg(long)]
        display_list: bool,
    },
    #[command(name = "submit-get", visible_alias = "get-submit")]
    SubmitGet {
        target: String,
        #[arg(long, default_value_t = 0)]
        form: usize,
        #[arg(long = "field")]
        fields: Vec<String>,
        #[arg(long, default_value_t = 100)]
        width: usize,
        #[arg(long, default_value_t = 4 * 1024 * 1024)]
        max_bytes: usize,
        #[arg(long)]
        json: bool,
        #[arg(long)]
        display_list: bool,
    },
    #[command(name = "submit-post", visible_alias = "post-submit")]
    SubmitPost {
        target: String,
        #[arg(long, default_value_t = 0)]
        form: usize,
        #[arg(long = "field")]
        fields: Vec<String>,
        #[arg(long, default_value_t = 100)]
        width: usize,
        #[arg(long, default_value_t = 4 * 1024 * 1024)]
        max_bytes: usize,
        #[arg(long)]
        json: bool,
        #[arg(long)]
        display_list: bool,
    },
    Resources {
        target: String,
        #[arg(long, default_value_t = 100)]
        width: usize,
        #[arg(long, default_value_t = 4 * 1024 * 1024)]
        max_bytes: usize,
        #[arg(long)]
        json: bool,
    },
    FetchResources {
        target: String,
        #[arg(long, default_value_t = 100)]
        width: usize,
        #[arg(long, default_value_t = 4 * 1024 * 1024)]
        max_bytes: usize,
        #[arg(long, default_value_t = 1024 * 1024)]
        resource_max_bytes: usize,
        #[arg(long)]
        json: bool,
    },
    Verify {
        manifest: PathBuf,
        #[arg(long)]
        json: bool,
    },
    VisualVerify {
        manifest: PathBuf,
        #[arg(long)]
        artifact_dir: Option<PathBuf>,
        #[arg(long)]
        baseline_dir: Option<PathBuf>,
        #[arg(long)]
        require_all_baselines: bool,
        #[arg(long)]
        max_diff_pixels: Option<usize>,
        #[arg(long)]
        max_diff_ratio: Option<f64>,
        #[arg(long)]
        json: bool,
    },
    CompareChromium {
        manifest: PathBuf,
        #[arg(long)]
        json: bool,
        #[arg(long, default_value_t = 0)]
        allow_failures: usize,
    },
    Wpt {
        manifest: PathBuf,
        #[arg(long)]
        expectations: Option<PathBuf>,
        #[arg(long = "subset")]
        subsets: Vec<String>,
        #[arg(long, default_value_t = 1)]
        repeat: usize,
        #[arg(long)]
        timeout_ms: Option<u64>,
        #[arg(long)]
        json: bool,
        #[arg(long)]
        report_output: Option<PathBuf>,
    },
    Capabilities,
    Coverage {
        #[arg(long)]
        json: bool,
        #[arg(long = "require")]
        required_features: Vec<String>,
        #[arg(long)]
        min_implemented_ratio: Option<f64>,
        #[arg(long)]
        max_missing: Option<usize>,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Command::Render {
            target,
            width,
            max_bytes,
            json,
            display_list,
        } => {
            ensure_static_target(&target)?;
            let render =
                load_and_render(&target, BrowserRenderOptions { width, max_bytes }).await?;
            print_render(render, json, display_list)?;
        }
        Command::RenderFile {
            path,
            width,
            json,
            display_list,
        } => {
            let bytes = std::fs::read(&path)?;
            let render = render_html(
                &path.display().to_string(),
                &bytes,
                BrowserRenderOptions {
                    width,
                    ..BrowserRenderOptions::default()
                },
            );
            print_render(render, json, display_list)?;
        }
        Command::HitTest {
            target,
            x,
            y,
            width,
            max_bytes,
            json,
        } => {
            ensure_static_target(&target)?;
            let render =
                load_and_render(&target, BrowserRenderOptions { width, max_bytes }).await?;
            let report = hit_test_render(&render, x, y);
            print_hit_test(&report, json)?;
        }
        Command::LayerTree {
            target,
            width,
            max_bytes,
            json,
        } => {
            ensure_static_target(&target)?;
            let render =
                load_and_render(&target, BrowserRenderOptions { width, max_bytes }).await?;
            let report = layer_tree_render(&render);
            print_layer_tree(&report, json)?;
        }
        Command::LayoutTree {
            target,
            width,
            max_bytes,
            json,
        } => {
            ensure_static_target(&target)?;
            let render =
                load_and_render(&target, BrowserRenderOptions { width, max_bytes }).await?;
            let report = layout_tree_render(&render);
            print_layout_tree(&report, json)?;
        }
        Command::Viewport {
            target,
            width,
            max_bytes,
            viewport_width,
            viewport_height,
            viewport_x,
            viewport_y,
            previous_x,
            previous_y,
            previous_width,
            previous_height,
            json,
        } => {
            ensure_static_target(&target)?;
            let render =
                load_and_render(&target, BrowserRenderOptions { width, max_bytes }).await?;
            let previous = parse_previous_viewport_state(
                previous_x,
                previous_y,
                previous_width,
                previous_height,
            )?;
            let report = browser_document_viewport(
                &render,
                BrowserViewportState {
                    x: viewport_x,
                    y: viewport_y,
                    width: viewport_width.unwrap_or(width),
                    height: viewport_height,
                },
                previous,
            );
            print_document_viewport(&report, json)?;
        }
        Command::ViewportFrame {
            target,
            width,
            max_bytes,
            cell_width,
            cell_height,
            viewport_width,
            viewport_height,
            viewport_x,
            viewport_y,
            previous_x,
            previous_y,
            previous_width,
            previous_height,
            output,
            json,
        } => {
            ensure_static_target(&target)?;
            let render =
                load_and_render(&target, BrowserRenderOptions { width, max_bytes }).await?;
            let previous = parse_previous_viewport_state(
                previous_x,
                previous_y,
                previous_width,
                previous_height,
            )?;
            let frame = browser_viewport_frame(
                &render,
                BrowserViewportState {
                    x: viewport_x,
                    y: viewport_y,
                    width: viewport_width.unwrap_or(width),
                    height: viewport_height,
                },
                previous,
                BrowserRasterOptions {
                    cell_width,
                    cell_height,
                    ..BrowserRasterOptions::default()
                },
            )?;
            print_viewport_frame(&frame, output.as_ref(), json)?;
        }
        Command::AccessibilityTree {
            target,
            width,
            max_bytes,
            json,
        } => {
            ensure_static_target(&target)?;
            let report =
                load_accessibility_tree(&target, BrowserRenderOptions { width, max_bytes }).await?;
            print_accessibility_tree(&report, json)?;
        }
        Command::Raster {
            target,
            width,
            max_bytes,
            cell_width,
            cell_height,
            viewport_x,
            viewport_y,
            viewport_width,
            viewport_height,
            output,
            json,
        } => {
            ensure_static_target(&target)?;
            let render =
                load_and_render(&target, BrowserRenderOptions { width, max_bytes }).await?;
            let options = BrowserRasterOptions {
                cell_width,
                cell_height,
                viewport_x,
                viewport_y,
                viewport_width,
                viewport_height,
                ..BrowserRasterOptions::default()
            };
            print_raster(&render, options, output.as_ref(), json)?;
        }
        Command::RasterFile {
            path,
            width,
            cell_width,
            cell_height,
            viewport_x,
            viewport_y,
            viewport_width,
            viewport_height,
            output,
            json,
        } => {
            let bytes = std::fs::read(&path)?;
            let render = render_html(
                &path.display().to_string(),
                &bytes,
                BrowserRenderOptions {
                    width,
                    ..BrowserRenderOptions::default()
                },
            );
            let options = BrowserRasterOptions {
                cell_width,
                cell_height,
                viewport_x,
                viewport_y,
                viewport_width,
                viewport_height,
                ..BrowserRasterOptions::default()
            };
            print_raster(&render, options, output.as_ref(), json)?;
        }
        Command::Screenshot {
            target,
            width,
            max_bytes,
            cell_width,
            cell_height,
            viewport_x,
            viewport_y,
            viewport_width,
            viewport_height,
            output,
            json,
        } => {
            ensure_static_target(&target)?;
            let render =
                load_and_render(&target, BrowserRenderOptions { width, max_bytes }).await?;
            let options = BrowserRasterOptions {
                cell_width,
                cell_height,
                viewport_x,
                viewport_y,
                viewport_width,
                viewport_height,
                ..BrowserRasterOptions::default()
            };
            print_screenshot(&render, options, output.as_ref(), json)?;
        }
        Command::ScreenshotFile {
            path,
            width,
            cell_width,
            cell_height,
            viewport_x,
            viewport_y,
            viewport_width,
            viewport_height,
            output,
            json,
        } => {
            let bytes = std::fs::read(&path)?;
            let render = render_html(
                &path.display().to_string(),
                &bytes,
                BrowserRenderOptions {
                    width,
                    ..BrowserRenderOptions::default()
                },
            );
            let options = BrowserRasterOptions {
                cell_width,
                cell_height,
                viewport_x,
                viewport_y,
                viewport_width,
                viewport_height,
                ..BrowserRasterOptions::default()
            };
            print_screenshot(&render, options, output.as_ref(), json)?;
        }
        Command::RenderStyled {
            target,
            width,
            max_bytes,
            resource_max_bytes,
            json,
            display_list,
        } => {
            ensure_static_target(&target)?;
            let mut session = BrowserSession::new(BrowserRenderOptions { width, max_bytes });
            session.navigate(&target).await?;
            let stylesheet_report = session
                .render_current_with_stylesheets(resource_max_bytes)
                .await?;
            let Some(render) = session.current() else {
                return Err(anyhow!("styled render produced no current page"));
            };
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&StyledRenderReport {
                        stylesheet_report,
                        render
                    })?
                );
            } else {
                print_render(render.clone(), false, display_list)?;
            }
        }
        Command::RenderScripted {
            target,
            width,
            max_bytes,
            resource_max_bytes,
            json,
            display_list,
        } => {
            ensure_static_target(&target)?;
            let mut session = BrowserSession::new(BrowserRenderOptions { width, max_bytes });
            session.navigate(&target).await?;
            let script_report = session
                .render_current_with_scripts(resource_max_bytes)
                .await?;
            let Some(render) = session.current() else {
                return Err(anyhow!("scripted render produced no current page"));
            };
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&ScriptedRenderReport {
                        script_report,
                        render
                    })?
                );
            } else {
                print_render(render.clone(), false, display_list)?;
            }
        }
        Command::RenderImages {
            target,
            width,
            max_bytes,
            resource_max_bytes,
            json,
            display_list,
        } => {
            ensure_static_target(&target)?;
            let mut session = BrowserSession::new(BrowserRenderOptions { width, max_bytes });
            session.navigate(&target).await?;
            let image_report = session
                .render_current_with_images(resource_max_bytes)
                .await?;
            let Some(render) = session.current() else {
                return Err(anyhow!("image render produced no current page"));
            };
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&ImageRenderReport {
                        image_report,
                        render
                    })?
                );
            } else {
                print_render(render.clone(), false, display_list)?;
            }
        }
        Command::Click {
            target,
            selector,
            width,
            max_bytes,
            json,
            display_list,
        } => {
            ensure_static_target(&target)?;
            let mut session = BrowserSession::new(BrowserRenderOptions { width, max_bytes });
            session.navigate(&target).await?;
            let render = session
                .click_selector_with_default_action(&selector)
                .await?
                .clone();
            print_render(render, json, display_list)?;
        }
        Command::ClickAt {
            target,
            x,
            y,
            viewport_x,
            viewport_y,
            width,
            max_bytes,
            json,
            display_list,
        } => {
            ensure_static_target(&target)?;
            let mut session = BrowserSession::new(BrowserRenderOptions { width, max_bytes });
            session.navigate(&target).await?;
            let render = session
                .click_at_with_default_action(
                    viewport_x.saturating_add(x),
                    viewport_y.saturating_add(y),
                )
                .await?
                .clone();
            print_render(render, json, display_list)?;
        }
        Command::Session {
            targets,
            width,
            max_bytes,
            back,
            forward,
            json,
            display_list,
        } => {
            if targets.is_empty() {
                return Err(anyhow!("session requires at least one target"));
            }
            let mut session = BrowserSession::new(BrowserRenderOptions { width, max_bytes });
            for target in &targets {
                ensure_static_target(target)?;
                session.navigate(target).await?;
            }
            for _ in 0..back {
                session.back()?;
            }
            for _ in 0..forward {
                session.forward()?;
            }
            print_session(&session, json, display_list)?;
        }
        Command::Browse {
            target,
            width,
            max_bytes,
            resource_max_bytes,
            viewport_width,
            viewport_height,
            viewport_x,
            viewport_y,
            commands,
            cookie_jar,
            local_storage,
            screenshot_output,
            no_interactive,
            json,
        } => {
            ensure_static_target(&target)?;
            let initial_cookie_jar = cookie_jar
                .as_deref()
                .map(load_browser_cookie_jar)
                .transpose()?
                .unwrap_or_default();
            let initial_local_storage = local_storage
                .as_deref()
                .map(load_browser_local_storage)
                .transpose()?
                .unwrap_or_default();
            let mut session = BrowserSession::new_with_state(
                BrowserRenderOptions { width, max_bytes },
                initial_cookie_jar,
                initial_local_storage,
            );
            session.navigate(&target).await?;
            let state = BrowserShellState {
                viewport_x,
                viewport_y,
                viewport_width: viewport_width.unwrap_or(width),
                viewport_height,
            };
            let options = BrowserRenderOptions { width, max_bytes };
            let mut tabs = BrowserShellTabs::new(session, state, options);
            if commands.is_empty() && !json && !no_interactive && io::stdin().is_terminal() {
                run_interactive_browser_shell(&mut tabs, resource_max_bytes).await?;
            } else {
                let mut last_command = None;
                for command in &commands {
                    let parsed = parse_browser_shell_command(command)?;
                    let keep_running = apply_browser_shell_tabs_command(
                        &mut tabs,
                        resource_max_bytes,
                        parsed.clone(),
                    )
                    .await?;
                    last_command = Some(parsed);
                    if !keep_running {
                        break;
                    }
                }
                if let Some(command) = last_command {
                    print_browser_shell_after_command(&tabs, &command, json)?;
                } else {
                    print_browser_shell(&tabs, json)?;
                }
            }
            let active = tabs.active()?;
            if let Some(path) = screenshot_output.as_deref() {
                write_browser_shell_screenshot(&active.session, active.state, path)?;
            }
            if let Some(path) = cookie_jar.as_deref() {
                save_browser_cookie_jar(path, &active.session.cookies_snapshot())?;
            }
            if let Some(path) = local_storage.as_deref() {
                save_browser_local_storage(path, &active.session.local_storage_snapshot())?;
            }
        }
        Command::App(args) => run_browser_app_cli(args).await?,
        Command::Window(args) => run_browser_window_cli(args).await?,
        Command::FormUrl {
            target,
            form,
            fields,
            width,
            max_bytes,
        } => {
            ensure_static_target(&target)?;
            let render =
                load_and_render(&target, BrowserRenderOptions { width, max_bytes }).await?;
            let Some(browser_form) = render.forms.get(form) else {
                return Err(anyhow!(
                    "form index {} not found; page has {} form(s)",
                    form,
                    render.forms.len()
                ));
            };
            let overrides = parse_fields(&fields)?;
            println!("{}", build_get_form_url(browser_form, &overrides)?);
        }
        Command::Submit {
            target,
            form,
            fields,
            width,
            max_bytes,
            json,
            display_list,
        } => {
            run_submit_form_command(
                &target,
                form,
                &fields,
                BrowserRenderOptions { width, max_bytes },
                json,
                display_list,
                BrowserFormSubmitMode::Auto,
            )
            .await?;
        }
        Command::SubmitGet {
            target,
            form,
            fields,
            width,
            max_bytes,
            json,
            display_list,
        } => {
            run_submit_form_command(
                &target,
                form,
                &fields,
                BrowserRenderOptions { width, max_bytes },
                json,
                display_list,
                BrowserFormSubmitMode::Get,
            )
            .await?;
        }
        Command::SubmitPost {
            target,
            form,
            fields,
            width,
            max_bytes,
            json,
            display_list,
        } => {
            run_submit_form_command(
                &target,
                form,
                &fields,
                BrowserRenderOptions { width, max_bytes },
                json,
                display_list,
                BrowserFormSubmitMode::Post,
            )
            .await?;
        }
        Command::Resources {
            target,
            width,
            max_bytes,
            json,
        } => {
            ensure_static_target(&target)?;
            let render =
                load_and_render(&target, BrowserRenderOptions { width, max_bytes }).await?;
            if json {
                println!("{}", serde_json::to_string_pretty(&render.resources)?);
            } else {
                for resource in &render.resources {
                    println!(
                        "{} {} -> {}",
                        resource.kind, resource.url, resource.resolved
                    );
                }
            }
        }
        Command::FetchResources {
            target,
            width,
            max_bytes,
            resource_max_bytes,
            json,
        } => {
            ensure_static_target(&target)?;
            let mut session = BrowserSession::new(BrowserRenderOptions { width, max_bytes });
            session.navigate(&target).await?;
            let report = session.fetch_current_resources(resource_max_bytes).await?;
            if json {
                println!("{}", serde_json::to_string_pretty(&report)?);
            } else {
                println!(
                    "resources: total={} fetched={} cached={} failed={} skipped={}",
                    report.total, report.fetched, report.cached, report.failed, report.skipped
                );
                for resource in &report.resources {
                    let source = resource.source.as_deref().unwrap_or("-");
                    let error = resource.error.as_deref().unwrap_or("");
                    println!(
                        "{} {} bytes={} {} -> {} {}",
                        resource.status,
                        resource.resource.kind,
                        resource.bytes,
                        resource.resource.url,
                        source,
                        error
                    );
                }
            }
        }
        Command::Verify { manifest, json } => {
            let report = verify_browser_fixtures(&manifest)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&report)?);
            } else {
                println!(
                    "browser fixtures: {}/{} passed",
                    report.passed, report.fixture_count
                );
                for failure in &report.failures {
                    println!(
                        "FAIL {} ({}) - {}",
                        failure.name, failure.path, failure.reason
                    );
                }
            }
            if report.failed > 0 {
                return Err(anyhow!(
                    "browser fixture verification failed: {} failure(s)",
                    report.failed
                ));
            }
        }
        Command::VisualVerify {
            manifest,
            artifact_dir,
            baseline_dir,
            require_all_baselines,
            max_diff_pixels,
            max_diff_ratio,
            json,
        } => {
            let report = verify_browser_visuals(
                &manifest,
                artifact_dir.as_deref(),
                baseline_dir.as_deref(),
                require_all_baselines,
                max_diff_pixels,
                max_diff_ratio,
            )?;
            if json {
                println!("{}", serde_json::to_string_pretty(&report)?);
            } else {
                print_visual_report(&report);
            }
            if report.failed > 0 {
                return Err(anyhow!(
                    "browser visual verification failed: {} failure(s)",
                    report.failed
                ));
            }
        }
        Command::CompareChromium {
            manifest,
            json,
            allow_failures,
        } => {
            let report = compare_browser_fixtures_with_chromium(&manifest)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&report)?);
            } else {
                print_chromium_parity_report(&report);
            }
            if report.failed > allow_failures {
                return Err(anyhow!(
                    "browser Chromium parity failed: {} failure(s), allowed {}",
                    report.failed,
                    allow_failures
                ));
            }
        }
        Command::Wpt {
            manifest,
            expectations,
            subsets,
            repeat,
            timeout_ms,
            json,
            report_output,
        } => {
            let report = run_browser_compat(BrowserCompatOptions {
                manifest,
                expectations,
                subsets,
                repeat,
                timeout_ms,
                gate: BrowserCompatGate::default(),
            })?;
            if let Some(path) = report_output {
                write_json_report(&path, &report)?;
            }
            if json {
                println!("{}", serde_json::to_string_pretty(&report)?);
            } else {
                print_browser_compat_report(&report);
            }
            if report.unexpected_count > 0 {
                return Err(anyhow!(
                    "browser compatibility failed: {} unexpected result(s)",
                    report.unexpected_count
                ));
            }
        }
        Command::Capabilities => {
            let report = browser_coverage_report();
            let implemented = report
                .features
                .iter()
                .filter(|feature| feature.status == BrowserFeatureState::Implemented)
                .map(|feature| feature.id.as_str())
                .collect::<Vec<_>>()
                .join(" ");
            println!("implemented: {implemented}");
            println!("unsupported: {}", unsupported_feature_summary().join(","));
        }
        Command::Coverage {
            json,
            required_features,
            min_implemented_ratio,
            max_missing,
        } => {
            let mut report = browser_coverage_report();
            let gate = BrowserCoverageGate {
                required_features,
                min_implemented_ratio,
                max_missing_features: max_missing,
            };
            let failed_gate = if gate.is_empty() {
                false
            } else {
                !report.apply_gate(gate)
            };
            if json {
                println!("{}", serde_json::to_string_pretty(&report)?);
            } else {
                print_coverage_report(&report);
            }
            if failed_gate {
                return Err(anyhow!(
                    "browser coverage gate failed: implemented_ratio {:.4}, missing {}, missing_required [{}]",
                    report.implemented_ratio,
                    report.missing_count,
                    report.missing_required_features.join(", ")
                ));
            }
        }
    }

    Ok(())
}

fn parse_fields(fields: &[String]) -> Result<Vec<(String, String)>> {
    fields
        .iter()
        .map(|field| {
            let Some((name, value)) = field.split_once('=') else {
                return Err(anyhow!("field override must be name=value: {field}"));
            };
            if name.is_empty() {
                return Err(anyhow!("field override name cannot be empty: {field}"));
            }
            Ok((name.to_owned(), value.to_owned()))
        })
        .collect()
}

#[derive(Serialize)]
struct SessionReport<'a> {
    history: BrowserHistorySnapshot,
    cookies: Vec<BrowserCookie>,
    current: Option<&'a BrowserRender>,
}

#[derive(Serialize)]
struct StyledRenderReport<'a> {
    stylesheet_report: BrowserStylesheetRenderReport,
    render: &'a BrowserRender,
}

#[derive(Serialize)]
struct ScriptedRenderReport<'a> {
    script_report: BrowserScriptRenderReport,
    render: &'a BrowserRender,
}

#[derive(Serialize)]
struct ImageRenderReport<'a> {
    image_report: BrowserImageRenderReport,
    render: &'a BrowserRender,
}

fn load_browser_cookie_jar(path: &Path) -> Result<BrowserCookieJar> {
    match std::fs::read(path) {
        Ok(bytes) => {
            if bytes.is_empty() {
                return Ok(BrowserCookieJar::default());
            }
            let cookies = serde_json::from_slice::<Vec<BrowserCookie>>(&bytes)
                .with_context(|| format!("failed to parse cookie jar {}", path.display()))?;
            Ok(BrowserCookieJar::from_cookies(cookies))
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(BrowserCookieJar::default()),
        Err(error) => {
            Err(error).with_context(|| format!("failed to read cookie jar {}", path.display()))
        }
    }
}

fn save_browser_cookie_jar(path: &Path, cookies: &[BrowserCookie]) -> Result<()> {
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent).with_context(|| {
            format!("failed to create cookie jar directory {}", parent.display())
        })?;
    }
    let bytes = serde_json::to_vec_pretty(cookies)
        .with_context(|| format!("failed to encode cookie jar {}", path.display()))?;
    std::fs::write(path, bytes)
        .with_context(|| format!("failed to write cookie jar {}", path.display()))?;
    Ok(())
}

fn load_browser_local_storage(path: &Path) -> Result<BrowserLocalStorage> {
    match std::fs::read(path) {
        Ok(bytes) => {
            if bytes.is_empty() {
                return Ok(BrowserLocalStorage::default());
            }
            serde_json::from_slice::<BrowserLocalStorage>(&bytes)
                .with_context(|| format!("failed to parse local storage {}", path.display()))
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(BrowserLocalStorage::default()),
        Err(error) => {
            Err(error).with_context(|| format!("failed to read local storage {}", path.display()))
        }
    }
}

fn save_browser_local_storage(path: &Path, local_storage: &BrowserLocalStorage) -> Result<()> {
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent).with_context(|| {
            format!(
                "failed to create local storage directory {}",
                parent.display()
            )
        })?;
    }
    let bytes = serde_json::to_vec_pretty(local_storage)
        .with_context(|| format!("failed to encode local storage {}", path.display()))?;
    std::fs::write(path, bytes)
        .with_context(|| format!("failed to write local storage {}", path.display()))?;
    Ok(())
}

async fn run_submit_form_command(
    target: &str,
    form_index: usize,
    fields: &[String],
    options: BrowserRenderOptions,
    json: bool,
    display_list: bool,
    mode: BrowserFormSubmitMode,
) -> Result<()> {
    ensure_static_target(target)?;
    let mut session = BrowserSession::new(options);
    session.navigate(target).await?;
    let overrides = parse_fields(fields)?;
    submit_session_form(&mut session, form_index, &overrides, mode).await?;
    print_session(&session, json, display_list)
}

async fn submit_session_form(
    session: &mut BrowserSession,
    form_index: usize,
    fields: &[(String, String)],
    mode: BrowserFormSubmitMode,
) -> Result<()> {
    match mode {
        BrowserFormSubmitMode::Auto => {
            session.submit_form(form_index, fields).await?;
        }
        BrowserFormSubmitMode::Get => {
            session.submit_get_form(form_index, fields).await?;
        }
        BrowserFormSubmitMode::Post => {
            ensure_current_form_method(session, form_index, "POST")?;
            session.submit_form(form_index, fields).await?;
        }
    }
    Ok(())
}

fn ensure_current_form_method(
    session: &BrowserSession,
    form_index: usize,
    expected: &str,
) -> Result<()> {
    let Some(current) = session.current() else {
        return Err(anyhow!("cannot submit form: session has no current page"));
    };
    let Some(form) = current.forms.get(form_index) else {
        return Err(anyhow!(
            "form index {} not found; current page has {} form(s)",
            form_index,
            current.forms.len()
        ));
    };
    if !form.method.eq_ignore_ascii_case(expected) {
        return Err(anyhow!(
            "form {} uses {}; expected {} form submission",
            form_index,
            form.method,
            expected
        ));
    }
    Ok(())
}

fn print_session(session: &BrowserSession, json: bool, display_list: bool) -> Result<()> {
    let history = session.snapshot();
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&SessionReport {
                history,
                cookies: session.cookies_snapshot(),
                current: session.current()
            })?
        );
        return Ok(());
    }

    let current_position = history.current_index.map(|index| index + 1).unwrap_or(0);
    println!("history: {current_position}/{}", history.entries.len());
    for (index, entry) in history.entries.iter().enumerate() {
        let marker = if Some(index) == history.current_index {
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
    let cookies = session.cookies_snapshot();
    if !cookies.is_empty() {
        println!(
            "cookies: {}",
            cookies
                .iter()
                .map(|cookie| format!("{}@{}{}", cookie.name, cookie.domain, cookie.path))
                .collect::<Vec<_>>()
                .join(", ")
        );
    }
    println!();
    if let Some(render) = session.current() {
        print_render(render.clone(), false, display_list)?;
    }
    Ok(())
}

async fn run_interactive_browser_shell(
    tabs: &mut BrowserShellTabs,
    resource_max_bytes: usize,
) -> Result<()> {
    print_browser_shell_help();
    print_browser_shell(tabs, false)?;
    let stdin = io::stdin();
    loop {
        print!("brutal-browser> ");
        io::stdout().flush()?;
        let mut input = String::new();
        if stdin.read_line(&mut input)? == 0 {
            break;
        }
        let command = input.trim();
        if command.is_empty() {
            continue;
        }
        let parsed = parse_browser_shell_command(command)?;
        if !apply_browser_shell_tabs_command(tabs, resource_max_bytes, parsed.clone()).await? {
            break;
        }
        print_browser_shell_after_command(tabs, &parsed, false)?;
    }
    Ok(())
}

async fn apply_browser_shell_tabs_command(
    tabs: &mut BrowserShellTabs,
    resource_max_bytes: usize,
    command: BrowserShellCommand,
) -> Result<bool> {
    if let Some(keep_running) = tabs.apply_tab_command(&command).await? {
        return Ok(keep_running);
    }
    let (session, state) = tabs.active_parts_mut()?;
    apply_browser_shell_command_parsed(session, state, resource_max_bytes, command).await
}

#[cfg(test)]
async fn apply_browser_shell_command(
    session: &mut BrowserSession,
    state: &mut BrowserShellState,
    resource_max_bytes: usize,
    input: &str,
) -> Result<bool> {
    let command = parse_browser_shell_command(input)?;
    apply_browser_shell_command_parsed(session, state, resource_max_bytes, command).await
}

async fn apply_browser_shell_command_parsed(
    session: &mut BrowserSession,
    state: &mut BrowserShellState,
    resource_max_bytes: usize,
    command: BrowserShellCommand,
) -> Result<bool> {
    match command {
        BrowserShellCommand::Open(target) => {
            let target = session.resolve_current_target(&target);
            ensure_static_target(&target)?;
            session.navigate(&target).await?;
            reset_browser_shell_viewport_to_current_location(session, state)?;
        }
        BrowserShellCommand::Back => {
            session.back()?;
            reset_browser_shell_viewport_to_current_location(session, state)?;
        }
        BrowserShellCommand::Forward => {
            session.forward()?;
            reset_browser_shell_viewport_to_current_location(session, state)?;
        }
        BrowserShellCommand::Reload => {
            session.reload().await?;
            reset_browser_shell_viewport_to_current_location(session, state)?;
        }
        BrowserShellCommand::Location
        | BrowserShellCommand::Cookies
        | BrowserShellCommand::LocalStorage
        | BrowserShellCommand::SessionStorage => {}
        BrowserShellCommand::ClearCookies => session.clear_cookies(),
        BrowserShellCommand::ClearLocalStorage => session.clear_local_storage(),
        BrowserShellCommand::ClearSessionStorage => session.clear_session_storage(),
        BrowserShellCommand::Click(selector) => {
            session
                .click_selector_with_default_action(&selector)
                .await?;
            reset_browser_shell_viewport_to_current_location(session, state)?;
        }
        BrowserShellCommand::ClickAt { x, y } => {
            let document_x = state.viewport_x.saturating_add(x);
            let document_y = state.viewport_y.saturating_add(y);
            session
                .click_at_with_default_action(document_x, document_y)
                .await?;
            reset_browser_shell_viewport_to_current_location(session, state)?;
        }
        BrowserShellCommand::Links | BrowserShellCommand::Forms => {}
        BrowserShellCommand::Link(target) => {
            match target {
                BrowserShellLinkTarget::Index(index) => {
                    session.activate_link(index).await?;
                }
                BrowserShellLinkTarget::Text(text) => {
                    session.activate_link_text(&text).await?;
                }
                BrowserShellLinkTarget::Selector(selector) => {
                    session.activate_link_selector(&selector).await?;
                }
            }
            reset_browser_shell_viewport_to_current_location(session, state)?;
        }
        BrowserShellCommand::Focus(selector) => {
            session.focus_selector(&selector)?;
            reset_browser_shell_viewport_to_current_location(session, state)?;
        }
        BrowserShellCommand::FocusNext => {
            session.focus_next_control()?;
            reset_browser_shell_viewport_to_current_location(session, state)?;
        }
        BrowserShellCommand::FocusPrevious => {
            session.focus_previous_control()?;
            reset_browser_shell_viewport_to_current_location(session, state)?;
        }
        BrowserShellCommand::TypeText(text) => {
            session.type_text(&text)?;
            reset_browser_shell_viewport_to_current_location(session, state)?;
        }
        BrowserShellCommand::DeleteTextBackward(count) => {
            session.delete_text_backward(count)?;
            reset_browser_shell_viewport_to_current_location(session, state)?;
        }
        BrowserShellCommand::ClearText => {
            session.clear_focused_text()?;
            reset_browser_shell_viewport_to_current_location(session, state)?;
        }
        BrowserShellCommand::SubmitFocused => {
            session.submit_focused_form().await?;
            reset_browser_shell_viewport_to_current_location(session, state)?;
        }
        BrowserShellCommand::ToggleFocused => {
            let focused = session
                .focused_control()
                .ok_or_else(|| anyhow!("cannot toggle focused control: no focused form control"))?;
            session.toggle_form_control(focused.form_index, focused.control_index)?;
            reset_browser_shell_viewport_to_current_location(session, state)?;
        }
        BrowserShellCommand::ToggleControl {
            form_index,
            control_index,
        } => {
            session.toggle_form_control(form_index, control_index)?;
            reset_browser_shell_viewport_to_current_location(session, state)?;
        }
        BrowserShellCommand::SelectFocused(value) => {
            session.select_focused_option(&value)?;
            reset_browser_shell_viewport_to_current_location(session, state)?;
        }
        BrowserShellCommand::SelectControl {
            form_index,
            control_index,
            value,
        } => {
            session.select_form_option(form_index, control_index, &value)?;
            reset_browser_shell_viewport_to_current_location(session, state)?;
        }
        BrowserShellCommand::Find { query, next } => {
            let start_y = if next {
                state.viewport_y.saturating_add(1)
            } else {
                state.viewport_y
            };
            state.viewport_y = find_browser_shell_text_line(session, *state, &query, start_y)?;
            state.viewport_x = 0;
            clamp_browser_shell_viewport(session, state)?;
        }
        BrowserShellCommand::Fill {
            form_index,
            name,
            value,
        } => {
            session.set_form_field(form_index, &name, &value)?;
            reset_browser_shell_viewport_to_current_location(session, state)?;
        }
        BrowserShellCommand::Submit {
            mode,
            form_index,
            fields,
        } => {
            submit_session_form(session, form_index, &fields, mode).await?;
            reset_browser_shell_viewport_to_current_location(session, state)?;
        }
        BrowserShellCommand::Styles => {
            session
                .render_current_with_stylesheets(resource_max_bytes)
                .await?;
            clamp_browser_shell_viewport(session, state)?;
        }
        BrowserShellCommand::Scripts => {
            session
                .render_current_with_scripts(resource_max_bytes)
                .await?;
            clamp_browser_shell_viewport(session, state)?;
        }
        BrowserShellCommand::Images => {
            session
                .render_current_with_images(resource_max_bytes)
                .await?;
            clamp_browser_shell_viewport(session, state)?;
        }
        BrowserShellCommand::Resources => {
            let _ = session.fetch_current_resources(resource_max_bytes).await?;
            clamp_browser_shell_viewport(session, state)?;
        }
        BrowserShellCommand::Scroll(delta) => {
            if !session.dispatch_wheel_event(0, delta)? {
                apply_signed_offset(&mut state.viewport_y, delta);
            }
            clamp_browser_shell_viewport(session, state)?;
        }
        BrowserShellCommand::HorizontalScroll(delta) => {
            if !session.dispatch_wheel_event(delta, 0)? {
                apply_signed_offset(&mut state.viewport_x, delta);
            }
            clamp_browser_shell_viewport(session, state)?;
        }
        BrowserShellCommand::Top => {
            state.viewport_y = 0;
            clamp_browser_shell_viewport(session, state)?;
        }
        BrowserShellCommand::Bottom => {
            let viewport = current_browser_shell_viewport(session, *state)?;
            state.viewport_x = viewport.x;
            state.viewport_y = viewport.max_scroll_y;
        }
        BrowserShellCommand::Tabs
        | BrowserShellCommand::NewTab(_)
        | BrowserShellCommand::SwitchTab(_)
        | BrowserShellCommand::CloseTab(_)
        | BrowserShellCommand::Render
        | BrowserShellCommand::History
        | BrowserShellCommand::Help => {}
        BrowserShellCommand::Quit => return Ok(false),
    }
    Ok(true)
}

fn apply_signed_offset(value: &mut usize, delta: isize) {
    if delta.is_negative() {
        *value = value.saturating_sub(delta.unsigned_abs());
    } else {
        *value = value.saturating_add(delta as usize);
    }
}

fn print_browser_shell_location(session: &BrowserSession, state: BrowserShellState) -> Result<()> {
    let viewport = current_browser_shell_viewport(session, state)?;
    let history = session.snapshot();
    println!("location: {}", viewport.source);
    if viewport.title.is_empty() {
        println!("title: (untitled)");
    } else {
        println!("title: {}", viewport.title);
    }
    println!(
        "history: {}/{}",
        history.current_index.map(|index| index + 1).unwrap_or(0),
        history.entries.len()
    );
    println!(
        "viewport: x={} y={} width={} height={} max_x={} max_y={} visible_boxes={}/{}",
        viewport.x,
        viewport.y,
        viewport.width,
        viewport.height,
        viewport.max_scroll_x,
        viewport.max_scroll_y,
        viewport.visible_layout_box_count,
        viewport.layout_box_count
    );
    Ok(())
}

fn print_browser_shell_cookies(session: &BrowserSession) -> Result<()> {
    let cookies = session.cookies_snapshot();
    if cookies.is_empty() {
        println!("cookies: none");
        return Ok(());
    }
    println!("cookies: {}", cookies.len());
    for (index, cookie) in cookies.iter().enumerate() {
        println!(
            "{index}: {}={} domain={} path={} secure={} http_only={} host_only={}",
            cookie.name,
            cookie.value,
            cookie.domain,
            cookie.path,
            cookie.secure,
            cookie.http_only,
            cookie.host_only
        );
    }
    Ok(())
}

fn print_browser_shell_storage(label: &str, entries: Vec<BrowserLocalStorageEntry>) {
    if entries.is_empty() {
        println!("{label}: none");
        return;
    }
    println!("{label}: {}", entries.len());
    for (index, entry) in entries.iter().enumerate() {
        println!(
            "{index}: origin={} key={} value={}",
            entry.origin, entry.key, entry.value
        );
    }
}

fn print_browser_shell_local_storage(session: &BrowserSession) {
    print_browser_shell_storage("localStorage", session.local_storage_entries());
}

fn print_browser_shell_session_storage(session: &BrowserSession) {
    print_browser_shell_storage("sessionStorage", session.session_storage_entries());
}

fn print_browser_shell_clear_cookies() {
    println!("cookies: cleared");
}

fn print_browser_shell_clear_local_storage() {
    println!("localStorage: cleared");
}

fn print_browser_shell_clear_session_storage() {
    println!("sessionStorage: cleared");
}

fn find_browser_shell_text_line(
    session: &BrowserSession,
    state: BrowserShellState,
    query: &str,
    start_y: usize,
) -> Result<usize> {
    let Some(render) = session.current() else {
        return Err(anyhow!("cannot find text: session has no current page"));
    };
    let query = query.trim();
    if query.is_empty() {
        return Err(anyhow!("find requires an argument"));
    }
    let viewport = current_browser_shell_viewport(session, state)?;
    let document = browser_text_viewport(
        render,
        BrowserTextViewportOptions {
            x: 0,
            y: 0,
            width: viewport.document_width.max(1),
            height: viewport.document_height.max(1),
        },
    );
    let needle = query.to_lowercase();
    let start_y = start_y.min(document.lines.len());

    if let Some((line, _)) = document
        .lines
        .iter()
        .enumerate()
        .skip(start_y)
        .find(|(_, line)| line.to_lowercase().contains(&needle))
    {
        return Ok(line);
    }
    if let Some((line, _)) = document
        .lines
        .iter()
        .enumerate()
        .take(start_y)
        .find(|(_, line)| line.to_lowercase().contains(&needle))
    {
        return Ok(line);
    }

    Err(anyhow!("text not found: {query:?}"))
}

fn print_browser_shell_after_command(
    tabs: &BrowserShellTabs,
    command: &BrowserShellCommand,
    json: bool,
) -> Result<()> {
    if json {
        return print_browser_shell(tabs, true);
    }
    let active = tabs.active()?;
    match command {
        BrowserShellCommand::Help => {
            print_browser_shell_help();
            Ok(())
        }
        BrowserShellCommand::Location => {
            print_browser_shell_location(&active.session, active.state)
        }
        BrowserShellCommand::Cookies => print_browser_shell_cookies(&active.session),
        BrowserShellCommand::LocalStorage => {
            print_browser_shell_local_storage(&active.session);
            Ok(())
        }
        BrowserShellCommand::SessionStorage => {
            print_browser_shell_session_storage(&active.session);
            Ok(())
        }
        BrowserShellCommand::ClearCookies => {
            print_browser_shell_clear_cookies();
            Ok(())
        }
        BrowserShellCommand::ClearLocalStorage => {
            print_browser_shell_clear_local_storage();
            Ok(())
        }
        BrowserShellCommand::ClearSessionStorage => {
            print_browser_shell_clear_session_storage();
            Ok(())
        }
        BrowserShellCommand::History => print_session(&active.session, false, false),
        BrowserShellCommand::Links => print_browser_shell_links(&active.session),
        BrowserShellCommand::Forms => print_browser_shell_forms(&active.session),
        BrowserShellCommand::Tabs => print_browser_shell_tabs(tabs),
        _ => print_browser_shell(tabs, false),
    }
}

fn print_browser_shell(tabs: &BrowserShellTabs, json: bool) -> Result<()> {
    let active = tabs.active()?;
    let viewport = current_browser_shell_viewport(&active.session, active.state)?;
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&browser_shell_report(
                &active.session,
                active.state,
                tabs.active_index(),
                tabs.summaries(),
            )?)?
        );
        return Ok(());
    }

    let title = if viewport.title.is_empty() {
        viewport.source.as_str()
    } else {
        viewport.title.as_str()
    };
    println!("# {title}");
    println!(
        "source={} viewport={}x{}+{}+{} max_scroll={}+{} document={}x{} commands visible={} culled={} boxes visible={}/{}",
        viewport.source,
        viewport.width,
        viewport.height,
        viewport.x,
        viewport.y,
        viewport.max_scroll_x,
        viewport.max_scroll_y,
        viewport.document_width,
        viewport.document_height,
        viewport.visible_command_count,
        viewport.culled_command_count,
        viewport.visible_layout_box_count,
        viewport.layout_box_count
    );
    if let Some(focused) = active.session.focused_control() {
        println!(
            "focused: form={} control={} {} name={} value={:?}",
            focused.form_index, focused.control_index, focused.kind, focused.name, focused.value
        );
    }
    println!();
    for line in &viewport.lines {
        println!("{line}");
    }
    Ok(())
}

fn print_browser_shell_tabs(tabs: &BrowserShellTabs) -> Result<()> {
    println!("tabs:");
    for tab in tabs.summaries() {
        let marker = if tab.active { "*" } else { " " };
        println!(
            "{marker}[{}] {} -> {} history={}",
            tab.index, tab.title, tab.source, tab.history_len
        );
    }
    Ok(())
}

fn print_browser_shell_links(session: &BrowserSession) -> Result<()> {
    let links = browser_shell_links(session);
    if links.is_empty() {
        println!("links: none");
        return Ok(());
    }
    println!("links:");
    for link in links {
        let text = if link.text.is_empty() {
            "(empty)"
        } else {
            link.text.as_str()
        };
        println!("[{}] {} -> {}", link.index, text, link.resolved);
    }
    Ok(())
}

fn print_browser_shell_forms(session: &BrowserSession) -> Result<()> {
    let forms = browser_shell_forms(session);
    if forms.is_empty() {
        println!("forms: none");
        return Ok(());
    }
    println!("forms:");
    for form in forms {
        println!(
            "[{}] {} action={} resolved={}",
            form.index, form.method, form.action, form.resolved_action
        );
        for control in form.controls {
            let name = if control.name.is_empty() {
                "(unnamed)"
            } else {
                control.name.as_str()
            };
            let options = if control.options.is_empty() {
                String::new()
            } else {
                let values = control
                    .options
                    .iter()
                    .map(|option| {
                        let marker = if option.selected { "*" } else { "" };
                        format!("{marker}{:?}:{}", option.value, option.label)
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                format!(" options=[{values}]")
            };
            println!(
                "  [{}] {} name={} value={:?} disabled={} checked={}{}",
                control.index,
                control.kind,
                name,
                control.value,
                control.disabled,
                control.checked,
                options
            );
        }
    }
    Ok(())
}

fn print_browser_shell_help() {
    println!(
        "commands: open <url|path>, location, cookies, local-storage, session-storage, clear-cookies, clear-local-storage, clear-session-storage, tabs, new-tab <url|path>, switch-tab <index>, close-tab [index], links, forms, link <index|text label|selector css>, back, forward, reload, refresh, click <selector>, click-at <x> <y>, tap <x> <y>, focus <selector>, tab, shift-tab, type <text>, backspace [count], clear-input, enter, space, toggle <form> <control>, choose <value>, select <form> <control> <value>, find <text>, find-next <text>, fill <form> <name=value>, field <form> <name=value>, submit <form> [name=value...], submit-get <form> [name=value...], submit-post <form> [name=value...], styles, scripts, images, resources, up/down/scroll, left/right, top, bottom, history, render, quit"
    );
}

fn print_coverage_report(report: &BrowserCoverageReport) {
    println!("browser_features: {}", report.feature_count);
    println!("browser_implemented: {}", report.implemented_count);
    println!("browser_partial: {}", report.partial_count);
    println!("browser_missing: {}", report.missing_count);
    println!("browser_implemented_ratio: {:.4}", report.implemented_ratio);
    if report.passed.is_some() {
        println!(
            "browser_required_features: {}",
            if report.required_features.is_empty() {
                "none".to_owned()
            } else {
                report.required_features.join(",")
            }
        );
        println!(
            "browser_missing_required_features: {}",
            if report.missing_required_features.is_empty() {
                "none".to_owned()
            } else {
                report.missing_required_features.join(",")
            }
        );
        println!(
            "browser_min_implemented_ratio: {}",
            option_f64(report.min_implemented_ratio)
        );
        println!(
            "browser_max_missing_features: {}",
            report
                .max_missing_features
                .map(|value| value.to_string())
                .unwrap_or_else(|| "none".to_owned())
        );
        println!("browser_gate_passed: {}", report.passed.unwrap_or(false));
    }

    for feature in &report.features {
        println!(
            "feature: {} {} {} - {}",
            feature.id,
            feature.category,
            feature_state_label(feature.status),
            feature.evidence
        );
    }
}

fn print_chromium_parity_report(report: &BrowserChromiumParityReport) {
    println!(
        "browser_chromium_parity: {}/{} passed",
        report.passed, report.fixture_count
    );
    if let Some(chrome) = &report.chrome {
        println!("browser_chromium_version: {chrome}");
    }
    for failure in &report.failures {
        println!(
            "FAIL {} ({}) - {}",
            failure.name, failure.path, failure.reason
        );
    }
    for comparison in &report.comparisons {
        println!(
            "fixture: {} title_match={} text_match={}",
            comparison.name, comparison.title_match, comparison.text_match
        );
    }
}

fn print_browser_compat_report(report: &BrowserCompatReport) {
    println!("browser_compat_engine: {}", report.engine);
    println!("browser_compat_suite: {}", report.suite);
    println!("browser_compat_manifest: {}", report.manifest);
    println!("browser_compat_manifest_hash: {}", report.manifest_hash);
    println!("browser_compat_suite_hash: {}", report.suite_hash);
    if let Some(expectation_file) = &report.expectation_file {
        println!("browser_compat_expectations: {expectation_file}");
    }
    if let Some(expectation_hash) = &report.expectation_hash {
        println!("browser_compat_expectation_hash: {expectation_hash}");
    }
    println!("browser_compat_suite_count: {}", report.suite_count);
    println!("browser_compat_selected_count: {}", report.selected_count);
    println!("browser_compat_run_count: {}", report.run_count);
    println!("browser_compat_repeat: {}", report.repeat);
    if !report.subsets.is_empty() {
        println!("browser_compat_subsets: {}", report.subsets.join(","));
    }
    if let Some(timeout_ms) = report.timeout_ms {
        println!("browser_compat_timeout_ms: {timeout_ms}");
    }
    println!("browser_compat_runnable_count: {}", report.runnable_count);
    println!("browser_compat_pass_count: {}", report.pass_count);
    println!("browser_compat_fail_count: {}", report.fail_count);
    println!("browser_compat_timeout_count: {}", report.timeout_count);
    println!("browser_compat_crash_count: {}", report.crash_count);
    println!("browser_compat_skipped_count: {}", report.skipped_count);
    println!(
        "browser_compat_unsupported_count: {}",
        report.unsupported_count
    );
    println!("browser_compat_flaky_count: {}", report.flaky_count);
    println!("browser_compat_expected_count: {}", report.expected_count);
    println!(
        "browser_compat_unexpected_count: {}",
        report.unexpected_count
    );
    println!("browser_compat_pass_rate: {:.4}", report.pass_rate);
    if let Some(passed) = report.passed {
        println!("browser_compat_gate_passed: {passed}");
    }
    for failure in &report.gate_failures {
        println!("browser_compat_gate_failure: {failure}");
    }
    for subsystem in &report.subsystems {
        println!(
            "browser_compat_subsystem: {:?} selected={} runnable={} pass={} fail={} crash={} skipped={} unsupported={} unexpected={} pass_rate={:.4}",
            subsystem.subsystem,
            subsystem.suite_count,
            subsystem.runnable_count,
            subsystem.pass_count,
            subsystem.fail_count,
            subsystem.crash_count,
            subsystem.skipped_count,
            subsystem.unsupported_count,
            subsystem.unexpected_count,
            subsystem.pass_rate
        );
    }
    for test in &report.tests {
        println!(
            "browser_compat_test: {:?} subsystem={} status={} expected_status={} expected={} attempt={}/{} duration={}us",
            test.id,
            test.subsystem,
            test.status,
            test.expected_status,
            test.expected,
            test.attempt,
            test.repeat_count,
            test.duration_us
        );
    }
}

fn write_json_report<T: Serialize>(path: &PathBuf, report: &T) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, serde_json::to_string_pretty(report)?)?;
    Ok(())
}

fn print_visual_report(report: &BrowserVisualReport) {
    println!(
        "browser_visuals: {}/{} checked passed, failures={}, missing_baseline={}",
        report.passed, report.checked, report.failed, report.missing_baseline
    );
    if let Some(artifact_dir) = &report.artifact_dir {
        println!("browser_visual_artifacts: {artifact_dir}");
    }
    if let Some(baseline_dir) = &report.baseline_dir {
        println!(
            "browser_visual_diffs: {}/{} passed, failures={}, baseline_dir={}, max_pixels={}, max_ratio={}",
            report.diff_passed,
            report.diff_checked,
            report.diff_failed,
            baseline_dir,
            report
                .max_diff_pixels
                .map(|value| value.to_string())
                .unwrap_or_else(|| "0".to_owned()),
            report
                .max_diff_ratio
                .map(|value| format!("{value:.6}"))
                .unwrap_or_else(|| "0.000000".to_owned())
        );
    }
    for failure in &report.failures {
        println!(
            "FAIL {} ({}) - {}",
            failure.name, failure.path, failure.reason
        );
    }
    for comparison in &report.comparisons {
        println!(
            "visual: {} matched={} hash={} expected={} size={}x{} diff_pixels={} diff_ratio={} diff_passed={} artifact={} baseline={} diff={}",
            comparison.name,
            comparison
                .matched
                .map(|matched| matched.to_string())
                .unwrap_or_else(|| "unbaselined".to_owned()),
            comparison.actual_raster_hash,
            comparison.expected_raster_hash.as_deref().unwrap_or("-"),
            comparison.width,
            comparison.height,
            comparison
                .diff_pixels
                .map(|value| value.to_string())
                .unwrap_or_else(|| "-".to_owned()),
            comparison
                .diff_ratio
                .map(|value| format!("{value:.6}"))
                .unwrap_or_else(|| "-".to_owned()),
            comparison
                .diff_passed
                .map(|value| value.to_string())
                .unwrap_or_else(|| "-".to_owned()),
            comparison.artifact.as_deref().unwrap_or("-"),
            comparison.baseline_artifact.as_deref().unwrap_or("-"),
            comparison.diff_artifact.as_deref().unwrap_or("-")
        );
    }
}

fn feature_state_label(state: BrowserFeatureState) -> &'static str {
    match state {
        BrowserFeatureState::Implemented => "implemented",
        BrowserFeatureState::Partial => "partial",
        BrowserFeatureState::Missing => "missing",
    }
}

fn option_f64(value: Option<f64>) -> String {
    value
        .map(|value| format!("{value:.4}"))
        .unwrap_or_else(|| "none".to_owned())
}

fn print_render(
    render: brutal_search::browser::BrowserRender,
    json: bool,
    display_list: bool,
) -> Result<()> {
    if json {
        println!("{}", serde_json::to_string_pretty(&render)?);
    } else if display_list {
        for command in &render.display_list {
            match command {
                brutal_search::browser::DisplayCommand::Text { x, y, text } => {
                    println!("text x={x} y={y} {text}");
                }
                brutal_search::browser::DisplayCommand::StyledText { x, y, text, shade } => {
                    println!("text x={x} y={y} shade={shade} {text}");
                }
                brutal_search::browser::DisplayCommand::Rect {
                    x,
                    y,
                    width,
                    height,
                    shade,
                } => {
                    println!("rect x={x} y={y} width={width} height={height} shade={shade}");
                }
                brutal_search::browser::DisplayCommand::Image {
                    x,
                    y,
                    width,
                    height,
                    shade,
                    alt,
                    url,
                    decoded_width,
                    decoded_height,
                    decoded_hash,
                    ..
                } => {
                    println!(
                        "image x={x} y={y} width={width} height={height} shade={shade} alt={} url={} decoded_width={} decoded_height={} decoded={}",
                        alt.as_deref().unwrap_or(""),
                        url.as_deref().unwrap_or(""),
                        decoded_width
                            .map(|value| value.to_string())
                            .unwrap_or_default(),
                        decoded_height
                            .map(|value| value.to_string())
                            .unwrap_or_default(),
                        decoded_hash.as_deref().unwrap_or("")
                    );
                }
            }
        }
    } else {
        if !render.title.is_empty() {
            println!("# {}", render.title);
            println!();
        }
        println!("{}", render.text);
    }
    Ok(())
}

fn print_raster(
    render: &BrowserRender,
    options: BrowserRasterOptions,
    output: Option<&PathBuf>,
    json: bool,
) -> Result<()> {
    let raster = rasterize_render(render, options)?;
    if let Some(path) = output {
        std::fs::write(path, raster.encode_pgm())?;
    }
    let report = raster_report(render, &raster, options);
    if json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        println!(
            "raster: {}x{} cells={}x{} commands={} ink={} hash={}",
            report.width,
            report.height,
            report.cell_width,
            report.cell_height,
            report.display_command_count,
            report.non_background_pixels,
            report.pixel_hash
        );
        if let Some(path) = output {
            println!("wrote: {}", path.display());
        }
    }
    Ok(())
}

fn print_screenshot(
    render: &BrowserRender,
    options: BrowserRasterOptions,
    output: Option<&PathBuf>,
    json: bool,
) -> Result<()> {
    let raster = rasterize_render_rgba(render, options)?;
    if let Some(path) = output {
        std::fs::write(path, raster.encode_png()?)?;
    }
    let report = rgba_raster_report(render, &raster, options);
    if json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        println!(
            "screenshot: {}x{} cells={}x{} commands={} ink={} format={} hash={}",
            report.width,
            report.height,
            report.cell_width,
            report.cell_height,
            report.display_command_count,
            report.non_background_pixels,
            report.artifact_format,
            report.pixel_hash
        );
        if let Some(path) = output {
            println!("wrote: {}", path.display());
        }
    }
    Ok(())
}

#[cfg(test)]
mod brutal_browser_tests;
