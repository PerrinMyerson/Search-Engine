use std::path::PathBuf;

use anyhow::{Result, bail};
use brutal_search::bench::{
    BenchOptions, BenchStatusReport, BrowserPerfGate, BrowserPerfOptions, BrowserPerfReport,
    CompetitorAuditReport, CompetitorReadinessReport, EvalGate, EvalOptions, EvalReport,
    EvidenceRegistryReport, GateOptions, GateReport, ReadinessClaim, ReadinessStatus, SmokeOptions,
    TraceabilityReport, run_browser_perf, run_competitor_audit_for_claim,
    run_competitor_readiness_for_claim, run_eval, run_evidence_registry_report, run_gate,
    run_search_bench, run_search_comparison, run_smoke, run_traceability_report,
    write_bench_status, write_bench_status_path,
};
use brutal_search::browser::{
    BrowserChromiumParityReport, BrowserCoverageGate, BrowserCoverageReport,
};
use brutal_search::browser_compat::{
    BrowserCompatGate, BrowserCompatOptions, BrowserCompatReport, run_browser_compat,
};
use clap::{Parser, Subcommand, ValueEnum};

#[derive(Debug, Parser)]
#[command(version, about = "Benchmark harness for Blackium Starium✴.")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

// Clap holds one parsed subcommand per process; boxing these fields would obscure the flags.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Subcommand)]
enum Command {
    Smoke {
        #[arg(long, default_value = "bench/fixtures/corpus")]
        corpus: PathBuf,
        #[arg(long, default_value = "target/brutal-smoke-index")]
        index: PathBuf,
        #[arg(long, default_value = "bench/queries.txt")]
        queries: PathBuf,
        #[arg(long, default_value_t = 20)]
        limit: usize,
        #[arg(long, default_value_t = 4)]
        warmup: usize,
        #[arg(long)]
        json: bool,
        #[arg(long)]
        save_report: bool,
        #[arg(long)]
        report_output: Option<PathBuf>,
    },
    Search {
        #[arg(long, default_value = ".brutal-index")]
        index: PathBuf,
        #[arg(long)]
        queries: PathBuf,
        #[arg(long, default_value_t = 20)]
        limit: usize,
        #[arg(long, default_value_t = 16)]
        warmup: usize,
        #[arg(long)]
        socket: Option<PathBuf>,
        #[arg(long, value_enum, default_value = "daemon")]
        mode: BenchMode,
        #[arg(long)]
        json: bool,
        #[arg(long)]
        chromium_baseline: bool,
        #[arg(long)]
        require_speedup: Option<f64>,
        #[arg(long)]
        save_report: bool,
        #[arg(long)]
        report_output: Option<PathBuf>,
    },
    Eval {
        #[arg(long, default_value = ".brutal-index")]
        index: PathBuf,
        #[arg(long, default_value = "bench/judgments.jsonl")]
        judgments: PathBuf,
        #[arg(long, default_value_t = 20)]
        limit: usize,
        #[arg(long)]
        json: bool,
        #[arg(long)]
        save_report: bool,
        #[arg(long)]
        report_output: Option<PathBuf>,
        #[arg(long)]
        require_mrr: Option<f64>,
        #[arg(long)]
        require_ndcg: Option<f64>,
        #[arg(long)]
        require_recall: Option<f64>,
        #[arg(long)]
        require_precision: Option<f64>,
        #[arg(long)]
        max_unresolved: Option<usize>,
    },
    Gate {
        #[arg(long, default_value = "bench/fixtures/corpus")]
        corpus: PathBuf,
        #[arg(long, default_value = "target/brutal-gate-index")]
        index: PathBuf,
        #[arg(long, default_value = "bench/queries.txt")]
        queries: PathBuf,
        #[arg(long, default_value = "bench/judgments.jsonl")]
        judgments: PathBuf,
        #[arg(long, default_value = "bench/browser-fixtures/manifest.json")]
        browser_manifest: PathBuf,
        #[arg(long, default_value_t = 20)]
        limit: usize,
        #[arg(long, default_value_t = 4)]
        warmup: usize,
        #[arg(long)]
        socket: Option<PathBuf>,
        #[arg(long, value_enum, default_value = "in-process")]
        mode: BenchMode,
        #[arg(long)]
        chromium_search_baseline: bool,
        #[arg(long)]
        require_speedup: Option<f64>,
        #[arg(long)]
        browser_chromium_parity: bool,
        #[arg(long)]
        browser_compat: bool,
        #[arg(long, default_value = "bench/wpt-subsets/manifest.json")]
        browser_compat_manifest: PathBuf,
        #[arg(long)]
        browser_compat_expectations: Option<PathBuf>,
        #[arg(long = "browser-compat-subset")]
        browser_compat_subsets: Vec<String>,
        #[arg(long, default_value_t = 1)]
        browser_compat_repeat: usize,
        #[arg(long)]
        browser_compat_timeout_ms: Option<u64>,
        #[arg(long)]
        browser_compat_min_pass_rate: Option<f64>,
        #[arg(long)]
        browser_compat_max_unexpected_failures: Option<usize>,
        #[arg(long)]
        browser_compat_max_failures: Option<usize>,
        #[arg(long)]
        browser_compat_max_timeouts: Option<usize>,
        #[arg(long)]
        browser_compat_max_crashes: Option<usize>,
        #[arg(long)]
        browser_compat_max_flakes: Option<usize>,
        #[arg(long)]
        browser_compat_max_skipped: Option<usize>,
        #[arg(long)]
        browser_compat_max_unsupported: Option<usize>,
        #[arg(long = "browser-compat-require-subsystem")]
        browser_compat_required_subsystems: Vec<String>,
        #[arg(long)]
        browser_compat_min_subsystem_pass_rate: Option<f64>,
        #[arg(long = "require-browser-feature")]
        require_browser_features: Vec<String>,
        #[arg(long)]
        min_browser_implemented_ratio: Option<f64>,
        #[arg(long)]
        max_browser_missing: Option<usize>,
        #[arg(long)]
        require_mrr: Option<f64>,
        #[arg(long)]
        require_ndcg: Option<f64>,
        #[arg(long)]
        require_recall: Option<f64>,
        #[arg(long)]
        require_precision: Option<f64>,
        #[arg(long)]
        max_unresolved: Option<usize>,
        #[arg(long)]
        json: bool,
        #[arg(long)]
        save_report: bool,
        #[arg(long)]
        report_output: Option<PathBuf>,
    },
    BrowserPerf {
        #[arg(long, default_value = "bench/browser-fixtures/manifest.json")]
        manifest: PathBuf,
        #[arg(long, default_value_t = 50)]
        iterations: usize,
        #[arg(long, default_value_t = 4)]
        warmup: usize,
        #[arg(long)]
        max_p95_us: Option<u128>,
        #[arg(long)]
        min_throughput_pages_per_sec: Option<f64>,
        #[arg(long)]
        chromium_baseline: bool,
        #[arg(long)]
        min_chromium_p95_speedup: Option<f64>,
        #[arg(long)]
        max_chromium_text_mismatches: Option<usize>,
        #[arg(long)]
        max_layer_metrics_p95_us: Option<u128>,
        #[arg(long)]
        min_total_layers: Option<usize>,
        #[arg(long)]
        min_total_image_layers: Option<usize>,
        #[arg(long)]
        max_layer_count: Option<usize>,
        #[arg(long)]
        max_image_layer_count: Option<usize>,
        #[arg(long)]
        json: bool,
        #[arg(long)]
        save_report: bool,
        #[arg(long, default_value = "target/brutal-browser-perf")]
        report_dir: PathBuf,
        #[arg(long)]
        report_output: Option<PathBuf>,
    },
    BrowserCompat {
        #[arg(long, default_value = "bench/wpt-subsets/manifest.json")]
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
        min_pass_rate: Option<f64>,
        #[arg(long)]
        max_unexpected_failures: Option<usize>,
        #[arg(long)]
        max_failures: Option<usize>,
        #[arg(long)]
        max_timeouts: Option<usize>,
        #[arg(long)]
        max_crashes: Option<usize>,
        #[arg(long)]
        max_flakes: Option<usize>,
        #[arg(long)]
        max_skipped: Option<usize>,
        #[arg(long)]
        max_unsupported: Option<usize>,
        #[arg(long = "require-subsystem")]
        required_subsystems: Vec<String>,
        #[arg(long)]
        min_subsystem_pass_rate: Option<f64>,
        #[arg(long)]
        json: bool,
        #[arg(long)]
        save_report: bool,
        #[arg(long, default_value = "target/brutal-browser-compat")]
        report_dir: PathBuf,
        #[arg(long)]
        report_output: Option<PathBuf>,
    },
    Readiness {
        #[arg(long)]
        json: bool,
        #[arg(long)]
        require_complete: bool,
        #[arg(long, value_enum, default_value = "combined")]
        claim: ReadinessClaimArg,
    },
    Traceability {
        #[arg(long)]
        json: bool,
        #[arg(long)]
        require_complete: bool,
        #[arg(long)]
        require_no_missing: bool,
        #[arg(long = "require-claim-complete", value_enum)]
        require_claim_complete: Vec<TraceabilityClaim>,
        #[arg(long = "require-milestone-complete", value_enum)]
        require_milestone_complete: Vec<TraceabilityMilestone>,
    },
    Evidence {
        #[arg(long)]
        json: bool,
        #[arg(long)]
        require_complete: bool,
    },
    Audit {
        #[arg(long)]
        json: bool,
        #[arg(long)]
        require_complete: bool,
        #[arg(long, value_enum, default_value = "combined")]
        claim: ReadinessClaimArg,
    },
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum BenchMode {
    Daemon,
    InProcess,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum TraceabilityClaim {
    Search,
    Browser,
    Combined,
}

impl TraceabilityClaim {
    fn report_name(self) -> &'static str {
        match self {
            TraceabilityClaim::Search => "google_style_search",
            TraceabilityClaim::Browser => "chromium_class_browser",
            TraceabilityClaim::Combined => "combined_competitor",
        }
    }
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum TraceabilityMilestone {
    M0,
    M1,
    M2,
    M3,
    M4,
    M5,
    M6,
}

impl TraceabilityMilestone {
    fn report_name(self) -> &'static str {
        match self {
            TraceabilityMilestone::M0 => "M0",
            TraceabilityMilestone::M1 => "M1",
            TraceabilityMilestone::M2 => "M2",
            TraceabilityMilestone::M3 => "M3",
            TraceabilityMilestone::M4 => "M4",
            TraceabilityMilestone::M5 => "M5",
            TraceabilityMilestone::M6 => "M6",
        }
    }
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum ReadinessClaimArg {
    Search,
    Browser,
    Combined,
}

impl From<ReadinessClaimArg> for ReadinessClaim {
    fn from(value: ReadinessClaimArg) -> Self {
        match value {
            ReadinessClaimArg::Search => ReadinessClaim::GoogleStyleSearch,
            ReadinessClaimArg::Browser => ReadinessClaim::ChromiumClassBrowser,
            ReadinessClaimArg::Combined => ReadinessClaim::CombinedCompetitor,
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Command::Smoke {
            corpus,
            index,
            queries,
            limit,
            warmup,
            json,
            save_report,
            report_output,
        } => {
            let report = run_smoke(SmokeOptions {
                corpus,
                index: index.clone(),
                queries,
                limit,
                warmup,
            })?;
            maybe_save_report(
                &index,
                save_report,
                report_output.as_deref(),
                &BenchStatusReport::Smoke(report.clone()),
            )?;
            if json {
                println!("{}", serde_json::to_string_pretty(&report)?);
            } else {
                println!("smoke_corpus: {}", report.corpus);
                println!("smoke_index: {}", report.index);
                println!("smoke_queries: {}", report.queries);
                println!("smoke_docs: {}", report.build.doc_count);
                println!("smoke_terms: {}", report.build.term_count);
                println!("smoke_query: {}", report.query);
                println!("smoke_results: {}", report.result_count);
                println!("smoke_top_doc_id: {}", report.top_doc_id);
                println!("smoke_rendered_bytes: {}", report.rendered_bytes);
                print_report("smoke_bench", &report.bench);
            }
        }
        Command::Search {
            index,
            queries,
            limit,
            warmup,
            socket,
            mode,
            json,
            chromium_baseline,
            require_speedup,
            save_report,
            report_output,
        } => {
            let index_dir = index.clone();
            let options = BenchOptions {
                index,
                queries,
                limit,
                warmup,
                socket,
                use_daemon: matches!(mode, BenchMode::Daemon),
            };

            if chromium_baseline {
                let mut comparison = run_search_comparison(options).await?;
                let failed_gate = require_speedup
                    .map(|required| !comparison.apply_gate(required))
                    .unwrap_or(false);
                maybe_save_report(
                    &index_dir,
                    save_report,
                    report_output.as_deref(),
                    &BenchStatusReport::Comparison(comparison.clone()),
                )?;
                if json {
                    println!("{}", serde_json::to_string_pretty(&comparison)?);
                } else {
                    print_report("rust", &comparison.rust);
                    println!("---");
                    print_report("chromium", &comparison.chromium);
                    println!("p95_speedup: {:.2}x", comparison.p95_speedup);
                    if let Some(required) = comparison.required_p95_speedup {
                        println!("required_p95_speedup: {required:.2}x");
                        println!("gate_passed: {}", comparison.passed.unwrap_or(false));
                    }
                }
                if failed_gate {
                    bail!(
                        "benchmark gate failed: p95 speedup {:.2}x is below required {:.2}x",
                        comparison.p95_speedup,
                        require_speedup.unwrap()
                    );
                }
            } else {
                if require_speedup.is_some() {
                    bail!("--require-speedup requires --chromium-baseline");
                }
                let report = run_search_bench(options).await?;
                maybe_save_report(
                    &index_dir,
                    save_report,
                    report_output.as_deref(),
                    &BenchStatusReport::Search(report.clone()),
                )?;
                if json {
                    println!("{}", serde_json::to_string_pretty(&report)?);
                } else {
                    print_report("rust", &report);
                }
            }
        }
        Command::Eval {
            index,
            judgments,
            limit,
            json,
            save_report,
            report_output,
            require_mrr,
            require_ndcg,
            require_recall,
            require_precision,
            max_unresolved,
        } => {
            let mut report = run_eval(EvalOptions {
                index: index.clone(),
                judgments,
                limit,
            })?;
            let gate = EvalGate {
                required_mrr: require_mrr,
                required_ndcg_at_k: require_ndcg,
                required_recall_at_k: require_recall,
                required_precision_at_k: require_precision,
                max_unresolved_judgment_count: max_unresolved,
            };
            let failed_gate = if gate.is_empty() {
                false
            } else {
                !report.apply_gate(gate)
            };
            maybe_save_report(
                &index,
                save_report,
                report_output.as_deref(),
                &BenchStatusReport::Eval(report.clone()),
            )?;
            if json {
                println!("{}", serde_json::to_string_pretty(&report)?);
            } else {
                print_eval_report(&report);
            }
            if failed_gate {
                bail!(
                    "relevance gate failed: mrr {:.4}, ndcg {:.4}, recall {:.4}, precision {:.4}, unresolved {}",
                    report.mean_reciprocal_rank,
                    report.mean_ndcg_at_k,
                    report.mean_recall_at_k,
                    report.mean_precision_at_k,
                    report.unresolved_judgment_count
                );
            }
        }
        Command::Gate {
            corpus,
            index,
            queries,
            judgments,
            browser_manifest,
            limit,
            warmup,
            socket,
            mode,
            chromium_search_baseline,
            require_speedup,
            browser_chromium_parity,
            browser_compat,
            browser_compat_manifest,
            browser_compat_expectations,
            browser_compat_subsets,
            browser_compat_repeat,
            browser_compat_timeout_ms,
            browser_compat_min_pass_rate,
            browser_compat_max_unexpected_failures,
            browser_compat_max_failures,
            browser_compat_max_timeouts,
            browser_compat_max_crashes,
            browser_compat_max_flakes,
            browser_compat_max_skipped,
            browser_compat_max_unsupported,
            browser_compat_required_subsystems,
            browser_compat_min_subsystem_pass_rate,
            require_browser_features,
            min_browser_implemented_ratio,
            max_browser_missing,
            require_mrr,
            require_ndcg,
            require_recall,
            require_precision,
            max_unresolved,
            json,
            save_report,
            report_output,
        } => {
            let report = run_gate(GateOptions {
                corpus,
                index: index.clone(),
                queries,
                judgments,
                browser_manifest,
                limit,
                warmup,
                socket,
                use_daemon: matches!(mode, BenchMode::Daemon),
                eval_gate: EvalGate {
                    required_mrr: require_mrr,
                    required_ndcg_at_k: require_ndcg,
                    required_recall_at_k: require_recall,
                    required_precision_at_k: require_precision,
                    max_unresolved_judgment_count: max_unresolved,
                },
                browser_coverage_gate: BrowserCoverageGate {
                    required_features: require_browser_features,
                    min_implemented_ratio: min_browser_implemented_ratio,
                    max_missing_features: max_browser_missing,
                },
                chromium_search_baseline,
                required_p95_speedup: require_speedup,
                browser_chromium_parity,
                browser_compat,
                browser_compat_manifest,
                browser_compat_expectations,
                browser_compat_subsets,
                browser_compat_repeat,
                browser_compat_timeout_ms,
                browser_compat_gate: BrowserCompatGate {
                    min_pass_rate: browser_compat_min_pass_rate,
                    max_unexpected_failures: browser_compat_max_unexpected_failures,
                    max_failures: browser_compat_max_failures,
                    max_timeouts: browser_compat_max_timeouts,
                    max_crashes: browser_compat_max_crashes,
                    max_flakes: browser_compat_max_flakes,
                    max_skipped: browser_compat_max_skipped,
                    max_unsupported: browser_compat_max_unsupported,
                    required_subsystems: browser_compat_required_subsystems,
                    min_subsystem_pass_rate: browser_compat_min_subsystem_pass_rate,
                },
            })
            .await?;
            maybe_save_report(
                &index,
                save_report,
                report_output.as_deref(),
                &BenchStatusReport::Gate(Box::new(report.clone())),
            )?;
            if json {
                println!("{}", serde_json::to_string_pretty(&report)?);
            } else {
                print_gate_report(&report);
            }
            if !report.passed {
                bail!("competition gate failed: {}", report.failures.join("; "));
            }
        }
        Command::BrowserPerf {
            manifest,
            iterations,
            warmup,
            max_p95_us,
            min_throughput_pages_per_sec,
            chromium_baseline,
            min_chromium_p95_speedup,
            max_chromium_text_mismatches,
            max_layer_metrics_p95_us,
            min_total_layers,
            min_total_image_layers,
            max_layer_count,
            max_image_layer_count,
            json,
            save_report,
            report_dir,
            report_output,
        } => {
            let mut report = run_browser_perf(BrowserPerfOptions {
                manifest,
                iterations,
                warmup,
                chromium_baseline,
            })?;
            let gate = BrowserPerfGate {
                max_p95_us,
                min_throughput_pages_per_sec,
                min_chromium_p95_speedup,
                max_chromium_text_mismatches,
                max_layer_metrics_p95_us,
                min_total_layers,
                min_total_image_layers,
                max_layer_count,
                max_image_layer_count,
            };
            let failed_gate = if gate.is_empty() {
                false
            } else {
                !report.apply_gate(gate)
            };
            maybe_save_report(
                &report_dir,
                save_report,
                report_output.as_deref(),
                &BenchStatusReport::BrowserPerf(Box::new(report.clone())),
            )?;
            if json {
                println!("{}", serde_json::to_string_pretty(&report)?);
            } else {
                print_browser_perf_report(&report);
            }
            if failed_gate {
                bail!(
                    "browser performance gate failed: p95 {} us, throughput {:.2} pages/sec",
                    report.p95_us,
                    report.throughput_pages_per_sec
                );
            }
        }
        Command::BrowserCompat {
            manifest,
            expectations,
            subsets,
            repeat,
            timeout_ms,
            min_pass_rate,
            max_unexpected_failures,
            max_failures,
            max_timeouts,
            max_crashes,
            max_flakes,
            max_skipped,
            max_unsupported,
            required_subsystems,
            min_subsystem_pass_rate,
            json,
            save_report,
            report_dir,
            report_output,
        } => {
            let gate = BrowserCompatGate {
                min_pass_rate,
                max_unexpected_failures,
                max_failures,
                max_timeouts,
                max_crashes,
                max_flakes,
                max_skipped,
                max_unsupported,
                required_subsystems,
                min_subsystem_pass_rate,
            };
            let mut report = run_browser_compat(BrowserCompatOptions {
                manifest,
                expectations,
                subsets,
                repeat,
                timeout_ms,
                gate: BrowserCompatGate::default(),
            })?;
            let failed_gate = if gate.is_empty() {
                false
            } else {
                !report.apply_gate(gate)
            };
            maybe_save_report(
                &report_dir,
                save_report,
                report_output.as_deref(),
                &BenchStatusReport::BrowserCompat(report.clone()),
            )?;
            if json {
                println!("{}", serde_json::to_string_pretty(&report)?);
            } else {
                print_browser_compat_report(&report);
            }
            if failed_gate {
                bail!(
                    "browser compatibility gate failed: pass_rate {:.4}, unexpected {}, failures [{}]",
                    report.pass_rate,
                    report.unexpected_count,
                    report.gate_failures.join("; ")
                );
            }
        }
        Command::Readiness {
            json,
            require_complete,
            claim,
        } => {
            let report = run_competitor_readiness_for_claim(claim.into());
            if json {
                println!("{}", serde_json::to_string_pretty(&report)?);
            } else {
                print_readiness_report(&report);
            }
            if require_complete && !report.passed {
                bail!(
                    "competitor readiness failed for {}: {} partial, {} missing",
                    report.claim,
                    report.partial_count,
                    report.missing_count
                );
            }
        }
        Command::Traceability {
            json,
            require_complete,
            require_no_missing,
            require_claim_complete,
            require_milestone_complete,
        } => {
            let report = run_traceability_report();
            if json {
                println!("{}", serde_json::to_string_pretty(&report)?);
            } else {
                print_traceability_report(&report);
            }
            if !report.passed {
                bail!(
                    "traceability validation failed: {}",
                    report.validation_errors.join("; ")
                );
            }
            if require_no_missing && report.missing_count > 0 {
                bail!(
                    "traceability missing-state gate failed: {} missing current states",
                    report.missing_count
                );
            }
            if require_complete && !report.complete {
                bail!(
                    "traceability completeness gate failed: {} partial, {} missing",
                    report.partial_count,
                    report.missing_count
                );
            }
            for claim in require_claim_complete {
                let claim_name = claim.report_name();
                let Some(claim_report) = report
                    .claim_reports
                    .iter()
                    .find(|claim_report| claim_report.claim == claim_name)
                else {
                    bail!("traceability claim gate failed: unknown claim {claim_name}");
                };
                if !claim_report.complete {
                    bail!(
                        "traceability claim gate failed for {}: {} partial, {} missing",
                        claim_report.claim,
                        claim_report.partial_count,
                        claim_report.missing_count
                    );
                }
            }
            for milestone in require_milestone_complete {
                let milestone_name = milestone.report_name();
                let Some(milestone_report) = report
                    .milestone_reports
                    .iter()
                    .find(|milestone_report| milestone_report.milestone == milestone_name)
                else {
                    bail!("traceability milestone gate failed: unknown milestone {milestone_name}");
                };
                if !milestone_report.complete {
                    bail!(
                        "traceability milestone gate failed for {}: {} partial, {} missing",
                        milestone_report.milestone,
                        milestone_report.partial_count,
                        milestone_report.missing_count
                    );
                }
            }
        }
        Command::Evidence {
            json,
            require_complete,
        } => {
            let report = run_evidence_registry_report();
            if json {
                println!("{}", serde_json::to_string_pretty(&report)?);
            } else {
                print_evidence_registry_report(&report);
            }
            if require_complete && !report.passed {
                bail!(
                    "evidence registry completeness gate failed: {} uncovered required IDs, {} unknown IDs, {} duplicate evidence IDs",
                    report.missing_required_ids.len(),
                    report.unknown_requirement_ids.len(),
                    report.duplicate_evidence_ids.len()
                );
            }
            if !report.passed {
                bail!(
                    "evidence registry validation failed: {}",
                    report.validation_errors.join("; ")
                );
            }
        }
        Command::Audit {
            json,
            require_complete,
            claim,
        } => {
            let report = run_competitor_audit_for_claim(claim.into());
            if json {
                println!("{}", serde_json::to_string_pretty(&report)?);
            } else {
                print_competitor_audit_report(&report);
            }
            if require_complete && !report.passed {
                bail!(
                    "competitor audit failed for {}: {}",
                    report.claim,
                    report.failures.join("; ")
                );
            }
        }
    }

    Ok(())
}

fn print_competitor_audit_report(report: &CompetitorAuditReport) {
    println!("audit_passed: {}", report.passed);
    println!("audit_claim: {}", report.claim);
    println!("audit_traceability_passed: {}", report.traceability_passed);
    println!(
        "audit_traceability_claim_complete: {}",
        report.traceability_claim_complete
    );
    println!("audit_evidence_passed: {}", report.evidence_passed);
    println!("audit_readiness_passed: {}", report.readiness_passed);
    println!("audit_failure_count: {}", report.failure_count);
    if report.failures.is_empty() {
        println!("audit_failures: none");
    } else {
        for failure in &report.failures {
            println!("audit_failure: {failure}");
        }
    }
    if report.partial_requirement_ids.is_empty() {
        println!("audit_partial_requirement_ids: none");
    } else {
        println!(
            "audit_partial_requirement_ids: {}",
            report.partial_requirement_ids.join(",")
        );
    }
    if report.missing_requirement_ids.is_empty() {
        println!("audit_missing_requirement_ids: none");
    } else {
        println!(
            "audit_missing_requirement_ids: {}",
            report.missing_requirement_ids.join(",")
        );
    }
    if report.partial_readiness_areas.is_empty() {
        println!("audit_partial_readiness_areas: none");
    } else {
        println!(
            "audit_partial_readiness_areas: {}",
            report.partial_readiness_areas.join(",")
        );
    }
    if report.missing_readiness_areas.is_empty() {
        println!("audit_missing_readiness_areas: none");
    } else {
        println!(
            "audit_missing_readiness_areas: {}",
            report.missing_readiness_areas.join(",")
        );
    }
}

fn print_evidence_registry_report(report: &EvidenceRegistryReport) {
    println!("evidence_passed: {}", report.passed);
    println!("evidence_rows: {}", report.row_count);
    println!(
        "evidence_covered_requirements: {}",
        report.covered_requirement_count
    );
    if report.missing_required_ids.is_empty() {
        println!("evidence_missing_required_ids: none");
    } else {
        println!(
            "evidence_missing_required_ids: {}",
            report.missing_required_ids.join(",")
        );
    }
    if report.unknown_requirement_ids.is_empty() {
        println!("evidence_unknown_requirement_ids: none");
    } else {
        println!(
            "evidence_unknown_requirement_ids: {}",
            report.unknown_requirement_ids.join(",")
        );
    }
    if report.duplicate_evidence_ids.is_empty() {
        println!("evidence_duplicate_ids: none");
    } else {
        println!(
            "evidence_duplicate_ids: {}",
            report.duplicate_evidence_ids.join(",")
        );
    }
    if report.validation_errors.is_empty() {
        println!("evidence_validation_errors: none");
    } else {
        for error in &report.validation_errors {
            println!("evidence_validation_error: {error}");
        }
    }
    for row in &report.rows {
        println!(
            "evidence_row: {} requirements={} required_for={}",
            row.evidence_id,
            row.requirement_ids.len(),
            row.required_for
        );
    }
}

fn print_traceability_report(report: &TraceabilityReport) {
    println!("traceability_passed: {}", report.passed);
    println!("traceability_complete: {}", report.complete);
    println!("traceability_required_rows: {}", report.required_count);
    println!("traceability_rows: {}", report.row_count);
    println!("traceability_implemented: {}", report.implemented_count);
    println!("traceability_partial: {}", report.partial_count);
    println!("traceability_missing: {}", report.missing_count);
    if report.missing_required_ids.is_empty() {
        println!("traceability_missing_required_ids: none");
    } else {
        println!(
            "traceability_missing_required_ids: {}",
            report.missing_required_ids.join(",")
        );
    }
    if report.unknown_ids.is_empty() {
        println!("traceability_unknown_ids: none");
    } else {
        println!("traceability_unknown_ids: {}", report.unknown_ids.join(","));
    }
    if report.unknown_readiness_areas.is_empty() {
        println!("traceability_unknown_readiness_areas: none");
    } else {
        println!(
            "traceability_unknown_readiness_areas: {}",
            report.unknown_readiness_areas.join(",")
        );
    }
    if report.unknown_milestones.is_empty() {
        println!("traceability_unknown_milestones: none");
    } else {
        println!(
            "traceability_unknown_milestones: {}",
            report.unknown_milestones.join(",")
        );
    }
    if report.unknown_claim_scopes.is_empty() {
        println!("traceability_unknown_claim_scopes: none");
    } else {
        println!(
            "traceability_unknown_claim_scopes: {}",
            report.unknown_claim_scopes.join(",")
        );
    }
    if report.mismatched_claim_scopes.is_empty() {
        println!("traceability_mismatched_claim_scopes: none");
    } else {
        println!(
            "traceability_mismatched_claim_scopes: {}",
            report.mismatched_claim_scopes.join(",")
        );
    }
    if report.duplicate_ids.is_empty() {
        println!("traceability_duplicate_ids: none");
    } else {
        println!(
            "traceability_duplicate_ids: {}",
            report.duplicate_ids.join(",")
        );
    }
    if report.validation_errors.is_empty() {
        println!("traceability_validation_errors: none");
    } else {
        for error in &report.validation_errors {
            println!("traceability_validation_error: {error}");
        }
    }
    for (area, count) in &report.rows_by_readiness_area {
        println!("traceability_readiness_area: {area} rows={count}");
    }
    for (milestone, count) in &report.rows_by_milestone {
        println!("traceability_milestone: {milestone} rows={count}");
    }
    for (claim_scope, count) in &report.rows_by_claim_scope {
        println!("traceability_claim_scope: {claim_scope} rows={count}");
    }
    for claim_report in &report.claim_reports {
        println!(
            "traceability_claim_report: {} complete={} requirements={} implemented={} partial={} missing={} scopes={}",
            claim_report.claim,
            claim_report.complete,
            claim_report.requirement_count,
            claim_report.implemented_count,
            claim_report.partial_count,
            claim_report.missing_count,
            claim_report.required_claim_scopes.join(",")
        );
        if claim_report.partial_ids.is_empty() {
            println!(
                "traceability_claim_report_partial_ids: {} none",
                claim_report.claim
            );
        } else {
            println!(
                "traceability_claim_report_partial_ids: {} {}",
                claim_report.claim,
                claim_report.partial_ids.join(",")
            );
        }
        if claim_report.missing_ids.is_empty() {
            println!(
                "traceability_claim_report_missing_ids: {} none",
                claim_report.claim
            );
        } else {
            println!(
                "traceability_claim_report_missing_ids: {} {}",
                claim_report.claim,
                claim_report.missing_ids.join(",")
            );
        }
    }
    for milestone_report in &report.milestone_reports {
        println!(
            "traceability_milestone_report: {} complete={} requirements={} implemented={} partial={} missing={} includes={}",
            milestone_report.milestone,
            milestone_report.complete,
            milestone_report.requirement_count,
            milestone_report.implemented_count,
            milestone_report.partial_count,
            milestone_report.missing_count,
            milestone_report.included_milestones.join(",")
        );
        if milestone_report.partial_ids.is_empty() {
            println!(
                "traceability_milestone_report_partial_ids: {} none",
                milestone_report.milestone
            );
        } else {
            println!(
                "traceability_milestone_report_partial_ids: {} {}",
                milestone_report.milestone,
                milestone_report.partial_ids.join(",")
            );
        }
        if milestone_report.missing_ids.is_empty() {
            println!(
                "traceability_milestone_report_missing_ids: {} none",
                milestone_report.milestone
            );
        } else {
            println!(
                "traceability_milestone_report_missing_ids: {} {}",
                milestone_report.milestone,
                milestone_report.missing_ids.join(",")
            );
        }
    }
    for requirement in &report.requirements {
        println!(
            "traceability_requirement: {} status={} area={} milestone={} claim_scope={} capability={}",
            requirement.id,
            readiness_status(requirement.current_state),
            requirement.readiness_area,
            requirement.milestone,
            requirement.claim_scope,
            requirement.required_capability
        );
    }
}

fn print_readiness_report(report: &CompetitorReadinessReport) {
    println!("readiness_passed: {}", report.passed);
    println!("readiness_claim: {}", report.claim);
    println!(
        "readiness_required_claim_scopes: {}",
        report.required_claim_scopes.join(",")
    );
    println!("readiness_standard: {}", report.standard);
    println!("readiness_areas: {}", report.area_count);
    println!("readiness_implemented: {}", report.implemented_count);
    println!("readiness_partial: {}", report.partial_count);
    println!("readiness_missing: {}", report.missing_count);
    for area in &report.areas {
        println!("readiness_area: {}", area.area);
        println!(
            "readiness_area_claim_scopes: {}",
            area.claim_scopes.join(",")
        );
        println!("readiness_area_status: {}", readiness_status(area.status));
        println!(
            "readiness_area_required_end_state: {}",
            area.required_end_state
        );
        println!("readiness_area_evidence_gate: {}", area.evidence_gate);
        if area.current_evidence.is_empty() {
            println!("readiness_area_current_evidence: none");
        } else {
            for evidence in &area.current_evidence {
                println!("readiness_area_current_evidence: {evidence}");
            }
        }
        if area.missing_work.is_empty() {
            println!("readiness_area_missing_work: none");
        } else {
            for missing in &area.missing_work {
                println!("readiness_area_missing_work: {missing}");
            }
        }
    }
}

fn readiness_status(status: ReadinessStatus) -> &'static str {
    match status {
        ReadinessStatus::Implemented => "implemented",
        ReadinessStatus::Partial => "partial",
        ReadinessStatus::Missing => "missing",
    }
}

fn maybe_save_report(
    index: &std::path::Path,
    save_report: bool,
    report_output: Option<&std::path::Path>,
    report: &BenchStatusReport,
) -> Result<()> {
    if let Some(path) = report_output {
        write_bench_status_path(path, report)?;
        eprintln!("saved_report: {}", path.display());
    } else if save_report {
        let path = write_bench_status(index, report)?;
        eprintln!("saved_report: {}", path.display());
    }
    Ok(())
}

fn print_gate_report(report: &GateReport) {
    println!("gate_passed: {}", report.passed);
    if report.failures.is_empty() {
        println!("gate_failures: none");
    } else {
        for failure in &report.failures {
            println!("gate_failure: {failure}");
        }
    }
    println!("gate_smoke_docs: {}", report.smoke.build.doc_count);
    println!("gate_smoke_terms: {}", report.smoke.build.term_count);
    print_report("gate_smoke_bench", &report.smoke.bench);
    print_eval_report(&report.eval);
    print_browser_coverage_report(&report.browser_coverage);
    if let Some(comparison) = &report.search_comparison {
        print_report("gate_rust", &comparison.rust);
        print_report("gate_chromium", &comparison.chromium);
        println!("gate_p95_speedup: {:.2}x", comparison.p95_speedup);
        if let Some(required) = comparison.required_p95_speedup {
            println!("gate_required_p95_speedup: {required:.2}x");
            println!("gate_speed_passed: {}", comparison.passed.unwrap_or(false));
        }
    } else {
        println!("gate_speed_comparison: skipped");
    }
    if let Some(parity) = &report.browser_chromium_parity {
        print_browser_chromium_parity_report(parity);
    } else {
        println!("gate_browser_chromium_parity: skipped");
    }
    if let Some(compat) = &report.browser_compat {
        print_browser_compat_report(compat);
    } else {
        println!("gate_browser_compat: skipped");
    }
}

fn print_eval_report(report: &EvalReport) {
    println!("eval_queries: {}", report.query_count);
    println!("eval_evaluated_queries: {}", report.evaluated_query_count);
    println!("eval_limit: {}", report.limit);
    println!("eval_mrr: {:.4}", report.mean_reciprocal_rank);
    println!("eval_ndcg_at_k: {:.4}", report.mean_ndcg_at_k);
    println!("eval_recall_at_k: {:.4}", report.mean_recall_at_k);
    println!("eval_precision_at_k: {:.4}", report.mean_precision_at_k);
    println!(
        "eval_unresolved_judgments: {}",
        report.unresolved_judgment_count
    );
    println!("eval_corpus_hash: {}", report.corpus_hash);
    println!("eval_index_hash: {}", report.index_hash);
    if report.passed.is_some() {
        println!("eval_required_mrr: {}", option_f64(report.required_mrr));
        println!(
            "eval_required_ndcg_at_k: {}",
            option_f64(report.required_ndcg_at_k)
        );
        println!(
            "eval_required_recall_at_k: {}",
            option_f64(report.required_recall_at_k)
        );
        println!(
            "eval_required_precision_at_k: {}",
            option_f64(report.required_precision_at_k)
        );
        println!(
            "eval_max_unresolved_judgment_count: {}",
            report
                .max_unresolved_judgment_count
                .map(|value| value.to_string())
                .unwrap_or_else(|| "none".to_owned())
        );
        println!("eval_gate_passed: {}", report.passed.unwrap_or(false));
    }
    for query in &report.queries {
        println!(
            "eval_query: {:?} rr={:.4} ndcg={:.4} recall={:.4} precision={:.4} relevant={} found={} unresolved={}",
            query.query,
            query.reciprocal_rank,
            query.ndcg_at_k,
            query.recall_at_k,
            query.precision_at_k,
            query.relevant_count,
            query.retrieved_relevant,
            query.unresolved_judgment_count
        );
    }
}

fn print_browser_coverage_report(report: &BrowserCoverageReport) {
    println!("browser_feature_count: {}", report.feature_count);
    println!("browser_implemented_count: {}", report.implemented_count);
    println!("browser_partial_count: {}", report.partial_count);
    println!("browser_missing_count: {}", report.missing_count);
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
        println!(
            "browser_coverage_passed: {}",
            report.passed.unwrap_or(false)
        );
    }
}

fn print_browser_chromium_parity_report(report: &BrowserChromiumParityReport) {
    println!("browser_parity_fixtures: {}", report.fixture_count);
    println!("browser_parity_passed: {}", report.passed);
    println!("browser_parity_failed: {}", report.failed);
    if let Some(chrome) = &report.chrome {
        println!("browser_parity_chrome: {chrome}");
    }
    for failure in &report.failures {
        println!(
            "browser_parity_failure: {} {} {}",
            failure.name, failure.path, failure.reason
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
    println!("browser_compat_subsystems: {}", report.subsystem_count);
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
            "browser_compat_subsystem: {:?} selected={} runnable={} pass={} fail={} timeout={} crash={} skipped={} unsupported={} unexpected={} pass_rate={:.4}",
            subsystem.subsystem,
            subsystem.suite_count,
            subsystem.runnable_count,
            subsystem.pass_count,
            subsystem.fail_count,
            subsystem.timeout_count,
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

fn print_browser_perf_report(report: &BrowserPerfReport) {
    println!("browser_perf_engine: {}", report.engine);
    println!("browser_perf_manifest: {}", report.manifest);
    println!("browser_perf_fixtures: {}", report.fixture_count);
    println!("browser_perf_iterations: {}", report.iteration_count);
    println!("browser_perf_warmup: {}", report.warmup);
    println!("browser_perf_samples: {}", report.sample_count);
    println!("browser_perf_p50_us: {}", report.p50_us);
    println!("browser_perf_p95_us: {}", report.p95_us);
    println!("browser_perf_p99_us: {}", report.p99_us);
    println!("browser_perf_raster_p50_us: {}", report.raster_p50_us);
    println!("browser_perf_raster_p95_us: {}", report.raster_p95_us);
    println!("browser_perf_raster_p99_us: {}", report.raster_p99_us);
    println!(
        "browser_perf_layer_metrics_p50_us: {}",
        report.layer_metrics_p50_us
    );
    println!(
        "browser_perf_layer_metrics_p95_us: {}",
        report.layer_metrics_p95_us
    );
    println!(
        "browser_perf_layer_metrics_p99_us: {}",
        report.layer_metrics_p99_us
    );
    println!(
        "browser_perf_throughput_pages_per_sec: {:.2}",
        report.throughput_pages_per_sec
    );
    println!("browser_perf_total_ms: {}", report.total_ms);
    println!(
        "browser_perf_total_rendered_bytes: {}",
        report.total_rendered_bytes
    );
    println!("browser_perf_total_dom_nodes: {}", report.total_dom_nodes);
    println!("browser_perf_total_css_rules: {}", report.total_css_rules);
    println!(
        "browser_perf_total_layout_boxes: {}",
        report.total_layout_boxes
    );
    println!(
        "browser_perf_total_paint_commands: {}",
        report.total_paint_commands
    );
    println!("browser_perf_total_layers: {}", report.total_layers);
    println!(
        "browser_perf_total_image_layers: {}",
        report.total_image_layers
    );
    println!("browser_perf_max_layer_count: {}", report.max_layer_count);
    println!(
        "browser_perf_max_image_layer_count: {}",
        report.max_image_layer_count
    );
    println!(
        "browser_perf_max_root_layer: {}x{}",
        report.max_root_layer_width, report.max_root_layer_height
    );
    println!("browser_perf_max_layer_area: {}", report.max_layer_area);
    println!("browser_perf_total_layer_area: {}", report.total_layer_area);
    println!(
        "browser_perf_total_layer_metrics_us: {}",
        report.total_layer_metrics_us
    );
    println!("browser_perf_total_raster_us: {}", report.total_raster_us);
    println!(
        "browser_perf_total_raster_pixels: {}",
        report.total_raster_pixels
    );
    println!(
        "browser_perf_total_raster_non_background_pixels: {}",
        report.total_raster_non_background_pixels
    );
    print_browser_phase_timings("browser_perf_phase", &report.phase_totals);
    println!("browser_perf_suite_hash: {}", report.suite_hash);
    if let Some(max_p95) = report.required_max_p95_us {
        println!("browser_perf_required_max_p95_us: {max_p95}");
    }
    if let Some(min_throughput) = report.required_min_throughput_pages_per_sec {
        println!("browser_perf_required_min_throughput_pages_per_sec: {min_throughput:.2}");
    }
    if let Some(speedup) = report.chromium_p95_speedup {
        println!("browser_perf_chromium_p95_speedup: {speedup:.3}");
    }
    if let Some(min_speedup) = report.required_min_chromium_p95_speedup {
        println!("browser_perf_required_min_chromium_p95_speedup: {min_speedup:.3}");
    }
    if let Some(max_mismatches) = report.required_max_chromium_text_mismatches {
        println!("browser_perf_required_max_chromium_text_mismatches: {max_mismatches}");
    }
    if let Some(baseline) = &report.chromium_baseline {
        println!("browser_perf_chromium_engine: {}", baseline.engine);
        println!("browser_perf_chromium_samples: {}", baseline.sample_count);
        println!(
            "browser_perf_chromium_text_matches: {}",
            baseline.text_match_count
        );
        println!(
            "browser_perf_chromium_text_mismatches: {}",
            baseline.text_mismatch_count
        );
        println!("browser_perf_chromium_p50_us: {}", baseline.p50_us);
        println!("browser_perf_chromium_p95_us: {}", baseline.p95_us);
        println!("browser_perf_chromium_p99_us: {}", baseline.p99_us);
        println!(
            "browser_perf_chromium_throughput_pages_per_sec: {:.2}",
            baseline.throughput_pages_per_sec
        );
        println!("browser_perf_chromium_total_ms: {}", baseline.total_ms);
    }
    if let Some(max_layer_metrics_p95) = report.required_max_layer_metrics_p95_us {
        println!("browser_perf_required_max_layer_metrics_p95_us: {max_layer_metrics_p95}");
    }
    if let Some(min_total_layers) = report.required_min_total_layers {
        println!("browser_perf_required_min_total_layers: {min_total_layers}");
    }
    if let Some(min_total_image_layers) = report.required_min_total_image_layers {
        println!("browser_perf_required_min_total_image_layers: {min_total_image_layers}");
    }
    if let Some(max_layer_count) = report.required_max_layer_count {
        println!("browser_perf_required_max_layer_count: {max_layer_count}");
    }
    if let Some(max_image_layer_count) = report.required_max_image_layer_count {
        println!("browser_perf_required_max_image_layer_count: {max_image_layer_count}");
    }
    if let Some(passed) = report.passed {
        println!("browser_perf_gate_passed: {passed}");
    }
    if let Some(rustc) = &report.rustc {
        println!("browser_perf_rustc: {rustc}");
    }
    if let Some(chrome) = &report.chrome {
        println!("browser_perf_chrome: {chrome}");
    }
    if let Some(os) = &report.os {
        println!("browser_perf_os: {os}");
    }
    if let Some(hardware) = &report.hardware {
        println!("browser_perf_hardware: {hardware}");
    }
    for fixture in &report.fixtures {
        println!(
            "browser_perf_fixture: {:?} p50={}us p95={}us p99={}us raster_p50={}us raster_p95={}us raster_p99={}us raster_total={}us layer_metrics_p50={}us layer_metrics_p95={}us layer_metrics_p99={}us layer_metrics_total={}us bytes={} nodes={} layout_boxes={} paint_commands={} layers={} image_layers={} root_layer={}x{} max_layer_area={} total_layer_area={} raster_pixels={} raster_non_background_pixels={}",
            fixture.name,
            fixture.p50_us,
            fixture.p95_us,
            fixture.p99_us,
            fixture.raster_p50_us,
            fixture.raster_p95_us,
            fixture.raster_p99_us,
            fixture.raster_total_us,
            fixture.layer_metrics_p50_us,
            fixture.layer_metrics_p95_us,
            fixture.layer_metrics_p99_us,
            fixture.layer_metrics_total_us,
            fixture.rendered_bytes,
            fixture.dom_node_count,
            fixture.layout_box_count,
            fixture.paint_command_count,
            fixture.layer_count,
            fixture.image_layer_count,
            fixture.root_layer_width,
            fixture.root_layer_height,
            fixture.max_layer_area,
            fixture.total_layer_area,
            fixture.raster_pixels,
            fixture.raster_non_background_pixels
        );
        print_browser_phase_timings(
            &format!(
                "browser_perf_fixture_phase_{}",
                fixture.name.replace(' ', "_")
            ),
            &fixture.phase_totals,
        );
    }
    if let Some(baseline) = &report.chromium_baseline {
        for fixture in &baseline.fixtures {
            println!(
                "browser_perf_chromium_fixture: {:?} p50={}us p95={}us p99={}us bytes={} text_match={} text_hash={}",
                fixture.name,
                fixture.p50_us,
                fixture.p95_us,
                fixture.p99_us,
                fixture.rendered_bytes,
                fixture
                    .text_match
                    .map(|matched| matched.to_string())
                    .unwrap_or_else(|| "unchecked".to_owned()),
                fixture.text_hash
            );
        }
    }
}

fn print_browser_phase_timings(
    label: &str,
    timings: &brutal_search::browser::BrowserRenderTimings,
) {
    println!("{label}_parse_us: {}", timings.parse_us);
    println!("{label}_script_us: {}", timings.script_us);
    println!("{label}_style_us: {}", timings.style_us);
    println!("{label}_collect_us: {}", timings.collect_us);
    println!("{label}_layout_us: {}", timings.layout_us);
    println!("{label}_total_us: {}", timings.total_us);
}

fn option_f64(value: Option<f64>) -> String {
    value
        .map(|value| format!("{value:.4}"))
        .unwrap_or_else(|| "none".to_owned())
}

fn print_report(label: &str, report: &brutal_search::bench::BenchReport) {
    println!("{label}_engine: {}", report.engine);
    println!("{label}_queries: {}", report.query_count);
    println!("{label}_limit: {}", report.limit);
    println!("{label}_p50_us: {}", report.p50_us);
    println!("{label}_p95_us: {}", report.p95_us);
    println!("{label}_p99_us: {}", report.p99_us);
    println!("{label}_throughput_qps: {:.2}", report.throughput_qps);
    println!("{label}_total_ms: {}", report.total_ms);
    println!("{label}_corpus_hash: {}", report.corpus_hash);
    println!("{label}_index_hash: {}", report.index_hash);
    if let Some(rustc) = &report.rustc {
        println!("{label}_rustc: {rustc}");
    }
    if let Some(chrome) = &report.chrome {
        println!("{label}_chrome: {chrome}");
    }
    if let Some(os) = &report.os {
        println!("{label}_os: {os}");
    }
    if let Some(hardware) = &report.hardware {
        println!("{label}_hardware: {hardware}");
    }
}
