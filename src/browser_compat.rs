use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};

use crate::browser::{
    BrowserFixture, BrowserFixtureHitTest, BrowserLayer, BrowserRasterOptions, BrowserRender,
    BrowserRenderOptions, DisplayCommand, hit_test_expectation_matches, hit_test_render,
    layer_tree_render, raster_report, rasterize_render, render_browser_fixture,
};

const STATUS_PASS: &str = "pass";
const STATUS_FAIL: &str = "fail";
const STATUS_TIMEOUT: &str = "timeout";
const STATUS_CRASH: &str = "crash";
const STATUS_UNSUPPORTED: &str = "unsupported";
const STATUS_SKIPPED: &str = "skipped";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrowserCompatManifest {
    #[serde(default = "default_suite_name", alias = "name")]
    pub suite: String,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub expectations: Option<PathBuf>,
    #[serde(default, alias = "fixtures")]
    pub tests: Vec<BrowserCompatTest>,
    #[serde(default)]
    pub metadata: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrowserCompatTest {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    pub path: PathBuf,
    #[serde(default = "default_subsystem")]
    pub subsystem: String,
    #[serde(default = "default_browser_compat_width")]
    pub width: usize,
    #[serde(default)]
    pub external_scripts: bool,
    #[serde(default)]
    pub click_selector: Option<String>,
    #[serde(default)]
    pub expected_title: Option<String>,
    #[serde(default)]
    pub expected_text: Option<String>,
    #[serde(default)]
    pub expected_display_list: Option<Vec<DisplayCommand>>,
    #[serde(default)]
    pub expected_hit_tests: Vec<BrowserFixtureHitTest>,
    #[serde(default)]
    pub expected_layers: Option<Vec<BrowserLayer>>,
    #[serde(default)]
    pub raster_viewport_x: Option<usize>,
    #[serde(default)]
    pub raster_viewport_y: Option<usize>,
    #[serde(default)]
    pub raster_viewport_width: Option<usize>,
    #[serde(default)]
    pub raster_viewport_height: Option<usize>,
    #[serde(default)]
    pub expected_visible_command_count: Option<usize>,
    #[serde(default)]
    pub expected_culled_command_count: Option<usize>,
    #[serde(default)]
    pub expected_raster_hash: Option<String>,
    #[serde(default, alias = "skip")]
    pub skipped: bool,
    #[serde(default)]
    pub unsupported: bool,
    #[serde(default)]
    pub flaky: bool,
    #[serde(default)]
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrowserCompatExpectation {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub path: Option<PathBuf>,
    #[serde(default, alias = "expected", alias = "expected_status")]
    pub status: Option<String>,
    #[serde(default)]
    pub reason: Option<String>,
    #[serde(default)]
    pub flaky: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrowserCompatOptions {
    pub manifest: PathBuf,
    #[serde(default)]
    pub expectations: Option<PathBuf>,
    #[serde(default)]
    pub subsets: Vec<String>,
    #[serde(default = "default_repeat")]
    pub repeat: usize,
    #[serde(default)]
    pub timeout_ms: Option<u64>,
    #[serde(default)]
    pub gate: BrowserCompatGate,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BrowserCompatGate {
    #[serde(default)]
    pub min_pass_rate: Option<f64>,
    #[serde(default)]
    pub max_unexpected_failures: Option<usize>,
    #[serde(default)]
    pub max_failures: Option<usize>,
    #[serde(default)]
    pub max_timeouts: Option<usize>,
    #[serde(default)]
    pub max_crashes: Option<usize>,
    #[serde(default)]
    pub max_flakes: Option<usize>,
    #[serde(default)]
    pub max_skipped: Option<usize>,
    #[serde(default)]
    pub max_unsupported: Option<usize>,
    #[serde(default)]
    pub required_subsystems: Vec<String>,
    #[serde(default)]
    pub min_subsystem_pass_rate: Option<f64>,
}

impl BrowserCompatGate {
    pub fn is_empty(&self) -> bool {
        self.min_pass_rate.is_none()
            && self.max_unexpected_failures.is_none()
            && self.max_failures.is_none()
            && self.max_timeouts.is_none()
            && self.max_crashes.is_none()
            && self.max_flakes.is_none()
            && self.max_skipped.is_none()
            && self.max_unsupported.is_none()
            && self.required_subsystems.is_empty()
            && self.min_subsystem_pass_rate.is_none()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrowserCompatReport {
    pub engine: String,
    pub suite: String,
    pub manifest: String,
    pub manifest_hash: String,
    pub suite_hash: String,
    pub expectation_file: Option<String>,
    pub expectation_hash: Option<String>,
    pub suite_count: usize,
    pub selected_count: usize,
    pub run_count: usize,
    pub repeat: usize,
    pub subsets: Vec<String>,
    pub timeout_ms: Option<u64>,
    pub subsystem_count: usize,
    pub runnable_count: usize,
    pub pass_count: usize,
    pub fail_count: usize,
    pub timeout_count: usize,
    pub crash_count: usize,
    pub skipped_count: usize,
    pub unsupported_count: usize,
    pub flaky_count: usize,
    pub expected_count: usize,
    pub unexpected_count: usize,
    pub pass_rate: f64,
    pub gate: Option<BrowserCompatGate>,
    pub gate_failures: Vec<String>,
    pub passed: Option<bool>,
    pub subsystems: Vec<BrowserCompatSubsystemReport>,
    pub tests: Vec<BrowserCompatTestReport>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrowserCompatSubsystemReport {
    pub subsystem: String,
    pub suite_count: usize,
    pub runnable_count: usize,
    pub pass_count: usize,
    pub fail_count: usize,
    pub timeout_count: usize,
    pub crash_count: usize,
    pub skipped_count: usize,
    pub unsupported_count: usize,
    pub flaky_count: usize,
    pub expected_count: usize,
    pub unexpected_count: usize,
    pub pass_rate: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrowserCompatTestReport {
    pub id: String,
    pub name: String,
    pub path: String,
    pub subsystem: String,
    pub status: String,
    pub expected_status: String,
    pub expected: bool,
    pub flaky: bool,
    pub reason: Option<String>,
    pub error: Option<String>,
    pub duration_us: u128,
    pub attempt: usize,
    pub repeat_count: usize,
    pub rendered_bytes: usize,
    pub dom_node_count: usize,
    pub css_rule_count: usize,
    pub layout_box_count: usize,
    pub paint_command_count: usize,
    pub expected_title: Option<String>,
    pub actual_title: Option<String>,
    pub expected_text: Option<String>,
    pub actual_text: Option<String>,
    pub expected_raster_hash: Option<String>,
    pub actual_raster_hash: Option<String>,
}

impl BrowserCompatReport {
    pub fn apply_gate(&mut self, gate: BrowserCompatGate) -> bool {
        let present_subsystems = self
            .subsystems
            .iter()
            .map(|subsystem| subsystem.subsystem.as_str())
            .collect::<BTreeSet<_>>();
        let mut failures = Vec::new();

        if let Some(min) = gate.min_pass_rate
            && self.pass_rate < min
        {
            failures.push(format!(
                "pass_rate {:.4} below required {:.4}",
                self.pass_rate, min
            ));
        }
        if let Some(max) = gate.max_unexpected_failures
            && self.unexpected_count > max
        {
            failures.push(format!(
                "unexpected_count {} exceeded allowed {}",
                self.unexpected_count, max
            ));
        }
        if let Some(max) = gate.max_failures
            && self.fail_count > max
        {
            failures.push(format!(
                "fail_count {} exceeded allowed {}",
                self.fail_count, max
            ));
        }
        if let Some(max) = gate.max_timeouts
            && self.timeout_count > max
        {
            failures.push(format!(
                "timeout_count {} exceeded allowed {}",
                self.timeout_count, max
            ));
        }
        if let Some(max) = gate.max_crashes
            && self.crash_count > max
        {
            failures.push(format!(
                "crash_count {} exceeded allowed {}",
                self.crash_count, max
            ));
        }
        if let Some(max) = gate.max_flakes
            && self.flaky_count > max
        {
            failures.push(format!(
                "flaky_count {} exceeded allowed {}",
                self.flaky_count, max
            ));
        }
        if let Some(max) = gate.max_skipped
            && self.skipped_count > max
        {
            failures.push(format!(
                "skipped_count {} exceeded allowed {}",
                self.skipped_count, max
            ));
        }
        if let Some(max) = gate.max_unsupported
            && self.unsupported_count > max
        {
            failures.push(format!(
                "unsupported_count {} exceeded allowed {}",
                self.unsupported_count, max
            ));
        }
        for required in &gate.required_subsystems {
            if !present_subsystems.contains(required.as_str()) {
                failures.push(format!("required subsystem {required:?} was not present"));
            }
        }
        if let Some(min) = gate.min_subsystem_pass_rate {
            for subsystem in &self.subsystems {
                if subsystem.runnable_count > 0 && subsystem.pass_rate < min {
                    failures.push(format!(
                        "subsystem {:?} pass_rate {:.4} below required {:.4}",
                        subsystem.subsystem, subsystem.pass_rate, min
                    ));
                }
            }
        }

        let passed = failures.is_empty();
        self.gate = Some(gate);
        self.gate_failures = failures;
        self.passed = Some(passed);
        passed
    }
}

pub fn run_browser_compat(options: BrowserCompatOptions) -> Result<BrowserCompatReport> {
    let manifest_text = fs::read_to_string(&options.manifest).with_context(|| {
        format!(
            "read browser compat manifest {}",
            options.manifest.display()
        )
    })?;
    let manifest_hash = blake3::hash(manifest_text.as_bytes()).to_hex().to_string();
    let manifest: BrowserCompatManifest =
        serde_json::from_str(&manifest_text).with_context(|| {
            format!(
                "parse browser compat manifest {}",
                options.manifest.display()
            )
        })?;
    ensure!(
        !manifest.tests.is_empty(),
        "browser compat manifest has no tests"
    );
    let base_dir = options.manifest.parent().unwrap_or_else(|| Path::new("."));
    let expectation_path = options.expectations.clone().or_else(|| {
        manifest
            .expectations
            .as_ref()
            .map(|path| resolve_manifest_path(base_dir, path))
    });
    let expectation_inputs = load_browser_compat_expectations(expectation_path.as_deref())?;
    let expectation_hash = expectation_inputs.hash.clone();
    let expectation_file = expectation_path
        .as_ref()
        .map(|path| path.display().to_string());
    let expectations = expectation_map(expectation_inputs.expectations)?;
    validate_browser_compat_tests(&manifest.tests, &expectations)?;
    let selected_tests = selected_browser_compat_tests(&manifest.tests, &options.subsets)?;
    let suite_hash = browser_compat_suite_hash(base_dir, manifest_hash.as_str(), &manifest.tests);

    ensure!(
        options.repeat > 0,
        "browser compat repeat must be greater than zero"
    );
    let mut tests = Vec::with_capacity(selected_tests.len().saturating_mul(options.repeat));
    for test in &selected_tests {
        for attempt in 1..=options.repeat {
            tests.push(run_browser_compat_test(
                base_dir,
                test,
                &expectations,
                attempt,
                options.repeat,
                options.timeout_ms,
            )?);
        }
    }

    let mut report = build_browser_compat_report(BrowserCompatReportInput {
        suite: manifest.suite,
        manifest: options.manifest.display().to_string(),
        manifest_hash,
        suite_hash,
        expectation_file,
        expectation_hash,
        selected_count: selected_tests.len(),
        repeat: options.repeat,
        subsets: options.subsets,
        timeout_ms: options.timeout_ms,
        tests,
    });
    if !options.gate.is_empty() {
        report.apply_gate(options.gate);
    }
    Ok(report)
}

#[derive(Debug, Clone)]
struct BrowserCompatExpectationInputs {
    expectations: Vec<BrowserCompatExpectation>,
    hash: Option<String>,
}

fn load_browser_compat_expectations(path: Option<&Path>) -> Result<BrowserCompatExpectationInputs> {
    let Some(path) = path else {
        return Ok(BrowserCompatExpectationInputs {
            expectations: Vec::new(),
            hash: None,
        });
    };
    let text = fs::read_to_string(path)
        .with_context(|| format!("read browser compat expectations {}", path.display()))?;
    let hash = Some(blake3::hash(text.as_bytes()).to_hex().to_string());
    let expectations = text
        .lines()
        .enumerate()
        .filter_map(|(index, line)| {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                None
            } else {
                Some(
                    serde_json::from_str::<BrowserCompatExpectation>(trimmed).with_context(|| {
                        format!(
                            "parse browser compat expectation {}:{}",
                            path.display(),
                            index + 1
                        )
                    }),
                )
            }
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(BrowserCompatExpectationInputs { expectations, hash })
}

fn expectation_map(
    expectations: Vec<BrowserCompatExpectation>,
) -> Result<BTreeMap<String, BrowserCompatExpectation>> {
    let mut map = BTreeMap::new();
    for expectation in expectations {
        let status = normalized_expectation_status(expectation.status.as_deref())?;
        let expectation = BrowserCompatExpectation {
            status,
            ..expectation
        };
        let key = expectation_key(&expectation)?;
        ensure!(
            map.insert(key.clone(), expectation).is_none(),
            "duplicate browser compat expectation for {key:?}"
        );
    }
    Ok(map)
}

fn validate_browser_compat_tests(
    tests: &[BrowserCompatTest],
    expectations: &BTreeMap<String, BrowserCompatExpectation>,
) -> Result<()> {
    let mut ids = BTreeSet::new();
    let mut expectation_keys = BTreeSet::new();
    for test in tests {
        let id = test_id(test);
        ensure!(
            ids.insert(id.clone()),
            "duplicate browser compat test id {id:?}"
        );
        for key in test_expectation_keys(test, &id, &test_name(test)) {
            expectation_keys.insert(key);
        }
    }
    for key in expectations.keys() {
        ensure!(
            expectation_keys.contains(key),
            "browser compat expectation {key:?} does not match any manifest test"
        );
    }
    Ok(())
}

fn selected_browser_compat_tests<'a>(
    tests: &'a [BrowserCompatTest],
    subsets: &[String],
) -> Result<Vec<&'a BrowserCompatTest>> {
    if subsets.is_empty() {
        return Ok(tests.iter().collect());
    }
    let subset_set = subsets.iter().map(String::as_str).collect::<BTreeSet<_>>();
    let selected = tests
        .iter()
        .filter(|test| subset_set.contains(test.subsystem.as_str()))
        .collect::<Vec<_>>();
    ensure!(
        !selected.is_empty(),
        "browser compat subset filter selected no tests"
    );
    Ok(selected)
}

fn run_browser_compat_test(
    base_dir: &Path,
    test: &BrowserCompatTest,
    expectations: &BTreeMap<String, BrowserCompatExpectation>,
    attempt: usize,
    repeat_count: usize,
    timeout_ms: Option<u64>,
) -> Result<BrowserCompatTestReport> {
    let path = resolve_manifest_path(base_dir, &test.path);
    let id = test_id(test);
    let name = test_name(test);
    let expectation = expectation_for_test(test, &id, &name, expectations);
    let expectation_status = expectation
        .and_then(|expectation| expectation.status.as_deref())
        .unwrap_or(default_expectation_status(test))
        .to_owned();
    let flaky = test.flaky
        || expectation.is_some_and(|expectation| {
            expectation.flaky || expectation.status.as_deref() == Some("flaky")
        });
    let reason = expectation
        .and_then(|expectation| expectation.reason.clone())
        .or_else(|| {
            test.tags
                .iter()
                .find_map(|tag| tag.strip_prefix("reason:").map(str::to_owned))
        });
    let context = BrowserCompatRunContext {
        id,
        name,
        path,
        expectation_status,
        flaky,
        reason,
        attempt,
        repeat_count,
    };

    if test.skipped || context.expectation_status == STATUS_SKIPPED {
        return Ok(excluded_test_report(test, &context, STATUS_SKIPPED));
    }
    if test.unsupported || context.expectation_status == STATUS_UNSUPPORTED {
        return Ok(excluded_test_report(test, &context, STATUS_UNSUPPORTED));
    }

    let started = Instant::now();
    let render_result = catch_unwind(AssertUnwindSafe(|| {
        let bytes = fs::read(&context.path)
            .with_context(|| format!("read browser compat fixture {}", context.path.display()))?;
        let fixture = BrowserFixture {
            name: test.name.clone(),
            path: context.path.clone(),
            width: test.width,
            external_scripts: test.external_scripts,
            click_selector: test.click_selector.clone(),
            expected_title: test.expected_title.clone(),
            expected_text: test.expected_text.clone(),
            expected_display_list: test.expected_display_list.clone(),
            expected_hit_tests: test.expected_hit_tests.clone(),
            expected_layers: test.expected_layers.clone(),
            raster_viewport_x: test.raster_viewport_x,
            raster_viewport_y: test.raster_viewport_y,
            raster_viewport_width: test.raster_viewport_width,
            raster_viewport_height: test.raster_viewport_height,
            expected_visible_command_count: test.expected_visible_command_count,
            expected_culled_command_count: test.expected_culled_command_count,
            expected_raster_hash: test.expected_raster_hash.clone(),
            expected_screenshot_hash: None,
        };
        render_browser_fixture(&context.path, &bytes, &fixture)
    }));
    let duration_us = started.elapsed().as_micros();

    if let Some(timeout_ms) = timeout_ms {
        let timeout_us = u128::from(timeout_ms).saturating_mul(1_000);
        if duration_us > timeout_us {
            return Ok(timed_out_test_report(
                test,
                &context,
                duration_us,
                timeout_ms,
            ));
        }
    }

    match render_result {
        Ok(Ok(render)) => Ok(rendered_test_report(test, &context, duration_us, render)?),
        Ok(Err(error)) => Ok(crashed_test_report(
            test,
            &context,
            duration_us,
            error.to_string(),
        )),
        Err(payload) => Ok(crashed_test_report(
            test,
            &context,
            duration_us,
            panic_message(payload),
        )),
    }
}

#[derive(Debug)]
struct BrowserCompatRunContext {
    id: String,
    name: String,
    path: PathBuf,
    expectation_status: String,
    flaky: bool,
    reason: Option<String>,
    attempt: usize,
    repeat_count: usize,
}

fn rendered_test_report(
    test: &BrowserCompatTest,
    context: &BrowserCompatRunContext,
    duration_us: u128,
    render: BrowserRender,
) -> Result<BrowserCompatTestReport> {
    let mut mismatches = Vec::new();
    if let Some(expected_title) = &test.expected_title
        && render.title != *expected_title
    {
        mismatches.push(format!(
            "title mismatch: expected {:?}, got {:?}",
            expected_title, render.title
        ));
    }
    if let Some(expected_text) = &test.expected_text
        && render.text != *expected_text
    {
        mismatches.push(format!(
            "text mismatch: expected {:?}, got {:?}",
            expected_text, render.text
        ));
    }
    if let Some(expected_display_list) = &test.expected_display_list
        && render.display_list != *expected_display_list
    {
        mismatches.push(format!(
            "display list mismatch: expected {:?}, got {:?}",
            expected_display_list, render.display_list
        ));
    }
    for expected_hit_test in &test.expected_hit_tests {
        let actual = hit_test_render(&render, expected_hit_test.x, expected_hit_test.y);
        if !hit_test_expectation_matches(actual.hit.as_ref(), expected_hit_test.expected.as_ref()) {
            mismatches.push(format!(
                "hit test mismatch at ({}, {}): expected {:?}, got {:?}",
                expected_hit_test.x, expected_hit_test.y, expected_hit_test.expected, actual.hit
            ));
        }
    }
    if let Some(expected_layers) = &test.expected_layers {
        let actual = layer_tree_render(&render);
        if actual.layers != *expected_layers {
            mismatches.push(format!(
                "layer tree mismatch: expected {:?}, got {:?}",
                expected_layers, actual.layers
            ));
        }
    }
    let mut actual_raster_hash = None;
    let needs_raster_check = test.expected_raster_hash.is_some()
        || test.expected_visible_command_count.is_some()
        || test.expected_culled_command_count.is_some();
    if needs_raster_check {
        let raster_options = compat_test_raster_options(test);
        let raster = rasterize_render(&render, raster_options)?;
        let report = raster_report(&render, &raster, raster_options);
        let actual = raster.pixel_hash();
        if let Some(expected_raster_hash) = &test.expected_raster_hash {
            if actual != *expected_raster_hash {
                mismatches.push(format!(
                    "raster hash mismatch: expected {:?}, got {:?}",
                    expected_raster_hash, actual
                ));
            }
            actual_raster_hash = Some(actual);
        }
        if let Some(expected_visible_command_count) = test.expected_visible_command_count
            && report.visible_command_count != expected_visible_command_count
        {
            mismatches.push(format!(
                "visible raster command count mismatch: expected {}, got {}",
                expected_visible_command_count, report.visible_command_count
            ));
        }
        if let Some(expected_culled_command_count) = test.expected_culled_command_count
            && report.culled_command_count != expected_culled_command_count
        {
            mismatches.push(format!(
                "culled raster command count mismatch: expected {}, got {}",
                expected_culled_command_count, report.culled_command_count
            ));
        }
    }

    let status = if mismatches.is_empty() {
        STATUS_PASS
    } else {
        STATUS_FAIL
    };
    let error = (!mismatches.is_empty()).then(|| mismatches.join("; "));
    Ok(BrowserCompatTestReport {
        id: context.id.clone(),
        name: context.name.clone(),
        path: context.path.display().to_string(),
        subsystem: test.subsystem.clone(),
        status: status.to_owned(),
        expected_status: expected_status_for_report(&context.expectation_status, status),
        expected: status_matches_expectation(status, &context.expectation_status),
        flaky: context.flaky,
        reason: context.reason.clone(),
        error,
        duration_us,
        attempt: context.attempt,
        repeat_count: context.repeat_count,
        rendered_bytes: render.text.len(),
        dom_node_count: render.dom_node_count,
        css_rule_count: render.css_rule_count,
        layout_box_count: render.layout_box_count,
        paint_command_count: render.paint_command_count,
        expected_title: test.expected_title.clone(),
        actual_title: Some(render.title),
        expected_text: test.expected_text.clone(),
        actual_text: Some(render.text),
        expected_raster_hash: test.expected_raster_hash.clone(),
        actual_raster_hash,
    })
}

fn compat_test_raster_options(test: &BrowserCompatTest) -> BrowserRasterOptions {
    BrowserRasterOptions {
        viewport_x: test.raster_viewport_x,
        viewport_y: test.raster_viewport_y,
        viewport_width: test.raster_viewport_width,
        viewport_height: test.raster_viewport_height,
        ..BrowserRasterOptions::default()
    }
}

fn crashed_test_report(
    test: &BrowserCompatTest,
    context: &BrowserCompatRunContext,
    duration_us: u128,
    error: String,
) -> BrowserCompatTestReport {
    BrowserCompatTestReport {
        id: context.id.clone(),
        name: context.name.clone(),
        path: context.path.display().to_string(),
        subsystem: test.subsystem.clone(),
        status: STATUS_CRASH.to_owned(),
        expected_status: expected_status_for_report(&context.expectation_status, STATUS_CRASH),
        expected: status_matches_expectation(STATUS_CRASH, &context.expectation_status),
        flaky: context.flaky,
        reason: context.reason.clone(),
        error: Some(error),
        duration_us,
        attempt: context.attempt,
        repeat_count: context.repeat_count,
        rendered_bytes: 0,
        dom_node_count: 0,
        css_rule_count: 0,
        layout_box_count: 0,
        paint_command_count: 0,
        expected_title: test.expected_title.clone(),
        actual_title: None,
        expected_text: test.expected_text.clone(),
        actual_text: None,
        expected_raster_hash: test.expected_raster_hash.clone(),
        actual_raster_hash: None,
    }
}

fn timed_out_test_report(
    test: &BrowserCompatTest,
    context: &BrowserCompatRunContext,
    duration_us: u128,
    timeout_ms: u64,
) -> BrowserCompatTestReport {
    BrowserCompatTestReport {
        id: context.id.clone(),
        name: context.name.clone(),
        path: context.path.display().to_string(),
        subsystem: test.subsystem.clone(),
        status: STATUS_TIMEOUT.to_owned(),
        expected_status: expected_status_for_report(&context.expectation_status, STATUS_TIMEOUT),
        expected: status_matches_expectation(STATUS_TIMEOUT, &context.expectation_status),
        flaky: context.flaky,
        reason: context.reason.clone(),
        error: Some(format!(
            "browser compat test exceeded timeout_ms {timeout_ms}: {duration_us} us"
        )),
        duration_us,
        attempt: context.attempt,
        repeat_count: context.repeat_count,
        rendered_bytes: 0,
        dom_node_count: 0,
        css_rule_count: 0,
        layout_box_count: 0,
        paint_command_count: 0,
        expected_title: test.expected_title.clone(),
        actual_title: None,
        expected_text: test.expected_text.clone(),
        actual_text: None,
        expected_raster_hash: test.expected_raster_hash.clone(),
        actual_raster_hash: None,
    }
}

fn excluded_test_report(
    test: &BrowserCompatTest,
    context: &BrowserCompatRunContext,
    status: &str,
) -> BrowserCompatTestReport {
    BrowserCompatTestReport {
        id: context.id.clone(),
        name: context.name.clone(),
        path: context.path.display().to_string(),
        subsystem: test.subsystem.clone(),
        status: status.to_owned(),
        expected_status: expected_status_for_report(&context.expectation_status, status),
        expected: status_matches_expectation(status, &context.expectation_status),
        flaky: context.flaky,
        reason: context.reason.clone(),
        error: None,
        duration_us: 0,
        attempt: context.attempt,
        repeat_count: context.repeat_count,
        rendered_bytes: 0,
        dom_node_count: 0,
        css_rule_count: 0,
        layout_box_count: 0,
        paint_command_count: 0,
        expected_title: test.expected_title.clone(),
        actual_title: None,
        expected_text: test.expected_text.clone(),
        actual_text: None,
        expected_raster_hash: test.expected_raster_hash.clone(),
        actual_raster_hash: None,
    }
}

struct BrowserCompatReportInput {
    suite: String,
    manifest: String,
    manifest_hash: String,
    suite_hash: String,
    expectation_file: Option<String>,
    expectation_hash: Option<String>,
    selected_count: usize,
    repeat: usize,
    subsets: Vec<String>,
    timeout_ms: Option<u64>,
    tests: Vec<BrowserCompatTestReport>,
}

fn build_browser_compat_report(input: BrowserCompatReportInput) -> BrowserCompatReport {
    let counts = BrowserCompatCounts::from_tests(&input.tests);
    let mut subsystem_counts = BTreeMap::<String, BrowserCompatCounts>::new();
    for test in &input.tests {
        subsystem_counts
            .entry(test.subsystem.clone())
            .or_default()
            .add_test(test);
    }
    let subsystems = subsystem_counts
        .into_iter()
        .map(|(subsystem, counts)| counts.into_subsystem_report(subsystem))
        .collect::<Vec<_>>();

    BrowserCompatReport {
        engine: "brutal-browser-compat-local".to_owned(),
        suite: input.suite,
        manifest: input.manifest,
        manifest_hash: input.manifest_hash,
        suite_hash: input.suite_hash,
        expectation_file: input.expectation_file,
        expectation_hash: input.expectation_hash,
        suite_count: counts.suite_count,
        selected_count: input.selected_count,
        run_count: input.tests.len(),
        repeat: input.repeat,
        subsets: input.subsets,
        timeout_ms: input.timeout_ms,
        subsystem_count: subsystems.len(),
        runnable_count: counts.runnable_count,
        pass_count: counts.pass_count,
        fail_count: counts.fail_count,
        timeout_count: counts.timeout_count,
        crash_count: counts.crash_count,
        skipped_count: counts.skipped_count,
        unsupported_count: counts.unsupported_count,
        flaky_count: counts.flaky_count,
        expected_count: counts.expected_count,
        unexpected_count: counts.unexpected_count,
        pass_rate: counts.pass_rate(),
        gate: None,
        gate_failures: Vec::new(),
        passed: None,
        subsystems,
        tests: input.tests,
    }
}

#[derive(Debug, Clone, Default)]
struct BrowserCompatCounts {
    suite_count: usize,
    runnable_count: usize,
    pass_count: usize,
    fail_count: usize,
    timeout_count: usize,
    crash_count: usize,
    skipped_count: usize,
    unsupported_count: usize,
    flaky_count: usize,
    expected_count: usize,
    unexpected_count: usize,
}

impl BrowserCompatCounts {
    fn from_tests(tests: &[BrowserCompatTestReport]) -> Self {
        let mut counts = Self::default();
        for test in tests {
            counts.add_test(test);
        }
        counts
    }

    fn add_test(&mut self, test: &BrowserCompatTestReport) {
        self.suite_count += 1;
        match test.status.as_str() {
            STATUS_PASS => {
                self.runnable_count += 1;
                self.pass_count += 1;
            }
            STATUS_FAIL => {
                self.runnable_count += 1;
                self.fail_count += 1;
            }
            STATUS_TIMEOUT => {
                self.runnable_count += 1;
                self.timeout_count += 1;
            }
            STATUS_CRASH => {
                self.runnable_count += 1;
                self.crash_count += 1;
            }
            STATUS_UNSUPPORTED => {
                self.unsupported_count += 1;
            }
            STATUS_SKIPPED => {
                self.skipped_count += 1;
            }
            _ => {}
        }
        if test.flaky {
            self.flaky_count += 1;
        }
        if test.expected {
            self.expected_count += 1;
        } else {
            self.unexpected_count += 1;
        }
    }

    fn pass_rate(&self) -> f64 {
        if self.runnable_count == 0 {
            0.0
        } else {
            self.pass_count as f64 / self.runnable_count as f64
        }
    }

    fn into_subsystem_report(self, subsystem: String) -> BrowserCompatSubsystemReport {
        BrowserCompatSubsystemReport {
            subsystem,
            suite_count: self.suite_count,
            runnable_count: self.runnable_count,
            pass_count: self.pass_count,
            fail_count: self.fail_count,
            timeout_count: self.timeout_count,
            crash_count: self.crash_count,
            skipped_count: self.skipped_count,
            unsupported_count: self.unsupported_count,
            flaky_count: self.flaky_count,
            expected_count: self.expected_count,
            unexpected_count: self.unexpected_count,
            pass_rate: self.pass_rate(),
        }
    }
}

fn status_matches_expectation(status: &str, expectation_status: &str) -> bool {
    if expectation_status == "flaky" {
        matches!(status, STATUS_PASS | STATUS_FAIL)
    } else {
        status == expectation_status
    }
}

fn expected_status_for_report(expectation_status: &str, _status: &str) -> String {
    if expectation_status == "flaky" {
        "flaky".to_owned()
    } else {
        expectation_status.to_owned()
    }
}

fn default_expectation_status(test: &BrowserCompatTest) -> &'static str {
    if test.skipped {
        STATUS_SKIPPED
    } else if test.unsupported {
        STATUS_UNSUPPORTED
    } else {
        STATUS_PASS
    }
}

fn normalized_expectation_status(status: Option<&str>) -> Result<Option<String>> {
    status.map(normalize_status).transpose()
}

fn normalize_status(status: &str) -> Result<String> {
    let normalized = status.trim().to_ascii_lowercase().replace('-', "_");
    let normalized = match normalized.as_str() {
        "pass" | "passed" | "ok" => STATUS_PASS,
        "fail" | "failed" | "failure" => STATUS_FAIL,
        "timeout" | "timed_out" => STATUS_TIMEOUT,
        "crash" | "crashed" | "error" => STATUS_CRASH,
        "unsupported" | "not_supported" => STATUS_UNSUPPORTED,
        "skip" | "skipped" | "not_run" => STATUS_SKIPPED,
        "flaky" => "flaky",
        _ => bail!("unknown browser compat expectation status {status:?}"),
    };
    Ok(normalized.to_owned())
}

fn expectation_key(expectation: &BrowserCompatExpectation) -> Result<String> {
    if let Some(id) = expectation.id.as_deref().filter(|id| !id.is_empty()) {
        return Ok(format!("id:{id}"));
    }
    if let Some(path) = &expectation.path {
        return Ok(format!("path:{}", normalize_path_key(path)));
    }
    if let Some(name) = expectation.name.as_deref().filter(|name| !name.is_empty()) {
        return Ok(format!("name:{name}"));
    }
    bail!("browser compat expectation must include id, path, or name")
}

fn expectation_for_test<'a>(
    test: &BrowserCompatTest,
    id: &str,
    name: &str,
    expectations: &'a BTreeMap<String, BrowserCompatExpectation>,
) -> Option<&'a BrowserCompatExpectation> {
    let id_key = format!("id:{id}");
    if let Some(expectation) = expectations.get(&id_key) {
        return Some(expectation);
    }
    let path_key = format!("path:{}", normalize_path_key(&test.path));
    if let Some(expectation) = expectations.get(&path_key) {
        return Some(expectation);
    }
    let name_key = format!("name:{name}");
    expectations.get(&name_key)
}

fn test_expectation_keys(test: &BrowserCompatTest, id: &str, name: &str) -> Vec<String> {
    vec![
        format!("id:{id}"),
        format!("path:{}", normalize_path_key(&test.path)),
        format!("name:{name}"),
    ]
}

fn browser_compat_suite_hash(
    base_dir: &Path,
    manifest_hash: &str,
    tests: &[BrowserCompatTest],
) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"brutal-browser-compat-suite-v1");
    hasher.update(manifest_hash.as_bytes());
    hasher.update(&[0]);
    for test in tests {
        let path = resolve_manifest_path(base_dir, &test.path);
        hasher.update(normalize_path_key(&test.path).as_bytes());
        hasher.update(&[0]);
        match fs::read(&path) {
            Ok(bytes) => hasher.update(&bytes),
            Err(error) => hasher.update(error.to_string().as_bytes()),
        };
        hasher.update(&[0]);
    }
    hasher.finalize().to_hex().to_string()
}

fn resolve_manifest_path(base_dir: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        base_dir.join(path)
    }
}

fn normalize_path_key(path: &Path) -> String {
    path.components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

fn test_id(test: &BrowserCompatTest) -> String {
    test.id
        .clone()
        .unwrap_or_else(|| normalize_path_key(&test.path))
}

fn test_name(test: &BrowserCompatTest) -> String {
    test.name.clone().unwrap_or_else(|| test_id(test))
}

fn panic_message(payload: Box<dyn std::any::Any + Send>) -> String {
    if let Some(message) = payload.downcast_ref::<&str>() {
        (*message).to_owned()
    } else if let Some(message) = payload.downcast_ref::<String>() {
        message.clone()
    } else {
        "browser compat test panicked".to_owned()
    }
}

fn default_suite_name() -> String {
    "local-browser-compat".to_owned()
}

fn default_subsystem() -> String {
    "general".to_owned()
}

fn default_browser_compat_width() -> usize {
    BrowserRenderOptions::default().width
}

fn default_repeat() -> usize {
    1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn browser_compat_runs_manifest_and_counts_subsystems() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("pass.html"),
            "<!doctype html><html><head><title>Pass</title></head><body><h1>Hello</h1></body></html>",
        )
        .unwrap();
        fs::write(
            dir.path().join("fail.html"),
            "<!doctype html><html><head><title>Fail</title></head><body><h1>Actual</h1></body></html>",
        )
        .unwrap();
        fs::write(
            dir.path().join("skip.html"),
            "<!doctype html><html><head><title>Skip</title></head><body><h1>Skip</h1></body></html>",
        )
        .unwrap();
        fs::write(
            dir.path().join("unsupported.html"),
            "<!doctype html><html><head><title>Unsupported</title></head><body><h1>Unsupported</h1></body></html>",
        )
        .unwrap();
        let manifest = dir.path().join("manifest.json");
        fs::write(
            &manifest,
            r#"{
              "suite": "unit-subset",
              "tests": [
                {"id":"pass","name":"passes","subsystem":"dom","path":"pass.html","expected_title":"Pass","expected_text":"Hello"},
                {"id":"fail","name":"fails","subsystem":"css","path":"fail.html","expected_text":"Expected"},
                {"id":"skip","name":"skips","subsystem":"css","path":"skip.html","skipped":true},
                {"id":"unsupported","name":"unsupported","subsystem":"network","path":"unsupported.html","unsupported":true}
              ]
            }"#,
        )
        .unwrap();

        let report = run_browser_compat(BrowserCompatOptions {
            manifest,
            expectations: None,
            subsets: Vec::new(),
            repeat: 1,
            timeout_ms: None,
            gate: BrowserCompatGate::default(),
        })
        .unwrap();

        assert_eq!(report.suite, "unit-subset");
        assert_eq!(report.suite_count, 4);
        assert_eq!(report.selected_count, 4);
        assert_eq!(report.run_count, 4);
        assert_eq!(report.repeat, 1);
        assert_eq!(report.runnable_count, 2);
        assert_eq!(report.pass_count, 1);
        assert_eq!(report.fail_count, 1);
        assert_eq!(report.skipped_count, 1);
        assert_eq!(report.unsupported_count, 1);
        assert_eq!(report.timeout_count, 0);
        assert_eq!(report.crash_count, 0);
        assert_eq!(report.pass_rate, 0.5);
        assert_eq!(report.subsystem_count, 3);
        assert_eq!(report.manifest_hash.len(), 64);
        assert_eq!(report.suite_hash.len(), 64);

        let css = report
            .subsystems
            .iter()
            .find(|subsystem| subsystem.subsystem == "css")
            .unwrap();
        assert_eq!(css.runnable_count, 1);
        assert_eq!(css.fail_count, 1);
        assert_eq!(css.skipped_count, 1);
    }

    #[test]
    fn browser_compat_applies_expectations_and_gate() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("known-fail.html"),
            "<!doctype html><title>Known</title><h1>Actual</h1>",
        )
        .unwrap();
        let manifest = dir.path().join("manifest.json");
        fs::write(
            &manifest,
            r#"{"suite":"expectations","tests":[{"id":"known-fail","subsystem":"dom","path":"known-fail.html","expected_text":"Expected"}]}"#,
        )
        .unwrap();
        let expectations = dir.path().join("expectations.jsonl");
        fs::write(
            &expectations,
            r#"{"id":"known-fail","status":"FAIL","reason":"not implemented yet","flaky":true}"#,
        )
        .unwrap();

        let report = run_browser_compat(BrowserCompatOptions {
            manifest,
            expectations: Some(expectations),
            subsets: Vec::new(),
            repeat: 1,
            timeout_ms: None,
            gate: BrowserCompatGate {
                max_unexpected_failures: Some(0),
                max_failures: Some(1),
                ..BrowserCompatGate::default()
            },
        })
        .unwrap();

        assert_eq!(report.fail_count, 1);
        assert_eq!(report.flaky_count, 1);
        assert_eq!(report.expected_count, 1);
        assert_eq!(report.unexpected_count, 0);
        assert_eq!(report.tests[0].expected_status, "fail");
        assert!(report.tests[0].expected);
        assert_eq!(
            report.tests[0].reason.as_deref(),
            Some("not implemented yet")
        );
        assert_eq!(report.expectation_hash.as_deref().map(str::len), Some(64));
        assert_eq!(report.passed, Some(true));
        assert!(report.gate_failures.is_empty());
    }

    #[test]
    fn browser_compat_expectation_can_skip_missing_fixture() {
        let dir = tempfile::tempdir().unwrap();
        let manifest = dir.path().join("manifest.json");
        fs::write(
            &manifest,
            r#"{"tests":[{"id":"missing","subsystem":"network","path":"missing.html","expected_text":"Never rendered"}]}"#,
        )
        .unwrap();
        let expectations = dir.path().join("expectations.jsonl");
        fs::write(&expectations, r#"{"id":"missing","status":"skip"}"#).unwrap();

        let report = run_browser_compat(BrowserCompatOptions {
            manifest,
            expectations: Some(expectations),
            subsets: Vec::new(),
            repeat: 1,
            timeout_ms: None,
            gate: BrowserCompatGate::default(),
        })
        .unwrap();

        assert_eq!(report.runnable_count, 0);
        assert_eq!(report.skipped_count, 1);
        assert_eq!(report.crash_count, 0);
        assert_eq!(report.tests[0].status, "skipped");
    }

    #[test]
    fn browser_compat_loads_manifest_expectations_and_name_alias() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("known-fail.html"),
            "<!doctype html><title>Known</title><h1>Actual</h1>",
        )
        .unwrap();
        let manifest = dir.path().join("manifest.json");
        fs::write(
            &manifest,
            r#"{
              "name": "manifest-named-suite",
              "expectations": "expectations.jsonl",
              "fixtures": [
                {"id":"known-fail","subsystem":"dom","path":"known-fail.html","expected_text":"Expected"}
              ]
            }"#,
        )
        .unwrap();
        fs::write(
            dir.path().join("expectations.jsonl"),
            r#"{"id":"known-fail","status":"FAIL","reason":"loaded from manifest"}"#,
        )
        .unwrap();

        let report = run_browser_compat(BrowserCompatOptions {
            manifest,
            expectations: None,
            subsets: Vec::new(),
            repeat: 1,
            timeout_ms: None,
            gate: BrowserCompatGate::default(),
        })
        .unwrap();

        assert_eq!(report.suite, "manifest-named-suite");
        assert!(
            report
                .expectation_file
                .as_deref()
                .is_some_and(|path| { path.ends_with("expectations.jsonl") })
        );
        assert_eq!(report.fail_count, 1);
        assert_eq!(report.unexpected_count, 0);
        assert_eq!(
            report.tests[0].reason.as_deref(),
            Some("loaded from manifest")
        );
    }

    #[test]
    fn browser_compat_reports_timeout_when_elapsed_exceeds_threshold() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("slow-enough.html"),
            format!(
                "<!doctype html><title>Timeout</title><body>{}</body>",
                "word ".repeat(10_000)
            ),
        )
        .unwrap();
        let manifest = dir.path().join("manifest.json");
        fs::write(
            &manifest,
            r#"{"tests":[{"id":"timeout","subsystem":"rendering","path":"slow-enough.html","expected_title":"Timeout"}]}"#,
        )
        .unwrap();

        let report = run_browser_compat(BrowserCompatOptions {
            manifest,
            expectations: None,
            subsets: Vec::new(),
            repeat: 1,
            timeout_ms: Some(0),
            gate: BrowserCompatGate {
                max_timeouts: Some(0),
                ..BrowserCompatGate::default()
            },
        })
        .unwrap();

        assert_eq!(report.timeout_count, 1);
        assert_eq!(report.crash_count, 0);
        assert_eq!(report.tests[0].status, "timeout");
        assert_eq!(report.passed, Some(false));
    }
}
