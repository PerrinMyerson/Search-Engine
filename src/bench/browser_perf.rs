use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use url::Url;

use crate::browser::{
    BrowserFixtureManifest, BrowserRasterOptions, BrowserRenderTimings,
    browser_fixture_raster_options, browser_layer_metrics, raster_report, rasterize_render,
    render_browser_fixture, render_browser_fixture_profiled,
};

use super::{chrome_program, chrome_version, command_output, hardware_summary, percentile};

#[derive(Debug, Clone)]
pub struct BrowserPerfOptions {
    pub manifest: PathBuf,
    pub iterations: usize,
    pub warmup: usize,
    pub chromium_baseline: bool,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct BrowserPerfGate {
    pub max_p95_us: Option<u128>,
    pub min_throughput_pages_per_sec: Option<f64>,
    pub min_chromium_p95_speedup: Option<f64>,
    pub max_chromium_text_mismatches: Option<usize>,
    pub max_layer_metrics_p95_us: Option<u128>,
    pub min_total_layers: Option<usize>,
    pub min_total_image_layers: Option<usize>,
    pub max_layer_count: Option<usize>,
    pub max_image_layer_count: Option<usize>,
}

impl BrowserPerfGate {
    pub fn is_empty(&self) -> bool {
        self.max_p95_us.is_none()
            && self.min_throughput_pages_per_sec.is_none()
            && self.min_chromium_p95_speedup.is_none()
            && self.max_chromium_text_mismatches.is_none()
            && self.max_layer_metrics_p95_us.is_none()
            && self.min_total_layers.is_none()
            && self.min_total_image_layers.is_none()
            && self.max_layer_count.is_none()
            && self.max_image_layer_count.is_none()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrowserPerfReport {
    pub engine: String,
    pub manifest: String,
    pub fixture_count: usize,
    pub iteration_count: usize,
    pub warmup: usize,
    pub sample_count: usize,
    #[serde(rename = "end_to_end_p50_us", alias = "p50_us")]
    pub p50_us: u128,
    #[serde(rename = "end_to_end_p95_us", alias = "p95_us")]
    pub p95_us: u128,
    #[serde(rename = "end_to_end_p99_us", alias = "p99_us")]
    pub p99_us: u128,
    #[serde(default)]
    pub raster_p50_us: u128,
    #[serde(default)]
    pub raster_p95_us: u128,
    #[serde(default)]
    pub raster_p99_us: u128,
    #[serde(default)]
    pub layer_metrics_p50_us: u128,
    #[serde(default)]
    pub layer_metrics_p95_us: u128,
    #[serde(default)]
    pub layer_metrics_p99_us: u128,
    pub throughput_pages_per_sec: f64,
    pub total_ms: u128,
    pub total_rendered_bytes: usize,
    pub total_dom_nodes: usize,
    pub total_css_rules: usize,
    pub total_layout_boxes: usize,
    pub total_paint_commands: usize,
    #[serde(default)]
    pub total_layers: usize,
    #[serde(default)]
    pub total_image_layers: usize,
    #[serde(default)]
    pub max_layer_count: usize,
    #[serde(default)]
    pub max_image_layer_count: usize,
    #[serde(default)]
    pub max_root_layer_width: usize,
    #[serde(default)]
    pub max_root_layer_height: usize,
    #[serde(default)]
    pub max_layer_area: usize,
    #[serde(default)]
    pub total_layer_area: usize,
    #[serde(default)]
    pub total_layer_metrics_us: u128,
    #[serde(default)]
    pub total_raster_us: u128,
    #[serde(default)]
    pub total_raster_pixels: usize,
    #[serde(default)]
    pub total_raster_non_background_pixels: usize,
    #[serde(default)]
    pub total_raster_visible_commands: usize,
    #[serde(default)]
    pub total_raster_culled_commands: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chromium_baseline: Option<BrowserPerfChromiumBaselineReport>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chromium_p95_speedup: Option<f64>,
    #[serde(rename = "render_phase_totals", alias = "phase_totals")]
    pub phase_totals: BrowserRenderTimings,
    pub suite_hash: String,
    pub rustc: Option<String>,
    pub chrome: Option<String>,
    pub os: Option<String>,
    pub hardware: Option<String>,
    pub required_max_p95_us: Option<u128>,
    pub required_min_throughput_pages_per_sec: Option<f64>,
    #[serde(default)]
    pub required_min_chromium_p95_speedup: Option<f64>,
    #[serde(default)]
    pub required_max_chromium_text_mismatches: Option<usize>,
    #[serde(default)]
    pub required_max_layer_metrics_p95_us: Option<u128>,
    #[serde(default)]
    pub required_min_total_layers: Option<usize>,
    #[serde(default)]
    pub required_min_total_image_layers: Option<usize>,
    #[serde(default)]
    pub required_max_layer_count: Option<usize>,
    #[serde(default)]
    pub required_max_image_layer_count: Option<usize>,
    pub passed: Option<bool>,
    pub fixtures: Vec<BrowserPerfFixtureReport>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrowserPerfChromiumBaselineReport {
    pub engine: String,
    pub chrome: Option<String>,
    pub sample_count: usize,
    pub text_match_count: usize,
    pub text_mismatch_count: usize,
    pub p50_us: u128,
    pub p95_us: u128,
    pub p99_us: u128,
    pub throughput_pages_per_sec: f64,
    pub total_ms: u128,
    pub fixtures: Vec<BrowserPerfChromiumFixtureReport>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrowserPerfChromiumFixtureReport {
    pub name: String,
    pub path: String,
    pub sample_count: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text_match: Option<bool>,
    pub text_hash: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_text_hash: Option<String>,
    pub p50_us: u128,
    pub p95_us: u128,
    pub p99_us: u128,
    pub rendered_bytes: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrowserPerfFixtureReport {
    pub name: String,
    pub path: String,
    pub sample_count: usize,
    #[serde(rename = "end_to_end_p50_us", alias = "p50_us")]
    pub p50_us: u128,
    #[serde(rename = "end_to_end_p95_us", alias = "p95_us")]
    pub p95_us: u128,
    #[serde(rename = "end_to_end_p99_us", alias = "p99_us")]
    pub p99_us: u128,
    #[serde(default)]
    pub raster_p50_us: u128,
    #[serde(default)]
    pub raster_p95_us: u128,
    #[serde(default)]
    pub raster_p99_us: u128,
    #[serde(default)]
    pub raster_total_us: u128,
    #[serde(default)]
    pub layer_metrics_p50_us: u128,
    #[serde(default)]
    pub layer_metrics_p95_us: u128,
    #[serde(default)]
    pub layer_metrics_p99_us: u128,
    #[serde(default)]
    pub layer_metrics_total_us: u128,
    pub rendered_bytes: usize,
    pub dom_node_count: usize,
    pub css_rule_count: usize,
    pub layout_box_count: usize,
    pub paint_command_count: usize,
    #[serde(default)]
    pub layer_count: usize,
    #[serde(default)]
    pub image_layer_count: usize,
    #[serde(default)]
    pub root_layer_width: usize,
    #[serde(default)]
    pub root_layer_height: usize,
    #[serde(default)]
    pub max_layer_area: usize,
    #[serde(default)]
    pub total_layer_area: usize,
    #[serde(default)]
    pub raster_width: usize,
    #[serde(default)]
    pub raster_height: usize,
    #[serde(default)]
    pub raster_pixels: usize,
    #[serde(default)]
    pub raster_non_background_pixels: usize,
    #[serde(default)]
    pub raster_visible_command_count: usize,
    #[serde(default)]
    pub raster_culled_command_count: usize,
    #[serde(rename = "render_phase_totals", alias = "phase_totals")]
    pub phase_totals: BrowserRenderTimings,
}

impl BrowserPerfReport {
    pub fn apply_gate(&mut self, gate: BrowserPerfGate) -> bool {
        let passed = gate.max_p95_us.is_none_or(|max| self.p95_us <= max)
            && gate
                .min_throughput_pages_per_sec
                .is_none_or(|min| self.throughput_pages_per_sec >= min)
            && gate.min_chromium_p95_speedup.is_none_or(|min| {
                self.chromium_p95_speedup
                    .is_some_and(|speedup| speedup >= min)
            })
            && gate.max_chromium_text_mismatches.is_none_or(|max| {
                self.chromium_baseline
                    .as_ref()
                    .is_some_and(|baseline| baseline.text_mismatch_count <= max)
            })
            && gate
                .max_layer_metrics_p95_us
                .is_none_or(|max| self.layer_metrics_p95_us <= max)
            && gate
                .min_total_layers
                .is_none_or(|min| self.total_layers >= min)
            && gate
                .min_total_image_layers
                .is_none_or(|min| self.total_image_layers >= min)
            && gate
                .max_layer_count
                .is_none_or(|max| self.max_layer_count <= max)
            && gate
                .max_image_layer_count
                .is_none_or(|max| self.max_image_layer_count <= max);
        self.required_max_p95_us = gate.max_p95_us;
        self.required_min_throughput_pages_per_sec = gate.min_throughput_pages_per_sec;
        self.required_min_chromium_p95_speedup = gate.min_chromium_p95_speedup;
        self.required_max_chromium_text_mismatches = gate.max_chromium_text_mismatches;
        self.required_max_layer_metrics_p95_us = gate.max_layer_metrics_p95_us;
        self.required_min_total_layers = gate.min_total_layers;
        self.required_min_total_image_layers = gate.min_total_image_layers;
        self.required_max_layer_count = gate.max_layer_count;
        self.required_max_image_layer_count = gate.max_image_layer_count;
        self.passed = Some(passed);
        passed
    }
}

#[derive(Debug, Clone)]
struct PreparedBrowserFixture {
    name: String,
    path: PathBuf,
    fixture: crate::browser::BrowserFixture,
    bytes: Vec<u8>,
}

#[derive(Debug, Default)]
struct BrowserFixtureTiming {
    timings: Vec<Duration>,
    raster_timings: Vec<Duration>,
    layer_metrics_timings: Vec<Duration>,
    rendered_bytes: usize,
    dom_node_count: usize,
    css_rule_count: usize,
    layout_box_count: usize,
    paint_command_count: usize,
    layer_count: usize,
    image_layer_count: usize,
    root_layer_width: usize,
    root_layer_height: usize,
    max_layer_area: usize,
    total_layer_area: usize,
    raster_width: usize,
    raster_height: usize,
    raster_pixels: usize,
    raster_non_background_pixels: usize,
    raster_visible_command_count: usize,
    raster_culled_command_count: usize,
    phase_totals: BrowserRenderTimings,
}

pub fn run_browser_perf(options: BrowserPerfOptions) -> Result<BrowserPerfReport> {
    anyhow::ensure!(
        options.iterations > 0,
        "browser-perf --iterations must be greater than zero"
    );
    let fixtures = load_browser_perf_fixtures(&options.manifest)?;
    anyhow::ensure!(
        !fixtures.is_empty(),
        "browser fixture manifest has no fixtures: {}",
        options.manifest.display()
    );

    for _ in 0..options.warmup {
        for fixture in &fixtures {
            let render = render_browser_fixture(&fixture.path, &fixture.bytes, &fixture.fixture)?;
            let _ = browser_layer_metrics(&render);
            let raster_options =
                browser_fixture_raster_options(&fixture.fixture, BrowserRasterOptions::default());
            let _ = rasterize_render(&render, raster_options)?;
        }
    }

    let mut global_timings = Vec::with_capacity(options.iterations * fixtures.len());
    let mut global_raster_timings = Vec::with_capacity(options.iterations * fixtures.len());
    let mut global_layer_metrics_timings = Vec::with_capacity(options.iterations * fixtures.len());
    let mut fixture_timings = fixtures
        .iter()
        .map(|_| BrowserFixtureTiming {
            timings: Vec::with_capacity(options.iterations),
            raster_timings: Vec::with_capacity(options.iterations),
            layer_metrics_timings: Vec::with_capacity(options.iterations),
            ..BrowserFixtureTiming::default()
        })
        .collect::<Vec<_>>();

    let started = Instant::now();
    for _ in 0..options.iterations {
        for (index, fixture) in fixtures.iter().enumerate() {
            let t0 = Instant::now();
            let profiled =
                render_browser_fixture_profiled(&fixture.path, &fixture.bytes, &fixture.fixture)?;
            let render = profiled.render;
            let raster_options =
                browser_fixture_raster_options(&fixture.fixture, BrowserRasterOptions::default());
            let raster_start = Instant::now();
            let raster = rasterize_render(&render, raster_options)?;
            let raster_elapsed = raster_start.elapsed();
            let raster_summary = raster_report(&render, &raster, raster_options);
            let elapsed = t0.elapsed();
            let layer_metrics_start = Instant::now();
            let layer_metrics = browser_layer_metrics(&render);
            let layer_metrics_elapsed = layer_metrics_start.elapsed();
            global_timings.push(elapsed);
            global_raster_timings.push(raster_elapsed);
            global_layer_metrics_timings.push(layer_metrics_elapsed);

            let timing = &mut fixture_timings[index];
            timing.timings.push(elapsed);
            timing.raster_timings.push(raster_elapsed);
            timing.layer_metrics_timings.push(layer_metrics_elapsed);
            timing.rendered_bytes = render.text.len();
            timing.dom_node_count = render.dom_node_count;
            timing.css_rule_count = render.css_rule_count;
            timing.layout_box_count = render.layout_box_count;
            timing.paint_command_count = render.paint_command_count;
            timing.layer_count = layer_metrics.layer_count;
            timing.image_layer_count = layer_metrics.image_layer_count;
            timing.root_layer_width = layer_metrics.root_layer_width;
            timing.root_layer_height = layer_metrics.root_layer_height;
            timing.max_layer_area = layer_metrics.max_layer_area;
            timing.total_layer_area = layer_metrics.total_layer_area;
            timing.raster_width = raster.width;
            timing.raster_height = raster.height;
            timing.raster_pixels = raster.pixels.len();
            timing.raster_non_background_pixels = raster.non_background_pixels();
            timing.raster_visible_command_count = raster_summary.visible_command_count;
            timing.raster_culled_command_count = raster_summary.culled_command_count;
            timing.phase_totals = timing.phase_totals.add(profiled.timings);
        }
    }
    let elapsed = started.elapsed();

    global_timings.sort_unstable();
    global_raster_timings.sort_unstable();
    global_layer_metrics_timings.sort_unstable();
    let sample_count = global_timings.len();
    let throughput_pages_per_sec = sample_count as f64 / elapsed.as_secs_f64().max(f64::EPSILON);

    let mut fixture_reports = Vec::with_capacity(fixtures.len());
    for (fixture, mut timing) in fixtures.iter().zip(fixture_timings) {
        timing.timings.sort_unstable();
        timing.raster_timings.sort_unstable();
        timing.layer_metrics_timings.sort_unstable();
        fixture_reports.push(BrowserPerfFixtureReport {
            name: fixture.name.clone(),
            path: fixture.path.display().to_string(),
            sample_count: timing.timings.len(),
            p50_us: percentile(&timing.timings, 0.50).as_micros(),
            p95_us: percentile(&timing.timings, 0.95).as_micros(),
            p99_us: percentile(&timing.timings, 0.99).as_micros(),
            raster_p50_us: percentile(&timing.raster_timings, 0.50).as_micros(),
            raster_p95_us: percentile(&timing.raster_timings, 0.95).as_micros(),
            raster_p99_us: percentile(&timing.raster_timings, 0.99).as_micros(),
            raster_total_us: timing
                .raster_timings
                .iter()
                .map(|duration| duration.as_micros())
                .sum(),
            layer_metrics_p50_us: percentile(&timing.layer_metrics_timings, 0.50).as_micros(),
            layer_metrics_p95_us: percentile(&timing.layer_metrics_timings, 0.95).as_micros(),
            layer_metrics_p99_us: percentile(&timing.layer_metrics_timings, 0.99).as_micros(),
            layer_metrics_total_us: timing
                .layer_metrics_timings
                .iter()
                .map(|duration| duration.as_micros())
                .sum(),
            rendered_bytes: timing.rendered_bytes,
            dom_node_count: timing.dom_node_count,
            css_rule_count: timing.css_rule_count,
            layout_box_count: timing.layout_box_count,
            paint_command_count: timing.paint_command_count,
            layer_count: timing.layer_count,
            image_layer_count: timing.image_layer_count,
            root_layer_width: timing.root_layer_width,
            root_layer_height: timing.root_layer_height,
            max_layer_area: timing.max_layer_area,
            total_layer_area: timing.total_layer_area,
            raster_width: timing.raster_width,
            raster_height: timing.raster_height,
            raster_pixels: timing.raster_pixels,
            raster_non_background_pixels: timing.raster_non_background_pixels,
            raster_visible_command_count: timing.raster_visible_command_count,
            raster_culled_command_count: timing.raster_culled_command_count,
            phase_totals: timing.phase_totals,
        });
    }

    let phase_totals = fixture_reports
        .iter()
        .fold(BrowserRenderTimings::default(), |total, fixture| {
            total.add(fixture.phase_totals)
        });

    let chromium_baseline = if options.chromium_baseline {
        Some(run_browser_perf_chromium_baseline(
            &fixtures,
            options.iterations,
            options.warmup,
        )?)
    } else {
        None
    };
    let chromium_p95_speedup = chromium_baseline.as_ref().map(|baseline| {
        baseline.p95_us as f64 / percentile(&global_timings, 0.95).as_micros().max(1) as f64
    });

    Ok(BrowserPerfReport {
        engine: "brutal-browser-fixture-render".to_owned(),
        manifest: options.manifest.display().to_string(),
        fixture_count: fixtures.len(),
        iteration_count: options.iterations,
        warmup: options.warmup,
        sample_count,
        p50_us: percentile(&global_timings, 0.50).as_micros(),
        p95_us: percentile(&global_timings, 0.95).as_micros(),
        p99_us: percentile(&global_timings, 0.99).as_micros(),
        raster_p50_us: percentile(&global_raster_timings, 0.50).as_micros(),
        raster_p95_us: percentile(&global_raster_timings, 0.95).as_micros(),
        raster_p99_us: percentile(&global_raster_timings, 0.99).as_micros(),
        layer_metrics_p50_us: percentile(&global_layer_metrics_timings, 0.50).as_micros(),
        layer_metrics_p95_us: percentile(&global_layer_metrics_timings, 0.95).as_micros(),
        layer_metrics_p99_us: percentile(&global_layer_metrics_timings, 0.99).as_micros(),
        throughput_pages_per_sec,
        total_ms: elapsed.as_millis(),
        total_rendered_bytes: fixture_reports
            .iter()
            .map(|fixture| fixture.rendered_bytes)
            .sum(),
        total_dom_nodes: fixture_reports
            .iter()
            .map(|fixture| fixture.dom_node_count)
            .sum(),
        total_css_rules: fixture_reports
            .iter()
            .map(|fixture| fixture.css_rule_count)
            .sum(),
        total_layout_boxes: fixture_reports
            .iter()
            .map(|fixture| fixture.layout_box_count)
            .sum(),
        total_paint_commands: fixture_reports
            .iter()
            .map(|fixture| fixture.paint_command_count)
            .sum(),
        total_layers: fixture_reports
            .iter()
            .map(|fixture| fixture.layer_count)
            .sum(),
        total_image_layers: fixture_reports
            .iter()
            .map(|fixture| fixture.image_layer_count)
            .sum(),
        max_layer_count: fixture_reports
            .iter()
            .map(|fixture| fixture.layer_count)
            .max()
            .unwrap_or_default(),
        max_image_layer_count: fixture_reports
            .iter()
            .map(|fixture| fixture.image_layer_count)
            .max()
            .unwrap_or_default(),
        max_root_layer_width: fixture_reports
            .iter()
            .map(|fixture| fixture.root_layer_width)
            .max()
            .unwrap_or_default(),
        max_root_layer_height: fixture_reports
            .iter()
            .map(|fixture| fixture.root_layer_height)
            .max()
            .unwrap_or_default(),
        max_layer_area: fixture_reports
            .iter()
            .map(|fixture| fixture.max_layer_area)
            .max()
            .unwrap_or_default(),
        total_layer_area: fixture_reports
            .iter()
            .map(|fixture| fixture.total_layer_area)
            .sum(),
        total_layer_metrics_us: fixture_reports
            .iter()
            .map(|fixture| fixture.layer_metrics_total_us)
            .sum(),
        total_raster_us: fixture_reports
            .iter()
            .map(|fixture| fixture.raster_total_us)
            .sum(),
        total_raster_pixels: fixture_reports
            .iter()
            .map(|fixture| fixture.raster_pixels)
            .sum(),
        total_raster_non_background_pixels: fixture_reports
            .iter()
            .map(|fixture| fixture.raster_non_background_pixels)
            .sum(),
        total_raster_visible_commands: fixture_reports
            .iter()
            .map(|fixture| fixture.raster_visible_command_count)
            .sum(),
        total_raster_culled_commands: fixture_reports
            .iter()
            .map(|fixture| fixture.raster_culled_command_count)
            .sum(),
        chromium_baseline,
        chromium_p95_speedup,
        phase_totals,
        suite_hash: browser_suite_hash(&options.manifest).unwrap_or_else(|_| "unknown".to_owned()),
        rustc: command_output("rustc", &["--version"]),
        chrome: chrome_version(),
        os: command_output("uname", &["-a"]),
        hardware: hardware_summary(),
        required_max_p95_us: None,
        required_min_throughput_pages_per_sec: None,
        required_min_chromium_p95_speedup: None,
        required_max_chromium_text_mismatches: None,
        required_max_layer_metrics_p95_us: None,
        required_min_total_layers: None,
        required_min_total_image_layers: None,
        required_max_layer_count: None,
        required_max_image_layer_count: None,
        passed: None,
        fixtures: fixture_reports,
    })
}

fn load_browser_perf_fixtures(manifest_path: &Path) -> Result<Vec<PreparedBrowserFixture>> {
    let manifest_text = fs::read_to_string(manifest_path)
        .with_context(|| format!("read browser fixture manifest {}", manifest_path.display()))?;
    let manifest: BrowserFixtureManifest = serde_json::from_str(&manifest_text)
        .with_context(|| format!("parse browser fixture manifest {}", manifest_path.display()))?;
    let base_dir = manifest_path.parent().unwrap_or_else(|| Path::new("."));
    manifest
        .fixtures
        .into_iter()
        .map(|fixture| {
            let path = if fixture.path.is_absolute() {
                fixture.path.clone()
            } else {
                base_dir.join(&fixture.path)
            };
            let name = fixture
                .name
                .clone()
                .unwrap_or_else(|| fixture.path.display().to_string());
            let bytes = fs::read(&path)
                .with_context(|| format!("read browser perf fixture {}", path.display()))?;
            Ok(PreparedBrowserFixture {
                name,
                path,
                fixture,
                bytes,
            })
        })
        .collect()
}

#[derive(Debug, Serialize)]
struct ChromiumBrowserPerfFixtureInput {
    name: String,
    path: String,
    width: usize,
    base_href: Option<String>,
    click_selector: Option<String>,
    html: String,
}

#[derive(Debug, Deserialize)]
struct ChromiumBrowserPerfDump {
    samples: Vec<ChromiumBrowserPerfSample>,
}

#[derive(Debug, Deserialize)]
struct ChromiumBrowserPerfSample {
    fixture_index: usize,
    elapsed_us: u64,
    rendered_bytes: usize,
    text: String,
}

#[derive(Debug, Default)]
struct ChromiumFixtureTiming {
    timings: Vec<Duration>,
    rendered_bytes: usize,
    text: String,
}

fn run_browser_perf_chromium_baseline(
    fixtures: &[PreparedBrowserFixture],
    iterations: usize,
    warmup: usize,
) -> Result<BrowserPerfChromiumBaselineReport> {
    let chrome = chrome_program().context("Chrome/Chromium executable not found")?;
    let inputs = fixtures
        .iter()
        .map(|fixture| {
            Ok(ChromiumBrowserPerfFixtureInput {
                name: fixture.name.clone(),
                path: fixture.path.display().to_string(),
                width: fixture.fixture.width,
                base_href: chromium_fixture_base_href(&fixture.path),
                click_selector: fixture.fixture.click_selector.clone(),
                html: chromium_fixture_html(fixture)?,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let wrapper = chromium_browser_perf_wrapper_html(&inputs, iterations, warmup)?;
    let path = std::env::temp_dir().join(format!(
        "brutal-browser-perf-chromium-{}.html",
        std::process::id()
    ));
    fs::write(&path, wrapper).with_context(|| format!("write {}", path.display()))?;

    let started = Instant::now();
    let output = Command::new(&chrome)
        .arg("--headless=new")
        .arg("--disable-gpu")
        .arg("--disable-background-networking")
        .arg("--disable-default-apps")
        .arg("--disable-extensions")
        .arg("--allow-file-access-from-files")
        .arg("--run-all-compositor-stages-before-draw")
        .arg("--virtual-time-budget=30000")
        .arg("--dump-dom")
        .arg(format!("file://{}", path.display()))
        .output()
        .with_context(|| format!("run Chromium browser perf {}", path.display()))?;
    let elapsed = started.elapsed();
    let _ = fs::remove_file(&path);

    let stdout = String::from_utf8_lossy(&output.stdout);
    let Some(json) = extract_chromium_browser_perf_json(&stdout) else {
        anyhow::ensure!(
            output.status.success(),
            "Chromium browser perf failed with status {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        );
        anyhow::bail!("Chromium browser perf output did not contain JSON");
    };
    let dump: ChromiumBrowserPerfDump = serde_json::from_str(json)?;
    chromium_browser_perf_report(fixtures, dump.samples, elapsed)
}

fn chromium_browser_perf_report(
    fixtures: &[PreparedBrowserFixture],
    samples: Vec<ChromiumBrowserPerfSample>,
    elapsed: Duration,
) -> Result<BrowserPerfChromiumBaselineReport> {
    let mut global_timings = Vec::with_capacity(samples.len());
    let mut fixture_timings = fixtures
        .iter()
        .map(|_| ChromiumFixtureTiming::default())
        .collect::<Vec<_>>();

    for sample in samples {
        let Some(timing) = fixture_timings.get_mut(sample.fixture_index) else {
            anyhow::bail!(
                "Chromium returned invalid fixture index {}",
                sample.fixture_index
            );
        };
        let duration = Duration::from_micros(sample.elapsed_us);
        timing.timings.push(duration);
        timing.rendered_bytes = sample.rendered_bytes;
        timing.text = sample.text;
        global_timings.push(duration);
    }
    global_timings.sort_unstable();
    for timing in &mut fixture_timings {
        timing.timings.sort_unstable();
    }

    let sample_count = global_timings.len();
    let throughput_pages_per_sec = sample_count as f64 / elapsed.as_secs_f64().max(f64::EPSILON);
    let fixture_reports = fixtures
        .iter()
        .zip(fixture_timings)
        .map(|(fixture, timing)| {
            let text = normalize_browser_perf_text(&timing.text);
            let expected_text = fixture
                .fixture
                .expected_text
                .as_deref()
                .map(normalize_browser_perf_text);
            let text_match = expected_text.as_ref().map(|expected| *expected == text);
            BrowserPerfChromiumFixtureReport {
                name: fixture.name.clone(),
                path: fixture.path.display().to_string(),
                sample_count: timing.timings.len(),
                text_match,
                text_hash: hash_browser_perf_text(&text),
                expected_text_hash: expected_text
                    .as_ref()
                    .map(|expected| hash_browser_perf_text(expected)),
                p50_us: percentile(&timing.timings, 0.50).as_micros(),
                p95_us: percentile(&timing.timings, 0.95).as_micros(),
                p99_us: percentile(&timing.timings, 0.99).as_micros(),
                rendered_bytes: timing.rendered_bytes,
            }
        })
        .collect::<Vec<_>>();
    let text_match_count = fixture_reports
        .iter()
        .filter(|fixture| fixture.text_match == Some(true))
        .count();
    let text_mismatch_count = fixture_reports
        .iter()
        .filter(|fixture| fixture.text_match == Some(false))
        .count();

    Ok(BrowserPerfChromiumBaselineReport {
        engine: "headless-chromium-iframe-render".to_owned(),
        chrome: chrome_version(),
        sample_count,
        text_match_count,
        text_mismatch_count,
        p50_us: percentile(&global_timings, 0.50).as_micros(),
        p95_us: percentile(&global_timings, 0.95).as_micros(),
        p99_us: percentile(&global_timings, 0.99).as_micros(),
        throughput_pages_per_sec,
        total_ms: elapsed.as_millis(),
        fixtures: fixture_reports,
    })
}

fn chromium_fixture_base_href(fixture_path: &Path) -> Option<String> {
    let parent = fixture_path.parent()?;
    let parent = fs::canonicalize(parent).ok()?;
    Url::from_directory_path(parent)
        .ok()
        .map(|url| url.to_string())
}

fn chromium_fixture_html(fixture: &PreparedBrowserFixture) -> Result<String> {
    let html = String::from_utf8_lossy(&fixture.bytes).into_owned();
    if fixture.fixture.external_scripts {
        return inline_local_external_scripts_for_chromium(&html, &fixture.path);
    }
    Ok(html)
}

fn inline_local_external_scripts_for_chromium(html: &str, fixture_path: &Path) -> Result<String> {
    let lower = html.to_ascii_lowercase();
    let mut output = String::with_capacity(html.len());
    let mut cursor = 0usize;

    while let Some(relative_start) = lower[cursor..].find("<script") {
        let script_start = cursor + relative_start;
        let Some(relative_open_end) = lower[script_start..].find('>') else {
            break;
        };
        let open_end = script_start + relative_open_end + 1;
        let opening_tag = &html[script_start..open_end];
        let Some(src) = script_src_attribute(opening_tag) else {
            output.push_str(&html[cursor..open_end]);
            cursor = open_end;
            continue;
        };
        let Some(relative_close_start) = lower[open_end..].find("</script>") else {
            output.push_str(&html[cursor..open_end]);
            cursor = open_end;
            continue;
        };
        let close_start = open_end + relative_close_start;
        let close_end = close_start + "</script>".len();

        output.push_str(&html[cursor..script_start]);
        output.push_str(&inline_chromium_script_tag(fixture_path, &src)?);
        cursor = close_end;
    }

    output.push_str(&html[cursor..]);
    Ok(output)
}

fn script_src_attribute(opening_tag: &str) -> Option<String> {
    let bytes = opening_tag.as_bytes();
    let mut i = opening_tag
        .to_ascii_lowercase()
        .find("script")?
        .saturating_add("script".len());

    while i < bytes.len() {
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= bytes.len() || matches!(bytes[i], b'>' | b'/') {
            break;
        }
        let name_start = i;
        while i < bytes.len()
            && (bytes[i].is_ascii_alphanumeric() || matches!(bytes[i], b':' | b'-' | b'_'))
        {
            i += 1;
        }
        if name_start == i {
            i += 1;
            continue;
        }
        let name = &opening_tag[name_start..i];
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        let value = if i < bytes.len() && bytes[i] == b'=' {
            i += 1;
            while i < bytes.len() && bytes[i].is_ascii_whitespace() {
                i += 1;
            }
            if i < bytes.len() && matches!(bytes[i], b'\'' | b'"') {
                let quote = bytes[i];
                i += 1;
                let value_start = i;
                while i < bytes.len() && bytes[i] != quote {
                    i += 1;
                }
                let value = opening_tag[value_start..i].to_owned();
                i += usize::from(i < bytes.len());
                value
            } else {
                let value_start = i;
                while i < bytes.len() && !bytes[i].is_ascii_whitespace() && bytes[i] != b'>' {
                    i += 1;
                }
                opening_tag[value_start..i].to_owned()
            }
        } else {
            String::new()
        };
        if name.eq_ignore_ascii_case("src") {
            return Some(html_escape::decode_html_entities(&value).into_owned());
        }
    }
    None
}

fn inline_chromium_script_tag(fixture_path: &Path, src: &str) -> Result<String> {
    let script_path = resolve_local_chromium_script_path(fixture_path, src)?;
    let script = fs::read_to_string(&script_path).with_context(|| {
        format!(
            "read Chromium perf fixture script {}",
            script_path.display()
        )
    })?;
    Ok(format!(
        r#"<script data-brutal-inlined-src="{}">
{}
</script>"#,
        html_escape::encode_double_quoted_attribute(src),
        script.replace("</script", "<\\/script")
    ))
}

fn resolve_local_chromium_script_path(fixture_path: &Path, src: &str) -> Result<PathBuf> {
    if let Ok(url) = Url::parse(src) {
        anyhow::ensure!(
            url.scheme() == "file",
            "Chromium perf can inline only local fixture scripts, got {src}"
        );
        return url.to_file_path().map_err(|_| {
            anyhow::anyhow!("script file URL cannot be converted to a local path: {src}")
        });
    }

    let src_without_url_parts = src
        .split(['?', '#'])
        .next()
        .filter(|path| !path.is_empty())
        .unwrap_or(src);
    let base = fixture_path.parent().unwrap_or_else(|| Path::new("."));
    Ok(base.join(src_without_url_parts))
}

fn chromium_browser_perf_wrapper_html(
    fixtures: &[ChromiumBrowserPerfFixtureInput],
    iterations: usize,
    warmup: usize,
) -> Result<String> {
    let fixtures_json = serde_json::to_string(fixtures)?.replace("</", "<\\/");
    Ok(format!(
        r#"<!doctype html>
<html>
<head><meta charset="utf-8"><title>Brutal Browser Chromium Perf</title></head>
<body>
<script>
const FIXTURES = {fixtures_json};
const ITERATIONS = {iterations};
const WARMUP = {warmup};
function escapeAttribute(value) {{
  return String(value)
    .replaceAll("&", "&amp;")
    .replaceAll("\"", "&quot;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;");
}}
function renderFixture(fixture, done) {{
  const frame = document.createElement("iframe");
  frame.style.position = "absolute";
  frame.style.left = "-100000px";
  frame.style.top = "0";
  frame.style.width = `${{Math.max(20, fixture.width) * 8}}px`;
  frame.style.height = "2000px";
  const base = fixture.base_href ? `<base href="${{escapeAttribute(fixture.base_href)}}">` : "";
  document.body.appendChild(frame);
  const started = performance.now();
  const doc = frame.contentWindow.document;
  doc.open();
  doc.write(base + fixture.html);
  doc.close();
  setTimeout(() => {{
    if (fixture.click_selector && frame.contentDocument) {{
      const target = frame.contentDocument.querySelector(fixture.click_selector);
      if (target) target.click();
    }}
    setTimeout(() => {{
      const body = frame.contentDocument ? frame.contentDocument.body : null;
      const text = body ? body.innerText : "";
      if (body) void body.offsetHeight;
      const elapsedUs = Math.max(0, Math.round((performance.now() - started) * 1000));
      frame.remove();
      done({{ elapsed_us: elapsedUs, rendered_bytes: text.length, text }});
    }}, 0);
  }}, 0);
}}
(function() {{
  const samples = [];
  let round = 0;
  let fixtureIndex = 0;
  function finish() {{
    const out = document.createElement("script");
    out.type = "application/json";
    out.id = "brutal-browser-perf-result";
    out.textContent = JSON.stringify({{ samples }}).replaceAll("</", "<\\/");
    document.documentElement.appendChild(out);
  }}
  function step() {{
    if (round >= WARMUP + ITERATIONS) {{
      finish();
      return;
    }}
    const currentRound = round;
    const currentFixtureIndex = fixtureIndex;
    renderFixture(FIXTURES[currentFixtureIndex], (sample) => {{
      if (currentRound >= WARMUP) {{
        samples.push({{
          fixture_index: currentFixtureIndex,
          elapsed_us: sample.elapsed_us,
          rendered_bytes: sample.rendered_bytes,
          text: sample.text
        }});
      }}
      fixtureIndex += 1;
      if (fixtureIndex >= FIXTURES.length) {{
        fixtureIndex = 0;
        round += 1;
      }}
      setTimeout(step, 0);
    }});
  }}
  setTimeout(step, 0);
}})();
</script>
</body>
</html>"#
    ))
}

fn normalize_browser_perf_text(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn hash_browser_perf_text(text: &str) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"brutal-browser-perf-text-v1");
    hasher.update(text.as_bytes());
    hasher.finalize().to_hex().to_string()
}

fn extract_chromium_browser_perf_json(dump: &str) -> Option<&str> {
    let marker = "id=\"brutal-browser-perf-result\"";
    let marker_index = dump.find(marker)?;
    let after_marker = &dump[marker_index..];
    let start = after_marker.find('>')? + marker_index + 1;
    let after_start = &dump[start..];
    let end = after_start.find("</script>")? + start;
    dump.get(start..end)
}

fn browser_suite_hash(manifest_path: &Path) -> Result<String> {
    let mut hasher = blake3::Hasher::new();
    let base_dir = manifest_path.parent().unwrap_or_else(|| Path::new("."));
    let mut files = fs::read_dir(base_dir)
        .with_context(|| format!("read browser suite directory {}", base_dir.display()))?
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| path.is_file())
        .filter(|path| browser_suite_hash_extension(path))
        .collect::<Vec<_>>();
    files.sort();
    for path in files {
        let file_name = path.file_name().unwrap_or_default().to_string_lossy();
        hasher.update(file_name.as_bytes());
        hasher.update(&[0]);
        hasher.update(&fs::read(&path).with_context(|| format!("read {}", path.display()))?);
        hasher.update(&[0]);
    }
    Ok(hasher.finalize().to_hex().to_string())
}

fn browser_suite_hash_extension(path: &Path) -> bool {
    matches!(
        path.extension()
            .and_then(|extension| extension.to_str())
            .map(str::to_ascii_lowercase)
            .as_deref(),
        Some("json" | "html" | "htm" | "xhtml" | "js" | "css")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn browser_fixture_with_expected_text(expected_text: &str) -> crate::browser::BrowserFixture {
        crate::browser::BrowserFixture {
            name: Some("fixture".to_owned()),
            path: PathBuf::from("page.html"),
            width: 80,
            external_scripts: false,
            click_selector: None,
            expected_title: None,
            expected_text: Some(expected_text.to_owned()),
            expected_display_list: None,
            expected_hit_tests: Vec::new(),
            expected_layers: None,
            raster_viewport_x: None,
            raster_viewport_y: None,
            raster_viewport_width: None,
            raster_viewport_height: None,
            expected_visible_command_count: None,
            expected_culled_command_count: None,
            expected_raster_hash: None,
            expected_screenshot_hash: None,
        }
    }

    #[test]
    fn chromium_perf_inlines_local_external_scripts() {
        let dir = tempfile::tempdir().unwrap();
        let page = dir.path().join("page.html");
        let script = dir.path().join("external.js");
        fs::write(&script, r#"document.body.textContent = "Loaded";"#).unwrap();

        let html = r#"<html><body><script src="external.js"></script></body></html>"#;
        let inlined = inline_local_external_scripts_for_chromium(html, &page).unwrap();

        assert!(!inlined.contains("<script src=\"external.js\""));
        assert!(inlined.contains("data-brutal-inlined-src=\"external.js\""));
        assert!(inlined.contains("document.body.textContent = \"Loaded\";"));
    }

    #[test]
    fn chromium_perf_report_counts_fixture_text_matches() {
        let fixture = PreparedBrowserFixture {
            name: "fixture".to_owned(),
            path: PathBuf::from("page.html"),
            fixture: browser_fixture_with_expected_text("After script"),
            bytes: Vec::new(),
        };

        let report = chromium_browser_perf_report(
            &[fixture],
            vec![ChromiumBrowserPerfSample {
                fixture_index: 0,
                elapsed_us: 10,
                rendered_bytes: "After script".len(),
                text: "After\nscript".to_owned(),
            }],
            Duration::from_millis(1),
        )
        .unwrap();

        assert_eq!(report.text_match_count, 1);
        assert_eq!(report.text_mismatch_count, 0);
        assert_eq!(report.fixtures[0].text_match, Some(true));
        assert_eq!(
            report.fixtures[0].text_hash,
            report.fixtures[0].expected_text_hash.clone().unwrap()
        );
    }
}
