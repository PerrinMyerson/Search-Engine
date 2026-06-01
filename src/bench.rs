use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[cfg(test)]
use crate::browser::BrowserRenderTimings;
use crate::browser::{
    BrowserChromiumParityReport, BrowserCoverageGate, BrowserCoverageReport,
    browser_coverage_report, compare_browser_fixtures_with_chromium,
};
use crate::browser_compat::{
    BrowserCompatGate, BrowserCompatOptions, BrowserCompatReport, run_browser_compat,
};
use crate::daemon::{default_socket_path, send_request};
use crate::index::{BuildStats, IndexBuildOptions, PreloadMode, SearchIndex, build_from_corpus};
use crate::protocol::{DaemonRequest, DaemonResponse};
use crate::query::SearchOptions;
use crate::render::render_target;
use crate::tokenizer::query_terms;

mod browser_perf;

pub use browser_perf::{
    BrowserPerfChromiumBaselineReport, BrowserPerfChromiumFixtureReport, BrowserPerfFixtureReport,
    BrowserPerfGate, BrowserPerfOptions, BrowserPerfReport, run_browser_perf,
};

pub const BENCH_STATUS_FILE: &str = "bench-status.json";

#[derive(Debug, Clone)]
pub struct BenchOptions {
    pub index: PathBuf,
    pub queries: PathBuf,
    pub limit: usize,
    pub warmup: usize,
    pub socket: Option<PathBuf>,
    pub use_daemon: bool,
}

#[derive(Debug, Clone)]
pub struct SmokeOptions {
    pub corpus: PathBuf,
    pub index: PathBuf,
    pub queries: PathBuf,
    pub limit: usize,
    pub warmup: usize,
}

#[derive(Debug, Clone)]
pub struct EvalOptions {
    pub index: PathBuf,
    pub judgments: PathBuf,
    pub limit: usize,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct EvalGate {
    pub required_mrr: Option<f64>,
    pub required_ndcg_at_k: Option<f64>,
    pub required_recall_at_k: Option<f64>,
    pub required_precision_at_k: Option<f64>,
    pub max_unresolved_judgment_count: Option<usize>,
}

impl EvalGate {
    pub fn is_empty(&self) -> bool {
        self.required_mrr.is_none()
            && self.required_ndcg_at_k.is_none()
            && self.required_recall_at_k.is_none()
            && self.required_precision_at_k.is_none()
            && self.max_unresolved_judgment_count.is_none()
    }
}

#[derive(Debug, Clone)]
pub struct GateOptions {
    pub corpus: PathBuf,
    pub index: PathBuf,
    pub queries: PathBuf,
    pub judgments: PathBuf,
    pub browser_manifest: PathBuf,
    pub limit: usize,
    pub warmup: usize,
    pub socket: Option<PathBuf>,
    pub use_daemon: bool,
    pub eval_gate: EvalGate,
    pub browser_coverage_gate: BrowserCoverageGate,
    pub chromium_search_baseline: bool,
    pub required_p95_speedup: Option<f64>,
    pub browser_chromium_parity: bool,
    pub browser_compat: bool,
    pub browser_compat_manifest: PathBuf,
    pub browser_compat_expectations: Option<PathBuf>,
    pub browser_compat_subsets: Vec<String>,
    pub browser_compat_repeat: usize,
    pub browser_compat_timeout_ms: Option<u64>,
    pub browser_compat_gate: BrowserCompatGate,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchReport {
    pub engine: String,
    pub query_count: usize,
    pub limit: usize,
    pub p50_us: u128,
    pub p95_us: u128,
    pub p99_us: u128,
    pub throughput_qps: f64,
    pub total_ms: u128,
    pub rustc: Option<String>,
    pub chrome: Option<String>,
    pub os: Option<String>,
    pub hardware: Option<String>,
    pub corpus_hash: String,
    pub index_hash: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchComparison {
    pub rust: BenchReport,
    pub chromium: BenchReport,
    pub p95_speedup: f64,
    pub required_p95_speedup: Option<f64>,
    pub passed: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SmokeReport {
    pub corpus: String,
    pub index: String,
    pub queries: String,
    pub build: BuildStats,
    pub query: String,
    pub result_count: usize,
    pub top_doc_id: u32,
    pub rendered_bytes: usize,
    pub bench: BenchReport,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvalReport {
    pub query_count: usize,
    pub evaluated_query_count: usize,
    pub limit: usize,
    pub mean_reciprocal_rank: f64,
    pub mean_ndcg_at_k: f64,
    pub mean_recall_at_k: f64,
    pub mean_precision_at_k: f64,
    pub unresolved_judgment_count: usize,
    pub required_mrr: Option<f64>,
    pub required_ndcg_at_k: Option<f64>,
    pub required_recall_at_k: Option<f64>,
    pub required_precision_at_k: Option<f64>,
    pub max_unresolved_judgment_count: Option<usize>,
    pub passed: Option<bool>,
    pub corpus_hash: String,
    pub index_hash: String,
    pub queries: Vec<EvalQueryReport>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvalQueryReport {
    pub query: String,
    pub relevant_count: usize,
    pub retrieved_relevant: usize,
    pub reciprocal_rank: f64,
    pub ndcg_at_k: f64,
    pub recall_at_k: f64,
    pub precision_at_k: f64,
    pub unresolved_judgment_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GateReport {
    pub passed: bool,
    pub failures: Vec<String>,
    pub smoke: SmokeReport,
    pub eval: EvalReport,
    pub search_comparison: Option<BenchComparison>,
    pub browser_coverage: BrowserCoverageReport,
    pub browser_chromium_parity: Option<BrowserChromiumParityReport>,
    pub browser_compat: Option<BrowserCompatReport>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReadinessStatus {
    Implemented,
    Partial,
    Missing,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReadinessAreaReport {
    pub area: String,
    pub claim_scopes: Vec<String>,
    pub required_end_state: String,
    pub evidence_gate: String,
    pub status: ReadinessStatus,
    pub current_evidence: Vec<String>,
    pub missing_work: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompetitorReadinessReport {
    pub passed: bool,
    pub claim: String,
    pub required_claim_scopes: Vec<String>,
    pub standard: String,
    pub area_count: usize,
    pub implemented_count: usize,
    pub partial_count: usize,
    pub missing_count: usize,
    pub areas: Vec<ReadinessAreaReport>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompetitorAuditReport {
    pub passed: bool,
    pub claim: String,
    pub traceability_passed: bool,
    pub traceability_claim_complete: bool,
    pub evidence_passed: bool,
    pub readiness_passed: bool,
    pub failure_count: usize,
    pub failures: Vec<String>,
    pub partial_requirement_ids: Vec<String>,
    pub missing_requirement_ids: Vec<String>,
    pub partial_readiness_areas: Vec<String>,
    pub missing_readiness_areas: Vec<String>,
    pub traceability: TraceabilityReport,
    pub evidence: EvidenceRegistryReport,
    pub readiness: CompetitorReadinessReport,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadinessClaim {
    CombinedCompetitor,
    GoogleStyleSearch,
    ChromiumClassBrowser,
}

impl ReadinessClaim {
    fn id(self) -> &'static str {
        match self {
            ReadinessClaim::CombinedCompetitor => "combined_competitor",
            ReadinessClaim::GoogleStyleSearch => "google_style_search",
            ReadinessClaim::ChromiumClassBrowser => "chromium_class_browser",
        }
    }

    fn required_scopes(self) -> &'static [&'static str] {
        match self {
            ReadinessClaim::CombinedCompetitor => &["Search", "Browser"],
            ReadinessClaim::GoogleStyleSearch => &["Search"],
            ReadinessClaim::ChromiumClassBrowser => &["Browser"],
        }
    }

    fn standard(self) -> &'static str {
        match self {
            ReadinessClaim::CombinedCompetitor => {
                "Do not claim combined search/browser product readiness until every required area is implemented with direct evidence gates."
            }
            ReadinessClaim::GoogleStyleSearch => {
                "Do not claim search product readiness until every search and shared area is implemented with direct evidence gates."
            }
            ReadinessClaim::ChromiumClassBrowser => {
                "Do not claim browser product readiness until every browser and shared area is implemented with direct evidence gates."
            }
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraceabilityRequirement {
    pub id: String,
    pub required_capability: String,
    pub necessary_steps: String,
    pub primary_gates: String,
    pub plan_owners: String,
    pub readiness_area: String,
    pub milestone: String,
    pub claim_scope: String,
    pub current_state: ReadinessStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraceabilityClaimReport {
    pub claim: String,
    pub required_claim_scopes: Vec<String>,
    pub requirement_count: usize,
    pub implemented_count: usize,
    pub partial_count: usize,
    pub missing_count: usize,
    pub complete: bool,
    pub implemented_ids: Vec<String>,
    pub partial_ids: Vec<String>,
    pub missing_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraceabilityMilestoneReport {
    pub milestone: String,
    pub included_milestones: Vec<String>,
    pub requirement_count: usize,
    pub implemented_count: usize,
    pub partial_count: usize,
    pub missing_count: usize,
    pub complete: bool,
    pub implemented_ids: Vec<String>,
    pub partial_ids: Vec<String>,
    pub missing_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraceabilityReport {
    pub passed: bool,
    pub complete: bool,
    pub required_count: usize,
    pub row_count: usize,
    pub implemented_count: usize,
    pub partial_count: usize,
    pub missing_count: usize,
    pub missing_required_ids: Vec<String>,
    pub unknown_ids: Vec<String>,
    pub unknown_readiness_areas: Vec<String>,
    pub unknown_milestones: Vec<String>,
    pub unknown_claim_scopes: Vec<String>,
    pub mismatched_claim_scopes: Vec<String>,
    pub duplicate_ids: Vec<String>,
    pub validation_errors: Vec<String>,
    pub rows_by_readiness_area: BTreeMap<String, usize>,
    pub rows_by_milestone: BTreeMap<String, usize>,
    pub rows_by_claim_scope: BTreeMap<String, usize>,
    pub claim_reports: Vec<TraceabilityClaimReport>,
    pub milestone_reports: Vec<TraceabilityMilestoneReport>,
    pub requirements: Vec<TraceabilityRequirement>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvidenceRegistryRow {
    pub evidence_id: String,
    pub requirement_ids: Vec<String>,
    pub required_for: String,
    pub artifact_or_command: String,
    pub produces: String,
    pub completion_standard: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvidenceRegistryReport {
    pub passed: bool,
    pub row_count: usize,
    pub covered_requirement_count: usize,
    pub missing_required_ids: Vec<String>,
    pub unknown_requirement_ids: Vec<String>,
    pub duplicate_evidence_ids: Vec<String>,
    pub validation_errors: Vec<String>,
    pub rows: Vec<EvidenceRegistryRow>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", content = "report", rename_all = "snake_case")]
pub enum BenchStatusReport {
    Search(BenchReport),
    Comparison(BenchComparison),
    Smoke(SmokeReport),
    Eval(EvalReport),
    BrowserPerf(Box<BrowserPerfReport>),
    BrowserCompat(BrowserCompatReport),
    Gate(Box<GateReport>),
}

impl BenchComparison {
    pub fn apply_gate(&mut self, required_p95_speedup: f64) -> bool {
        let passed = self.p95_speedup >= required_p95_speedup;
        self.required_p95_speedup = Some(required_p95_speedup);
        self.passed = Some(passed);
        passed
    }
}

impl EvalReport {
    pub fn apply_gate(&mut self, gate: EvalGate) -> bool {
        let passed = meets_minimum(self.mean_reciprocal_rank, gate.required_mrr)
            && meets_minimum(self.mean_ndcg_at_k, gate.required_ndcg_at_k)
            && meets_minimum(self.mean_recall_at_k, gate.required_recall_at_k)
            && meets_minimum(self.mean_precision_at_k, gate.required_precision_at_k)
            && gate
                .max_unresolved_judgment_count
                .is_none_or(|max| self.unresolved_judgment_count <= max);
        self.required_mrr = gate.required_mrr;
        self.required_ndcg_at_k = gate.required_ndcg_at_k;
        self.required_recall_at_k = gate.required_recall_at_k;
        self.required_precision_at_k = gate.required_precision_at_k;
        self.max_unresolved_judgment_count = gate.max_unresolved_judgment_count;
        self.passed = Some(passed);
        passed
    }
}

fn meets_minimum(actual: f64, required: Option<f64>) -> bool {
    required.is_none_or(|required| actual >= required)
}

pub fn default_bench_status_path(index_dir: impl AsRef<Path>) -> PathBuf {
    index_dir.as_ref().join(BENCH_STATUS_FILE)
}

pub fn read_bench_status(index_dir: impl AsRef<Path>) -> Result<Option<BenchStatusReport>> {
    let path = default_bench_status_path(index_dir);
    if !path.exists() {
        return Ok(None);
    }
    let bytes = fs::read(&path).with_context(|| format!("read {}", path.display()))?;
    let report =
        serde_json::from_slice(&bytes).with_context(|| format!("parse {}", path.display()))?;
    Ok(Some(report))
}

pub fn write_bench_status(
    index_dir: impl AsRef<Path>,
    report: &BenchStatusReport,
) -> Result<PathBuf> {
    let path = default_bench_status_path(index_dir);
    write_bench_status_path(&path, report)?;
    Ok(path)
}

pub fn write_bench_status_path(path: impl AsRef<Path>, report: &BenchStatusReport) -> Result<()> {
    let path = path.as_ref();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }
    let json = serde_json::to_string_pretty(report)?;
    fs::write(path, json).with_context(|| format!("write {}", path.display()))?;
    Ok(())
}

pub fn run_smoke(options: SmokeOptions) -> Result<SmokeReport> {
    let build = build_from_corpus(
        &options.corpus,
        &options.index,
        IndexBuildOptions::default(),
    )?;
    let queries = read_queries(&options.queries)?;
    let query = queries.first().context("query file is empty")?.clone();

    let index = SearchIndex::open(&options.index, PreloadMode::Lazy)?;
    let results = index.search(
        &query,
        SearchOptions {
            limit: options.limit,
        },
    )?;
    let top = results
        .first()
        .with_context(|| format!("smoke query returned no results: {query}"))?;
    let rendered = render_target(&index, &top.doc_id.to_string())?;
    anyhow::ensure!(
        !rendered.trim().is_empty(),
        "rendered smoke document was empty"
    );

    let bench = run_in_process_bench(
        BenchOptions {
            index: options.index.clone(),
            queries: options.queries.clone(),
            limit: options.limit,
            warmup: options.warmup,
            socket: None,
            use_daemon: false,
        },
        queries,
    )?;

    Ok(SmokeReport {
        corpus: options.corpus.display().to_string(),
        index: options.index.display().to_string(),
        queries: options.queries.display().to_string(),
        build,
        query,
        result_count: results.len(),
        top_doc_id: top.doc_id,
        rendered_bytes: rendered.len(),
        bench,
    })
}

pub fn run_eval(options: EvalOptions) -> Result<EvalReport> {
    anyhow::ensure!(options.limit > 0, "eval --limit must be greater than zero");
    let index = SearchIndex::open(&options.index, PreloadMode::Aggressive)?;
    let judgments = read_eval_judgments(&options.judgments, &index)?;
    anyhow::ensure!(!judgments.is_empty(), "judgment file is empty");

    let mut reports = Vec::new();
    let mut unresolved_judgment_count = 0usize;
    let mut mrr_sum = 0.0;
    let mut ndcg_sum = 0.0;
    let mut recall_sum = 0.0;
    let mut precision_sum = 0.0;

    for judgment in &judgments {
        unresolved_judgment_count += judgment.unresolved_judgment_count;
        if judgment.relevant.is_empty() {
            continue;
        }
        let report = evaluate_query(&index, judgment, options.limit)?;
        mrr_sum += report.reciprocal_rank;
        ndcg_sum += report.ndcg_at_k;
        recall_sum += report.recall_at_k;
        precision_sum += report.precision_at_k;
        reports.push(report);
    }

    let evaluated_query_count = reports.len();
    anyhow::ensure!(
        evaluated_query_count > 0,
        "no judgments resolved to documents in the index"
    );
    let denom = evaluated_query_count as f64;

    Ok(EvalReport {
        query_count: judgments.len(),
        evaluated_query_count,
        limit: options.limit,
        mean_reciprocal_rank: mrr_sum / denom,
        mean_ndcg_at_k: ndcg_sum / denom,
        mean_recall_at_k: recall_sum / denom,
        mean_precision_at_k: precision_sum / denom,
        unresolved_judgment_count,
        required_mrr: None,
        required_ndcg_at_k: None,
        required_recall_at_k: None,
        required_precision_at_k: None,
        max_unresolved_judgment_count: None,
        passed: None,
        corpus_hash: index.manifest().corpus_hash.clone(),
        index_hash: index_hash(index.root()).unwrap_or_else(|_| "unknown".to_owned()),
        queries: reports,
    })
}

pub async fn run_gate(options: GateOptions) -> Result<GateReport> {
    let smoke = run_smoke(SmokeOptions {
        corpus: options.corpus.clone(),
        index: options.index.clone(),
        queries: options.queries.clone(),
        limit: options.limit,
        warmup: options.warmup,
    })?;

    let mut eval = run_eval(EvalOptions {
        index: options.index.clone(),
        judgments: options.judgments.clone(),
        limit: options.limit,
    })?;
    if !options.eval_gate.is_empty() {
        eval.apply_gate(options.eval_gate);
    }

    let mut browser_coverage = browser_coverage_report();
    if !options.browser_coverage_gate.is_empty() {
        browser_coverage.apply_gate(options.browser_coverage_gate.clone());
    }

    let run_speed_comparison =
        options.chromium_search_baseline || options.required_p95_speedup.is_some();
    let search_comparison = if run_speed_comparison {
        let mut comparison = run_search_comparison(BenchOptions {
            index: options.index.clone(),
            queries: options.queries.clone(),
            limit: options.limit,
            warmup: options.warmup,
            socket: options.socket.clone(),
            use_daemon: options.use_daemon,
        })
        .await?;
        if let Some(required) = options.required_p95_speedup {
            comparison.apply_gate(required);
        }
        Some(comparison)
    } else {
        None
    };

    let browser_chromium_parity = if options.browser_chromium_parity {
        Some(compare_browser_fixtures_with_chromium(
            &options.browser_manifest,
        )?)
    } else {
        None
    };

    let browser_compat = if options.browser_compat {
        let mut report = run_browser_compat(BrowserCompatOptions {
            manifest: options.browser_compat_manifest.clone(),
            expectations: options.browser_compat_expectations.clone(),
            subsets: options.browser_compat_subsets.clone(),
            repeat: options.browser_compat_repeat,
            timeout_ms: options.browser_compat_timeout_ms,
            gate: BrowserCompatGate::default(),
        })?;
        if !options.browser_compat_gate.is_empty() {
            report.apply_gate(options.browser_compat_gate.clone());
        }
        Some(report)
    } else {
        None
    };

    let mut failures = Vec::new();
    if eval.passed == Some(false) {
        failures.push(format!(
            "relevance gate failed: mrr {:.4}, ndcg {:.4}, recall {:.4}, precision {:.4}, unresolved {}",
            eval.mean_reciprocal_rank,
            eval.mean_ndcg_at_k,
            eval.mean_recall_at_k,
            eval.mean_precision_at_k,
            eval.unresolved_judgment_count
        ));
    }
    if browser_coverage.passed == Some(false) {
        failures.push(format!(
            "browser coverage gate failed: implemented {:.4}, missing {}",
            browser_coverage.implemented_ratio, browser_coverage.missing_count
        ));
    }
    if let Some(comparison) = &search_comparison
        && comparison.passed == Some(false)
    {
        failures.push(format!(
            "speed gate failed: p95 speedup {:.2}x is below required {:.2}x",
            comparison.p95_speedup,
            comparison.required_p95_speedup.unwrap_or_default()
        ));
    }
    if let Some(parity) = &browser_chromium_parity
        && parity.failed > 0
    {
        failures.push(format!(
            "browser Chromium parity failed: {}/{} fixtures failed",
            parity.failed, parity.fixture_count
        ));
    }
    if let Some(compat) = &browser_compat
        && compat.passed == Some(false)
    {
        failures.push(format!(
            "browser compatibility gate failed: pass_rate {:.4}, unexpected {}, failures [{}]",
            compat.pass_rate,
            compat.unexpected_count,
            compat.gate_failures.join("; ")
        ));
    }

    Ok(GateReport {
        passed: failures.is_empty(),
        failures,
        smoke,
        eval,
        search_comparison,
        browser_coverage,
        browser_chromium_parity,
        browser_compat,
    })
}

pub fn run_competitor_readiness() -> CompetitorReadinessReport {
    run_competitor_readiness_for_claim(ReadinessClaim::CombinedCompetitor)
}

pub fn run_competitor_readiness_for_claim(claim: ReadinessClaim) -> CompetitorReadinessReport {
    let mut report = build_competitor_readiness_report();
    apply_readiness_claim(&mut report, claim);
    report
}

pub fn run_competitor_audit() -> CompetitorAuditReport {
    run_competitor_audit_for_claim(ReadinessClaim::CombinedCompetitor)
}

pub fn run_competitor_audit_for_claim(claim: ReadinessClaim) -> CompetitorAuditReport {
    let traceability = run_traceability_report();
    let evidence = run_evidence_registry_report();
    let readiness = run_competitor_readiness_for_claim(claim);
    let claim_id = claim.id().to_owned();

    let traceability_claim = traceability
        .claim_reports
        .iter()
        .find(|claim_report| claim_report.claim == claim_id);
    let traceability_claim_complete =
        traceability.passed && traceability_claim.is_some_and(|claim_report| claim_report.complete);
    let partial_requirement_ids = traceability_claim
        .map(|claim_report| claim_report.partial_ids.clone())
        .unwrap_or_default();
    let missing_requirement_ids = traceability_claim
        .map(|claim_report| claim_report.missing_ids.clone())
        .unwrap_or_default();
    let partial_readiness_areas = readiness
        .areas
        .iter()
        .filter(|area| area.status == ReadinessStatus::Partial)
        .map(|area| area.area.clone())
        .collect::<Vec<_>>();
    let missing_readiness_areas = readiness
        .areas
        .iter()
        .filter(|area| area.status == ReadinessStatus::Missing)
        .map(|area| area.area.clone())
        .collect::<Vec<_>>();

    let mut failures = Vec::new();
    if !traceability.passed {
        failures.extend(
            traceability
                .validation_errors
                .iter()
                .map(|error| format!("traceability validation: {error}")),
        );
    }
    match traceability_claim {
        Some(claim_report) if !claim_report.complete => failures.push(format!(
            "traceability claim {} incomplete: {} partial, {} missing",
            claim_report.claim, claim_report.partial_count, claim_report.missing_count
        )),
        Some(_) => {}
        None => failures.push(format!("traceability claim report missing for {claim_id}")),
    }
    if !evidence.passed {
        failures.extend(
            evidence
                .validation_errors
                .iter()
                .map(|error| format!("evidence registry: {error}")),
        );
    }
    if !readiness.passed {
        failures.push(format!(
            "readiness claim {} incomplete: {} partial, {} missing",
            readiness.claim, readiness.partial_count, readiness.missing_count
        ));
    }

    CompetitorAuditReport {
        passed: traceability.passed
            && traceability_claim_complete
            && evidence.passed
            && readiness.passed,
        claim: claim_id,
        traceability_passed: traceability.passed,
        traceability_claim_complete,
        evidence_passed: evidence.passed,
        readiness_passed: readiness.passed,
        failure_count: failures.len(),
        failures,
        partial_requirement_ids,
        missing_requirement_ids,
        partial_readiness_areas,
        missing_readiness_areas,
        traceability,
        evidence,
        readiness,
    }
}

fn build_competitor_readiness_report() -> CompetitorReadinessReport {
    use ReadinessStatus::{Implemented, Missing, Partial};

    let browser_coverage = browser_coverage_report();
    let traceability_matrix = readiness_traceability_evidence();
    let evidence_registry = readiness_evidence_registry_evidence();
    let plan_docs = readiness_file_evidence(&[
        ReadinessFileSpec {
            path: "docs/PROGRAM_PLAN.md",
            summary: "PROGRAM_PLAN.md lists search crawling, indexing, relevance, serving, product, browser engine, JS/Web APIs, security, platform, and operations gates.",
            required_markers: &[
                "## Workstream A: Search Corpus Acquisition",
                "## Workstream C: Indexing And Storage",
                "## Workstream E: Ranking, Quality, And Relevance",
                "## Workstream F: Browser Engine Foundations",
                "## Workstream G: JavaScript And Web APIs",
                "## Workstream H: Security, Privacy, And Compliance",
                "## Workstream I: Infrastructure And Operations",
                "## Completion Audit Checklist",
            ],
        },
        ReadinessFileSpec {
            path: "docs/COMPETITOR_ROADMAP.md",
            summary: "COMPETITOR_ROADMAP.md separates fast search mode from browser-product milestones.",
            required_markers: &[
                "An independent browser engine",
                "A fast search/extraction mode",
                "### Milestone 6: Full Desktop Browser Product",
                "## Completion Definition",
            ],
        },
        ReadinessFileSpec {
            path: "docs/PERFORMANT_RUST_BROWSER_PLAN.md",
            summary: "PERFORMANT_RUST_BROWSER_PLAN.md records the performance-first Rust browser architecture, subsystem sequence, instrumentation, and acceptance gates.",
            required_markers: &[
                "## Performance Objective",
                "## Engine Architecture Steps",
                "## Networking And Loading",
                "## Parser, DOM, And CSS",
                "## JavaScript And Web APIs",
                "## Layout, Paint, Raster, And Compositor",
                "## Performance Instrumentation",
                "## Implementation Sequence",
                "## Acceptance Gates",
                "brutal-bench browser-perf",
            ],
        },
        ReadinessFileSpec {
            path: "docs/REQUIREMENTS_TRACEABILITY.md",
            summary: "REQUIREMENTS_TRACEABILITY.md maps every public-facing search, browser, security, platform, operations, benchmark, and governance requirement to gates, owners, milestones, claim scopes, and current state.",
            required_markers: &[
                "## Objective Boundary",
                "## Evidence States",
                "## Traceability Matrix",
                "Claim scopes are machine-validated",
                "## Release Claim Rules",
                "## Maintenance Rules",
            ],
        },
        ReadinessFileSpec {
            path: "docs/EVIDENCE_REGISTRY.md",
            summary: "EVIDENCE_REGISTRY.md maps traceability requirements to reproducible commands, fixtures, reports, external suites, release bundles, and completion standards.",
            required_markers: &[
                "## Evidence Rules",
                "## Registry",
                "EV-TRACEABILITY",
                "EV-READINESS",
                "EV-SEARCH-PERF",
                "EV-BROWSER-FIXTURES",
                "EV-BROWSER-PERF",
                "EV-WPT-SUBSETS",
                "EV-SECURITY-PRIVACY",
                "EV-OPERATIONS",
                "## Release Bundle Layout",
            ],
        },
        ReadinessFileSpec {
            path: "docs/BROWSER_RENDERING_COMPOSITOR_PLAN.md",
            summary: "BROWSER_RENDERING_COMPOSITOR_PLAN.md records paint/raster/compositor ownership, display-list, rasterization, compositor, visual regression, performance, security, and implementation gates.",
            required_markers: &[
                "## Rendering Standard",
                "## Display List And Paint Model",
                "## Rasterization",
                "## Compositor",
                "## Visual Regression Gates",
                "## Performance Gates",
                "## Security And Reliability Gates",
                "## Implementation Sequence",
                "REQ-BROWSER-PAINT-COMPOSITOR",
            ],
        },
    ]);
    let security_docs = readiness_file_evidence(&[ReadinessFileSpec {
        path: "docs/SECURITY_PRIVACY_PLAN.md",
        summary: "docs/SECURITY_PRIVACY_PLAN.md records assets, threat actors, trust boundaries, browser/search security requirements, and required gates.",
        required_markers: &[
            "## Assets",
            "## Threat Actors",
            "## Trust Boundaries",
            "## Browser Security Requirements",
            "## Search Security And Privacy Requirements",
            "## Security Gates",
        ],
    }]);
    let platform_docs = readiness_file_evidence(&[ReadinessFileSpec {
        path: "docs/PLATFORM_COMPLETENESS_PLAN.md",
        summary: "docs/PLATFORM_COMPLETENESS_PLAN.md records platform subsystem targets for fonts/text, images/SVG, CSS visual effects, canvas/GPU, media, accessibility, input, storage, devtools, extensions, packaging, updates, and search integration.",
        required_markers: &[
            "## Subsystem Matrix",
            "Fonts and text shaping",
            "Canvas/WebGL/WebGPU",
            "## Compatibility Gates",
            "## Performance Gates",
            "## Packaging Gates",
            "## Search Integration Gates",
        ],
    }]);
    let operations_docs = readiness_file_evidence(&[ReadinessFileSpec {
        path: "docs/OPERATIONS_RELIABILITY_PLAN.md",
        summary: "docs/OPERATIONS_RELIABILITY_PLAN.md records service topology, SLOs, observability, backup/restore, deployment, failure-injection, capacity, and incident gates.",
        required_markers: &[
            "## Service Topology",
            "## SLOs And Error Budgets",
            "## Observability",
            "## Backup And Restore",
            "## Deployment And Rollback",
            "## Failure Injection",
            "## Operations Gates",
        ],
    }]);
    let crawling_artifacts = readiness_file_evidence(&[
        ReadinessFileSpec {
            path: "src/crawler.rs",
            summary: "src/crawler.rs implements bounded crawl options, multi-seed crawling, robots checks, frontier-backed crawling, and document snapshots.",
            required_markers: &[
                "pub struct CrawlOptions",
                "pub async fn crawl_many",
                "crawl_with_frontier",
                "fetch_seed_robots",
                "robots_allowed",
                "document_snapshot_path",
            ],
        },
        ReadinessFileSpec {
            path: "src/frontier.rs",
            summary: "src/frontier.rs persists crawl frontier state, host stats, retry status, and recrawl planning.",
            required_markers: &[
                "pub struct FrontierStore",
                "pub struct FrontierStats",
                "pub struct HostStats",
                "pub fn recrawl_plan",
            ],
        },
        ReadinessFileSpec {
            path: "src/robots.rs",
            summary: "src/robots.rs parses robots.txt, sitemap hints, crawl delay, and per-path allow rules.",
            required_markers: &[
                "pub struct RobotsTxt",
                "pub async fn fetch",
                "pub fn parse",
                "pub fn allowed",
                "pub fn sitemaps",
                "pub fn crawl_delay",
            ],
        },
        ReadinessFileSpec {
            path: "src/sitemap.rs",
            summary: "src/sitemap.rs loads sitemap URLs, discovers sitemap sources from robots.txt, dedupes nested sitemap entries, and enforces byte caps.",
            required_markers: &[
                "pub async fn load_sitemap_seeds",
                "pub async fn discover_sitemap_sources_from_robots",
                "fn parse_sitemap_xml",
                "fn decode_sitemap_bytes",
            ],
        },
        ReadinessFileSpec {
            path: "src/recrawl.rs",
            summary: "src/recrawl.rs parses recrawl manifests with due-time filtering for seeds, domains, and sitemaps.",
            required_markers: &[
                "pub struct RecrawlManifest",
                "pub struct RecrawlScheduleOptions",
                "pub fn load_recrawl_manifest_with_options",
                "pub fn parse_recrawl_manifest_with_options",
            ],
        },
        ReadinessFileSpec {
            path: "src/scheduler.rs",
            summary: "src/scheduler.rs runs bounded recrawl rounds, compares content hashes, rebuilds the local index, and reports changed/unchanged/missing counts.",
            required_markers: &[
                "pub struct RecrawlSchedulerOptions",
                "pub struct RecrawlRoundReport",
                "pub async fn run_recrawl_scheduler",
                "pub async fn run_recrawl_round",
                "RecrawlChangeCounts",
            ],
        },
    ]);
    let indexing_artifacts = readiness_file_evidence(&[
        ReadinessFileSpec {
            path: "src/extract.rs",
            summary: "src/extract.rs extracts static HTML text, title, metadata, headings, links, canonical URL, language, and robots noindex while skipping non-text tags.",
            required_markers: &[
                "pub struct ExtractedPage",
                "pub fn extract_html",
                "fn push_break",
                "fn is_skip_tag",
                "b\"script\" | b\"style\" | b\"template\" | b\"svg\"",
                "robots_noindex",
            ],
        },
        ReadinessFileSpec {
            path: "src/index.rs",
            summary: "src/index.rs builds the custom local inverted index with manifests, compressed postings, mmap-backed text blobs, duplicate metadata, and corpus hashes.",
            required_markers: &[
                "pub struct SearchIndex",
                "const POSTINGS",
                "const TEXTS",
                "pub fn build_from_corpus",
                "pub fn build_from_fielded_documents",
                "MmapOptions::new().map",
                "fn encode_postings",
                "fn decode_postings_bytes",
                "corpus_hash",
            ],
        },
        ReadinessFileSpec {
            path: "src/varint.rs",
            summary: "src/varint.rs provides compact integer coding for posting-list storage.",
            required_markers: &["pub fn put_u32", "pub fn read_u32"],
        },
    ]);
    let relevance_artifacts = readiness_file_evidence(&[
        ReadinessFileSpec {
            path: "src/query.rs",
            summary: "src/query.rs implements BM25-style query execution with site/filetype/language/date filters, phrase filters, OR groups, exclusion, snippets, and duplicate collapse.",
            required_markers: &[
                "pub fn search_index",
                "struct ParsedQuery",
                "or_term_groups",
                "struct PhraseFilter",
                "fn parse_site_filter",
                "fn parse_filetype_filter",
                "fn parse_language_filter",
                "fn parse_after_filter",
                "fn text_contains_phrase",
            ],
        },
        ReadinessFileSpec {
            path: "src/index.rs",
            summary: "src/index.rs exposes prefix suggestions, spelling corrections, field boosts, authority score metadata, and duplicate-cluster metrics.",
            required_markers: &[
                "pub fn suggest",
                "pub fn spellcheck",
                "pub fn weighted_tf",
                "duplicate_cluster_count",
                "authority_score",
            ],
        },
        ReadinessFileSpec {
            path: "src/bench.rs",
            summary: "src/bench.rs implements relevance evaluation with MRR, NDCG@K, recall@K, precision@K, unresolved judgment counts, corpus hashes, and index hashes.",
            required_markers: &[
                "pub struct EvalReport",
                "pub fn run_eval",
                "mean_ndcg_at_k",
                "mean_recall_at_k",
                "mean_precision_at_k",
                "unresolved_judgment_count",
            ],
        },
        ReadinessFileSpec {
            path: "bench/judgments.jsonl",
            summary: "bench/judgments.jsonl provides deterministic judged-query fixtures for local relevance gates.",
            required_markers: &["\"query\"", "\"relevant\"", "\"grade\""],
        },
    ]);
    let serving_artifacts = readiness_file_evidence(&[
        ReadinessFileSpec {
            path: "src/daemon.rs",
            summary: "src/daemon.rs implements the resident hot-index daemon, Unix socket protocol handling, search/render/stats/suggest/spell requests, and daemon benchmark requests.",
            required_markers: &[
                "pub async fn run_daemon",
                "pub async fn send_request",
                "DaemonRequest::Search",
                "DaemonRequest::Render",
                "DaemonRequest::BenchSearch",
                "DaemonRequest::Stats",
            ],
        },
        ReadinessFileSpec {
            path: "src/server.rs",
            summary: "src/server.rs implements the local HTTP search UI/API, render endpoint, crawl status page, benchmark status page, and API tests.",
            required_markers: &[
                "pub async fn run_search_server",
                "\"/api/search\"",
                "\"/api/render\"",
                "\"/api/suggest\"",
                "\"/api/spell\"",
                "\"/api/crawl-status\"",
                "\"/api/bench-status\"",
                "crawl_status_page",
            ],
        },
        ReadinessFileSpec {
            path: "src/protocol.rs",
            summary: "src/protocol.rs defines the daemon request/response protocol for search, render, suggestions, spelling, stats, and benchmark timing.",
            required_markers: &[
                "pub enum DaemonRequest",
                "Search",
                "Suggest",
                "Spell",
                "Render",
                "BenchSearch",
                "Stats",
            ],
        },
        ReadinessFileSpec {
            path: "src/render.rs",
            summary: "src/render.rs renders full plaintext by document id or indexed URL from the mmap-backed index.",
            required_markers: &[
                "pub fn render_target",
                "target.parse::<u32>()",
                "doc_id_for_url",
            ],
        },
        ReadinessFileSpec {
            path: "src/bin/brutal-bench.rs",
            summary: "src/bin/brutal-bench.rs wires benchmark commands that can run the Chromium search baseline, require p95 speedup, verify browser parity, run browser performance fixture suites, and persist reports.",
            required_markers: &[
                "run_search_comparison",
                "run_browser_perf",
                "chromium_search_baseline",
                "require_speedup",
                "BrowserPerf",
                "browser_chromium_parity",
                "save_report",
            ],
        },
    ]);
    let browser_artifacts = readiness_file_evidence(&[
        ReadinessFileSpec {
            path: "src/browser.rs",
            summary: "src/browser.rs implements the current independent static browser runtime: DOM-like tree, display text, styled text color, link extraction plus resolved-link session activation and shell anchor-click default navigation, deterministic display-list hit testing used by the coordinate-click shell gate, responsive image source selection, image placeholder commands, retained paint-backed layout-box snapshots plus visible viewport layout-box state, dirty-region accounting, RGBA viewport frame surfaces, and BrowserApp re-exports, local layer-tree/debug snapshot reporting for the supported layout/display-list subset, deterministic CPU text/styled-text/rectangle/image/SVG/PNG-subset raster output with cached decoded pixels, RGBA8 PNG screenshot artifact export over that raster path, cache-backed session image rerendering, visual baseline verification, CSS display/color/background/border/padding/margin/sizing/max-width/auto-margin handling, JavaScript style property mutations and location readback over the supported inline CSS/URL subset, resource discovery/fetching, forms, BrowserSession text-like form field state merged into GET submissions, sessions, cookies, origin-scoped localStorage, in-memory sessionStorage, deterministic timer tasks, tiny scripting, events, and coverage API re-exports.",
            required_markers: &[
                "pub struct BrowserRender",
                "pub use app::",
                "pub struct BrowserLayoutBox",
                "pub struct BrowserVisibleLayoutBox",
                "pub struct BrowserDocumentViewportReport",
                "pub struct BrowserViewportFrameReport",
                "pub fn browser_viewport_frame",
                "pub fn browser_document_viewport",
                "pub fn layout_tree_render",
                "pub async fn activate_link",
                "pub async fn activate_link_text",
                "pub async fn activate_link_selector",
                "pub async fn click_selector_with_default_action",
                "pub async fn click_at_with_default_action",
                "pub fn set_form_field",
                "effective_form_overrides",
                "form_control_accepts_fill_state",
                "pub fn current_links",
                "pub struct BrowserRaster",
                "pub fn rasterize_render",
                "pub struct BrowserRgbaRaster",
                "pub fn rasterize_render_rgba",
                "pub struct BrowserVisualReport",
                "pub fn verify_browser_visuals",
                "pub fn hit_test_render",
                "pub struct BrowserHitTestReport",
                "pub fn layer_tree_render",
                "pub fn browser_layer_metrics",
                "pub struct BrowserLayerTreeReport",
                "pub struct BrowserLayerMetrics",
                "render_current_with_images",
                "srcset",
                "pub struct BrowserLocalStorage",
                "MAX_TIMER_TASKS_PER_RENDER",
                "pub enum DisplayCommand",
                "pub fn render_html",
                "pub fn render_html_with_external_css",
                "pub fn render_html_with_external_css_and_scripts",
                "pub fn render_html_with_click",
                "fn execute_js_tree_mutation",
                "fn resolve_js_insert_nodes",
                "NodeKind::DocumentFragment",
                "fn closest_matching_selector",
                "fn set_inner_html",
                "fn set_element_boolean_property",
                "fn evaluate_js_location_expression",
                "fn element_child_ids",
                "fn execute_js_set_attribute",
                "fn execute_js_class_list_mutation",
                "fn resolve_js_node_list_ref",
                "fn execute_js_style_mutation",
                "fn execute_js_web_storage",
                "fn execute_js_timer",
                "fn dispatch_lifecycle_event",
                "fn execute_js_add_event_listener",
            ],
        },
        ReadinessFileSpec {
            path: "src/browser/app.rs",
            summary: "src/browser/app.rs owns the reusable Rust browser app state model for tabs, navigation, viewport scrolling, input actions, and presentable RGBA viewport frames.",
            required_markers: &[
                "pub struct BrowserApp",
                "pub enum BrowserAppAction",
                "pub struct BrowserAppFindState",
                "pub struct BrowserAppWindowFrame",
                "pub struct BrowserAppWindowFrameReport",
                "pub struct BrowserAppWindowFrameOptions",
                "pub enum BrowserAppWindowHit",
                "pub struct BrowserAppWindowClickReport",
                "pub struct BrowserAppReport",
                "pub visible_text: Vec<String>",
                "browser_text_viewport",
                "pub async fn open",
                "pub async fn apply_action",
                "pub fn present_frame",
                "pub fn present_window_frame",
                "pub fn present_window_frame_with_options",
                "pub fn hit_test_window",
                "pub async fn click_window",
                "browser_viewport_frame",
                "BrowserAppAction::NewTab",
                "BrowserAppAction::Click",
                "BrowserAppAction::FindText",
                "BrowserAppAction::SetViewportOrigin",
                "BrowserAppAction::ActivateLinkText",
            ],
        },
        ReadinessFileSpec {
            path: "src/bin/brutal_browser_app/mod.rs",
            summary: "src/bin/brutal_browser_app/mod.rs wires the scripted and interactive/stdin brutal-browser app command over BrowserApp so future native shells can reuse the same tab, navigation, input, find-state, profile-state, viewport, and frame-output contract.",
            required_markers: &[
                "pub(crate) struct BrowserAppCli",
                "pub(crate) async fn run_browser_app_cli",
                "run_interactive_browser_app_shell",
                "run_browser_app_stdin",
                "BrowserAppProfile",
                "BrowserAppCliCommand::BookmarkAdd",
                "BrowserAppCliCommand::ProfileHistory",
                "load_browser_app_profile",
                "save_browser_app_profile",
                "window_output",
                "WindowClick",
                "window_frame_for_presented_frame",
                "BrowserApp::open",
                "BrowserApp::open_with_state",
                "BrowserAppAction::Open",
                "BrowserAppAction::Click",
                "BrowserAppAction::FindText",
                "BrowserAppAction::NewTab",
                "BrowserAppAction::SetViewportOrigin",
                "load_browser_cookie_jar",
                "save_browser_local_storage",
                "write_browser_app_frame",
            ],
        },
        ReadinessFileSpec {
            path: "src/bin/brutal_browser_window/mod.rs",
            summary: "src/bin/brutal_browser_window/mod.rs wires the feature-gated native window shell over BrowserApp so the Rust browser can present its own RGBA window frame and route local mouse, wheel, and keyboard input through the app state boundary.",
            required_markers: &[
                "pub(crate) struct BrowserWindowCli",
                "pub(crate) async fn run_browser_window_cli",
                "native-window",
                "minifb",
                "BrowserApp::open_with_state",
                "present_window_frame",
                "click_window",
                "BrowserAppAction::Scroll",
                "BrowserAppAction::SetViewport",
                "BrowserAppAction::TypeText",
                "BrowserAppAction::SubmitFocused",
                "BrowserWindowMode",
                "begin_browser_window_location_input",
                "browser_window_frame_options",
                "browser_viewport_size_for_window_pixels",
                "set_input_callback",
                "BrowserAppWindowFrameOptions",
                "rgba_to_native_window_buffer",
                "wheel_delta_to_scroll_cells",
            ],
        },
        ReadinessFileSpec {
            path: "src/browser/coverage.rs",
            summary: "src/browser/coverage.rs owns browser feature-fixture coverage catalog reporting, implemented/partial/missing counts, unsupported feature summaries, and coverage gates.",
            required_markers: &[
                "pub enum BrowserFeatureState",
                "pub struct BrowserCoverageReport",
                "pub fn browser_coverage_report",
                "pub fn unsupported_feature_summary",
                "\"text-color-paint\"",
                "\"css-max-width-auto-margin-layout\"",
                "\"responsive-image-selection\"",
                "\"network-image-render\"",
                "\"display-list-hit-testing\"",
                "\"layer-tree-snapshot\"",
                "\"retained-layout-tree\"",
                "\"rgba-screenshot-artifact\"",
                "\"image-decode-cache\"",
                "\"viewport-raster-culling\"",
                "\"browser-viewport-layout-state\"",
                "\"browser-viewport-invalidation\"",
                "\"browser-viewport-frame-surface\"",
                "\"browser-app-state-surface\"",
                "\"browser-app-cli-surface\"",
                "\"browser-app-interactive-shell\"",
                "\"browser-app-visible-viewport\"",
                "\"browser-app-profile-history-bookmarks\"",
                "\"browser-app-find-text\"",
                "\"browser-app-window-frame\"",
                "\"browser-app-window-hit-testing\"",
                "\"browser-native-window-shell\"",
                "\"browser-native-window-location-input\"",
                "\"browser-shell-cli\"",
                "\"browser-shell-visual-frame\"",
                "\"browser-shell-tabs\"",
                "\"browser-shell-link-activation\"",
                "\"browser-shell-anchor-click-default\"",
                "\"browser-shell-coordinate-click\"",
                "\"browser-shell-form-fill-state\"",
                "\"browser-session-urlencoded-post-form-submit\"",
                "\"browser-session-form-submit-button-click-default\"",
                "\"browser-session-form-reset-click-default\"",
                "\"dom-tree-mutation\"",
                "\"dom-node-traversal\"",
                "\"dom-insertion-methods\"",
                "\"document-fragment\"",
                "\"dom-selector-methods\"",
                "\"dom-inner-html-mutation\"",
                "\"dom-form-control-properties\"",
                "\"dom-location-readback\"",
                "\"dom-style-property-mutation\"",
                "\"dom-class-list-mutation\"",
                "\"dom-query-collections\"",
                "\"document-lifecycle-events\"",
                "\"browser-shell-session-storage-inspection\"",
                "\"browser-shell-clear-session-storage\"",
            ],
        },
        ReadinessFileSpec {
            path: "bench/browser-fixtures/manifest.json",
            summary: "bench/browser-fixtures/manifest.json verifies static rendering, display-list raster hashes for every browser fixture, display-list hit-test cases, layer-tree snapshot cases, viewport-window CPU raster culling counts, rectangle paint/raster commands, the text-color.html styled text color fixture, block background, border, padding, margin, sizing, and max-width auto-margin layout commands, image placeholder, responsive image source selection, SVG-subset image, and data URI PNG image paint commands, inline scripts, runtime node creation, tree mutation, DOM traversal, insertion convenience methods, DocumentFragment insertion, selector element methods, innerHTML DOM mutation, form control DOM properties, location readback DOM properties, attribute mutation/readback, style property mutation/readback, classList mutation/readback, DOM query collections, localStorage, sessionStorage, timer queue mutation, document/window lifecycle events, external scripts, inline click handlers, addEventListener click handlers, compound CSS selector display, attribute CSS selector display, and complex selector query/click routing.",
            required_markers: &[
                "static text and display none",
                "expected_hit_tests",
                "expected_layers",
                "expected_raster_hash",
                "viewport raster culling",
                "expected_visible_command_count",
                "expected_culled_command_count",
                "rect paint command",
                "text-color.html",
                "text color paint command",
                "background paint command",
                "border paint command",
                "padding layout command",
                "margin layout command",
                "block size layout command",
                "max-width auto-margin layout command",
                "image placeholder paint command",
                "svg image decode paint command",
                "responsive image selection paint command",
                "data URI PNG decode paint command",
                "inline JavaScript DOM text mutation",
                "inline JavaScript create and append",
                "DOM tree mutation methods",
                "DOM node traversal properties",
                "DOM insertion convenience methods",
                "DocumentFragment insertion",
                "Selector element methods",
                "innerHTML DOM mutation",
                "form control DOM properties",
                "location readback DOM properties",
                "setAttribute DOM mutation",
                "getAttribute string binding mutation",
                "style property DOM mutation",
                "classList DOM mutation",
                "DOM query collections",
                "localStorage DOM mutation",
                "sessionStorage DOM mutation",
                "timer task queue mutation",
                "document and window lifecycle events",
                "external JavaScript create and append",
                "inline onclick event mutation",
                "addEventListener click mutation",
                "compound CSS selector display",
                "attribute CSS selector display",
                "complex selector query and click",
            ],
        },
        ReadinessFileSpec {
            path: "bench/document-pages/manifest.json",
            summary: "bench/document-pages/manifest.json checks the Stage 1 document-page corpus with visible text and RGBA screenshot hashes.",
            required_markers: &[
                "stage1 document article",
                "stage1 search results document",
                "expected_text",
                "expected_screenshot_hash",
            ],
        },
        ReadinessFileSpec {
            path: "src/bench.rs",
            summary: "src/bench.rs implements the browser performance fixture-suite report with p50/p95/p99, throughput, phase timings, suite hash, environment metadata, and per-fixture DOM/layout/paint/layer counts plus local layer-tree shape metrics.",
            required_markers: &[
                "pub fn run_browser_perf",
                "pub struct BrowserPerfReport",
                "pub struct BrowserPerfFixtureReport",
                "throughput_pages_per_sec",
                "phase_totals",
                "suite_hash",
                "total_layout_boxes",
                "total_paint_commands",
                "total_layers",
                "total_image_layers",
                "layer_metrics_p95_us",
                "max_layer_metrics_p95_us",
                "min_total_image_layers",
            ],
        },
    ]);
    let js_artifacts = readiness_file_evidence(&[
        ReadinessFileSpec {
            path: "src/browser.rs",
            summary: "src/browser.rs contains the tiny JavaScript execution path for statement dispatch, string expression evaluation, DOM tree mutation, DOM traversal, insertion convenience methods, DocumentFragment insertion, selector element methods, innerHTML DOM mutation, form control DOM properties, location readback DOM properties, DOM attribute reads/writes, classList mutation/readback over supported class selectors, DOM query collection bindings, style property mutation/readback over supported inline CSS declarations, origin-scoped localStorage, BrowserSession-scoped sessionStorage, deterministic timer tasks, document/window lifecycle listeners, external scripts, and click listener registration.",
            required_markers: &[
                "fn execute_js_statement",
                "fn execute_js_tree_mutation",
                "fn resolve_js_insert_nodes",
                "document.createDocumentFragment()",
                "NodeKind::DocumentFragment",
                "parse_js_method_call(expression, \".matches\")",
                "parse_js_method_call(expression, \".closest\")",
                "strip_suffix(\".innerHTML\")",
                "fn set_inner_html",
                "document.head",
                "\".value\"",
                "\".checked\"",
                "fn set_element_boolean_property",
                "fn evaluate_js_location_expression",
                "window.location.href",
                "document.URL",
                "fn element_child_ids",
                "fn execute_js_set_attribute",
                "fn execute_js_class_list_mutation",
                "node_list_bindings",
                "fn execute_js_style_mutation",
                "fn set_element_style_property",
                "fn execute_js_web_storage",
                "fn execute_js_timer",
                "fn drain_timer_tasks",
                "fn dispatch_lifecycle_event",
                "fn execute_js_add_event_listener",
                "fn evaluate_js_string_expression",
                "parse_js_method_call(expression, \".getAttribute\")",
                "parse_js_method_call(expression, \".classList.contains\")",
                "parse_js_method_call(expression, \".querySelectorAll\")",
                "strip_suffix(\".firstElementChild\")",
                "strip_suffix(\".children\")",
                "parse_js_method_call(expression, \".getPropertyValue\")",
                "parse_js_method_call(statement, \".setAttribute\")",
                "parse_js_method_call(statement, \".insertBefore\")",
                "parse_js_method_call(statement, \".replaceChildren\")",
                "parse_js_method_call(statement, \".replaceWith\")",
                "parse_js_method_call(statement, \".classList.add\")",
                "parse_js_method_call(statement, \".setProperty\")",
                "parse_js_method_call(statement, \".setItem\")",
                "sessionStorage.length",
                "parse_js_named_call(expression, &[\"setTimeout\", \"window.setTimeout\"])",
                "is_supported_lifecycle_event",
                "parse_js_method_call(statement, \".addEventListener\")",
            ],
        },
        ReadinessFileSpec {
            path: "bench/browser-fixtures/get-attribute.html",
            summary: "bench/browser-fixtures/get-attribute.html exercises getAttribute, string bindings, text mutation, and cloned attribute mutation.",
            required_markers: &["getAttribute", "setAttribute", "textContent", "appendChild"],
        },
        ReadinessFileSpec {
            path: "bench/browser-fixtures/dom-tree-mutation.html",
            summary: "bench/browser-fixtures/dom-tree-mutation.html exercises insertBefore, replaceChild, element.remove, parentNode/removeChild, null-reference append insertion, and connected-tree query readback.",
            required_markers: &[
                "insertBefore",
                "replaceChild",
                ".remove()",
                "parentNode.removeChild",
                "insertBefore(tail, null)",
                "querySelectorAll",
            ],
        },
        ReadinessFileSpec {
            path: "bench/browser-fixtures/dom-node-traversal.html",
            summary: "bench/browser-fixtures/dom-node-traversal.html exercises children, childNodes, first/last child, first/last element child, sibling traversal, childElementCount, and nodeType readback.",
            required_markers: &[
                "firstElementChild",
                "nextElementSibling",
                "lastElementChild",
                "children.length",
                "childElementCount",
                "children.item(1)",
                "childNodes[0]",
                "previousElementSibling",
                "nodeType",
            ],
        },
        ReadinessFileSpec {
            path: "bench/browser-fixtures/dom-insertion-methods.html",
            summary: "bench/browser-fixtures/dom-insertion-methods.html exercises append, prepend, before, after, replaceWith, replaceChildren, and string-to-text-node insertion.",
            required_markers: &[
                ".before(",
                ".prepend(",
                ".append(",
                ".after(",
                "replaceWith",
                "replaceChildren",
                "createTextNode",
            ],
        },
        ReadinessFileSpec {
            path: "bench/browser-fixtures/document-fragment.html",
            summary: "bench/browser-fixtures/document-fragment.html exercises createDocumentFragment, fragment appendChild/append, replaceChildren(fragment), fragment child draining, childNodes length, and nodeType/nodeName readback.",
            required_markers: &[
                "createDocumentFragment",
                "fragment.appendChild",
                "fragment.append",
                "replaceChildren(fragment)",
                "fragment.childNodes.length",
                "fragment.nodeType",
                "lastElementChild.nodeName",
            ],
        },
        ReadinessFileSpec {
            path: "bench/browser-fixtures/selector-methods.html",
            summary: "bench/browser-fixtures/selector-methods.html exercises Element.matches and Element.closest over compound, descendant, and attribute selectors.",
            required_markers: &[
                ".matches(",
                ".closest(",
                "p.title.primary",
                "main[data-view=\"results\"]",
                "target.closest(\"section\").id",
            ],
        },
        ReadinessFileSpec {
            path: "bench/browser-fixtures/inner-html-mutation.html",
            summary: "bench/browser-fixtures/inner-html-mutation.html exercises innerHTML assignment/append, parsed child query/readback, document.head, selector methods over inserted children, and simple innerHTML serialization.",
            required_markers: &[
                "root.innerHTML =",
                "root.innerHTML +=",
                "document.head.nodeName",
                "root.querySelector",
                "root.innerHTML",
            ],
        },
        ReadinessFileSpec {
            path: "bench/browser-fixtures/form-control-properties.html",
            summary: "bench/browser-fixtures/form-control-properties.html exercises value/name/type/action/method/checked/disabled DOM property mutation and readback over extracted form state.",
            required_markers: &[
                "form.action",
                "form.method",
                "q.value",
                "q.type",
                "fast.checked",
                "fast.disabled",
                "go.value",
            ],
        },
        ReadinessFileSpec {
            path: "bench/browser-fixtures/location-readback.html",
            summary: "bench/browser-fixtures/location-readback.html exercises location, document.URL, and document.location pathname readback into DOM text.",
            required_markers: &[
                "location.href",
                "document.URL",
                "document.location.pathname",
                "state.textContent",
            ],
        },
        ReadinessFileSpec {
            path: "bench/browser-fixtures/style-property-mutation.html",
            summary: "bench/browser-fixtures/style-property-mutation.html exercises element.style assignment, setProperty, getPropertyValue, display hiding, and supported inline CSS paint updates.",
            required_markers: &[
                "style.color",
                "style.setProperty",
                "style.display",
                "style.getPropertyValue",
            ],
        },
        ReadinessFileSpec {
            path: "bench/browser-fixtures/class-list-mutation.html",
            summary: "bench/browser-fixtures/class-list-mutation.html exercises classList add/remove/toggle/contains/length, class selector style application, and DOM text readback.",
            required_markers: &[
                "classList.remove",
                "classList.add",
                "classList.toggle",
                "classList.contains",
                "classList.length",
            ],
        },
        ReadinessFileSpec {
            path: "bench/browser-fixtures/query-collections.html",
            summary: "bench/browser-fixtures/query-collections.html exercises querySelectorAll/getElementsByClassName/getElementsByTagName collection bindings, item/index access, scoped queries, length readback, and text property reads.",
            required_markers: &[
                "querySelectorAll",
                "getElementsByClassName",
                "getElementsByTagName",
                "cards.item",
                "cards[1]",
                "cards.length",
            ],
        },
        ReadinessFileSpec {
            path: "bench/browser-fixtures/listener-event.html",
            summary: "bench/browser-fixtures/listener-event.html exercises addEventListener click dispatch and post-click DOM text mutation.",
            required_markers: &["addEventListener", "click", "textContent"],
        },
        ReadinessFileSpec {
            path: "bench/browser-fixtures/local-storage.html",
            summary: "bench/browser-fixtures/local-storage.html exercises localStorage setItem/getItem/length and DOM text mutation.",
            required_markers: &[
                "localStorage.setItem",
                "localStorage.getItem",
                "localStorage.length",
            ],
        },
        ReadinessFileSpec {
            path: "bench/browser-fixtures/session-storage.html",
            summary: "bench/browser-fixtures/session-storage.html exercises sessionStorage setItem/getItem/length and DOM text mutation.",
            required_markers: &[
                "sessionStorage.setItem",
                "sessionStorage.getItem",
                "sessionStorage.length",
            ],
        },
        ReadinessFileSpec {
            path: "bench/browser-fixtures/timer-task-queue.html",
            summary: "bench/browser-fixtures/timer-task-queue.html exercises setTimeout queue ordering and DOM text mutation.",
            required_markers: &[
                "setTimeout",
                "textContent += \" sync\"",
                "textContent += \" timer\"",
            ],
        },
        ReadinessFileSpec {
            path: "bench/browser-fixtures/lifecycle-events.html",
            summary: "bench/browser-fixtures/lifecycle-events.html exercises document DOMContentLoaded and window load listeners, load-handler timer scheduling, and DOM text mutation ordering.",
            required_markers: &[
                "DOMContentLoaded",
                "window.addEventListener",
                "setTimeout",
                "textContent += \" load\"",
            ],
        },
        ReadinessFileSpec {
            path: "bench/browser-fixtures/external-script-page.html",
            summary: "bench/browser-fixtures/external-script-page.html proves external script loading participates in scripted render fixtures.",
            required_markers: &["<script src=\"external-script.js\">"],
        },
    ]);

    let areas = vec![
        ReadinessAreaReport {
            area: "Plan Coverage".to_owned(),
            claim_scopes: readiness_claim_scopes(&["Search", "Browser"]),
            required_end_state:
                "Every search and browser product claim maps to an explicit workstream, exit gate, and completion audit item."
                    .to_owned(),
            evidence_gate:
                "docs/PROGRAM_PLAN.md completion matrix and docs/COMPETITOR_ROADMAP.md completion definition stay in sync with CLI gates."
                    .to_owned(),
            status: if plan_docs.passed && traceability_matrix.passed && evidence_registry.passed {
                Implemented
            } else {
                Missing
            },
            current_evidence: merge_current_evidence(
                merge_current_evidence(plan_docs.current_evidence, traceability_matrix.current_evidence),
                evidence_registry.current_evidence,
            ),
            missing_work: merge_missing_work(
                merge_missing_work(
                    merge_missing_work(plan_docs.missing_work, traceability_matrix.missing_work),
                    evidence_registry.missing_work,
                ),
                vec!["Keep this readiness command wired into release/CI policy before any public product claim.".to_owned()],
            ),
        },
        ReadinessAreaReport {
            area: "Search Crawling And Freshness".to_owned(),
            claim_scopes: readiness_claim_scopes(&["Search"]),
            required_end_state:
                "Continuous polite web crawl with robots, sitemap discovery, recrawl scheduling, change detection, and failure recovery."
                    .to_owned(),
            evidence_gate:
                "Large replayable crawl reports with host stats, freshness lag, changed/unchanged/missing counts, retry accounting, and crash recovery proof."
                    .to_owned(),
            status: partial_status(&crawling_artifacts),
            current_evidence: crawling_artifacts.current_evidence,
            missing_work: merge_missing_work(
                crawling_artifacts.missing_work,
                vec![
                "Continuous production crawl service and large web-scale crawl reports.".to_owned(),
                "Freshness SLOs, failure injection, and long-running recovery evidence.".to_owned(),
                ],
            ),
        },
        ReadinessAreaReport {
            area: "Search Indexing And Storage".to_owned(),
            claim_scopes: readiness_claim_scopes(&["Search"]),
            required_end_state:
                "Incremental, sharded, durable index builds with corruption checks, rollback, replication, and shard health."
                    .to_owned(),
            evidence_gate:
                "Index integrity verifier, shard fanout tests, rollback drills, corpus/index hashes, and recovery benchmarks."
                    .to_owned(),
            status: partial_status(&indexing_artifacts),
            current_evidence: indexing_artifacts.current_evidence,
            missing_work: merge_missing_work(
                indexing_artifacts.missing_work,
                vec![
                "Distributed sharding, replication, incremental merge policy, and rollback tooling.".to_owned(),
                "Corrupt-index recovery and multi-shard query fanout gates.".to_owned(),
                ],
            ),
        },
        ReadinessAreaReport {
            area: "Search Relevance And Quality".to_owned(),
            claim_scopes: readiness_claim_scopes(&["Search"]),
            required_end_state:
                "Ranking with link analysis, field weighting, dedupe, spam/quality controls, freshness, query understanding, and judged-query regression gates."
                    .to_owned(),
            evidence_gate:
                "Large judged query suites with NDCG/MRR/recall/precision, spam metrics, duplicate metrics, and regression thresholds."
                    .to_owned(),
            status: partial_status(&relevance_artifacts),
            current_evidence: relevance_artifacts.current_evidence,
            missing_work: merge_missing_work(
                relevance_artifacts.missing_work,
                vec![
                "Large representative judged query set and editorial/replay tooling.".to_owned(),
                "Spam, abuse, freshness, personalization/privacy, and learning-to-rank evaluation.".to_owned(),
                ],
            ),
        },
        ReadinessAreaReport {
            area: "Search Serving And Product UX".to_owned(),
            claim_scopes: readiness_claim_scopes(&["Search"]),
            required_end_state:
                "Low-latency daemon/shard serving plus user-facing Google-style search workflows, filters, snippets, render views, status, and benchmarks."
                    .to_owned(),
            evidence_gate:
                "API/UI integration tests, load tests, p50/p95/p99 dashboards, usability fixtures, and production-like smoke runs."
                    .to_owned(),
            status: partial_status(&serving_artifacts),
            current_evidence: serving_artifacts.current_evidence,
            missing_work: merge_missing_work(
                serving_artifacts.missing_work,
                vec![
                "Distributed serving, production query logs/privacy controls, UX test suite, and high-load dashboards.".to_owned(),
                "Full web-scale result product semantics and abuse-resistant public endpoints.".to_owned(),
                ],
            ),
        },
        ReadinessAreaReport {
            area: "Browser Engine".to_owned(),
            claim_scopes: readiness_claim_scopes(&["Browser"]),
            required_end_state:
                "Independent network, parser, DOM, CSS cascade/layout, paint/raster, compositor, navigation, cache, and shell capable of modern pages."
                    .to_owned(),
            evidence_gate:
                "Standards subsets, curated page set, screenshot/visual regressions, layout/paint benchmarks, and Chrome parity reports."
                    .to_owned(),
            status: partial_status(&browser_artifacts),
            current_evidence: merge_current_evidence(
                vec![
                format!(
                    "brutal-browser coverage reports {}/{} implemented features.",
                    browser_coverage.implemented_count, browser_coverage.feature_count
                ),
                ],
                browser_artifacts.current_evidence,
            ),
            missing_work: merge_missing_work(
                browser_artifacts.missing_work,
                vec![
                "Full CSS cascade/layout, paint/raster, compositor, fonts, images, navigation semantics, GUI browser shell/browser chrome, OS windowing, tab/process isolation, accessibility, devtools, and Chromium parity remain outside the local CLI shell subset.".to_owned(),
                "URL-encoded POST form submission through BrowserSession/CLI is implemented only as the narrow browser-session-urlencoded-post-form-submit gate; broad POST form submission remains missing.".to_owned(),
                "Form submit-control click default action through BrowserSession/CLI is implemented only as the narrow browser-session-form-submit-button-click-default gate; full form events, validation, focus/input behavior, and UI remain missing.".to_owned(),
                "Form reset-control click default action through BrowserSession/CLI is implemented only as the narrow browser-session-form-reset-click-default gate; full form events, validation, focus/input behavior, and UI remain missing.".to_owned(),
                "Screenshot gates and large curated modern-page compatibility suite.".to_owned(),
                ],
            ),
        },
        ReadinessAreaReport {
            area: "JavaScript And Web APIs".to_owned(),
            claim_scopes: readiness_claim_scopes(&["Search", "Browser"]),
            required_end_state:
                "JIT-class JavaScript strategy, event loop, DOM/Web IDL bindings, timers, fetch, modules, workers, storage APIs, and error handling."
                    .to_owned(),
            evidence_gate:
                "Web Platform Test subsets, JS benchmarks, API compatibility matrix, timeout/sandbox tests, and rendered extraction parity."
                    .to_owned(),
            status: partial_status(&js_artifacts),
            current_evidence: js_artifacts.current_evidence,
            missing_work: merge_missing_work(
                js_artifacts.missing_work,
                vec![
                "General JavaScript parser/VM/JIT or embedded engine strategy.".to_owned(),
                "Event loop, promises, modules, fetch/XHR, workers, DOM/Web API bindings, and compatibility tests.".to_owned(),
                ],
            ),
        },
        ReadinessAreaReport {
            area: "Security And Privacy".to_owned(),
            claim_scopes: readiness_claim_scopes(&["Search", "Browser"]),
            required_end_state:
                "Origin model, sandboxing/site isolation, CSP, mixed-content policy, permissions, privacy controls, abuse controls, and legal/compliance workflow."
                    .to_owned(),
            evidence_gate:
                "Threat model, sandbox tests, fuzzing, permission/privacy conformance, abuse tests, and policy review gates."
                    .to_owned(),
            status: if security_docs.passed { Partial } else { Missing },
            current_evidence: security_docs.current_evidence,
            missing_work: merge_missing_work(
                security_docs.missing_work,
                vec![
                "Process sandbox, site isolation, and origin/security policy enforcement.".to_owned(),
                "Privacy controls, abuse controls, compliance workflow, fuzzing, and passing sandbox/policy gates.".to_owned(),
                ],
            ),
        },
        ReadinessAreaReport {
            area: "Platform Completeness".to_owned(),
            claim_scopes: readiness_claim_scopes(&["Browser"]),
            required_end_state:
                "Fonts/text shaping, images, media, canvas/WebGL/WebGPU, accessibility tree, devtools, extensions policy, packaging, and updates."
                    .to_owned(),
            evidence_gate:
                "Feature coverage gates, platform smoke suites, accessibility audits, media/canvas benchmarks, devtools tests, and signed packages."
                    .to_owned(),
            status: if platform_docs.passed { Partial } else { Missing },
            current_evidence: platform_docs.current_evidence,
            missing_work: merge_missing_work(
                platform_docs.missing_work,
                vec![
                "Implemented platform subsystems for fonts/text shaping, images/SVG, canvas/GPU, media, accessibility, input/editing, storage/profiles, downloads, devtools, extensions, packaging, and updates.".to_owned(),
                "Passing platform smoke suites, WPT subsets, visual regression, accessibility audits, media/canvas benchmarks, signed packages, and platform QA.".to_owned(),
                ],
            ),
        },
        ReadinessAreaReport {
            area: "Operations And Reliability".to_owned(),
            claim_scopes: readiness_claim_scopes(&["Search", "Browser"]),
            required_end_state:
                "Observability, crash recovery, deploy/rollback, backups, incident response, cost controls, SLOs, and failure drills."
                    .to_owned(),
            evidence_gate:
                "SLO dashboards, logs/traces/metrics, failure-injection tests, restore drills, load tests, and deployment runbooks."
                    .to_owned(),
            status: if operations_docs.passed { Partial } else { Missing },
            current_evidence: operations_docs.current_evidence,
            missing_work: merge_missing_work(
                operations_docs.missing_work,
                vec![
                "Implemented production deployment topology, metrics/logs/traces, dashboards, alerting, backups, and restore drills.".to_owned(),
                "Failure-injection automation, load tests, release manifests, rollback drills, cost controls, and incident runbooks.".to_owned(),
                ],
            ),
        },
    ];

    let implemented_count = areas
        .iter()
        .filter(|area| area.status == Implemented)
        .count();
    let partial_count = areas.iter().filter(|area| area.status == Partial).count();
    let missing_count = areas.iter().filter(|area| area.status == Missing).count();
    let area_count = areas.len();

    CompetitorReadinessReport {
        passed: missing_count == 0 && partial_count == 0,
        claim: "combined_competitor".to_owned(),
        required_claim_scopes: readiness_claim_scopes(&["Search", "Browser"]),
        standard: ReadinessClaim::CombinedCompetitor.standard().to_owned(),
        area_count,
        implemented_count,
        partial_count,
        missing_count,
        areas,
    }
}

fn apply_readiness_claim(report: &mut CompetitorReadinessReport, claim: ReadinessClaim) {
    let required_scopes = claim.required_scopes();
    report.areas.retain(|area| {
        area.claim_scopes
            .iter()
            .any(|scope| required_scopes.contains(&scope.as_str()))
    });
    report.implemented_count = report
        .areas
        .iter()
        .filter(|area| area.status == ReadinessStatus::Implemented)
        .count();
    report.partial_count = report
        .areas
        .iter()
        .filter(|area| area.status == ReadinessStatus::Partial)
        .count();
    report.missing_count = report
        .areas
        .iter()
        .filter(|area| area.status == ReadinessStatus::Missing)
        .count();
    report.area_count = report.areas.len();
    report.passed = report.missing_count == 0 && report.partial_count == 0;
    report.claim = claim.id().to_owned();
    report.required_claim_scopes = readiness_claim_scopes(required_scopes);
    report.standard = claim.standard().to_owned();
}

fn readiness_claim_scopes(scopes: &[&str]) -> Vec<String> {
    scopes.iter().map(|scope| (*scope).to_owned()).collect()
}

struct ReadinessFileSpec {
    path: &'static str,
    summary: &'static str,
    required_markers: &'static [&'static str],
}

struct ReadinessEvidence {
    passed: bool,
    current_evidence: Vec<String>,
    missing_work: Vec<String>,
}

const REQUIRED_TRACEABILITY_IDS: &[&str] = &[
    "REQ-SEARCH-CORPUS",
    "REQ-SEARCH-EXTRACTION",
    "REQ-SEARCH-RENDERED-EXTRACTION",
    "REQ-SEARCH-INDEX-STORAGE",
    "REQ-SEARCH-QUERY-SERVING",
    "REQ-SEARCH-RELEVANCE",
    "REQ-SEARCH-PRODUCT",
    "REQ-SEARCH-PRIVACY-ABUSE",
    "REQ-BROWSER-ENGINE",
    "REQ-BROWSER-NETWORKING",
    "REQ-BROWSER-PARSER-DOM",
    "REQ-BROWSER-CSS-LAYOUT",
    "REQ-BROWSER-PAINT-COMPOSITOR",
    "REQ-BROWSER-JS-WEB-APIS",
    "REQ-BROWSER-PLATFORM",
    "REQ-BROWSER-SHELL-DISTRIBUTION",
    "REQ-BROWSER-SECURITY",
    "REQ-BENCHMARKS-STANDARDS",
    "REQ-OPERATIONS-RELIABILITY",
    "REQ-GOVERNANCE-CLAIMS",
];

const TRACEABILITY_READINESS_AREAS: &[&str] = &[
    "Plan Coverage",
    "Search Crawling And Freshness",
    "Search Indexing And Storage",
    "Search Relevance And Quality",
    "Search Serving And Product UX",
    "Browser Engine",
    "JavaScript And Web APIs",
    "Security And Privacy",
    "Platform Completeness",
    "Operations And Reliability",
];

const TRACEABILITY_MILESTONES: &[&str] = &["M0", "M1", "M2", "M3", "M4", "M5", "M6"];

const TRACEABILITY_CLAIM_SCOPES: &[&str] = &["Search", "Browser", "Shared"];

pub fn run_evidence_registry_report() -> EvidenceRegistryReport {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("docs/EVIDENCE_REGISTRY.md");
    let required = REQUIRED_TRACEABILITY_IDS
        .iter()
        .copied()
        .collect::<BTreeSet<_>>();
    let text = match fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) => {
            return EvidenceRegistryReport {
                passed: false,
                row_count: 0,
                covered_requirement_count: 0,
                missing_required_ids: REQUIRED_TRACEABILITY_IDS
                    .iter()
                    .map(|id| (*id).to_owned())
                    .collect(),
                unknown_requirement_ids: Vec::new(),
                duplicate_evidence_ids: Vec::new(),
                validation_errors: vec![format!(
                    "docs/EVIDENCE_REGISTRY.md is missing or unreadable: {error}"
                )],
                rows: Vec::new(),
            };
        }
    };

    let mut rows = Vec::new();
    let mut covered_requirement_ids = BTreeSet::new();
    let mut unknown_requirement_ids = BTreeSet::new();
    let mut evidence_counts = BTreeMap::<String, usize>::new();
    let mut validation_errors = Vec::new();

    for (line_no, line) in text.lines().enumerate() {
        let trimmed = line.trim();
        if !trimmed.starts_with("| EV-") {
            continue;
        }
        let columns = trimmed
            .trim_matches('|')
            .split('|')
            .map(str::trim)
            .collect::<Vec<_>>();
        if columns.len() != 6 {
            validation_errors.push(format!(
                "docs/EVIDENCE_REGISTRY.md line {} has {} columns; expected 6",
                line_no + 1,
                columns.len()
            ));
            continue;
        }

        let evidence_id = columns[0].to_owned();
        *evidence_counts.entry(evidence_id.clone()).or_default() += 1;
        for (offset, value) in columns.iter().enumerate() {
            if value.is_empty() {
                validation_errors.push(format!(
                    "docs/EVIDENCE_REGISTRY.md row {evidence_id} has an empty column {}",
                    offset + 1
                ));
            }
            if value.contains("TODO") || value.contains("TBD") {
                validation_errors.push(format!(
                    "docs/EVIDENCE_REGISTRY.md row {evidence_id} still contains a placeholder"
                ));
            }
        }

        let requirement_ids = parse_evidence_requirement_ids(columns[1]);
        if requirement_ids.is_empty() {
            validation_errors.push(format!(
                "docs/EVIDENCE_REGISTRY.md row {evidence_id} maps no requirement IDs"
            ));
        }
        for requirement_id in &requirement_ids {
            if required.contains(requirement_id.as_str()) {
                covered_requirement_ids.insert(requirement_id.clone());
            } else {
                unknown_requirement_ids.insert(requirement_id.clone());
            }
        }

        rows.push(EvidenceRegistryRow {
            evidence_id,
            requirement_ids,
            required_for: columns[2].to_owned(),
            artifact_or_command: columns[3].to_owned(),
            produces: columns[4].to_owned(),
            completion_standard: columns[5].to_owned(),
        });
    }

    if rows.is_empty() {
        validation_errors
            .push("docs/EVIDENCE_REGISTRY.md contains no EV-* evidence rows".to_owned());
    }

    let missing_required_ids = required
        .iter()
        .filter(|id| !covered_requirement_ids.contains(**id))
        .map(|id| (*id).to_owned())
        .collect::<Vec<_>>();
    if !missing_required_ids.is_empty() {
        validation_errors.push(format!(
            "docs/EVIDENCE_REGISTRY.md is missing evidence coverage for requirements: {}",
            missing_required_ids.join(", ")
        ));
    }

    let unknown_requirement_ids = unknown_requirement_ids.into_iter().collect::<Vec<_>>();
    if !unknown_requirement_ids.is_empty() {
        validation_errors.push(format!(
            "docs/EVIDENCE_REGISTRY.md has unknown requirement IDs: {}",
            unknown_requirement_ids.join(", ")
        ));
    }

    let duplicate_evidence_ids = evidence_counts
        .iter()
        .filter_map(|(id, count)| (*count > 1).then_some(id.clone()))
        .collect::<Vec<_>>();
    if !duplicate_evidence_ids.is_empty() {
        validation_errors.push(format!(
            "docs/EVIDENCE_REGISTRY.md has duplicate evidence rows: {}",
            duplicate_evidence_ids.join(", ")
        ));
    }

    EvidenceRegistryReport {
        passed: validation_errors.is_empty(),
        row_count: rows.len(),
        covered_requirement_count: covered_requirement_ids.len(),
        missing_required_ids,
        unknown_requirement_ids,
        duplicate_evidence_ids,
        validation_errors,
        rows,
    }
}

fn parse_evidence_requirement_ids(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(|part| part.trim().trim_matches('`').trim().to_owned())
        .filter(|part| !part.is_empty())
        .collect()
}

pub fn run_traceability_report() -> TraceabilityReport {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("docs/REQUIREMENTS_TRACEABILITY.md");
    let text = match fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) => {
            return TraceabilityReport {
                passed: false,
                complete: false,
                required_count: REQUIRED_TRACEABILITY_IDS.len(),
                row_count: 0,
                implemented_count: 0,
                partial_count: 0,
                missing_count: 0,
                missing_required_ids: REQUIRED_TRACEABILITY_IDS
                    .iter()
                    .map(|id| (*id).to_owned())
                    .collect(),
                unknown_ids: Vec::new(),
                unknown_readiness_areas: Vec::new(),
                unknown_milestones: Vec::new(),
                unknown_claim_scopes: Vec::new(),
                mismatched_claim_scopes: Vec::new(),
                duplicate_ids: Vec::new(),
                validation_errors: vec![format!(
                    "docs/REQUIREMENTS_TRACEABILITY.md is missing or unreadable: {error}"
                )],
                rows_by_readiness_area: BTreeMap::new(),
                rows_by_milestone: BTreeMap::new(),
                rows_by_claim_scope: BTreeMap::new(),
                claim_reports: Vec::new(),
                milestone_reports: Vec::new(),
                requirements: Vec::new(),
            };
        }
    };

    let mut validation_errors = Vec::new();
    let mut requirements = Vec::new();
    for (line_no, line) in text.lines().enumerate() {
        let trimmed = line.trim();
        if !trimmed.starts_with("| REQ-") {
            continue;
        }
        let columns = trimmed
            .trim_matches('|')
            .split('|')
            .map(str::trim)
            .collect::<Vec<_>>();
        if columns.len() != 9 {
            validation_errors.push(format!(
                "docs/REQUIREMENTS_TRACEABILITY.md line {} has {} columns; expected 9",
                line_no + 1,
                columns.len()
            ));
            continue;
        }
        let id = columns[0].to_owned();
        for (offset, value) in columns.iter().enumerate() {
            if value.is_empty() {
                validation_errors.push(format!(
                    "docs/REQUIREMENTS_TRACEABILITY.md row {id} has an empty column {}",
                    offset + 1
                ));
            }
            if value.contains("TODO") || value.contains("TBD") {
                validation_errors.push(format!(
                    "docs/REQUIREMENTS_TRACEABILITY.md row {id} still contains a placeholder"
                ));
            }
        }
        let Some(current_state) = parse_traceability_status(columns[8]) else {
            validation_errors.push(format!(
                "docs/REQUIREMENTS_TRACEABILITY.md row {id} has invalid Current state {:?}",
                columns[8]
            ));
            continue;
        };
        requirements.push(TraceabilityRequirement {
            id,
            required_capability: columns[1].to_owned(),
            necessary_steps: columns[2].to_owned(),
            primary_gates: columns[3].to_owned(),
            plan_owners: columns[4].to_owned(),
            readiness_area: columns[5].to_owned(),
            milestone: columns[6].to_owned(),
            claim_scope: columns[7].to_owned(),
            current_state,
        });
    }

    let required = REQUIRED_TRACEABILITY_IDS
        .iter()
        .copied()
        .collect::<BTreeSet<_>>();
    let known_areas = TRACEABILITY_READINESS_AREAS
        .iter()
        .copied()
        .collect::<BTreeSet<_>>();
    let known_milestones = TRACEABILITY_MILESTONES
        .iter()
        .copied()
        .collect::<BTreeSet<_>>();
    let known_claim_scopes = TRACEABILITY_CLAIM_SCOPES
        .iter()
        .copied()
        .collect::<BTreeSet<_>>();
    let mut seen = BTreeSet::new();
    let mut seen_areas = BTreeSet::new();
    let mut seen_milestones = BTreeSet::new();
    let mut seen_claim_scopes = BTreeSet::new();
    let mut counts_by_id = BTreeMap::<String, usize>::new();
    let mut rows_by_readiness_area = BTreeMap::<String, usize>::new();
    let mut rows_by_milestone = BTreeMap::<String, usize>::new();
    let mut rows_by_claim_scope = BTreeMap::<String, usize>::new();
    let mut mismatched_claim_scopes = Vec::new();
    let mut implemented = 0usize;
    let mut partial = 0usize;
    let mut missing = 0usize;

    for requirement in &requirements {
        seen.insert(requirement.id.as_str());
        seen_areas.insert(requirement.readiness_area.as_str());
        seen_milestones.insert(requirement.milestone.as_str());
        seen_claim_scopes.insert(requirement.claim_scope.as_str());
        *counts_by_id.entry(requirement.id.clone()).or_default() += 1;
        *rows_by_readiness_area
            .entry(requirement.readiness_area.clone())
            .or_default() += 1;
        *rows_by_milestone
            .entry(requirement.milestone.clone())
            .or_default() += 1;
        *rows_by_claim_scope
            .entry(requirement.claim_scope.clone())
            .or_default() += 1;
        if let Some(expected) = expected_traceability_claim_scope(&requirement.id)
            && requirement.claim_scope != expected
        {
            mismatched_claim_scopes.push(format!(
                "{} expected {} got {}",
                requirement.id, expected, requirement.claim_scope
            ));
        }
        match requirement.current_state {
            ReadinessStatus::Implemented => implemented += 1,
            ReadinessStatus::Partial => partial += 1,
            ReadinessStatus::Missing => missing += 1,
        }
    }

    let missing_required_ids = required
        .difference(&seen)
        .map(|id| (*id).to_owned())
        .collect::<Vec<_>>();
    if !missing_required_ids.is_empty() {
        validation_errors.push(format!(
            "docs/REQUIREMENTS_TRACEABILITY.md is missing requirement rows: {}",
            missing_required_ids.join(", ")
        ));
    }

    let unknown_ids = seen
        .difference(&required)
        .map(|id| (*id).to_owned())
        .collect::<Vec<_>>();
    if !unknown_ids.is_empty() {
        validation_errors.push(format!(
            "docs/REQUIREMENTS_TRACEABILITY.md has unknown requirement rows: {}",
            unknown_ids.join(", ")
        ));
    }

    let unknown_readiness_areas = seen_areas
        .difference(&known_areas)
        .map(|area| (*area).to_owned())
        .collect::<Vec<_>>();
    if !unknown_readiness_areas.is_empty() {
        validation_errors.push(format!(
            "docs/REQUIREMENTS_TRACEABILITY.md has unknown readiness areas: {}",
            unknown_readiness_areas.join(", ")
        ));
    }

    let unknown_milestones = seen_milestones
        .difference(&known_milestones)
        .map(|milestone| (*milestone).to_owned())
        .collect::<Vec<_>>();
    if !unknown_milestones.is_empty() {
        validation_errors.push(format!(
            "docs/REQUIREMENTS_TRACEABILITY.md has unknown milestones: {}",
            unknown_milestones.join(", ")
        ));
    }

    let unknown_claim_scopes = seen_claim_scopes
        .difference(&known_claim_scopes)
        .map(|scope| (*scope).to_owned())
        .collect::<Vec<_>>();
    if !unknown_claim_scopes.is_empty() {
        validation_errors.push(format!(
            "docs/REQUIREMENTS_TRACEABILITY.md has unknown claim scopes: {}",
            unknown_claim_scopes.join(", ")
        ));
    }
    if !mismatched_claim_scopes.is_empty() {
        validation_errors.push(format!(
            "docs/REQUIREMENTS_TRACEABILITY.md has mismatched claim scopes: {}",
            mismatched_claim_scopes.join(", ")
        ));
    }

    let duplicate_ids = counts_by_id
        .iter()
        .filter_map(|(id, count)| (*count > 1).then_some(id.clone()))
        .collect::<Vec<_>>();
    if !duplicate_ids.is_empty() {
        validation_errors.push(format!(
            "docs/REQUIREMENTS_TRACEABILITY.md has duplicate requirement rows: {}",
            duplicate_ids.join(", ")
        ));
    }

    let passed = validation_errors.is_empty();
    let complete = passed && partial == 0 && missing == 0;
    let claim_reports = traceability_claim_reports(&requirements, passed);
    let milestone_reports = traceability_milestone_reports(&requirements, passed);

    TraceabilityReport {
        passed,
        complete,
        required_count: REQUIRED_TRACEABILITY_IDS.len(),
        row_count: requirements.len(),
        implemented_count: implemented,
        partial_count: partial,
        missing_count: missing,
        missing_required_ids,
        unknown_ids,
        unknown_readiness_areas,
        unknown_milestones,
        unknown_claim_scopes,
        mismatched_claim_scopes,
        duplicate_ids,
        validation_errors,
        rows_by_readiness_area,
        rows_by_milestone,
        rows_by_claim_scope,
        claim_reports,
        milestone_reports,
        requirements,
    }
}

fn traceability_claim_reports(
    requirements: &[TraceabilityRequirement],
    validation_passed: bool,
) -> Vec<TraceabilityClaimReport> {
    [
        (
            "google_style_search",
            vec!["Search".to_owned(), "Shared".to_owned()],
        ),
        (
            "chromium_class_browser",
            vec!["Browser".to_owned(), "Shared".to_owned()],
        ),
        (
            "combined_competitor",
            vec![
                "Search".to_owned(),
                "Browser".to_owned(),
                "Shared".to_owned(),
            ],
        ),
    ]
    .into_iter()
    .map(|(claim, required_claim_scopes)| {
        let mut implemented_ids = Vec::new();
        let mut partial_ids = Vec::new();
        let mut missing_ids = Vec::new();

        for requirement in requirements {
            if !required_claim_scopes.contains(&requirement.claim_scope) {
                continue;
            }
            match requirement.current_state {
                ReadinessStatus::Implemented => implemented_ids.push(requirement.id.clone()),
                ReadinessStatus::Partial => partial_ids.push(requirement.id.clone()),
                ReadinessStatus::Missing => missing_ids.push(requirement.id.clone()),
            }
        }

        let implemented_count = implemented_ids.len();
        let partial_count = partial_ids.len();
        let missing_count = missing_ids.len();
        TraceabilityClaimReport {
            claim: claim.to_owned(),
            required_claim_scopes,
            requirement_count: implemented_count + partial_count + missing_count,
            implemented_count,
            partial_count,
            missing_count,
            complete: validation_passed && partial_count == 0 && missing_count == 0,
            implemented_ids,
            partial_ids,
            missing_ids,
        }
    })
    .collect()
}

fn traceability_milestone_reports(
    requirements: &[TraceabilityRequirement],
    validation_passed: bool,
) -> Vec<TraceabilityMilestoneReport> {
    TRACEABILITY_MILESTONES
        .iter()
        .enumerate()
        .map(|(milestone_index, milestone)| {
            let included_milestones = TRACEABILITY_MILESTONES
                .iter()
                .take(milestone_index + 1)
                .map(|milestone| (*milestone).to_owned())
                .collect::<Vec<_>>();
            let mut implemented_ids = Vec::new();
            let mut partial_ids = Vec::new();
            let mut missing_ids = Vec::new();

            for requirement in requirements {
                let Some(requirement_milestone_index) =
                    traceability_milestone_index(&requirement.milestone)
                else {
                    continue;
                };
                if requirement_milestone_index > milestone_index {
                    continue;
                }
                match requirement.current_state {
                    ReadinessStatus::Implemented => implemented_ids.push(requirement.id.clone()),
                    ReadinessStatus::Partial => partial_ids.push(requirement.id.clone()),
                    ReadinessStatus::Missing => missing_ids.push(requirement.id.clone()),
                }
            }

            let implemented_count = implemented_ids.len();
            let partial_count = partial_ids.len();
            let missing_count = missing_ids.len();
            TraceabilityMilestoneReport {
                milestone: (*milestone).to_owned(),
                included_milestones,
                requirement_count: implemented_count + partial_count + missing_count,
                implemented_count,
                partial_count,
                missing_count,
                complete: validation_passed && partial_count == 0 && missing_count == 0,
                implemented_ids,
                partial_ids,
                missing_ids,
            }
        })
        .collect()
}

fn traceability_milestone_index(milestone: &str) -> Option<usize> {
    TRACEABILITY_MILESTONES
        .iter()
        .position(|known_milestone| *known_milestone == milestone)
}

fn expected_traceability_claim_scope(id: &str) -> Option<&'static str> {
    if id.starts_with("REQ-SEARCH-") {
        Some("Search")
    } else if id.starts_with("REQ-BROWSER-") {
        Some("Browser")
    } else if matches!(
        id,
        "REQ-BENCHMARKS-STANDARDS" | "REQ-OPERATIONS-RELIABILITY" | "REQ-GOVERNANCE-CLAIMS"
    ) {
        Some("Shared")
    } else {
        None
    }
}

fn readiness_traceability_evidence() -> ReadinessEvidence {
    let report = run_traceability_report();
    let missing_state_ids = report
        .requirements
        .iter()
        .filter(|requirement| requirement.current_state == ReadinessStatus::Missing)
        .map(|requirement| requirement.id.clone())
        .collect::<Vec<_>>();
    let mut current_evidence = vec![format!(
        "docs/REQUIREMENTS_TRACEABILITY.md contains {} required requirement rows: {} implemented, {} partial, {} missing current states.",
        report.row_count, report.implemented_count, report.partial_count, report.missing_count
    )];
    if !report.rows_by_claim_scope.is_empty() {
        let claim_scope_summary = report
            .rows_by_claim_scope
            .iter()
            .map(|(scope, count)| format!("{scope}={count}"))
            .collect::<Vec<_>>()
            .join(", ");
        current_evidence.push(format!(
            "Traceability release claim scopes are mapped as: {claim_scope_summary}."
        ));
    }
    if !missing_state_ids.is_empty() {
        current_evidence.push(format!(
            "Traceability matrix intentionally records missing implementation evidence for: {}.",
            missing_state_ids.join(", ")
        ));
    }

    ReadinessEvidence {
        passed: report.passed,
        current_evidence,
        missing_work: report.validation_errors,
    }
}

fn readiness_evidence_registry_evidence() -> ReadinessEvidence {
    let report = run_evidence_registry_report();
    let mut current_evidence = vec![format!(
        "docs/EVIDENCE_REGISTRY.md maps {} evidence rows covering {} required requirement IDs.",
        report.row_count, report.covered_requirement_count
    )];
    if !report.missing_required_ids.is_empty() {
        current_evidence.push(format!(
            "Evidence registry is missing requirement coverage for: {}.",
            report.missing_required_ids.join(", ")
        ));
    }
    if !report.unknown_requirement_ids.is_empty() {
        current_evidence.push(format!(
            "Evidence registry includes unknown requirement IDs: {}.",
            report.unknown_requirement_ids.join(", ")
        ));
    }

    ReadinessEvidence {
        passed: report.passed,
        current_evidence,
        missing_work: report.validation_errors,
    }
}

fn parse_traceability_status(value: &str) -> Option<ReadinessStatus> {
    match value {
        "Implemented" => Some(ReadinessStatus::Implemented),
        "Partial" => Some(ReadinessStatus::Partial),
        "Missing" => Some(ReadinessStatus::Missing),
        _ => None,
    }
}

fn readiness_file_evidence(specs: &[ReadinessFileSpec]) -> ReadinessEvidence {
    let mut current_evidence = Vec::new();
    let mut missing_work = Vec::new();

    for spec in specs {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(spec.path);
        let text = match fs::read_to_string(&path) {
            Ok(text) => text,
            Err(error) => {
                missing_work.push(format!("{} is missing or unreadable: {error}", spec.path));
                continue;
            }
        };

        let missing_markers: Vec<&str> = spec
            .required_markers
            .iter()
            .copied()
            .filter(|marker| !text.contains(marker))
            .collect();
        if missing_markers.is_empty() {
            current_evidence.push(format!(
                "{} exists and contains {} required evidence markers. {}",
                spec.path,
                spec.required_markers.len(),
                spec.summary
            ));
        } else {
            missing_work.push(format!(
                "{} is missing required readiness markers: {}",
                spec.path,
                missing_markers.join(", ")
            ));
        }
    }

    ReadinessEvidence {
        passed: missing_work.is_empty(),
        current_evidence,
        missing_work,
    }
}

fn partial_status(evidence: &ReadinessEvidence) -> ReadinessStatus {
    if evidence.passed {
        ReadinessStatus::Partial
    } else {
        ReadinessStatus::Missing
    }
}

fn merge_current_evidence(mut first: Vec<String>, second: Vec<String>) -> Vec<String> {
    first.extend(second);
    first
}

fn merge_missing_work(mut first: Vec<String>, second: Vec<String>) -> Vec<String> {
    first.extend(second);
    first
}

pub async fn run_search_bench(options: BenchOptions) -> Result<BenchReport> {
    let queries = read_queries(&options.queries)?;
    anyhow::ensure!(!queries.is_empty(), "query file is empty");

    if options.use_daemon {
        run_daemon_bench(options, queries).await
    } else {
        run_in_process_bench(options, queries)
    }
}

pub async fn run_search_comparison(options: BenchOptions) -> Result<BenchComparison> {
    let rust = run_search_bench(options.clone()).await?;
    let chromium = run_chromium_search_bench(&options)?;
    let p95_speedup = chromium.p95_us as f64 / rust.p95_us.max(1) as f64;
    Ok(BenchComparison {
        rust,
        chromium,
        p95_speedup,
        required_p95_speedup: None,
        passed: None,
    })
}

pub fn run_chromium_search_bench(options: &BenchOptions) -> Result<BenchReport> {
    let chrome = chrome_program().context("Chrome/Chromium executable not found")?;
    let queries = read_queries(&options.queries)?;
    anyhow::ensure!(!queries.is_empty(), "query file is empty");

    let index = SearchIndex::open(&options.index, PreloadMode::Aggressive)?;
    let data = build_chromium_data(&index, &queries, options.limit, options.warmup)?;
    let html = chromium_baseline_html(&data)?;
    let path = std::env::temp_dir().join(format!(
        "brutal-search-chromium-baseline-{}.html",
        std::process::id()
    ));
    fs::write(&path, html)?;

    let output = Command::new(chrome)
        .arg("--headless=new")
        .arg("--disable-gpu")
        .arg("--disable-background-networking")
        .arg("--disable-default-apps")
        .arg("--disable-extensions")
        .arg("--run-all-compositor-stages-before-draw")
        .arg("--dump-dom")
        .arg(format!("file://{}", path.display()))
        .output()
        .with_context(|| format!("run Chromium baseline {}", path.display()))?;
    let _ = fs::remove_file(&path);

    anyhow::ensure!(
        output.status.success(),
        "Chromium baseline failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    let json = extract_json_object(&stdout).context("Chromium output did not contain JSON")?;
    let mut report: BenchReport = serde_json::from_str(json)?;
    report.chrome = chrome_version();
    report.rustc = command_output("rustc", &["--version"]);
    report.os = command_output("uname", &["-a"]);
    report.hardware = hardware_summary();
    report.index_hash = index_hash(&options.index).unwrap_or_else(|_| "unknown".to_owned());
    Ok(report)
}

async fn run_daemon_bench(options: BenchOptions, queries: Vec<String>) -> Result<BenchReport> {
    let socket = options
        .socket
        .unwrap_or_else(|| default_socket_path(&options.index));

    let response = send_request(
        &socket,
        &DaemonRequest::BenchSearch {
            queries,
            limit: options.limit,
            warmup: options.warmup,
        },
    )
    .await?;
    let (timings, elapsed) = match response {
        DaemonResponse::BenchSearch {
            timings_us,
            total_us,
        } => (
            timings_us.into_iter().map(Duration::from_micros).collect(),
            Duration::from_micros(total_us),
        ),
        DaemonResponse::Error { message } => anyhow::bail!(message),
        other => anyhow::bail!("unexpected daemon response: {other:?}"),
    };

    let stats = send_request(&socket, &DaemonRequest::Stats).await?;
    let corpus_hash = match stats {
        DaemonResponse::Stats { corpus_hash, .. } => corpus_hash,
        _ => "unknown".to_owned(),
    };

    Ok(report(
        "brutal-searchd",
        &options.index,
        options.limit,
        timings,
        elapsed,
        corpus_hash,
    ))
}

fn run_in_process_bench(options: BenchOptions, queries: Vec<String>) -> Result<BenchReport> {
    let index = SearchIndex::open(&options.index, PreloadMode::Aggressive)?;

    for query in queries.iter().take(options.warmup) {
        let _ = index.search(
            query,
            SearchOptions {
                limit: options.limit,
            },
        )?;
    }

    let started = Instant::now();
    let mut timings = Vec::with_capacity(queries.len());
    for query in &queries {
        let t0 = Instant::now();
        let results = index.search(
            query,
            SearchOptions {
                limit: options.limit,
            },
        )?;
        let _rendered = render_results_len(&results);
        timings.push(t0.elapsed());
    }
    let elapsed = started.elapsed();

    Ok(report(
        "brutal-search-in-process",
        index.root(),
        options.limit,
        timings,
        elapsed,
        index.manifest().corpus_hash.clone(),
    ))
}

fn report(
    engine: &str,
    index_dir: &Path,
    limit: usize,
    mut timings: Vec<Duration>,
    elapsed: Duration,
    corpus_hash: String,
) -> BenchReport {
    timings.sort_unstable();
    let query_count = timings.len();
    let p50 = percentile(&timings, 0.50);
    let p95 = percentile(&timings, 0.95);
    let p99 = percentile(&timings, 0.99);
    let throughput_qps = query_count as f64 / elapsed.as_secs_f64().max(f64::EPSILON);

    BenchReport {
        engine: engine.to_owned(),
        query_count,
        limit,
        p50_us: p50.as_micros(),
        p95_us: p95.as_micros(),
        p99_us: p99.as_micros(),
        throughput_qps,
        total_ms: elapsed.as_millis(),
        rustc: command_output("rustc", &["--version"]),
        chrome: chrome_version(),
        os: command_output("uname", &["-a"]),
        hardware: hardware_summary(),
        corpus_hash,
        index_hash: index_hash(index_dir).unwrap_or_else(|_| "unknown".to_owned()),
    }
}

fn percentile(timings: &[Duration], q: f64) -> Duration {
    if timings.is_empty() {
        return Duration::ZERO;
    }
    let idx = ((timings.len() - 1) as f64 * q).ceil() as usize;
    timings[idx.min(timings.len() - 1)]
}

fn read_queries(path: &Path) -> Result<Vec<String>> {
    let text = fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    Ok(text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(str::to_owned)
        .collect())
}

#[derive(Debug)]
struct ResolvedJudgment {
    query: String,
    relevant: BTreeMap<u32, u32>,
    unresolved_judgment_count: usize,
}

#[derive(Debug, Deserialize)]
struct JudgmentLine {
    query: String,
    relevant: Vec<JudgmentTarget>,
}

#[derive(Debug, Deserialize)]
struct JudgmentTarget {
    doc_id: Option<u32>,
    url: Option<String>,
    #[serde(default = "default_grade")]
    grade: u32,
}

fn default_grade() -> u32 {
    1
}

fn read_eval_judgments(path: &Path, index: &SearchIndex) -> Result<Vec<ResolvedJudgment>> {
    let text = fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    let mut judgments = Vec::new();
    for (line_index, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let record: JudgmentLine = serde_json::from_str(line)
            .with_context(|| format!("parse {}:{}", path.display(), line_index + 1))?;
        let query = record.query.trim();
        anyhow::ensure!(
            !query.is_empty(),
            "empty query in {}:{}",
            path.display(),
            line_index + 1
        );

        let mut relevant: BTreeMap<u32, u32> = BTreeMap::new();
        let mut unresolved_judgment_count = 0usize;
        for target in record.relevant {
            if target.grade == 0 {
                continue;
            }
            let doc_id = if let Some(doc_id) = target.doc_id {
                Some(doc_id)
            } else if let Some(url) = target.url.as_deref() {
                index.doc_id_for_url(url)
            } else {
                None
            };

            let Some(doc_id) = doc_id else {
                unresolved_judgment_count += 1;
                continue;
            };
            if index.doc(doc_id).is_none() {
                unresolved_judgment_count += 1;
                continue;
            }
            relevant
                .entry(doc_id)
                .and_modify(|grade| *grade = (*grade).max(target.grade))
                .or_insert(target.grade);
        }

        judgments.push(ResolvedJudgment {
            query: query.to_owned(),
            relevant,
            unresolved_judgment_count,
        });
    }
    Ok(judgments)
}

fn evaluate_query(
    index: &SearchIndex,
    judgment: &ResolvedJudgment,
    limit: usize,
) -> Result<EvalQueryReport> {
    let results = index.search(&judgment.query, SearchOptions { limit })?;
    let mut seen_relevant = BTreeSet::new();
    let mut retrieved_relevant = 0usize;
    let mut reciprocal_rank = 0.0;
    let mut dcg = 0.0;

    for (position, result) in results.iter().enumerate() {
        let rank = position + 1;
        if let Some(&grade) = judgment.relevant.get(&result.doc_id) {
            dcg += discounted_gain(grade, rank);
            if seen_relevant.insert(result.doc_id) {
                retrieved_relevant += 1;
                if reciprocal_rank == 0.0 {
                    reciprocal_rank = 1.0 / rank as f64;
                }
            }
        }
    }

    let mut ideal_grades: Vec<u32> = judgment.relevant.values().copied().collect();
    ideal_grades.sort_unstable_by(|left, right| right.cmp(left));
    let ideal_dcg: f64 = ideal_grades
        .iter()
        .take(limit)
        .enumerate()
        .map(|(position, &grade)| discounted_gain(grade, position + 1))
        .sum();
    let ndcg_at_k = if ideal_dcg > 0.0 {
        dcg / ideal_dcg
    } else {
        0.0
    };
    let relevant_count = judgment.relevant.len();

    Ok(EvalQueryReport {
        query: judgment.query.clone(),
        relevant_count,
        retrieved_relevant,
        reciprocal_rank,
        ndcg_at_k,
        recall_at_k: retrieved_relevant as f64 / relevant_count as f64,
        precision_at_k: retrieved_relevant as f64 / limit as f64,
        unresolved_judgment_count: judgment.unresolved_judgment_count,
    })
}

fn discounted_gain(grade: u32, rank: usize) -> f64 {
    let gain = 2.0_f64.powi(grade as i32) - 1.0;
    gain / ((rank + 1) as f64).log2()
}

#[derive(Debug, Serialize)]
struct ChromiumData {
    docs: Vec<ChromiumDoc>,
    terms: BTreeMap<String, ChromiumTerm>,
    queries: Vec<String>,
    limit: usize,
    warmup: usize,
    doc_count: u32,
    avg_doc_len: f32,
    corpus_hash: String,
    index_hash: String,
}

#[derive(Debug, Serialize)]
struct ChromiumDoc {
    id: u32,
    url: String,
    title: String,
    len: u32,
    text: String,
}

#[derive(Debug, Serialize)]
struct ChromiumTerm {
    df: u32,
    postings: Vec<[u32; 3]>,
}

fn build_chromium_data(
    index: &SearchIndex,
    queries: &[String],
    limit: usize,
    warmup: usize,
) -> Result<ChromiumData> {
    let mut needed_terms = BTreeMap::<String, ()>::new();
    for query in queries {
        for term in query_terms(query) {
            needed_terms.insert(term, ());
        }
    }

    let mut terms = BTreeMap::new();
    for term in needed_terms.keys() {
        let Some(entry) = index.term_entry(term) else {
            continue;
        };
        let Some(postings) = index.postings(term)? else {
            continue;
        };
        terms.insert(
            term.clone(),
            ChromiumTerm {
                df: entry.doc_freq,
                postings: postings
                    .iter()
                    .map(|posting| {
                        [
                            posting.doc_id,
                            posting.tf,
                            posting.positions.first().copied().unwrap_or(0),
                        ]
                    })
                    .collect(),
            },
        );
    }

    let docs = index
        .docs()
        .iter()
        .map(|doc| ChromiumDoc {
            id: doc.id,
            url: doc.url.clone(),
            title: doc.title.clone(),
            len: doc.term_count,
            text: index.text(doc.id).unwrap_or("").to_owned(),
        })
        .collect();

    Ok(ChromiumData {
        docs,
        terms,
        queries: queries.to_owned(),
        limit,
        warmup,
        doc_count: index.manifest().doc_count,
        avg_doc_len: index.manifest().avg_doc_len,
        corpus_hash: index.manifest().corpus_hash.clone(),
        index_hash: index_hash(index.root()).unwrap_or_else(|_| "unknown".to_owned()),
    })
}

fn chromium_baseline_html(data: &ChromiumData) -> Result<String> {
    let data_json = serde_json::to_string(data)?.replace("</", "<\\/");
    Ok(format!(
        r#"<!doctype html>
<meta charset="utf-8">
<title>Blackium Starium✴ Chromium Baseline</title>
<main id="results"></main>
<script>
const DATA = {data_json};
const RESULTS = document.getElementById("results");

function tokenize(text) {{
  const out = [];
  const re = /[A-Za-z0-9]+/g;
  let match;
  while ((match = re.exec(text)) !== null) out.push(match[0].toLowerCase());
  return [...new Set(out)];
}}

function snippet(text, center, maxLen) {{
  if (text.length <= maxLen) return text.replaceAll("\n", " ");
  const half = Math.floor(maxLen / 2);
  const start = Math.max(0, center - half);
  const end = Math.min(text.length, center + half);
  return (start > 0 ? "..." : "") + text.slice(start, end).replaceAll("\n", " ").trim() + (end < text.length ? "..." : "");
}}

function search(query) {{
  const scores = new Map();
  const firstPos = new Map();
  for (const term of tokenize(query)) {{
    const entry = DATA.terms[term];
    if (!entry) continue;
    const idf = Math.log(((DATA.doc_count - entry.df + 0.5) / (entry.df + 0.5)) + 1.0);
    for (const posting of entry.postings) {{
      const doc = DATA.docs[posting[0]];
      const tf = posting[1];
      const score = idf * (tf * 2.2) / (tf + 1.2 * (0.25 + 0.75 * (doc.len / Math.max(DATA.avg_doc_len, 1.0))));
      scores.set(doc.id, (scores.get(doc.id) || 0) + score);
      if (!firstPos.has(doc.id)) firstPos.set(doc.id, posting[2]);
    }}
  }}
  const ranked = Array.from(scores.entries()).sort((a, b) => (b[1] - a[1]) || (a[0] - b[0])).slice(0, DATA.limit);
  const fragment = document.createDocumentFragment();
  for (const [docId, score] of ranked) {{
    const doc = DATA.docs[docId];
    const row = document.createElement("article");
    row.textContent = `${{doc.id}} ${{score.toFixed(4)}} ${{doc.url}}\n${{doc.title}}\n${{snippet(doc.text, firstPos.get(doc.id) || 0, 220)}}`;
    fragment.appendChild(row);
  }}
  RESULTS.replaceChildren(fragment);
  return ranked.length;
}}

for (const query of DATA.queries.slice(0, DATA.warmup)) search(query);
const timings = [];
let estimatedTotalUs = 0;
function measureQuery(query) {{
  let repeats = 1;
  let elapsedUs = 0;
  while (repeats <= 4096) {{
    const t0 = performance.now();
    for (let i = 0; i < repeats; i++) search(query);
    elapsedUs = (performance.now() - t0) * 1000;
    if (elapsedUs >= 1000 || repeats === 4096) break;
    repeats *= 2;
  }}
  const perQueryUs = elapsedUs / repeats;
  estimatedTotalUs += perQueryUs;
  return Math.max(1, Math.round(perQueryUs));
}}
for (const query of DATA.queries) {{
  timings.push(measureQuery(query));
}}
const totalMs = estimatedTotalUs / 1000;
timings.sort((a, b) => a - b);
function percentile(q) {{
  if (timings.length === 0) return 0;
  const idx = Math.min(timings.length - 1, Math.ceil((timings.length - 1) * q));
  return timings[idx];
}}
document.body.textContent = JSON.stringify({{
  engine: "headless-chromium-js",
  query_count: DATA.queries.length,
  limit: DATA.limit,
  p50_us: percentile(0.50),
  p95_us: percentile(0.95),
  p99_us: percentile(0.99),
  throughput_qps: DATA.queries.length / Math.max(totalMs / 1000, Number.EPSILON),
  total_ms: Math.round(totalMs),
  rustc: null,
  chrome: null,
  os: null,
  hardware: null,
  corpus_hash: DATA.corpus_hash,
  index_hash: DATA.index_hash
}});
</script>"#
    ))
}

fn extract_json_object(text: &str) -> Option<&str> {
    let start = text.find('{')?;
    let end = text.rfind('}')?;
    text.get(start..=end)
}

fn render_results_len(results: &[crate::query::SearchResult]) -> usize {
    results
        .iter()
        .map(|result| result.url.len() + result.title.len() + result.snippet.len() + 16)
        .sum()
}

fn command_output(program: &str, args: &[&str]) -> Option<String> {
    let output = Command::new(program).args(args).output().ok()?;
    if !output.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

fn hardware_summary() -> Option<String> {
    command_output("sysctl", &["-n", "machdep.cpu.brand_string"])
        .or_else(|| command_output("lscpu", &[]))
        .or_else(|| command_output("uname", &["-m"]))
}

fn index_hash(index_dir: &Path) -> Result<String> {
    let mut hasher = blake3::Hasher::new();
    for file_name in [
        "manifest.json",
        "docs.bin",
        "field_docs.bin",
        "lexicon.bin",
        "postings.bin",
        "texts.bin",
    ] {
        let path = index_dir.join(file_name);
        if !path.exists() {
            continue;
        }
        hasher.update(file_name.as_bytes());
        hasher.update(&[0]);
        hasher.update(&fs::read(&path).with_context(|| format!("read {}", path.display()))?);
        hasher.update(&[0]);
    }
    Ok(hasher.finalize().to_hex().to_string())
}

fn chrome_version() -> Option<String> {
    command_output("chromium", &["--version"])
        .or_else(|| command_output("google-chrome", &["--version"]))
        .or_else(|| {
            command_output(
                "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
                &["--version"],
            )
        })
}

fn chrome_program() -> Option<String> {
    if command_output("chromium", &["--version"]).is_some() {
        return Some("chromium".to_owned());
    }
    if command_output("google-chrome", &["--version"]).is_some() {
        return Some("google-chrome".to_owned());
    }
    let mac = "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome";
    if Path::new(mac).exists() {
        return Some(mac.to_owned());
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report_fixture(engine: &str, p95_us: u128) -> BenchReport {
        BenchReport {
            engine: engine.to_owned(),
            query_count: 1,
            limit: 20,
            p50_us: p95_us,
            p95_us,
            p99_us: p95_us,
            throughput_qps: 1.0,
            total_ms: 1,
            rustc: None,
            chrome: None,
            os: None,
            hardware: None,
            corpus_hash: "corpus".to_owned(),
            index_hash: "index".to_owned(),
        }
    }

    #[test]
    fn comparison_gate_records_pass_fail() {
        let mut comparison = BenchComparison {
            rust: report_fixture("rust", 10),
            chromium: report_fixture("chromium", 100),
            p95_speedup: 10.0,
            required_p95_speedup: None,
            passed: None,
        };

        assert!(comparison.apply_gate(10.0));
        assert_eq!(comparison.required_p95_speedup, Some(10.0));
        assert_eq!(comparison.passed, Some(true));

        assert!(!comparison.apply_gate(12.0));
        assert_eq!(comparison.passed, Some(false));
    }

    #[test]
    fn eval_gate_records_pass_fail() {
        let mut report = EvalReport {
            query_count: 1,
            evaluated_query_count: 1,
            limit: 10,
            mean_reciprocal_rank: 0.90,
            mean_ndcg_at_k: 0.80,
            mean_recall_at_k: 0.70,
            mean_precision_at_k: 0.20,
            unresolved_judgment_count: 1,
            required_mrr: None,
            required_ndcg_at_k: None,
            required_recall_at_k: None,
            required_precision_at_k: None,
            max_unresolved_judgment_count: None,
            passed: None,
            corpus_hash: "corpus".to_owned(),
            index_hash: "index".to_owned(),
            queries: Vec::new(),
        };

        assert!(report.apply_gate(EvalGate {
            required_mrr: Some(0.90),
            required_ndcg_at_k: Some(0.80),
            required_recall_at_k: Some(0.70),
            required_precision_at_k: Some(0.20),
            max_unresolved_judgment_count: Some(1),
        }));
        assert_eq!(report.required_ndcg_at_k, Some(0.80));
        assert_eq!(report.max_unresolved_judgment_count, Some(1));
        assert_eq!(report.passed, Some(true));

        assert!(!report.apply_gate(EvalGate {
            required_mrr: Some(0.95),
            ..EvalGate::default()
        }));
        assert_eq!(report.passed, Some(false));
    }

    #[test]
    fn browser_perf_gate_records_pass_fail() {
        let mut report = BrowserPerfReport {
            engine: "browser".to_owned(),
            manifest: "manifest.json".to_owned(),
            fixture_count: 1,
            iteration_count: 1,
            warmup: 0,
            sample_count: 1,
            p50_us: 10,
            p95_us: 10,
            p99_us: 10,
            raster_p50_us: 2,
            raster_p95_us: 2,
            raster_p99_us: 2,
            layer_metrics_p50_us: 1,
            layer_metrics_p95_us: 1,
            layer_metrics_p99_us: 1,
            throughput_pages_per_sec: 100.0,
            total_ms: 1,
            total_rendered_bytes: 4,
            total_dom_nodes: 3,
            total_css_rules: 0,
            total_layout_boxes: 1,
            total_paint_commands: 1,
            total_layers: 1,
            total_image_layers: 0,
            max_layer_count: 1,
            max_image_layer_count: 0,
            max_root_layer_width: 80,
            max_root_layer_height: 1,
            max_layer_area: 80,
            total_layer_area: 80,
            total_layer_metrics_us: 1,
            total_raster_us: 2,
            total_raster_pixels: 16,
            total_raster_non_background_pixels: 4,
            total_raster_visible_commands: 1,
            total_raster_culled_commands: 0,
            chromium_baseline: None,
            chromium_p95_speedup: None,
            phase_totals: BrowserRenderTimings {
                parse_us: 1,
                script_us: 1,
                style_us: 1,
                collect_us: 1,
                layout_us: 1,
                total_us: 5,
            },
            suite_hash: "suite".to_owned(),
            rustc: None,
            chrome: None,
            os: None,
            hardware: None,
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
            fixtures: Vec::new(),
        };

        assert!(report.apply_gate(BrowserPerfGate {
            max_p95_us: Some(10),
            min_throughput_pages_per_sec: Some(100.0),
            min_chromium_p95_speedup: None,
            max_chromium_text_mismatches: None,
            max_layer_metrics_p95_us: Some(1),
            min_total_layers: Some(1),
            min_total_image_layers: Some(0),
            max_layer_count: Some(1),
            max_image_layer_count: Some(0),
        }));
        assert_eq!(report.required_max_p95_us, Some(10));
        assert_eq!(report.required_max_layer_metrics_p95_us, Some(1));
        assert_eq!(report.required_min_total_layers, Some(1));
        assert_eq!(report.passed, Some(true));

        report.chromium_p95_speedup = Some(2.0);
        assert!(report.apply_gate(BrowserPerfGate {
            max_p95_us: Some(10),
            min_throughput_pages_per_sec: Some(100.0),
            min_chromium_p95_speedup: Some(1.5),
            max_chromium_text_mismatches: None,
            max_layer_metrics_p95_us: Some(1),
            min_total_layers: Some(1),
            min_total_image_layers: Some(0),
            max_layer_count: Some(1),
            max_image_layer_count: Some(0),
        }));
        assert_eq!(report.required_min_chromium_p95_speedup, Some(1.5));

        report.chromium_baseline = Some(crate::bench::BrowserPerfChromiumBaselineReport {
            engine: "chromium".to_owned(),
            chrome: None,
            sample_count: 1,
            text_match_count: 1,
            text_mismatch_count: 0,
            p50_us: 20,
            p95_us: 20,
            p99_us: 20,
            throughput_pages_per_sec: 10.0,
            total_ms: 2,
            fixtures: Vec::new(),
        });
        assert!(report.apply_gate(BrowserPerfGate {
            max_p95_us: Some(10),
            min_throughput_pages_per_sec: Some(100.0),
            min_chromium_p95_speedup: Some(1.5),
            max_chromium_text_mismatches: Some(0),
            max_layer_metrics_p95_us: Some(1),
            min_total_layers: Some(1),
            min_total_image_layers: Some(0),
            max_layer_count: Some(1),
            max_image_layer_count: Some(0),
        }));
        assert_eq!(report.required_max_chromium_text_mismatches, Some(0));

        assert!(!report.apply_gate(BrowserPerfGate {
            max_p95_us: Some(9),
            min_throughput_pages_per_sec: Some(100.0),
            min_chromium_p95_speedup: None,
            max_chromium_text_mismatches: None,
            max_layer_metrics_p95_us: Some(1),
            min_total_layers: Some(1),
            min_total_image_layers: Some(0),
            max_layer_count: Some(1),
            max_image_layer_count: Some(0),
        }));
        assert_eq!(report.passed, Some(false));
    }

    #[test]
    fn browser_perf_runs_fixture_manifest() {
        let dir = tempfile::tempdir().unwrap();
        let page = dir.path().join("page.html");
        fs::write(
            &page,
            r#"
            <!doctype html>
            <html><head><title>Perf</title></head>
            <body><h1 id="out">Before</h1><script>document.getElementById("out").textContent = "After";</script></body></html>
            "#,
        )
        .unwrap();
        let image_page = dir.path().join("image.html");
        fs::write(
            &image_page,
            r#"
            <!doctype html>
            <html><head><title>Image Perf</title></head>
            <body><p>Before</p><img src="missing.png" width="10" height="2"><p>After</p></body></html>
            "#,
        )
        .unwrap();
        let manifest = dir.path().join("manifest.json");
        fs::write(
            &manifest,
            r#"{"fixtures":[{"name":"perf fixture","path":"page.html","width":80,"expected_title":"Perf","expected_text":"After"},{"name":"image perf fixture","path":"image.html","width":40,"raster_viewport_y":1,"raster_viewport_height":1,"expected_visible_command_count":1,"expected_culled_command_count":2,"expected_title":"Image Perf","expected_text":"Before\nAfter"}]}"#,
        )
        .unwrap();

        let report = run_browser_perf(BrowserPerfOptions {
            manifest,
            iterations: 2,
            warmup: 1,
            chromium_baseline: false,
        })
        .unwrap();

        assert_eq!(report.fixture_count, 2);
        assert_eq!(report.iteration_count, 2);
        assert_eq!(report.sample_count, 4);
        assert_eq!(report.fixtures[0].name, "perf fixture");
        assert_eq!(report.fixtures[0].rendered_bytes, "After".len());
        assert!(report.phase_totals.total_us > 0);
        assert!(report.fixtures[0].phase_totals.total_us > 0);
        let json = serde_json::to_value(&report).unwrap();
        assert!(json.get("end_to_end_p95_us").is_some());
        assert!(json.get("p95_us").is_none());
        assert!(json.get("render_phase_totals").is_some());
        assert!(json.get("phase_totals").is_none());
        assert!(json["fixtures"][0].get("end_to_end_p95_us").is_some());
        assert!(json["fixtures"][0].get("render_phase_totals").is_some());
        assert!(report.raster_p95_us > 0);
        assert!(report.total_raster_us > 0);
        assert!(report.total_raster_pixels > 0);
        assert!(report.total_raster_non_background_pixels > 0);
        assert_eq!(report.total_raster_visible_commands, 2);
        assert_eq!(report.total_raster_culled_commands, 2);
        assert_eq!(report.total_layers, 3);
        assert_eq!(report.total_image_layers, 1);
        assert_eq!(report.max_layer_count, 2);
        assert_eq!(report.max_image_layer_count, 1);
        assert_eq!(report.max_root_layer_width, 80);
        assert!(report.max_root_layer_height >= 1);
        assert!(report.max_layer_area > 0);
        assert_eq!(
            report.total_layer_area,
            report
                .fixtures
                .iter()
                .map(|fixture| fixture.total_layer_area)
                .sum::<usize>()
        );
        assert_eq!(
            report.total_layer_metrics_us,
            report
                .fixtures
                .iter()
                .map(|fixture| fixture.layer_metrics_total_us)
                .sum()
        );
        assert!(report.fixtures[0].raster_p95_us > 0);
        assert!(report.fixtures[0].raster_total_us > 0);
        assert!(report.fixtures[0].raster_pixels > 0);
        assert!(report.fixtures[0].raster_non_background_pixels > 0);
        assert_eq!(report.fixtures[0].layer_count, 1);
        assert_eq!(report.fixtures[0].image_layer_count, 0);
        assert_eq!(report.fixtures[0].root_layer_width, 80);
        assert!(report.fixtures[0].root_layer_height > 0);
        assert!(report.fixtures[0].max_layer_area > 0);
        assert!(report.fixtures[0].total_layer_area > 0);
        assert_eq!(report.fixtures[1].layer_count, 2);
        assert_eq!(report.fixtures[1].image_layer_count, 1);
        assert_eq!(report.fixtures[1].root_layer_width, 40);
        assert!(report.fixtures[1].root_layer_height > 0);
        assert!(report.fixtures[1].max_layer_area > 0);
        assert!(report.fixtures[1].total_layer_area > report.fixtures[1].max_layer_area);
        assert_eq!(report.fixtures[1].raster_height, 20);
        assert_eq!(report.fixtures[1].raster_visible_command_count, 1);
        assert_eq!(report.fixtures[1].raster_culled_command_count, 2);
        assert!(report.throughput_pages_per_sec > 0.0);
        assert_ne!(report.suite_hash, "unknown");
    }

    #[test]
    fn traceability_report_validates_required_requirement_rows() {
        let report = run_traceability_report();

        assert!(report.passed);
        assert!(!report.complete);
        assert_eq!(report.required_count, 20);
        assert_eq!(report.row_count, 20);
        assert_eq!(report.implemented_count, 0);
        assert_eq!(report.partial_count, 20);
        assert_eq!(report.missing_count, 0);
        assert!(report.missing_required_ids.is_empty());
        assert!(report.unknown_ids.is_empty());
        assert!(report.unknown_readiness_areas.is_empty());
        assert!(report.unknown_milestones.is_empty());
        assert!(report.unknown_claim_scopes.is_empty());
        assert!(report.mismatched_claim_scopes.is_empty());
        assert!(report.duplicate_ids.is_empty());
        assert!(report.validation_errors.is_empty());
        assert_eq!(
            report.rows_by_readiness_area.get("Browser Engine"),
            Some(&5)
        );
        assert_eq!(report.rows_by_readiness_area.get("Plan Coverage"), Some(&2));
        assert_eq!(
            report.rows_by_readiness_area.get("JavaScript And Web APIs"),
            Some(&2)
        );
        assert_eq!(report.rows_by_milestone.get("M0"), Some(&2));
        assert_eq!(report.rows_by_milestone.get("M2"), Some(&5));
        assert_eq!(report.rows_by_milestone.get("M4"), Some(&1));
        assert_eq!(report.rows_by_milestone.get("M5"), Some(&5));
        assert_eq!(report.rows_by_milestone.get("M6"), Some(&4));
        assert_eq!(report.rows_by_claim_scope.get("Search"), Some(&8));
        assert_eq!(report.rows_by_claim_scope.get("Browser"), Some(&9));
        assert_eq!(report.rows_by_claim_scope.get("Shared"), Some(&3));
        let search_claim = report
            .claim_reports
            .iter()
            .find(|claim| claim.claim == "google_style_search")
            .unwrap();
        assert!(!search_claim.complete);
        assert_eq!(search_claim.requirement_count, 11);
        assert_eq!(search_claim.implemented_count, 0);
        assert_eq!(search_claim.partial_count, 11);
        assert_eq!(search_claim.missing_count, 0);
        assert!(
            search_claim
                .partial_ids
                .contains(&"REQ-SEARCH-RENDERED-EXTRACTION".to_owned())
        );
        assert!(
            search_claim
                .partial_ids
                .contains(&"REQ-BENCHMARKS-STANDARDS".to_owned())
        );
        let browser_claim = report
            .claim_reports
            .iter()
            .find(|claim| claim.claim == "chromium_class_browser")
            .unwrap();
        assert!(!browser_claim.complete);
        assert_eq!(browser_claim.requirement_count, 12);
        assert_eq!(browser_claim.partial_count, 12);
        assert!(
            browser_claim
                .partial_ids
                .contains(&"REQ-BROWSER-PAINT-COMPOSITOR".to_owned())
        );
        assert!(
            browser_claim
                .partial_ids
                .contains(&"REQ-GOVERNANCE-CLAIMS".to_owned())
        );
        let combined_claim = report
            .claim_reports
            .iter()
            .find(|claim| claim.claim == "combined_competitor")
            .unwrap();
        assert!(!combined_claim.complete);
        assert_eq!(combined_claim.requirement_count, 20);
        assert_eq!(combined_claim.partial_count, 20);
        let milestone_m0 = report
            .milestone_reports
            .iter()
            .find(|milestone| milestone.milestone == "M0")
            .unwrap();
        assert!(!milestone_m0.complete);
        assert_eq!(milestone_m0.included_milestones, vec!["M0".to_owned()]);
        assert_eq!(milestone_m0.requirement_count, 2);
        assert_eq!(milestone_m0.partial_count, 2);
        assert!(
            milestone_m0
                .partial_ids
                .contains(&"REQ-BENCHMARKS-STANDARDS".to_owned())
        );
        let milestone_m4 = report
            .milestone_reports
            .iter()
            .find(|milestone| milestone.milestone == "M4")
            .unwrap();
        assert!(!milestone_m4.complete);
        assert_eq!(
            milestone_m4.included_milestones,
            vec![
                "M0".to_owned(),
                "M1".to_owned(),
                "M2".to_owned(),
                "M3".to_owned(),
                "M4".to_owned(),
            ]
        );
        assert_eq!(milestone_m4.requirement_count, 11);
        assert_eq!(milestone_m4.partial_count, 11);
        assert!(
            milestone_m4
                .partial_ids
                .contains(&"REQ-SEARCH-RENDERED-EXTRACTION".to_owned())
        );
        assert!(
            !milestone_m4
                .partial_ids
                .contains(&"REQ-BROWSER-ENGINE".to_owned())
        );
        let milestone_m6 = report
            .milestone_reports
            .iter()
            .find(|milestone| milestone.milestone == "M6")
            .unwrap();
        assert!(!milestone_m6.complete);
        assert_eq!(milestone_m6.requirement_count, 20);
        assert_eq!(milestone_m6.partial_count, 20);
        assert!(
            milestone_m6
                .partial_ids
                .contains(&"REQ-BROWSER-SECURITY".to_owned())
        );
        assert!(report.requirements.iter().any(|requirement| {
            requirement.id == "REQ-BROWSER-PAINT-COMPOSITOR"
                && requirement.current_state == ReadinessStatus::Partial
                && requirement.readiness_area == "Browser Engine"
                && requirement.milestone == "M5"
                && requirement.claim_scope == "Browser"
        }));
        assert!(report.requirements.iter().any(|requirement| {
            requirement.id == "REQ-SEARCH-RENDERED-EXTRACTION"
                && requirement.current_state == ReadinessStatus::Partial
                && requirement.readiness_area == "JavaScript And Web APIs"
                && requirement.milestone == "M4"
                && requirement.claim_scope == "Search"
        }));
        assert!(report.requirements.iter().any(|requirement| {
            requirement.id == "REQ-SEARCH-CORPUS"
                && requirement.current_state == ReadinessStatus::Partial
                && requirement.readiness_area == "Search Crawling And Freshness"
                && requirement.milestone == "M2"
                && requirement.claim_scope == "Search"
        }));
        assert!(report.requirements.iter().any(|requirement| {
            requirement.id == "REQ-BENCHMARKS-STANDARDS"
                && requirement.current_state == ReadinessStatus::Partial
                && requirement.readiness_area == "Plan Coverage"
                && requirement.milestone == "M0"
                && requirement.claim_scope == "Shared"
        }));
    }

    #[test]
    fn evidence_registry_report_covers_required_requirements() {
        let report = run_evidence_registry_report();

        assert!(report.passed);
        assert_eq!(report.row_count, 19);
        assert_eq!(report.covered_requirement_count, 20);
        assert!(report.missing_required_ids.is_empty());
        assert!(report.unknown_requirement_ids.is_empty());
        assert!(report.duplicate_evidence_ids.is_empty());
        assert!(report.validation_errors.is_empty());

        let js_api = report
            .rows
            .iter()
            .find(|row| row.evidence_id == "EV-JS-WEB-API")
            .unwrap();
        assert!(
            js_api
                .requirement_ids
                .contains(&"REQ-SEARCH-RENDERED-EXTRACTION".to_owned())
        );
        assert!(
            js_api
                .requirement_ids
                .contains(&"REQ-BROWSER-JS-WEB-APIS".to_owned())
        );

        let wpt = report
            .rows
            .iter()
            .find(|row| row.evidence_id == "EV-WPT-SUBSETS")
            .unwrap();
        assert!(
            wpt.requirement_ids
                .contains(&"REQ-BROWSER-SECURITY".to_owned())
        );
    }

    #[test]
    fn competitor_audit_combines_claim_gates() {
        let report = run_competitor_audit_for_claim(ReadinessClaim::GoogleStyleSearch);

        assert!(!report.passed);
        assert_eq!(report.claim, "google_style_search");
        assert!(report.traceability_passed);
        assert!(!report.traceability_claim_complete);
        assert!(report.evidence_passed);
        assert!(!report.readiness_passed);
        assert_eq!(report.failure_count, report.failures.len());
        assert!(report.failures.iter().any(|failure| {
            failure.contains("traceability claim google_style_search incomplete")
        }));
        assert!(
            report.failures.iter().any(|failure| {
                failure.contains("readiness claim google_style_search incomplete")
            })
        );
        assert!(
            report
                .partial_requirement_ids
                .contains(&"REQ-SEARCH-CORPUS".to_owned())
        );
        assert!(
            report
                .partial_requirement_ids
                .contains(&"REQ-GOVERNANCE-CLAIMS".to_owned())
        );
        assert!(report.missing_requirement_ids.is_empty());
        assert!(
            report
                .partial_readiness_areas
                .contains(&"Search Crawling And Freshness".to_owned())
        );
        assert!(report.missing_readiness_areas.is_empty());
        assert_eq!(report.evidence.covered_requirement_count, 20);
        assert_eq!(report.traceability.row_count, 20);
        assert_eq!(report.readiness.area_count, 8);
    }

    #[test]
    fn competitor_readiness_keeps_unfinished_claims_explicit() {
        let report = run_competitor_readiness();

        assert!(!report.passed);
        assert_eq!(report.claim, "combined_competitor");
        assert_eq!(
            report.required_claim_scopes,
            vec!["Search".to_owned(), "Browser".to_owned()]
        );
        assert!(report.area_count >= 10);
        assert_eq!(
            report.area_count,
            report.implemented_count + report.partial_count + report.missing_count
        );
        assert!(report.implemented_count >= 1);
        assert!(report.partial_count > 0);
        assert_eq!(report.missing_count, 0);
        assert!(report.areas.iter().any(|area| {
            area.area == "Plan Coverage" && area.status == ReadinessStatus::Implemented
        }));
        let plan = report
            .areas
            .iter()
            .find(|area| area.area == "Plan Coverage")
            .unwrap();
        assert!(
            plan.current_evidence
                .iter()
                .any(|evidence| evidence.contains("docs/REQUIREMENTS_TRACEABILITY.md"))
        );
        assert!(
            plan.current_evidence
                .iter()
                .any(|evidence| evidence.contains("docs/EVIDENCE_REGISTRY.md"))
        );
        assert!(plan.current_evidence.iter().any(|evidence| {
            evidence.contains("maps 19 evidence rows covering 20 required requirement IDs")
        }));
        assert!(plan.current_evidence.iter().any(|evidence| {
            evidence.contains("20 required requirement rows")
                && evidence.contains("0 missing current states")
        }));
        assert!(plan.current_evidence.iter().any(|evidence| {
            evidence.contains("Browser=9")
                && evidence.contains("Search=8")
                && evidence.contains("Shared=3")
        }));
        assert!(
            plan.current_evidence
                .iter()
                .any(|evidence| { evidence.contains("docs/BROWSER_RENDERING_COMPOSITOR_PLAN.md") })
        );
        assert!(report.areas.iter().any(|area| {
            area.area == "Browser Engine" && area.status == ReadinessStatus::Partial
        }));
        assert!(report.areas.iter().any(|area| {
            area.area == "Security And Privacy" && area.status == ReadinessStatus::Partial
        }));
        assert!(report.areas.iter().any(|area| {
            area.area == "Operations And Reliability" && area.status == ReadinessStatus::Partial
        }));
        assert!(report.areas.iter().any(|area| {
            area.area == "Platform Completeness" && area.status == ReadinessStatus::Partial
        }));
        let crawl = report
            .areas
            .iter()
            .find(|area| area.area == "Search Crawling And Freshness")
            .unwrap();
        assert!(
            crawl
                .current_evidence
                .iter()
                .any(|evidence| evidence.contains("src/crawler.rs"))
        );
        let relevance = report
            .areas
            .iter()
            .find(|area| area.area == "Search Relevance And Quality")
            .unwrap();
        assert!(
            relevance
                .current_evidence
                .iter()
                .any(|evidence| evidence.contains("bench/judgments.jsonl"))
        );
        let browser = report
            .areas
            .iter()
            .find(|area| area.area == "Browser Engine")
            .unwrap();
        assert!(
            browser
                .current_evidence
                .iter()
                .any(|evidence| evidence.contains("src/browser.rs"))
        );
        let js = report
            .areas
            .iter()
            .find(|area| area.area == "JavaScript And Web APIs")
            .unwrap();
        assert!(
            js.current_evidence
                .iter()
                .any(|evidence| evidence.contains("bench/browser-fixtures/get-attribute.html"))
        );
    }

    #[test]
    fn competitor_readiness_can_focus_search_or_browser_claims() {
        let search = run_competitor_readiness_for_claim(ReadinessClaim::GoogleStyleSearch);

        assert!(!search.passed);
        assert_eq!(search.claim, "google_style_search");
        assert_eq!(search.required_claim_scopes, vec!["Search".to_owned()]);
        assert_eq!(search.area_count, 8);
        assert_eq!(search.implemented_count, 1);
        assert_eq!(search.partial_count, 7);
        assert_eq!(search.missing_count, 0);
        assert!(
            search
                .areas
                .iter()
                .all(|area| area.claim_scopes.contains(&"Search".to_owned()))
        );
        assert!(
            search
                .areas
                .iter()
                .any(|area| area.area == "Search Crawling And Freshness")
        );
        assert!(
            search
                .areas
                .iter()
                .any(|area| area.area == "JavaScript And Web APIs")
        );
        assert!(
            !search
                .areas
                .iter()
                .any(|area| area.area == "Browser Engine")
        );
        assert!(
            !search
                .areas
                .iter()
                .any(|area| area.area == "Platform Completeness")
        );

        let browser = run_competitor_readiness_for_claim(ReadinessClaim::ChromiumClassBrowser);

        assert!(!browser.passed);
        assert_eq!(browser.claim, "chromium_class_browser");
        assert_eq!(browser.required_claim_scopes, vec!["Browser".to_owned()]);
        assert_eq!(browser.area_count, 6);
        assert_eq!(browser.implemented_count, 1);
        assert_eq!(browser.partial_count, 5);
        assert_eq!(browser.missing_count, 0);
        assert!(
            browser
                .areas
                .iter()
                .all(|area| area.claim_scopes.contains(&"Browser".to_owned()))
        );
        assert!(
            browser
                .areas
                .iter()
                .any(|area| area.area == "Browser Engine")
        );
        assert!(
            browser
                .areas
                .iter()
                .any(|area| area.area == "Platform Completeness")
        );
        assert!(
            browser
                .areas
                .iter()
                .any(|area| area.area == "JavaScript And Web APIs")
        );
        assert!(
            !browser
                .areas
                .iter()
                .any(|area| area.area == "Search Crawling And Freshness")
        );
        assert!(
            !browser
                .areas
                .iter()
                .any(|area| area.area == "Search Relevance And Quality")
        );
    }

    #[test]
    fn bench_status_round_trips_from_index_directory() {
        let dir = tempfile::tempdir().unwrap();
        let status = BenchStatusReport::Search(report_fixture("rust<script>", 17));
        let path = write_bench_status(dir.path(), &status).unwrap();

        assert_eq!(path, dir.path().join(BENCH_STATUS_FILE));
        let loaded = read_bench_status(dir.path()).unwrap().unwrap();
        let BenchStatusReport::Search(report) = loaded else {
            panic!("expected search report");
        };
        assert_eq!(report.engine, "rust<script>");
        assert_eq!(report.p95_us, 17);
    }

    #[test]
    fn eval_reports_relevance_metrics_from_judgments() {
        let dir = tempfile::tempdir().unwrap();
        let corpus = dir.path().join("corpus");
        fs::create_dir_all(&corpus).unwrap();
        fs::write(
            corpus.join("one.html"),
            "<!doctype html><title>Brutal Smoke</title><h1>Brutal smoke search</h1><p>rust benchmark fixture</p>",
        )
        .unwrap();
        fs::write(
            corpus.join("two.html"),
            "<!doctype html><title>Browser Smoke</title><h1>browser runtime</h1><p>static rendering fixture</p>",
        )
        .unwrap();
        let index = dir.path().join("index");
        build_from_corpus(&corpus, &index, IndexBuildOptions::default()).unwrap();
        let search_index = SearchIndex::open(&index, PreloadMode::Lazy).unwrap();
        let brutal_doc = search_index
            .search("brutal smoke", SearchOptions { limit: 2 })
            .unwrap()
            .first()
            .unwrap()
            .doc_id;
        let browser_doc = search_index
            .search("browser runtime", SearchOptions { limit: 2 })
            .unwrap()
            .first()
            .unwrap()
            .doc_id;
        drop(search_index);

        let judgments = dir.path().join("judgments.jsonl");
        fs::write(
            &judgments,
            format!(
                "{{\"query\":\"brutal smoke\",\"relevant\":[{{\"doc_id\":{brutal_doc},\"grade\":3}}]}}\n\
                 {{\"query\":\"browser runtime\",\"relevant\":[{{\"doc_id\":{browser_doc},\"grade\":2}}]}}\n\
                 {{\"query\":\"missing\",\"relevant\":[{{\"url\":\"https://missing.example/\",\"grade\":1}}]}}\n"
            ),
        )
        .unwrap();

        let report = run_eval(EvalOptions {
            index,
            judgments,
            limit: 5,
        })
        .unwrap();

        assert_eq!(report.query_count, 3);
        assert_eq!(report.evaluated_query_count, 2);
        assert_eq!(report.unresolved_judgment_count, 1);
        assert_eq!(report.mean_reciprocal_rank, 1.0);
        assert!(report.mean_ndcg_at_k > 0.99);
        assert_eq!(report.mean_recall_at_k, 1.0);
        assert!(report.mean_precision_at_k > 0.0);
        assert_ne!(report.index_hash, "unknown");
    }

    #[test]
    fn smoke_pipeline_builds_searches_renders_and_benches() {
        let dir = tempfile::tempdir().unwrap();
        let corpus = dir.path().join("corpus");
        fs::create_dir_all(&corpus).unwrap();
        fs::write(
            corpus.join("one.html"),
            "<!doctype html><title>Brutal Smoke</title><h1>Brutal smoke search</h1><p>rust benchmark fixture</p>",
        )
        .unwrap();
        fs::write(
            corpus.join("two.html"),
            "<!doctype html><title>Browser Smoke</title><h1>browser runtime</h1><p>static rendering fixture</p>",
        )
        .unwrap();
        let queries = dir.path().join("queries.txt");
        fs::write(&queries, "brutal smoke\nbrowser runtime\n").unwrap();
        let index = dir.path().join("index");

        let report = run_smoke(SmokeOptions {
            corpus,
            index,
            queries,
            limit: 5,
            warmup: 1,
        })
        .unwrap();

        assert_eq!(report.build.doc_count, 2);
        assert_eq!(report.query, "brutal smoke");
        assert!(report.result_count > 0);
        assert!(report.rendered_bytes > 0);
        assert_eq!(report.bench.query_count, 2);
        assert_ne!(report.bench.index_hash, "unknown");
    }
}
