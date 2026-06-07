use std::collections::{BTreeMap, HashMap, HashSet};
use std::env;
use std::io::{BufRead, Write};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use brutal_search::crawler::{
    CrawlBoundary, CrawlOptions, crawl_many, domain_to_seed, load_domain_file, load_seed_file,
};
use brutal_search::daemon::{default_socket_path, send_request};
use brutal_search::frontier::{
    DEFAULT_MAX_FAILED_FRONTIER_RECORDS, FrontierStats, FrontierStore, RecrawlPlanEntry, unix_now,
};
use brutal_search::index::{
    IndexBuildOptions, PreloadMode, SearchIndex, TermCorrection, TermSuggestion, build_from_corpus,
    build_from_fielded_documents,
};
use brutal_search::protocol::{DaemonRequest, DaemonResponse};
use brutal_search::query::{SearchOptions, SearchResult};
use brutal_search::recrawl::{RecrawlScheduleOptions, load_recrawl_manifest_with_options};
use brutal_search::render::render_target;
use brutal_search::scheduler::{
    RecrawlCrawlOptions, RecrawlRoundReport, RecrawlSchedulerOptions, run_recrawl_round,
};
use brutal_search::server::run_search_server;
use brutal_search::sitemap::{
    SitemapLoadOptions, discover_sitemap_sources_from_robots, load_sitemap_seeds,
};
use brutal_search::web_search::{
    DEFAULT_CACHE_MAX_BYTES, DEFAULT_CACHE_MAX_ENTRIES, DEFAULT_RESULT_LOG_MAX_BYTES,
    DEFAULT_RESULT_LOG_MAX_ENTRIES, WebSearchStorageArtifactState,
    WebSearchStorageCompactionOptions, WebSearchStorageCompactionReport,
    compact_web_search_storage_from_env,
};
use clap::{Parser, Subcommand, ValueEnum};

const INDEX_STORAGE_ARTIFACTS: &[&str] = &[
    "manifest.json",
    "docs.bin",
    "field_docs.bin",
    "lexicon.bin",
    "postings.bin",
    "texts.bin",
    "frontier.bin",
    "crawl-docs.jsonl",
    "web-cache.jsonl",
    "brave-results.jsonl",
    "browser-documents.jsonl",
    "bench-status.json",
];
const WEB_STORAGE_COMPACT_SUGGEST_DUPLICATES: usize = 1;
const WEB_STORAGE_COMPACT_SUGGEST_MIN_ENTRIES: usize = 1024;
const WEB_STORAGE_QUERY_EXAMPLE_LIMIT: usize = 3;
const WEB_STORAGE_PROVIDER_GROWTH_LIMIT: usize = 3;
const DEFAULT_WEB_STORAGE_STALE_SECS: u64 = 30 * 24 * 60 * 60;
const DEFAULT_RECRAWL_PLAN_OUTPUT_MAX_BYTES: u64 = 8 * 1024 * 1024;
const DEFAULT_INDEX_STORAGE_BUDGET_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Debug, Parser)]
#[command(version, about = "Brutally fast static HTML text search.")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    Crawl {
        seed_url: Option<String>,
        #[arg(long = "domain")]
        domains: Vec<String>,
        #[arg(long)]
        domain_file: Option<PathBuf>,
        #[arg(long)]
        seed_file: Option<PathBuf>,
        #[arg(long = "recrawl-manifest")]
        recrawl_manifests: Vec<PathBuf>,
        #[arg(long)]
        include_future_recrawls: bool,
        #[arg(long = "sitemap")]
        sitemaps: Vec<String>,
        #[arg(long)]
        discover_sitemaps: bool,
        #[arg(long, default_value_t = 200_000)]
        max_sitemap_urls: usize,
        #[arg(long, default_value_t = 1024)]
        max_sitemaps: usize,
        #[arg(long, default_value = ".brutal-index")]
        index: PathBuf,
        #[arg(long, default_value_t = 50_000)]
        max_pages: usize,
        #[arg(long, default_value_t = 6)]
        max_depth: usize,
        #[arg(long, default_value_t = 64)]
        concurrency: usize,
        #[arg(long, default_value_t = 4 * 1024 * 1024)]
        max_bytes: usize,
        #[arg(long)]
        ignore_robots: bool,
        #[arg(long)]
        ignore_noindex: bool,
        #[arg(long, default_value_t = 0)]
        min_body_terms: u32,
        #[arg(long, value_enum, default_value = "same-host")]
        boundary: CliCrawlBoundary,
        #[arg(long, default_value_t = 4)]
        max_fetching_per_host: usize,
    },
    Index {
        corpus_dir: PathBuf,
        #[arg(long, default_value = ".brutal-index")]
        index: PathBuf,
        #[arg(long)]
        ignore_noindex: bool,
        #[arg(long, default_value_t = 0)]
        min_body_terms: u32,
    },
    Search {
        query: String,
        #[arg(long, default_value = ".brutal-index")]
        index: PathBuf,
        #[arg(long, default_value_t = 20)]
        limit: usize,
        #[arg(long)]
        socket: Option<PathBuf>,
        #[arg(long)]
        no_daemon: bool,
    },
    Suggest {
        prefix: String,
        #[arg(long, default_value = ".brutal-index")]
        index: PathBuf,
        #[arg(long, default_value_t = 10)]
        limit: usize,
        #[arg(long)]
        socket: Option<PathBuf>,
        #[arg(long)]
        no_daemon: bool,
    },
    Spell {
        term: String,
        #[arg(long, default_value = ".brutal-index")]
        index: PathBuf,
        #[arg(long, default_value_t = 5)]
        limit: usize,
        #[arg(long)]
        socket: Option<PathBuf>,
        #[arg(long)]
        no_daemon: bool,
    },
    Render {
        target: String,
        #[arg(long, default_value = ".brutal-index")]
        index: PathBuf,
        #[arg(long)]
        socket: Option<PathBuf>,
        #[arg(long)]
        no_daemon: bool,
    },
    Stats {
        #[arg(long, default_value = ".brutal-index")]
        index: PathBuf,
        #[arg(long)]
        socket: Option<PathBuf>,
        #[arg(long)]
        no_daemon: bool,
    },
    CompactWebCache {
        #[arg(long, default_value = ".brutal-index")]
        index: PathBuf,
        #[arg(long)]
        dry_run: bool,
        #[arg(long)]
        force: bool,
        #[arg(long, default_value_t = 0)]
        min_entries: usize,
    },
    RecrawlPlan {
        #[arg(long, default_value = ".brutal-index")]
        index: PathBuf,
        #[arg(long)]
        output: Option<PathBuf>,
        #[arg(long, default_value_t = 7 * 24 * 60 * 60)]
        interval_secs: u64,
        #[arg(long, default_value_t = 10_000)]
        limit: usize,
    },
    RecrawlScheduler {
        #[arg(long, default_value = ".brutal-index")]
        index: PathBuf,
        #[arg(long, default_value_t = 7 * 24 * 60 * 60)]
        interval_secs: u64,
        #[arg(long, default_value_t = 1000)]
        batch_size: usize,
        #[arg(long, default_value_t = 300)]
        poll_secs: u64,
        #[arg(long)]
        max_rounds: Option<usize>,
        #[arg(long, default_value_t = 0)]
        max_depth: usize,
        #[arg(long, default_value_t = 64)]
        concurrency: usize,
        #[arg(long, default_value_t = 4 * 1024 * 1024)]
        max_bytes: usize,
        #[arg(long)]
        ignore_robots: bool,
        #[arg(long, value_enum, default_value = "same-host")]
        boundary: CliCrawlBoundary,
        #[arg(long, default_value_t = 4)]
        max_fetching_per_host: usize,
    },
    Serve {
        #[arg(long, default_value = ".brutal-index")]
        index: PathBuf,
        #[arg(long, default_value = "127.0.0.1:8765")]
        addr: SocketAddr,
        #[arg(long, default_value = "aggressive")]
        preload: PreloadMode,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Command::Crawl {
            seed_url,
            domains,
            domain_file,
            seed_file,
            recrawl_manifests,
            include_future_recrawls,
            mut sitemaps,
            discover_sitemaps,
            max_sitemap_urls,
            max_sitemaps,
            index,
            max_pages,
            max_depth,
            concurrency,
            max_bytes,
            ignore_robots,
            ignore_noindex,
            min_body_terms,
            boundary,
            max_fetching_per_host,
        } => {
            let mut seeds = Vec::new();
            let mut robots_discovery_seeds = Vec::new();
            let mut recrawl_seeds = Vec::new();
            let mut recrawl_sitemaps = Vec::new();
            if let Some(seed_url) = seed_url {
                robots_discovery_seeds.push(seed_url.clone());
                seeds.push(seed_url);
            }
            for domain in domains {
                let seed = domain_to_seed(&domain)?;
                robots_discovery_seeds.push(seed.clone());
                seeds.push(seed);
            }
            if let Some(domain_file) = domain_file.as_deref() {
                let domain_file_seeds = load_domain_file(domain_file)?;
                robots_discovery_seeds.extend(domain_file_seeds.iter().cloned());
                seeds.extend(domain_file_seeds);
            }
            if let Some(seed_file) = seed_file.as_deref() {
                let seed_file_seeds = load_seed_file(seed_file)?;
                robots_discovery_seeds.extend(seed_file_seeds.iter().cloned());
                seeds.extend(seed_file_seeds);
            }
            let recrawl_options = if include_future_recrawls {
                RecrawlScheduleOptions::include_all()
            } else {
                RecrawlScheduleOptions::due_at(unix_now())
            };
            for recrawl_manifest in recrawl_manifests {
                let recrawl =
                    load_recrawl_manifest_with_options(&recrawl_manifest, recrawl_options)?;
                if recrawl.skipped_future > 0 {
                    eprintln!(
                        "recrawl manifest {} skipped {} future entries",
                        recrawl_manifest.display(),
                        recrawl.skipped_future
                    );
                }
                recrawl_seeds.extend(recrawl.seeds.iter().cloned());
                robots_discovery_seeds.extend(recrawl.seeds.iter().cloned());
                seeds.extend(recrawl.seeds);
                recrawl_sitemaps.extend(recrawl.sitemaps);
            }
            if discover_sitemaps {
                sitemaps.extend(
                    discover_sitemap_sources_from_robots(
                        robots_discovery_seeds.iter().map(String::as_str),
                        max_bytes,
                    )
                    .await?,
                );
            }
            if !recrawl_sitemaps.is_empty() {
                let recrawl_sitemap_seeds = load_sitemap_seeds(
                    recrawl_sitemaps.iter().map(String::as_str),
                    SitemapLoadOptions {
                        max_sitemaps,
                        max_urls: max_sitemap_urls,
                        max_bytes,
                    },
                )
                .await?;
                recrawl_seeds.extend(recrawl_sitemap_seeds.iter().cloned());
                seeds.extend(recrawl_sitemap_seeds);
            }
            if !sitemaps.is_empty() {
                seeds.extend(
                    load_sitemap_seeds(
                        sitemaps.iter().map(String::as_str),
                        SitemapLoadOptions {
                            max_sitemaps,
                            max_urls: max_sitemap_urls,
                            max_bytes,
                        },
                    )
                    .await?,
                );
            }
            if seeds.is_empty() {
                bail!(
                    "provide a seed URL, --domain, --domain-file, --seed-file, --recrawl-manifest, or --sitemap"
                );
            }

            let frontier_path = index.join("frontier.bin");
            let document_snapshot_path = index.join("crawl-docs.jsonl");
            let docs = crawl_many(
                seeds.iter().map(String::as_str),
                CrawlOptions {
                    max_pages,
                    max_depth,
                    concurrency,
                    max_bytes,
                    ignore_robots,
                    boundary: boundary.into(),
                    frontier_path: Some(frontier_path),
                    document_snapshot_path: Some(document_snapshot_path),
                    max_fetching_per_host,
                    recrawl_seeds,
                },
            )
            .await?;
            let stats = build_from_fielded_documents(
                docs,
                index,
                build_options(ignore_noindex, min_body_terms),
            )?;
            print_build_stats(&stats);
        }
        Command::Index {
            corpus_dir,
            index,
            ignore_noindex,
            min_body_terms,
        } => {
            let stats = build_from_corpus(
                corpus_dir,
                index,
                build_options(ignore_noindex, min_body_terms),
            )?;
            print_build_stats(&stats);
        }
        Command::Search {
            query,
            index,
            limit,
            socket,
            no_daemon,
        } => {
            let results = if !no_daemon {
                match try_daemon_search(&index, socket.as_deref(), &query, limit).await {
                    Ok(Some(results)) => results,
                    Ok(None) => one_shot_search(&index, &query, limit)?,
                    Err(error) => {
                        eprintln!("daemon unavailable: {error:#}");
                        one_shot_search(&index, &query, limit)?
                    }
                }
            } else {
                one_shot_search(&index, &query, limit)?
            };
            print_results(&results);
        }
        Command::Suggest {
            prefix,
            index,
            limit,
            socket,
            no_daemon,
        } => {
            let suggestions = if !no_daemon {
                match try_daemon_suggest(&index, socket.as_deref(), &prefix, limit).await {
                    Ok(Some(suggestions)) => suggestions,
                    Ok(None) => one_shot_suggest(&index, &prefix, limit)?,
                    Err(error) => {
                        eprintln!("daemon unavailable: {error:#}");
                        one_shot_suggest(&index, &prefix, limit)?
                    }
                }
            } else {
                one_shot_suggest(&index, &prefix, limit)?
            };
            print_suggestions(&suggestions);
        }
        Command::Spell {
            term,
            index,
            limit,
            socket,
            no_daemon,
        } => {
            let corrections = if !no_daemon {
                match try_daemon_spell(&index, socket.as_deref(), &term, limit).await {
                    Ok(Some(corrections)) => corrections,
                    Ok(None) => one_shot_spell(&index, &term, limit)?,
                    Err(error) => {
                        eprintln!("daemon unavailable: {error:#}");
                        one_shot_spell(&index, &term, limit)?
                    }
                }
            } else {
                one_shot_spell(&index, &term, limit)?
            };
            print_corrections(&corrections);
        }
        Command::Render {
            target,
            index,
            socket,
            no_daemon,
        } => {
            let text = if !no_daemon {
                match try_daemon_render(&index, socket.as_deref(), &target).await {
                    Ok(Some(text)) => text,
                    Ok(None) => one_shot_render(&index, &target)?,
                    Err(error) => {
                        eprintln!("daemon unavailable: {error:#}");
                        one_shot_render(&index, &target)?
                    }
                }
            } else {
                one_shot_render(&index, &target)?
            };
            println!("{text}");
        }
        Command::Stats {
            index,
            socket,
            no_daemon,
        } => {
            if !no_daemon {
                match try_daemon_stats(&index, socket.as_deref()).await {
                    Ok(true) => return Ok(()),
                    Ok(false) => {}
                    Err(error) => eprintln!("daemon unavailable: {error:#}"),
                }
            }
            let index = SearchIndex::open(index, PreloadMode::Lazy)?;
            let manifest = index.manifest();
            println!("docs: {}", manifest.doc_count);
            println!("terms: {}", manifest.term_count);
            println!("total_terms: {}", manifest.total_terms);
            println!("avg_doc_len: {:.2}", manifest.avg_doc_len);
            println!("duplicate_clusters: {}", manifest.duplicate_cluster_count);
            println!("duplicate_docs: {}", manifest.duplicate_doc_count);
            println!("skipped_noindex: {}", manifest.skipped_noindex_count);
            println!("skipped_thin: {}", manifest.skipped_thin_count);
            println!("max_authority_score: {:.4}", manifest.max_authority_score);
            println!("corpus_hash: {}", manifest.corpus_hash);
            print_index_storage_stats(index.root())?;
        }
        Command::CompactWebCache {
            index,
            dry_run,
            force,
            min_entries,
        } => {
            let options = WebSearchStorageCompactionOptions {
                dry_run,
                min_entries,
            };
            if !dry_run {
                let preflight = compact_web_search_storage_from_env(
                    &index,
                    WebSearchStorageCompactionOptions {
                        dry_run: true,
                        min_entries,
                    },
                )?;
                if !web_storage_compaction_apply_is_justified(&preflight, force) {
                    println!(
                        "apply_guard: skipped; projected compaction would not remove bytes or duplicate rows"
                    );
                    println!("apply_guard_force_hint: rerun with --force to rewrite anyway");
                    print_web_storage_compaction_report(&preflight);
                    return Ok(());
                }
                println!("apply_guard: proceeding; projected compaction removes storage pressure");
            }
            let report = compact_web_search_storage_from_env(&index, options)?;
            print_web_storage_compaction_report(&report);
        }
        Command::RecrawlPlan {
            index,
            output,
            interval_secs,
            limit,
        } => {
            let frontier = FrontierStore::open(index.join("frontier.bin"))?;
            let plan = frontier.recrawl_plan(unix_now(), interval_secs, limit);
            write_recrawl_plan(output.as_deref(), &plan)?;
            eprintln!("recrawl plan entries: {}", plan.len());
        }
        Command::RecrawlScheduler {
            index,
            interval_secs,
            batch_size,
            poll_secs,
            max_rounds,
            max_depth,
            concurrency,
            max_bytes,
            ignore_robots,
            boundary,
            max_fetching_per_host,
        } => {
            let options = RecrawlSchedulerOptions {
                index,
                interval_secs,
                batch_size,
                poll_secs,
                max_rounds,
                crawl: RecrawlCrawlOptions {
                    max_depth,
                    concurrency,
                    max_bytes,
                    ignore_robots,
                    boundary: boundary.into(),
                    max_fetching_per_host,
                },
            };
            run_recrawl_scheduler_cli(options).await?;
        }
        Command::Serve {
            index,
            addr,
            preload,
        } => {
            run_search_server(index, addr, preload).await?;
        }
    }

    Ok(())
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum CliCrawlBoundary {
    SameHost,
    AnyDomain,
}

impl From<CliCrawlBoundary> for CrawlBoundary {
    fn from(value: CliCrawlBoundary) -> Self {
        match value {
            CliCrawlBoundary::SameHost => CrawlBoundary::SameHost,
            CliCrawlBoundary::AnyDomain => CrawlBoundary::AnyDomain,
        }
    }
}

fn build_options(ignore_noindex: bool, min_body_terms: u32) -> IndexBuildOptions {
    IndexBuildOptions {
        respect_noindex: !ignore_noindex,
        min_body_terms,
        ..IndexBuildOptions::default()
    }
}

async fn try_daemon_search(
    index: &std::path::Path,
    socket: Option<&std::path::Path>,
    query: &str,
    limit: usize,
) -> Result<Option<Vec<SearchResult>>> {
    let socket = socket
        .map(PathBuf::from)
        .unwrap_or_else(|| default_socket_path(index));
    if !socket.exists() {
        return Ok(None);
    }

    match send_request(
        &socket,
        &DaemonRequest::Search {
            query: query.to_owned(),
            limit,
        },
    )
    .await?
    {
        DaemonResponse::Search { results } => Ok(Some(results)),
        DaemonResponse::Error { message } => bail!(message),
        other => bail!("unexpected daemon response: {other:?}"),
    }
}

async fn try_daemon_spell(
    index: &std::path::Path,
    socket: Option<&std::path::Path>,
    term: &str,
    limit: usize,
) -> Result<Option<Vec<TermCorrection>>> {
    let socket = socket
        .map(PathBuf::from)
        .unwrap_or_else(|| default_socket_path(index));
    if !socket.exists() {
        return Ok(None);
    }

    match send_request(
        &socket,
        &DaemonRequest::Spell {
            term: term.to_owned(),
            limit,
        },
    )
    .await?
    {
        DaemonResponse::Spell { corrections } => Ok(Some(corrections)),
        DaemonResponse::Error { message } => bail!(message),
        other => bail!("unexpected daemon response: {other:?}"),
    }
}

async fn try_daemon_suggest(
    index: &std::path::Path,
    socket: Option<&std::path::Path>,
    prefix: &str,
    limit: usize,
) -> Result<Option<Vec<TermSuggestion>>> {
    let socket = socket
        .map(PathBuf::from)
        .unwrap_or_else(|| default_socket_path(index));
    if !socket.exists() {
        return Ok(None);
    }

    match send_request(
        &socket,
        &DaemonRequest::Suggest {
            prefix: prefix.to_owned(),
            limit,
        },
    )
    .await?
    {
        DaemonResponse::Suggest { suggestions } => Ok(Some(suggestions)),
        DaemonResponse::Error { message } => bail!(message),
        other => bail!("unexpected daemon response: {other:?}"),
    }
}

async fn try_daemon_render(
    index: &std::path::Path,
    socket: Option<&std::path::Path>,
    target: &str,
) -> Result<Option<String>> {
    let socket = socket
        .map(PathBuf::from)
        .unwrap_or_else(|| default_socket_path(index));
    if !socket.exists() {
        return Ok(None);
    }

    match send_request(
        &socket,
        &DaemonRequest::Render {
            target: target.to_owned(),
        },
    )
    .await?
    {
        DaemonResponse::Render { text } => Ok(Some(text)),
        DaemonResponse::Error { message } => bail!(message),
        other => bail!("unexpected daemon response: {other:?}"),
    }
}

async fn try_daemon_stats(
    index: &std::path::Path,
    socket: Option<&std::path::Path>,
) -> Result<bool> {
    let socket = socket
        .map(PathBuf::from)
        .unwrap_or_else(|| default_socket_path(index));
    if !socket.exists() {
        return Ok(false);
    }

    match send_request(&socket, &DaemonRequest::Stats).await? {
        DaemonResponse::Stats {
            doc_count,
            term_count,
            total_terms,
            avg_doc_len,
            duplicate_cluster_count,
            duplicate_doc_count,
            skipped_noindex_count,
            skipped_thin_count,
            max_authority_score,
            corpus_hash,
        } => {
            println!("docs: {doc_count}");
            println!("terms: {term_count}");
            println!("total_terms: {total_terms}");
            println!("avg_doc_len: {avg_doc_len:.2}");
            println!("duplicate_clusters: {duplicate_cluster_count}");
            println!("duplicate_docs: {duplicate_doc_count}");
            println!("skipped_noindex: {skipped_noindex_count}");
            println!("skipped_thin: {skipped_thin_count}");
            println!("max_authority_score: {max_authority_score:.4}");
            println!("corpus_hash: {corpus_hash}");
            print_index_storage_stats(index)?;
            Ok(true)
        }
        DaemonResponse::Error { message } => bail!(message),
        other => bail!("unexpected daemon response: {other:?}"),
    }
}

fn one_shot_search(
    index: &std::path::Path,
    query: &str,
    limit: usize,
) -> Result<Vec<SearchResult>> {
    let index = SearchIndex::open(index, PreloadMode::Lazy)?;
    index.search(query, SearchOptions { limit })
}

fn one_shot_suggest(
    index: &std::path::Path,
    prefix: &str,
    limit: usize,
) -> Result<Vec<TermSuggestion>> {
    let index = SearchIndex::open(index, PreloadMode::Lazy)?;
    Ok(index.suggest(prefix, limit))
}

fn one_shot_spell(
    index: &std::path::Path,
    term: &str,
    limit: usize,
) -> Result<Vec<TermCorrection>> {
    let index = SearchIndex::open(index, PreloadMode::Lazy)?;
    Ok(index.spellcheck(term, limit))
}

fn one_shot_render(index: &std::path::Path, target: &str) -> Result<String> {
    let index = SearchIndex::open(index, PreloadMode::Lazy)?;
    render_target(&index, target)
}

fn print_suggestions(suggestions: &[TermSuggestion]) {
    for suggestion in suggestions {
        println!(
            "{}\tdf={}\tcf={}",
            suggestion.term, suggestion.doc_freq, suggestion.collection_freq
        );
    }
}

fn print_corrections(corrections: &[TermCorrection]) {
    for correction in corrections {
        println!(
            "{}\tdistance={}\tdf={}\tcf={}",
            correction.term, correction.distance, correction.doc_freq, correction.collection_freq
        );
    }
}

fn print_results(results: &[SearchResult]) {
    for (rank, result) in results.iter().enumerate() {
        println!(
            "{}. [{}] {:.4} {}",
            rank + 1,
            result.doc_id,
            result.score,
            result.url
        );
        if result.authority_score > 0.0 {
            println!("   authority: {:.4}", result.authority_score);
        }
        if let Some(language) = &result.language {
            println!("   language: {language}");
        }
        if let Some(fetched_at_unix) = result.fetched_at_unix {
            println!("   fetched_at_unix: {fetched_at_unix}");
        }
        if !result.title.is_empty() {
            println!("   {}", result.title);
        }
        if !result.snippet.is_empty() {
            println!("   {}", result.snippet);
        }
        if result.duplicate_count > 1 {
            println!(
                "   duplicate cluster: {} docs (representative {})",
                result.duplicate_count, result.duplicate_of
            );
        }
    }
}

fn print_build_stats(stats: &brutal_search::BuildStats) {
    println!("docs: {}", stats.doc_count);
    println!("terms: {}", stats.term_count);
    println!("total_terms: {}", stats.total_terms);
    println!("avg_doc_len: {:.2}", stats.avg_doc_len);
    println!("duplicate_clusters: {}", stats.duplicate_cluster_count);
    println!("duplicate_docs: {}", stats.duplicate_doc_count);
    println!("skipped_noindex: {}", stats.skipped_noindex_count);
    println!("skipped_thin: {}", stats.skipped_thin_count);
    println!("max_authority_score: {:.4}", stats.max_authority_score);
    println!("corpus_hash: {}", stats.corpus_hash);
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct IndexStorageArtifact {
    name: &'static str,
    bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct IndexStorageStats {
    total_bytes: u64,
    artifacts: Vec<IndexStorageArtifact>,
    web_artifacts: Vec<WebStorageArtifactStats>,
    browser_document_bytes: u64,
    browser_document_rows: usize,
    browser_document_unique_rows: usize,
    browser_document_duplicate_rows: usize,
    browser_document_unique_row_bytes: u64,
    browser_document_duplicate_row_bytes: u64,
    crawl_frontier_bytes: u64,
    crawl_frontier_stats: Option<FrontierStats>,
    crawl_snapshot_bytes: u64,
    crawl_snapshot_entries: usize,
    crawl_snapshot_unique_entries: usize,
    crawl_snapshot_duplicate_entries: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct WebStorageArtifactStats {
    name: &'static str,
    bytes: u64,
    entries: usize,
    result_rows: usize,
    durable_result_rows: usize,
    incomplete_result_rows: usize,
    unique_entries: usize,
    duplicate_entries: usize,
    unique_row_bytes: u64,
    duplicate_row_bytes: u64,
    query_count: usize,
    query_examples: Vec<String>,
    provider_count: usize,
    provider_growth: Vec<String>,
    max_entries_per_query: usize,
    oldest_fetched_at_unix: Option<u64>,
    newest_fetched_at_unix: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct WebStoragePressureSummary {
    artifact_count: usize,
    bytes: u64,
    entries: usize,
    result_rows: usize,
    durable_result_rows: usize,
    incomplete_result_rows: usize,
    unique_entries: usize,
    duplicate_entries: usize,
    unique_row_bytes: u64,
    duplicate_row_bytes: u64,
    max_entries_per_query: usize,
    stale_artifacts: usize,
    suggested_dry_runs: usize,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct WebStorageProviderGrowth {
    entries: usize,
    bytes: u64,
    result_rows: usize,
}

fn collect_index_storage_stats(index: &Path) -> Result<IndexStorageStats> {
    let mut stats = IndexStorageStats {
        total_bytes: 0,
        artifacts: Vec::new(),
        web_artifacts: Vec::new(),
        browser_document_bytes: 0,
        browser_document_rows: 0,
        browser_document_unique_rows: 0,
        browser_document_duplicate_rows: 0,
        browser_document_unique_row_bytes: 0,
        browser_document_duplicate_row_bytes: 0,
        crawl_frontier_bytes: 0,
        crawl_frontier_stats: None,
        crawl_snapshot_bytes: 0,
        crawl_snapshot_entries: 0,
        crawl_snapshot_unique_entries: 0,
        crawl_snapshot_duplicate_entries: 0,
    };

    for name in INDEX_STORAGE_ARTIFACTS {
        let path = index.join(name);
        let metadata = match std::fs::metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("read index storage artifact {}", path.display()));
            }
        };
        if !metadata.is_file() {
            continue;
        }
        let bytes = metadata.len();
        stats.total_bytes = stats.total_bytes.saturating_add(bytes);
        stats.artifacts.push(IndexStorageArtifact { name, bytes });
        match *name {
            "frontier.bin" => {
                stats.crawl_frontier_bytes = bytes;
                stats.crawl_frontier_stats = Some(FrontierStore::open(&path)?.stats());
            }
            "crawl-docs.jsonl" => {
                stats.crawl_snapshot_bytes = bytes;
                let snapshot = crawl_snapshot_artifact_stats(&path)?;
                stats.crawl_snapshot_entries = snapshot.entries;
                stats.crawl_snapshot_unique_entries = snapshot.unique_entries;
                stats.crawl_snapshot_duplicate_entries = snapshot.duplicate_entries;
            }
            "browser-documents.jsonl" => {
                let documents = browser_document_artifact_stats(&path)?;
                stats.browser_document_bytes = bytes;
                stats.browser_document_rows = documents.rows;
                stats.browser_document_unique_rows = documents.unique_rows;
                stats.browser_document_duplicate_rows = documents.duplicate_rows;
                stats.browser_document_unique_row_bytes = documents.unique_row_bytes;
                stats.browser_document_duplicate_row_bytes = documents.duplicate_row_bytes;
            }
            _ => {}
        }
        if matches!(*name, "web-cache.jsonl" | "brave-results.jsonl") {
            stats
                .web_artifacts
                .push(collect_web_storage_artifact_stats(name, &path)?);
        }
    }

    Ok(stats)
}

fn print_index_storage_stats(index: &Path) -> Result<()> {
    let stats = collect_index_storage_stats(index)?;
    println!("index_storage_bytes: {}", stats.total_bytes);
    for artifact in &stats.artifacts {
        println!(
            "index_storage_artifact_bytes: {} {}",
            artifact.name, artifact.bytes
        );
    }
    for line in crawl_storage_pressure_summary_lines(&stats) {
        println!("{line}");
    }
    for line in browser_document_storage_pressure_summary_lines(&stats) {
        println!("{line}");
    }
    let retention_config = web_storage_retention_config();
    for line in web_storage_retention_config_lines(retention_config) {
        println!("{line}");
    }
    let now = unix_now();
    let stale_secs = web_storage_stale_threshold_secs();
    let web_summary = web_storage_pressure_summary(&stats.web_artifacts, now, stale_secs);
    for line in storage_pressure_rollup_lines(&stats, &web_summary) {
        println!("{line}");
    }
    for line in storage_budget_pressure_lines(&stats, &web_summary, retention_config) {
        println!("{line}");
    }
    for line in storage_cleanup_readiness_lines(&stats, &web_summary) {
        println!("{line}");
    }
    for line in storage_snapshot_readiness_lines(&stats, &web_summary) {
        println!("{line}");
    }
    for line in web_storage_pressure_summary_lines(&stats.web_artifacts, now, stale_secs) {
        println!("{line}");
    }
    for artifact in stats.web_artifacts {
        println!(
            "web_storage_artifact_entries: {} {}",
            artifact.name, artifact.entries
        );
        println!(
            "web_storage_artifact_result_rows: {} {}",
            artifact.name, artifact.result_rows
        );
        println!(
            "web_storage_artifact_durable_result_rows: {} {}",
            artifact.name, artifact.durable_result_rows
        );
        println!(
            "web_storage_artifact_incomplete_result_rows: {} {}",
            artifact.name, artifact.incomplete_result_rows
        );
        println!(
            "web_storage_artifact_unique_entries: {} {}",
            artifact.name, artifact.unique_entries
        );
        println!(
            "web_storage_artifact_duplicate_entries: {} {}",
            artifact.name, artifact.duplicate_entries
        );
        println!(
            "web_storage_artifact_queries: {} {}",
            artifact.name, artifact.query_count
        );
        println!(
            "web_storage_artifact_providers: {} {}",
            artifact.name, artifact.provider_count
        );
        println!(
            "web_storage_artifact_max_entries_per_query: {} {}",
            artifact.name, artifact.max_entries_per_query
        );
        if let Some(oldest) = artifact.oldest_fetched_at_unix {
            println!(
                "web_storage_artifact_oldest_fetched_at_unix: {} {}",
                artifact.name, oldest
            );
        }
        if let Some(newest) = artifact.newest_fetched_at_unix {
            println!(
                "web_storage_artifact_newest_fetched_at_unix: {} {}",
                artifact.name, newest
            );
        }
        if let Some(age_secs) = web_storage_oldest_age_secs(&artifact, now) {
            println!(
                "web_storage_artifact_oldest_age_secs: {} {}",
                artifact.name, age_secs
            );
        }
        if let Some(suggestion) = web_storage_compaction_suggestion(&artifact, now, stale_secs) {
            println!(
                "web_storage_compaction_suggestion: {} {}",
                artifact.name, suggestion
            );
        }
    }
    Ok(())
}

fn crawl_storage_pressure_summary_lines(stats: &IndexStorageStats) -> Vec<String> {
    let retained_bytes = stats
        .crawl_frontier_bytes
        .saturating_add(stats.crawl_snapshot_bytes);
    if retained_bytes == 0 && stats.crawl_snapshot_entries == 0 {
        return Vec::new();
    }

    let mut lines = vec![
        format!(
            "crawl_storage_pressure_summary: retained_bytes={} frontier_bytes={} snapshot_bytes={} snapshot_entries={}",
            retained_bytes,
            stats.crawl_frontier_bytes,
            stats.crawl_snapshot_bytes,
            stats.crawl_snapshot_entries
        ),
        format!("crawl_storage_retained_bytes: {retained_bytes}"),
        format!(
            "crawl_storage_frontier_bytes: {}",
            stats.crawl_frontier_bytes
        ),
        format!(
            "crawl_storage_snapshot_bytes: {}",
            stats.crawl_snapshot_bytes
        ),
        format!(
            "crawl_storage_snapshot_entries: {}",
            stats.crawl_snapshot_entries
        ),
        format!(
            "crawl_storage_snapshot_unique_entries: {}",
            stats.crawl_snapshot_unique_entries
        ),
        format!(
            "crawl_storage_snapshot_duplicate_entries: {}",
            stats.crawl_snapshot_duplicate_entries
        ),
        format!(
            "crawl_storage_snapshot_projected_entries_after: {}",
            stats.crawl_snapshot_unique_entries
        ),
        format!(
            "crawl_storage_snapshot_projected_entries_removed: {}",
            stats.crawl_snapshot_duplicate_entries
        ),
    ];
    if let Some(frontier) = &stats.crawl_frontier_stats {
        lines.push(format!(
            "crawl_storage_frontier_records: {}",
            frontier.total
        ));
        lines.push(format!(
            "crawl_storage_frontier_queued: {}",
            frontier.queued
        ));
        lines.push(format!(
            "crawl_storage_frontier_fetching: {}",
            frontier.fetching
        ));
        lines.push(format!(
            "crawl_storage_frontier_fetched: {}",
            frontier.fetched
        ));
        lines.push(format!(
            "crawl_storage_frontier_failed: {}",
            frontier.failed
        ));
        lines.push(format!(
            "crawl_storage_frontier_deferred: {}",
            frontier.deferred
        ));
        lines.push(format!(
            "crawl_storage_frontier_failed_record_cap: {}",
            DEFAULT_MAX_FAILED_FRONTIER_RECORDS
        ));
        let projected_failed_after = frontier.failed.min(DEFAULT_MAX_FAILED_FRONTIER_RECORDS);
        let projected_failed_removed = frontier.failed.saturating_sub(projected_failed_after);
        lines.push(format!(
            "crawl_storage_frontier_failed_projected_after: {}",
            projected_failed_after
        ));
        lines.push(format!(
            "crawl_storage_frontier_failed_projected_removed: {}",
            projected_failed_removed
        ));
        lines.push(format!(
            "crawl_storage_frontier_failed_zero_removal: {}",
            projected_failed_removed == 0
        ));
        if projected_failed_removed > 0 {
            lines.push(
                "crawl_storage_frontier_dry_run_note: failed frontier records exceed retention cap and are removable by frontier compaction without deleting fetched documents"
                    .to_owned(),
            );
            lines.push(
                "crawl_storage_frontier_apply_guard: report-only; stats does not mutate frontier.bin, run a dedicated frontier compaction apply path only after projected_removed is nonzero"
                    .to_owned(),
            );
        } else {
            lines.push(
                "crawl_storage_frontier_apply_guard: zero-removal; frontier compaction apply would be pointless because all failed records are retained"
                    .to_owned(),
            );
        }
    }
    if stats.crawl_snapshot_duplicate_entries > 0 {
        lines.push(
            "crawl_storage_snapshot_dry_run_note: duplicate crawl-docs rows are retained until crawl snapshot compaction rewrites latest unique docs".to_owned(),
        );
    }
    lines
}

fn browser_document_storage_pressure_summary_lines(stats: &IndexStorageStats) -> Vec<String> {
    if stats.browser_document_bytes == 0 && stats.browser_document_rows == 0 {
        return Vec::new();
    }

    let zero_removal = stats.browser_document_duplicate_rows == 0
        && stats.browser_document_duplicate_row_bytes == 0;
    let mut lines = vec![
        format!(
            "browser_document_storage_summary: bytes={} rows={} unique_rows={} duplicate_rows={} projected_rows_after={} projected_rows_removed={} projected_row_bytes_after={} projected_row_bytes_removed={} zero_removal={}",
            stats.browser_document_bytes,
            stats.browser_document_rows,
            stats.browser_document_unique_rows,
            stats.browser_document_duplicate_rows,
            stats.browser_document_unique_rows,
            stats.browser_document_duplicate_rows,
            stats.browser_document_unique_row_bytes,
            stats.browser_document_duplicate_row_bytes,
            zero_removal
        ),
        format!(
            "browser_document_storage_bytes: {}",
            stats.browser_document_bytes
        ),
        format!(
            "browser_document_storage_rows: {}",
            stats.browser_document_rows
        ),
        format!(
            "browser_document_storage_unique_rows: {}",
            stats.browser_document_unique_rows
        ),
        format!(
            "browser_document_storage_duplicate_rows: {}",
            stats.browser_document_duplicate_rows
        ),
        format!(
            "browser_document_storage_projected_rows_after: {}",
            stats.browser_document_unique_rows
        ),
        format!(
            "browser_document_storage_projected_rows_removed: {}",
            stats.browser_document_duplicate_rows
        ),
        format!(
            "browser_document_storage_projected_row_bytes_after: {}",
            stats.browser_document_unique_row_bytes
        ),
        format!(
            "browser_document_storage_projected_row_bytes_removed: {}",
            stats.browser_document_duplicate_row_bytes
        ),
        format!("browser_document_storage_zero_removal: {zero_removal}"),
    ];
    if zero_removal {
        lines.push(
            "browser_document_storage_dry_run_note: all browser document rows are retained; cleanup would remove nothing"
                .to_owned(),
        );
    } else {
        lines.push(
            "browser_document_storage_dry_run_note: duplicate browser document rows are removable by a future browser-document compaction without rewriting live index data"
                .to_owned(),
        );
    }
    lines
}

fn storage_pressure_rollup_lines(
    stats: &IndexStorageStats,
    web_summary: &WebStoragePressureSummary,
) -> Vec<String> {
    let crawl_bytes = stats
        .crawl_frontier_bytes
        .saturating_add(stats.crawl_snapshot_bytes);
    let core_bytes = storage_core_index_bytes(stats, web_summary);
    let frontier_records = stats
        .crawl_frontier_stats
        .as_ref()
        .map(|frontier| frontier.total)
        .unwrap_or(0);
    vec![
        format!(
            "storage_pressure_summary: total_bytes={} core_index_bytes={} web_bytes={} browser_document_bytes={} crawl_bytes={} web_entries={} web_duplicates={} snapshot_entries={} frontier_records={}",
            stats.total_bytes,
            core_bytes,
            web_summary.bytes,
            stats.browser_document_bytes,
            crawl_bytes,
            web_summary.entries,
            web_summary.duplicate_entries,
            stats.crawl_snapshot_entries,
            frontier_records
        ),
        format!("storage_pressure_total_bytes: {}", stats.total_bytes),
        format!("storage_pressure_core_index_bytes: {core_bytes}"),
        format!("storage_pressure_web_bytes: {}", web_summary.bytes),
        format!(
            "storage_pressure_browser_document_bytes: {}",
            stats.browser_document_bytes
        ),
        format!("storage_pressure_crawl_bytes: {crawl_bytes}"),
    ]
}

fn storage_budget_pressure_lines(
    stats: &IndexStorageStats,
    web_summary: &WebStoragePressureSummary,
    retention_config: WebStorageRetentionConfig,
) -> Vec<String> {
    let crawl_bytes = stats
        .crawl_frontier_bytes
        .saturating_add(stats.crawl_snapshot_bytes);
    let core_bytes = storage_core_index_bytes(stats, web_summary);
    let budget_bytes = index_storage_budget_bytes();
    let remaining_bytes = budget_bytes.saturating_sub(stats.total_bytes);
    let status = if stats.total_bytes > budget_bytes {
        "over-budget"
    } else {
        "within-budget"
    };
    let web_budget_bytes = retention_config
        .cache_max_bytes
        .saturating_add(retention_config.result_log_max_bytes);

    vec![
        format!(
            "storage_budget_summary: status={} total_bytes={} budget_bytes={} remaining_bytes={} core_index_bytes={} web_bytes={} web_budget_bytes={} browser_document_bytes={} crawl_bytes={}",
            status,
            stats.total_bytes,
            budget_bytes,
            remaining_bytes,
            core_bytes,
            web_summary.bytes,
            web_budget_bytes,
            stats.browser_document_bytes,
            crawl_bytes
        ),
        format!("storage_budget_status: {status}"),
        format!("storage_budget_total_bytes: {}", stats.total_bytes),
        format!("storage_budget_bytes: {budget_bytes}"),
        format!("storage_budget_remaining_bytes: {remaining_bytes}"),
        format!("storage_budget_core_index_bytes: {core_bytes}"),
        format!("storage_budget_web_bytes: {}", web_summary.bytes),
        format!("storage_budget_web_budget_bytes: {web_budget_bytes}"),
        format!(
            "storage_budget_browser_document_bytes: {}",
            stats.browser_document_bytes
        ),
        format!("storage_budget_crawl_bytes: {crawl_bytes}"),
        "storage_budget_report_mode: report-only".to_owned(),
        "storage_budget_apply_guard: report-only; stats does not mutate .brutal-index, run dry-run compaction commands only when removable bytes are nonzero".to_owned(),
    ]
}

fn storage_cleanup_readiness_lines(
    stats: &IndexStorageStats,
    web_summary: &WebStoragePressureSummary,
) -> Vec<String> {
    let frontier_failed_removable_records = stats
        .crawl_frontier_stats
        .as_ref()
        .map(frontier_failed_projected_removed)
        .unwrap_or(0);
    let known_removable_row_bytes = web_summary
        .duplicate_row_bytes
        .saturating_add(stats.browser_document_duplicate_row_bytes);
    let removable_rows = web_summary
        .duplicate_entries
        .saturating_add(stats.browser_document_duplicate_rows)
        .saturating_add(stats.crawl_snapshot_duplicate_entries)
        .saturating_add(frontier_failed_removable_records);
    let retained_bytes = stats.total_bytes.saturating_sub(known_removable_row_bytes);
    let safe_to_clean = removable_rows > 0 || known_removable_row_bytes > 0;
    let status = if safe_to_clean {
        "cleanup-available"
    } else {
        "zero-removal"
    };

    let mut lines = vec![
        format!(
            "storage_cleanup_readiness: status={} report_mode=report-only retained_bytes={} known_removable_row_bytes={} removable_rows={} web_removable_rows={} browser_document_removable_rows={} snapshot_removable_rows={} frontier_failed_removable_records={}",
            status,
            retained_bytes,
            known_removable_row_bytes,
            removable_rows,
            web_summary.duplicate_entries,
            stats.browser_document_duplicate_rows,
            stats.crawl_snapshot_duplicate_entries,
            frontier_failed_removable_records
        ),
        format!("storage_cleanup_status: {status}"),
        "storage_cleanup_report_mode: report-only".to_owned(),
        format!("storage_cleanup_safe_to_clean: {safe_to_clean}"),
        format!("storage_cleanup_pointless: {}", !safe_to_clean),
        format!("storage_cleanup_retained_bytes: {retained_bytes}"),
        format!(
            "storage_cleanup_known_removable_row_bytes: {}",
            known_removable_row_bytes
        ),
        format!("storage_cleanup_removable_rows: {removable_rows}"),
        format!(
            "storage_cleanup_web_removable_rows: {}",
            web_summary.duplicate_entries
        ),
        format!(
            "storage_cleanup_browser_document_removable_rows: {}",
            stats.browser_document_duplicate_rows
        ),
        format!(
            "storage_cleanup_snapshot_removable_rows: {}",
            stats.crawl_snapshot_duplicate_entries
        ),
        format!(
            "storage_cleanup_frontier_failed_removable_records: {}",
            frontier_failed_removable_records
        ),
        "storage_cleanup_apply_guard: report-only; stats does not mutate .brutal-index, run dry-run/apply cleanup only when storage_cleanup_safe_to_clean is true".to_owned(),
    ];
    if safe_to_clean {
        lines.push(
            "storage_cleanup_note: cleanup candidates exist; review component dry-run output before applying any rewrite"
                .to_owned(),
        );
    } else {
        lines.push(
            "storage_cleanup_note: cleanup would be pointless because all tracked storage rows are retained"
                .to_owned(),
        );
    }
    lines
}

fn storage_core_index_bytes(
    stats: &IndexStorageStats,
    web_summary: &WebStoragePressureSummary,
) -> u64 {
    let crawl_bytes = stats
        .crawl_frontier_bytes
        .saturating_add(stats.crawl_snapshot_bytes);
    stats
        .total_bytes
        .saturating_sub(crawl_bytes)
        .saturating_sub(stats.browser_document_bytes)
        .saturating_sub(web_summary.bytes)
}

fn frontier_failed_projected_removed(frontier: &FrontierStats) -> usize {
    frontier
        .failed
        .saturating_sub(frontier.failed.min(DEFAULT_MAX_FAILED_FRONTIER_RECORDS))
}

fn index_storage_budget_bytes() -> u64 {
    web_storage_env_u64("BRUTAL_INDEX_STORAGE_BUDGET_BYTES")
        .unwrap_or(DEFAULT_INDEX_STORAGE_BUDGET_BYTES)
}

fn storage_snapshot_readiness_lines(
    stats: &IndexStorageStats,
    web_summary: &WebStoragePressureSummary,
) -> Vec<String> {
    let crawl_bytes = stats
        .crawl_frontier_bytes
        .saturating_add(stats.crawl_snapshot_bytes);
    let frontier_records = stats
        .crawl_frontier_stats
        .as_ref()
        .map(|frontier| frontier.total)
        .unwrap_or(0);
    let status = if web_summary.suggested_dry_runs > 0 {
        "needs-web-compaction"
    } else {
        "ready"
    };
    let mut lines = vec![
        format!(
            "storage_snapshot_readiness: status={} total_bytes={} web_bytes={} browser_document_bytes={} browser_document_rows={} browser_document_duplicates={} crawl_bytes={} web_entries={} web_result_rows={} web_unique_entries={} web_duplicates={} web_duplicate_row_bytes={} web_suggested_dry_runs={} snapshot_entries={} frontier_records={}",
            status,
            stats.total_bytes,
            web_summary.bytes,
            stats.browser_document_bytes,
            stats.browser_document_rows,
            stats.browser_document_duplicate_rows,
            crawl_bytes,
            web_summary.entries,
            web_summary.result_rows,
            web_summary.unique_entries,
            web_summary.duplicate_entries,
            web_summary.duplicate_row_bytes,
            web_summary.suggested_dry_runs,
            stats.crawl_snapshot_entries,
            frontier_records
        ),
        format!("storage_snapshot_status: {status}"),
        format!(
            "storage_snapshot_browser_document_rows: {}",
            stats.browser_document_rows
        ),
        format!(
            "storage_snapshot_browser_document_duplicates: {}",
            stats.browser_document_duplicate_rows
        ),
        format!(
            "storage_snapshot_web_suggested_dry_runs: {}",
            web_summary.suggested_dry_runs
        ),
        format!("storage_snapshot_frontier_records: {}", frontier_records),
    ];
    if web_summary.suggested_dry_runs > 0 {
        lines.push(format!(
            "storage_snapshot_cleanup_hint: brutal-search compact-web-cache --dry-run --min-entries {}",
            web_summary.entries.max(1)
        ));
    }
    lines
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct CrawlSnapshotArtifactStats {
    entries: usize,
    unique_entries: usize,
    duplicate_entries: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct BrowserDocumentArtifactStats {
    rows: usize,
    unique_rows: usize,
    duplicate_rows: usize,
    unique_row_bytes: u64,
    duplicate_row_bytes: u64,
}

fn browser_document_artifact_stats(path: &Path) -> Result<BrowserDocumentArtifactStats> {
    let file = std::fs::File::open(path)
        .with_context(|| format!("open browser document jsonl artifact {}", path.display()))?;
    let reader = std::io::BufReader::new(file);
    let mut rows = 0usize;
    let mut unique_rows = 0usize;
    let mut duplicate_rows = 0usize;
    let mut unique_row_bytes = 0u64;
    let mut duplicate_row_bytes = 0u64;
    let mut keys = HashSet::new();

    for line in reader.lines() {
        let line = line
            .with_context(|| format!("read browser document jsonl artifact {}", path.display()))?;
        if line.trim().is_empty() {
            continue;
        }
        rows = rows.saturating_add(1);
        let row_bytes = u64::try_from(line.len()).unwrap_or(u64::MAX);
        let value: serde_json::Value = serde_json::from_str(&line).with_context(|| {
            format!("decode browser document jsonl artifact {}", path.display())
        })?;
        if let Some(key) = browser_document_storage_key(&value) {
            if keys.insert(key) {
                unique_rows = unique_rows.saturating_add(1);
                unique_row_bytes = unique_row_bytes.saturating_add(row_bytes);
            } else {
                duplicate_rows = duplicate_rows.saturating_add(1);
                duplicate_row_bytes = duplicate_row_bytes.saturating_add(row_bytes);
            }
        } else {
            unique_rows = unique_rows.saturating_add(1);
            unique_row_bytes = unique_row_bytes.saturating_add(row_bytes);
        }
    }

    Ok(BrowserDocumentArtifactStats {
        rows,
        unique_rows,
        duplicate_rows,
        unique_row_bytes,
        duplicate_row_bytes,
    })
}

fn browser_document_storage_key(value: &serde_json::Value) -> Option<String> {
    let url = first_json_string(
        value,
        &["url", "document_url", "source", "target", "final_url"],
    )?;
    let session = first_json_string(value, &["session_id", "session", "tab_id", "tab"]);
    Some(match session {
        Some(session) => format!("{session}\0{url}"),
        None => url.to_owned(),
    })
}

fn first_json_string<'a>(value: &'a serde_json::Value, keys: &[&str]) -> Option<&'a str> {
    keys.iter()
        .find_map(|key| value.get(*key).and_then(|value| value.as_str()))
        .filter(|value| !value.is_empty())
}

fn crawl_snapshot_artifact_stats(path: &Path) -> Result<CrawlSnapshotArtifactStats> {
    let file = std::fs::File::open(path)
        .with_context(|| format!("open index storage jsonl artifact {}", path.display()))?;
    let reader = std::io::BufReader::new(file);
    let mut entries = 0usize;
    let mut urls = HashSet::new();
    for line in reader.lines() {
        let line =
            line.with_context(|| format!("read index storage jsonl artifact {}", path.display()))?;
        if !line.trim().is_empty() {
            entries = entries.saturating_add(1);
            let value: serde_json::Value = serde_json::from_str(&line).with_context(|| {
                format!("decode crawl snapshot jsonl artifact {}", path.display())
            })?;
            if let Some(url) = value.get("url").and_then(|url| url.as_str()) {
                urls.insert(url.to_owned());
            }
        }
    }
    let unique_entries = urls.len();
    Ok(CrawlSnapshotArtifactStats {
        entries,
        unique_entries,
        duplicate_entries: entries.saturating_sub(unique_entries),
    })
}

fn web_storage_pressure_summary_lines(
    artifacts: &[WebStorageArtifactStats],
    now: u64,
    stale_secs: u64,
) -> Vec<String> {
    let summary = web_storage_pressure_summary(artifacts, now, stale_secs);
    let mut lines = vec![
        format!(
            "web_storage_pressure_summary: artifacts={} bytes={} entries={} result_rows={} durable_result_rows={} incomplete_result_rows={} unique_entries={} duplicates={} duplicate_row_bytes={} stale_artifacts={} suggested_dry_runs={}",
            summary.artifact_count,
            summary.bytes,
            summary.entries,
            summary.result_rows,
            summary.durable_result_rows,
            summary.incomplete_result_rows,
            summary.unique_entries,
            summary.duplicate_entries,
            summary.duplicate_row_bytes,
            summary.stale_artifacts,
            summary.suggested_dry_runs
        ),
        format!("web_storage_pressure_artifacts: {}", summary.artifact_count),
        format!("web_storage_pressure_bytes: {}", summary.bytes),
        format!("web_storage_pressure_entries: {}", summary.entries),
        format!("web_storage_pressure_result_rows: {}", summary.result_rows),
        format!(
            "web_storage_pressure_durable_result_rows: {}",
            summary.durable_result_rows
        ),
        format!(
            "web_storage_pressure_incomplete_result_rows: {}",
            summary.incomplete_result_rows
        ),
        format!(
            "web_storage_pressure_unique_entries: {}",
            summary.unique_entries
        ),
        format!(
            "web_storage_pressure_projected_entries_after: {}",
            summary.unique_entries
        ),
        format!(
            "web_storage_pressure_projected_entries_removed: {}",
            summary.duplicate_entries
        ),
        format!(
            "web_storage_pressure_projected_row_bytes_after: {}",
            summary.unique_row_bytes
        ),
        format!(
            "web_storage_pressure_projected_row_bytes_removed: {}",
            summary.duplicate_row_bytes
        ),
        format!(
            "web_storage_pressure_retained_result_rows: {}",
            summary.result_rows
        ),
        format!(
            "web_storage_pressure_removable_row_bytes: {}",
            summary.duplicate_row_bytes
        ),
        format!(
            "web_storage_pressure_zero_removal: {}",
            summary.duplicate_entries == 0 && summary.duplicate_row_bytes == 0
        ),
        format!(
            "web_storage_pressure_duplicate_entries: {}",
            summary.duplicate_entries
        ),
        format!(
            "web_storage_pressure_duplicate_row_bytes: {}",
            summary.duplicate_row_bytes
        ),
        format!(
            "web_storage_pressure_max_entries_per_query: {}",
            summary.max_entries_per_query
        ),
        format!(
            "web_storage_pressure_stale_artifacts: {}",
            summary.stale_artifacts
        ),
        format!(
            "web_storage_pressure_suggested_dry_runs: {}",
            summary.suggested_dry_runs
        ),
    ];
    if summary.suggested_dry_runs > 0 {
        lines.push(format!(
            "web_storage_pressure_suggestion: brutal-search compact-web-cache --dry-run --min-entries {}",
            summary.entries.max(1)
        ));
    }
    lines.extend(web_storage_export_readiness_lines(
        artifacts, &summary, now, stale_secs,
    ));
    lines
}

fn web_storage_export_readiness_lines(
    artifacts: &[WebStorageArtifactStats],
    summary: &WebStoragePressureSummary,
    now: u64,
    stale_secs: u64,
) -> Vec<String> {
    let cache_query_buckets = artifacts
        .iter()
        .find(|artifact| artifact.name == "web-cache.jsonl")
        .map(|artifact| artifact.query_count)
        .unwrap_or(0);
    let cache_replayable_result_rows = artifacts
        .iter()
        .find(|artifact| artifact.name == "web-cache.jsonl")
        .map(|artifact| artifact.durable_result_rows)
        .unwrap_or(0);
    let result_log_unique_urls = artifacts
        .iter()
        .find(|artifact| artifact.name == "brave-results.jsonl")
        .map(|artifact| artifact.unique_entries)
        .unwrap_or(0);
    let result_log_query_buckets = artifacts
        .iter()
        .find(|artifact| artifact.name == "brave-results.jsonl")
        .map(|artifact| artifact.query_count)
        .unwrap_or(0);
    let replay_missing_query_examples = web_storage_replay_missing_query_examples(artifacts);
    let provider_buckets = artifacts
        .iter()
        .map(|artifact| artifact.provider_count)
        .max()
        .unwrap_or(0);
    let replay_missing_query_buckets = result_log_query_buckets.saturating_sub(cache_query_buckets);
    let status = if summary.result_rows == 0 {
        "empty"
    } else if summary.incomplete_result_rows == 0 {
        "ready"
    } else {
        "partial"
    };
    let replay_status = if cache_replayable_result_rows > 0 {
        "ready"
    } else if result_log_unique_urls > 0 {
        "miss-risk"
    } else {
        "empty"
    };
    let newest_age_secs = artifacts
        .iter()
        .filter_map(|artifact| artifact.newest_fetched_at_unix)
        .max()
        .map(|newest| now.saturating_sub(newest));
    let oldest_age_secs = artifacts
        .iter()
        .filter_map(|artifact| artifact.oldest_fetched_at_unix)
        .min()
        .map(|oldest| now.saturating_sub(oldest));
    let staleness_status = match newest_age_secs {
        None => "unknown",
        Some(age_secs) if stale_secs > 0 && age_secs > stale_secs => "stale",
        Some(_) => "fresh",
    };
    let compaction_reason = if summary.duplicate_row_bytes > 0 {
        "duplicate-bytes"
    } else if replay_missing_query_buckets > 0 {
        "replay-misses"
    } else if provider_buckets > 1 {
        "multiple-providers"
    } else if staleness_status == "stale" {
        "stale-cache"
    } else {
        "zero-removal"
    };
    vec![
        format!(
            "web_storage_export_readiness: status={status} report_only=true cache_query_buckets={cache_query_buckets} unique_result_urls={result_log_unique_urls} durable_result_rows={} incomplete_result_rows={} duplicate_rows={}",
            summary.durable_result_rows,
            summary.incomplete_result_rows,
            summary.duplicate_entries
        ),
        format!(
            "web_storage_replay_readiness: status={replay_status} report_only=true cache_query_buckets={cache_query_buckets} replayable_result_rows={cache_replayable_result_rows} result_log_unique_urls={result_log_unique_urls}"
        ),
        format!(
            "web_storage_export_manifest: report_only=true export_status={status} replay_status={replay_status} staleness_status={staleness_status} newest_age_secs={} stale_after_secs={stale_secs} retained_bytes={} removable_bytes={} retained_rows={} removable_rows={} cache_query_buckets={cache_query_buckets} unique_result_urls={result_log_unique_urls}",
            newest_age_secs
                .map(|age_secs| age_secs.to_string())
                .unwrap_or_else(|| "unknown".to_owned()),
            summary.unique_row_bytes,
            summary.duplicate_row_bytes,
            summary.unique_entries,
            summary.duplicate_entries
        ),
        format!(
            "web_storage_replay_query_coverage: report_only=true cache_query_buckets={cache_query_buckets} result_log_query_buckets={result_log_query_buckets} missing_query_buckets={replay_missing_query_buckets}"
        ),
        format!(
            "web_storage_replay_missing_query_examples: report_only=true limit={WEB_STORAGE_QUERY_EXAMPLE_LIMIT} examples={}",
            web_storage_format_query_examples(&replay_missing_query_examples)
        ),
        format!(
            "web_storage_provider_growth: report_only=true limit={WEB_STORAGE_PROVIDER_GROWTH_LIMIT} {}",
            web_storage_provider_growth_summary(artifacts)
        ),
        format!(
            "web_storage_replay_staleness: status={staleness_status} report_only=true newest_age_secs={} oldest_age_secs={} stale_after_secs={stale_secs}",
            newest_age_secs
                .map(|age_secs| age_secs.to_string())
                .unwrap_or_else(|| "unknown".to_owned()),
            oldest_age_secs
                .map(|age_secs| age_secs.to_string())
                .unwrap_or_else(|| "unknown".to_owned())
        ),
        format!(
            "web_storage_compaction_decision: report_only=true reason={compaction_reason} duplicate_row_bytes={} missing_query_buckets={replay_missing_query_buckets} provider_buckets={provider_buckets} staleness_status={staleness_status}",
            summary.duplicate_row_bytes
        ),
        format!("web_storage_provider_buckets: {provider_buckets}"),
        format!("web_storage_export_cache_query_buckets: {cache_query_buckets}"),
        format!("web_storage_replay_missing_query_buckets: {replay_missing_query_buckets}"),
        format!("web_storage_replayable_result_rows: {cache_replayable_result_rows}"),
        format!("web_storage_export_unique_result_urls: {result_log_unique_urls}"),
        format!(
            "web_storage_export_durable_result_rows: {}",
            summary.durable_result_rows
        ),
        format!(
            "web_storage_export_incomplete_result_rows: {}",
            summary.incomplete_result_rows
        ),
        format!(
            "web_storage_export_duplicate_rows: {}",
            summary.duplicate_entries
        ),
        "web_storage_export_note: report-only; does not rewrite .brutal-index or cached web artifacts".to_owned(),
    ]
}

fn web_storage_replay_missing_query_examples(artifacts: &[WebStorageArtifactStats]) -> Vec<String> {
    let cache_queries = artifacts
        .iter()
        .find(|artifact| artifact.name == "web-cache.jsonl")
        .map(|artifact| artifact.query_examples.as_slice())
        .unwrap_or(&[]);
    let result_log_queries = artifacts
        .iter()
        .find(|artifact| artifact.name == "brave-results.jsonl")
        .map(|artifact| artifact.query_examples.as_slice())
        .unwrap_or(&[]);
    let cache_queries = cache_queries
        .iter()
        .map(String::as_str)
        .collect::<HashSet<_>>();
    let mut examples = result_log_queries
        .iter()
        .filter(|query| !cache_queries.contains(query.as_str()))
        .take(WEB_STORAGE_QUERY_EXAMPLE_LIMIT)
        .cloned()
        .collect::<Vec<_>>();
    examples.sort();
    examples
}

fn web_storage_format_query_examples(examples: &[String]) -> String {
    if examples.is_empty() {
        return "none".to_owned();
    }
    examples
        .iter()
        .map(|query| web_storage_sanitize_token(query))
        .collect::<Vec<_>>()
        .join(",")
}

fn web_storage_provider_growth_summary(artifacts: &[WebStorageArtifactStats]) -> String {
    let summaries = artifacts
        .iter()
        .filter(|artifact| !artifact.provider_growth.is_empty())
        .map(|artifact| format!("{}={}", artifact.name, artifact.provider_growth.join("|")))
        .collect::<Vec<_>>();
    if summaries.is_empty() {
        "providers=none".to_owned()
    } else {
        summaries.join(" ")
    }
}

fn web_storage_sanitize_token(value: &str) -> String {
    value.replace(|ch: char| ch.is_whitespace() || ch == ',' || ch == '|', "_")
}

fn web_storage_pressure_summary(
    artifacts: &[WebStorageArtifactStats],
    now: u64,
    stale_secs: u64,
) -> WebStoragePressureSummary {
    WebStoragePressureSummary {
        artifact_count: artifacts.len(),
        bytes: artifacts.iter().fold(0_u64, |total, artifact| {
            total.saturating_add(artifact.bytes)
        }),
        entries: artifacts.iter().map(|artifact| artifact.entries).sum(),
        result_rows: artifacts.iter().map(|artifact| artifact.result_rows).sum(),
        durable_result_rows: artifacts
            .iter()
            .map(|artifact| artifact.durable_result_rows)
            .sum(),
        incomplete_result_rows: artifacts
            .iter()
            .map(|artifact| artifact.incomplete_result_rows)
            .sum(),
        unique_entries: artifacts
            .iter()
            .map(|artifact| artifact.unique_entries)
            .sum(),
        duplicate_entries: artifacts
            .iter()
            .map(|artifact| artifact.duplicate_entries)
            .sum(),
        unique_row_bytes: artifacts.iter().fold(0_u64, |total, artifact| {
            total.saturating_add(artifact.unique_row_bytes)
        }),
        duplicate_row_bytes: artifacts.iter().fold(0_u64, |total, artifact| {
            total.saturating_add(artifact.duplicate_row_bytes)
        }),
        max_entries_per_query: artifacts
            .iter()
            .map(|artifact| artifact.max_entries_per_query)
            .max()
            .unwrap_or(0),
        stale_artifacts: artifacts
            .iter()
            .filter(|artifact| {
                stale_secs > 0
                    && web_storage_oldest_age_secs(artifact, now)
                        .is_some_and(|age_secs| age_secs > stale_secs)
            })
            .count(),
        suggested_dry_runs: artifacts
            .iter()
            .filter(|artifact| {
                web_storage_compaction_suggestion(artifact, now, stale_secs).is_some()
            })
            .count(),
    }
}

fn web_storage_compaction_suggestion(
    artifact: &WebStorageArtifactStats,
    now: u64,
    stale_secs: u64,
) -> Option<String> {
    if artifact.duplicate_entries >= WEB_STORAGE_COMPACT_SUGGEST_DUPLICATES {
        return Some(format!(
            "brutal-search compact-web-cache --dry-run --min-entries {}",
            artifact.entries.max(artifact.duplicate_entries)
        ));
    }
    if artifact.entries >= WEB_STORAGE_COMPACT_SUGGEST_MIN_ENTRIES {
        return Some(format!(
            "brutal-search compact-web-cache --dry-run --min-entries {}",
            WEB_STORAGE_COMPACT_SUGGEST_MIN_ENTRIES
        ));
    }
    if stale_secs > 0
        && web_storage_oldest_age_secs(artifact, now).is_some_and(|age_secs| age_secs > stale_secs)
    {
        return Some("brutal-search compact-web-cache --dry-run --min-entries 1".to_owned());
    }
    None
}

fn web_storage_oldest_age_secs(artifact: &WebStorageArtifactStats, now: u64) -> Option<u64> {
    artifact
        .oldest_fetched_at_unix
        .map(|oldest| now.saturating_sub(oldest))
}

fn web_storage_stale_threshold_secs() -> u64 {
    env::var("BRUTAL_WEB_CACHE_TTL_SECS")
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
        .unwrap_or(DEFAULT_WEB_STORAGE_STALE_SECS)
}

fn print_web_storage_compaction_report(report: &WebSearchStorageCompactionReport) {
    println!("dry_run: {}", report.dry_run);
    println!("skipped: {}", report.skipped);
    if report.dry_run && !report.skipped {
        println!("dry_run_note: no files were rewritten; projected_after shows retained rows");
    }
    for line in web_storage_compaction_snapshot_readiness_lines(report) {
        println!("{line}");
    }
    for line in web_storage_result_log_query_cap_lines(web_storage_result_log_query_cap()) {
        println!("{line}");
    }
    print_web_storage_compaction_artifact(
        "web-cache",
        &report.cache_path,
        report.cache_before,
        report.cache_after,
        report.cache_projected_after,
    );
    print_web_storage_compaction_artifact(
        "brave-results",
        &report.result_log_path,
        report.result_log_before,
        report.result_log_after,
        report.result_log_projected_after,
    );
}

fn print_web_storage_compaction_artifact(
    label: &str,
    path: &Path,
    before: WebSearchStorageArtifactState,
    after: WebSearchStorageArtifactState,
    projected_after: WebSearchStorageArtifactState,
) {
    for line in web_storage_compaction_artifact_lines(label, path, before, after, projected_after) {
        println!("{line}");
    }
}

fn web_storage_compaction_snapshot_readiness_lines(
    report: &WebSearchStorageCompactionReport,
) -> Vec<String> {
    let before_bytes = report
        .cache_before
        .bytes
        .saturating_add(report.result_log_before.bytes);
    let projected_bytes = report
        .cache_projected_after
        .bytes
        .saturating_add(report.result_log_projected_after.bytes);
    let projected_duplicates = report
        .cache_projected_after
        .duplicate_entries
        .saturating_add(report.result_log_projected_after.duplicate_entries);
    let projected_removed_bytes = before_bytes.saturating_sub(projected_bytes);
    let projected_removed_duplicates = web_storage_compaction_projected_removed_duplicates(report);
    let zero_removal = web_storage_compaction_zero_removal(report);
    let cache_removed_bytes = report
        .cache_before
        .bytes
        .saturating_sub(report.cache_projected_after.bytes);
    let result_log_removed_bytes = report
        .result_log_before
        .bytes
        .saturating_sub(report.result_log_projected_after.bytes);
    let cache_removed_duplicates = report
        .cache_before
        .duplicate_entries
        .saturating_sub(report.cache_projected_after.duplicate_entries);
    let result_log_removed_duplicates = report
        .result_log_before
        .duplicate_entries
        .saturating_sub(report.result_log_projected_after.duplicate_entries);
    let status = if report.skipped {
        "skipped"
    } else if zero_removal {
        "zero-removal"
    } else if projected_duplicates == 0 {
        "ready"
    } else {
        "needs-compaction"
    };

    vec![
        format!(
            "web_storage_snapshot_readiness: status={} projected_bytes={} projected_removed_bytes={} projected_duplicates={}",
            status, projected_bytes, projected_removed_bytes, projected_duplicates
        ),
        format!("web_storage_snapshot_projected_bytes: {projected_bytes}"),
        format!("web_storage_snapshot_projected_removed_bytes: {projected_removed_bytes}"),
        format!("web_storage_snapshot_projected_duplicates: {projected_duplicates}"),
        format!("web_storage_snapshot_retained_bytes: {projected_bytes}"),
        format!("web_storage_snapshot_removable_bytes: {projected_removed_bytes}"),
        format!("web_storage_snapshot_removable_duplicates: {projected_removed_duplicates}"),
        format!(
            "web_storage_snapshot_cache_retained_bytes: {}",
            report.cache_projected_after.bytes
        ),
        format!("web_storage_snapshot_cache_removable_bytes: {cache_removed_bytes}"),
        format!("web_storage_snapshot_cache_removable_duplicates: {cache_removed_duplicates}"),
        format!(
            "web_storage_snapshot_result_log_retained_bytes: {}",
            report.result_log_projected_after.bytes
        ),
        format!("web_storage_snapshot_result_log_removable_bytes: {result_log_removed_bytes}"),
        format!(
            "web_storage_snapshot_result_log_removable_duplicates: {result_log_removed_duplicates}"
        ),
        format!(
            "web_storage_snapshot_cleanup_scope: report-only dry_run={} cache_path={} result_log_path={}",
            report.dry_run,
            report.cache_path.display(),
            report.result_log_path.display()
        ),
        format!("web_storage_snapshot_zero_removal: {zero_removal}"),
    ]
}

fn web_storage_compaction_apply_is_justified(
    report: &WebSearchStorageCompactionReport,
    force: bool,
) -> bool {
    if force {
        return true;
    }
    if report.skipped {
        return false;
    }
    web_storage_compaction_projected_removed_bytes(report) > 0
        || web_storage_compaction_projected_removed_duplicates(report) > 0
}

fn web_storage_compaction_projected_removed_bytes(
    report: &WebSearchStorageCompactionReport,
) -> u64 {
    report
        .cache_before
        .bytes
        .saturating_add(report.result_log_before.bytes)
        .saturating_sub(
            report
                .cache_projected_after
                .bytes
                .saturating_add(report.result_log_projected_after.bytes),
        )
}

fn web_storage_compaction_projected_removed_duplicates(
    report: &WebSearchStorageCompactionReport,
) -> usize {
    report
        .cache_before
        .duplicate_entries
        .saturating_add(report.result_log_before.duplicate_entries)
        .saturating_sub(
            report
                .cache_projected_after
                .duplicate_entries
                .saturating_add(report.result_log_projected_after.duplicate_entries),
        )
}

fn web_storage_compaction_zero_removal(report: &WebSearchStorageCompactionReport) -> bool {
    !report.skipped
        && web_storage_compaction_projected_removed_bytes(report) == 0
        && web_storage_compaction_projected_removed_duplicates(report) == 0
}

fn web_storage_compaction_artifact_lines(
    label: &str,
    path: &Path,
    before: WebSearchStorageArtifactState,
    after: WebSearchStorageArtifactState,
    projected_after: WebSearchStorageArtifactState,
) -> Vec<String> {
    vec![
        format!("{label}_path: {}", path.display()),
        format!("{label}_bytes_before: {}", before.bytes),
        format!("{label}_bytes_after: {}", after.bytes),
        format!("{label}_bytes_projected_after: {}", projected_after.bytes),
        format!(
            "{label}_bytes_projected_retained: {}",
            projected_after.bytes
        ),
        format!(
            "{label}_bytes_removed: {}",
            before.bytes.saturating_sub(after.bytes)
        ),
        format!(
            "{label}_bytes_projected_removed: {}",
            before.bytes.saturating_sub(projected_after.bytes)
        ),
        format!("{label}_entries_before: {}", before.entries),
        format!("{label}_entries_after: {}", after.entries),
        format!(
            "{label}_entries_projected_after: {}",
            projected_after.entries
        ),
        format!(
            "{label}_entries_projected_retained: {}",
            projected_after.entries
        ),
        format!(
            "{label}_entries_removed: {}",
            before.entries.saturating_sub(after.entries)
        ),
        format!(
            "{label}_entries_projected_removed: {}",
            before.entries.saturating_sub(projected_after.entries)
        ),
        format!("{label}_unique_entries_before: {}", before.unique_entries),
        format!("{label}_unique_entries_after: {}", after.unique_entries),
        format!(
            "{label}_unique_entries_projected_after: {}",
            projected_after.unique_entries
        ),
        format!(
            "{label}_duplicate_entries_before: {}",
            before.duplicate_entries
        ),
        format!(
            "{label}_duplicate_entries_after: {}",
            after.duplicate_entries
        ),
        format!(
            "{label}_duplicate_entries_projected_after: {}",
            projected_after.duplicate_entries
        ),
        format!(
            "{label}_duplicate_entries_projected_removed: {}",
            before
                .duplicate_entries
                .saturating_sub(projected_after.duplicate_entries)
        ),
    ]
}

fn web_storage_result_log_query_cap() -> usize {
    web_storage_env_usize("BRUTAL_WEB_RESULT_LOG_MAX_ENTRIES_PER_QUERY").unwrap_or(0)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct WebStorageRetentionConfig {
    cache_max_entries: usize,
    cache_max_bytes: u64,
    result_log_max_entries: usize,
    result_log_max_bytes: u64,
    result_log_max_entries_per_query: usize,
}

fn web_storage_retention_config() -> WebStorageRetentionConfig {
    WebStorageRetentionConfig {
        cache_max_entries: web_storage_env_usize("BRUTAL_WEB_CACHE_MAX_ENTRIES")
            .unwrap_or(DEFAULT_CACHE_MAX_ENTRIES),
        cache_max_bytes: web_storage_env_u64("BRUTAL_WEB_CACHE_MAX_BYTES")
            .unwrap_or(DEFAULT_CACHE_MAX_BYTES),
        result_log_max_entries: web_storage_env_usize("BRUTAL_WEB_RESULT_LOG_MAX_ENTRIES")
            .unwrap_or(DEFAULT_RESULT_LOG_MAX_ENTRIES),
        result_log_max_bytes: web_storage_env_u64("BRUTAL_WEB_RESULT_LOG_MAX_BYTES")
            .unwrap_or(DEFAULT_RESULT_LOG_MAX_BYTES),
        result_log_max_entries_per_query: web_storage_result_log_query_cap(),
    }
}

fn web_storage_env_usize(name: &str) -> Option<usize> {
    env::var(name)
        .ok()
        .and_then(|value| value.trim().parse::<usize>().ok())
}

fn web_storage_env_u64(name: &str) -> Option<u64> {
    env::var(name)
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
}

fn web_storage_retention_config_lines(config: WebStorageRetentionConfig) -> Vec<String> {
    vec![
        format!(
            "web_storage_retention_summary: web-cache_max_entries={} web-cache_max_bytes={} brave-results_max_entries={} brave-results_max_bytes={} brave-results_max_entries_per_query={}",
            config.cache_max_entries,
            config.cache_max_bytes,
            config.result_log_max_entries,
            config.result_log_max_bytes,
            config.result_log_max_entries_per_query
        ),
        format!(
            "web_storage_cache_max_entries: {}",
            config.cache_max_entries
        ),
        format!("web_storage_cache_max_bytes: {}", config.cache_max_bytes),
        format!(
            "web_storage_result_log_max_entries: {}",
            config.result_log_max_entries
        ),
        format!(
            "web_storage_result_log_max_bytes: {}",
            config.result_log_max_bytes
        ),
        format!(
            "web_storage_result_log_max_entries_per_query: {}",
            config.result_log_max_entries_per_query
        ),
        "web_storage_retention_note: normal search preserves durable web-cache and brave-results rows while enforcing global entry/byte caps; per-query caps apply during compact-web-cache dry-run/compaction when configured".to_owned(),
    ]
}

fn web_storage_result_log_query_cap_lines(max_entries_per_query: usize) -> Vec<String> {
    if max_entries_per_query == 0 {
        return Vec::new();
    }
    vec![
        format!("brave-results_entries_per_query_cap: {max_entries_per_query}"),
        "brave-results_entries_per_query_cap_note: applies only to compact-web-cache and dry-run projection; normal search append is unchanged".to_owned(),
    ]
}

fn collect_web_storage_artifact_stats(
    name: &'static str,
    path: &Path,
) -> Result<WebStorageArtifactStats> {
    let file = std::fs::File::open(path)
        .with_context(|| format!("open web storage artifact {}", path.display()))?;
    let bytes = file
        .metadata()
        .with_context(|| format!("read web storage artifact metadata {}", path.display()))?
        .len();
    let mut stats = WebStorageArtifactStats {
        name,
        bytes,
        entries: 0,
        result_rows: 0,
        durable_result_rows: 0,
        incomplete_result_rows: 0,
        unique_entries: 0,
        duplicate_entries: 0,
        unique_row_bytes: 0,
        duplicate_row_bytes: 0,
        query_count: 0,
        query_examples: Vec::new(),
        provider_count: 0,
        provider_growth: Vec::new(),
        max_entries_per_query: 0,
        oldest_fetched_at_unix: None,
        newest_fetched_at_unix: None,
    };
    let mut unique_keys = std::collections::HashSet::new();
    let mut query_counts = HashMap::new();
    let mut providers = std::collections::HashSet::new();
    let mut provider_growth = BTreeMap::new();

    for (line_no, line) in std::io::BufRead::lines(std::io::BufReader::new(file)).enumerate() {
        let line = line.with_context(|| {
            format!(
                "read line {} from web storage artifact {}",
                line_no + 1,
                path.display()
            )
        })?;
        if line.trim().is_empty() {
            continue;
        }
        stats.entries += 1;
        let row_bytes = u64::try_from(line.len()).unwrap_or(u64::MAX);
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&line) else {
            continue;
        };
        let durability = web_storage_result_row_durability(name, &value);
        stats.result_rows = stats.result_rows.saturating_add(durability.total);
        stats.durable_result_rows = stats.durable_result_rows.saturating_add(durability.durable);
        stats.incomplete_result_rows = stats
            .incomplete_result_rows
            .saturating_add(durability.incomplete);
        if let Some(key) = web_storage_unique_key(name, &value) {
            if unique_keys.insert(key) {
                stats.unique_entries += 1;
                stats.unique_row_bytes = stats.unique_row_bytes.saturating_add(row_bytes);
            } else {
                stats.duplicate_entries += 1;
                stats.duplicate_row_bytes = stats.duplicate_row_bytes.saturating_add(row_bytes);
            }
        }
        if let Some(query) = value
            .get("normalized_query")
            .and_then(|value| value.as_str())
            .filter(|query| !query.is_empty())
        {
            *query_counts.entry(query.to_owned()).or_insert(0usize) += 1;
        }
        if let Some(provider) = value
            .get("provider")
            .and_then(|value| value.as_str())
            .filter(|provider| !provider.is_empty())
        {
            providers.insert(provider.to_owned());
            let growth =
                provider_growth
                    .entry(provider.to_owned())
                    .or_insert(WebStorageProviderGrowth {
                        entries: 0,
                        bytes: 0,
                        result_rows: 0,
                    });
            growth.entries = growth.entries.saturating_add(1);
            growth.bytes = growth.bytes.saturating_add(row_bytes);
            growth.result_rows = growth.result_rows.saturating_add(durability.total);
        }
        let Some(fetched_at_unix) = value
            .get("fetched_at_unix")
            .and_then(|value| value.as_u64())
        else {
            continue;
        };
        stats.oldest_fetched_at_unix = Some(
            stats
                .oldest_fetched_at_unix
                .map_or(fetched_at_unix, |oldest| oldest.min(fetched_at_unix)),
        );
        stats.newest_fetched_at_unix = Some(
            stats
                .newest_fetched_at_unix
                .map_or(fetched_at_unix, |newest| newest.max(fetched_at_unix)),
        );
    }
    let mut query_examples = query_counts.keys().cloned().collect::<Vec<_>>();
    query_examples.sort();
    query_examples.truncate(WEB_STORAGE_QUERY_EXAMPLE_LIMIT);
    stats.query_count = query_counts.len();
    stats.query_examples = query_examples;
    stats.provider_count = providers.len();
    stats.provider_growth = provider_growth
        .into_iter()
        .take(WEB_STORAGE_PROVIDER_GROWTH_LIMIT)
        .map(|(provider, growth)| {
            format!(
                "{}:entries={}:bytes={}:result_rows={}",
                web_storage_sanitize_token(&provider),
                growth.entries,
                growth.bytes,
                growth.result_rows
            )
        })
        .collect();
    stats.max_entries_per_query = query_counts.into_values().max().unwrap_or(0);

    Ok(stats)
}

#[derive(Debug, Clone, Copy, Default)]
struct WebStorageResultRowDurability {
    total: usize,
    durable: usize,
    incomplete: usize,
}

fn web_storage_result_row_durability(
    name: &str,
    value: &serde_json::Value,
) -> WebStorageResultRowDurability {
    match name {
        "web-cache.jsonl" => {
            let Some(results) = value.get("results").and_then(|value| value.as_array()) else {
                return WebStorageResultRowDurability::default();
            };
            let has_entry_metadata = json_string_present(value, "normalized_query")
                && json_string_present(value, "provider")
                && value
                    .get("fetched_at_unix")
                    .and_then(|value| value.as_u64())
                    .is_some();
            let durable = results
                .iter()
                .filter(|result| {
                    has_entry_metadata
                        && json_string_present(result, "url")
                        && result
                            .get("title")
                            .and_then(|value| value.as_str())
                            .is_some()
                        && result
                            .get("snippet")
                            .and_then(|value| value.as_str())
                            .is_some()
                })
                .count();
            WebStorageResultRowDurability {
                total: results.len(),
                durable,
                incomplete: results.len().saturating_sub(durable),
            }
        }
        "brave-results.jsonl" => {
            let durable = json_string_present(value, "normalized_query")
                && json_string_present(value, "provider")
                && json_string_present(value, "url")
                && value
                    .get("title")
                    .and_then(|value| value.as_str())
                    .is_some()
                && value
                    .get("snippet")
                    .and_then(|value| value.as_str())
                    .is_some()
                && value
                    .get("fetched_at_unix")
                    .and_then(|value| value.as_u64())
                    .is_some()
                && value.get("rank").and_then(|value| value.as_u64()).is_some();
            WebStorageResultRowDurability {
                total: 1,
                durable: usize::from(durable),
                incomplete: usize::from(!durable),
            }
        }
        _ => WebStorageResultRowDurability::default(),
    }
}

fn json_string_present(value: &serde_json::Value, key: &str) -> bool {
    value
        .get(key)
        .and_then(|value| value.as_str())
        .is_some_and(|value| !value.is_empty())
}

fn web_storage_unique_key(name: &str, value: &serde_json::Value) -> Option<String> {
    match name {
        "web-cache.jsonl" => value
            .get("normalized_query")
            .and_then(|value| value.as_str())
            .filter(|query| !query.is_empty())
            .map(|query| query.to_owned()),
        "brave-results.jsonl" => {
            let query = value
                .get("normalized_query")
                .and_then(|value| value.as_str())
                .filter(|query| !query.is_empty())?;
            let provider = value
                .get("provider")
                .and_then(|value| value.as_str())
                .unwrap_or_default();
            let url = value
                .get("url")
                .and_then(|value| value.as_str())
                .filter(|url| !url.is_empty())?;
            Some(format!("{query}\t{provider}\t{url}"))
        }
        _ => None,
    }
}

async fn run_recrawl_scheduler_cli(options: RecrawlSchedulerOptions) -> Result<()> {
    let mut round = 0usize;
    loop {
        if options.max_rounds == Some(0) {
            return Ok(());
        }

        round += 1;
        let report = run_recrawl_round(&options, round).await?;
        print_scheduler_report(&report)?;

        if options
            .max_rounds
            .is_some_and(|max_rounds| round >= max_rounds)
        {
            return Ok(());
        }

        if options.poll_secs > 0 {
            tokio::time::sleep(std::time::Duration::from_secs(options.poll_secs)).await;
        }
    }
}

fn print_scheduler_report(report: &RecrawlRoundReport) -> Result<()> {
    println!("{}", serde_json::to_string(report)?);
    std::io::stdout().lock().flush()?;
    Ok(())
}

fn write_recrawl_plan(output: Option<&std::path::Path>, plan: &[RecrawlPlanEntry]) -> Result<()> {
    if let Some(path) = output {
        write_recrawl_plan_file(path, plan, recrawl_plan_output_max_bytes())?;
    } else {
        let stdout = std::io::stdout();
        let mut stdout = stdout.lock();
        write_recrawl_plan_entries(&mut stdout, plan)?;
    }

    Ok(())
}

fn write_recrawl_plan_file(
    path: &std::path::Path,
    plan: &[RecrawlPlanEntry],
    max_bytes: u64,
) -> Result<()> {
    let lines = recrawl_plan_entry_lines(plan)?;
    let total_bytes = lines
        .iter()
        .fold(0u64, |total, line| total.saturating_add(line.len() as u64));
    if max_bytes > 0 && total_bytes > max_bytes {
        bail!(
            "recrawl plan output {} is {} bytes, above BRUTAL_RECRAWL_PLAN_OUTPUT_MAX_BYTES={}",
            path.display(),
            total_bytes,
            max_bytes
        );
    }

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("create recrawl plan output parent {}", parent.display()))?;
    }
    let tmp_path = path.with_extension("tmp");
    {
        let mut file = std::fs::File::create(&tmp_path)
            .with_context(|| format!("create recrawl plan temp {}", tmp_path.display()))?;
        for line in &lines {
            file.write_all(line)?;
        }
        file.flush()?;
    }
    std::fs::rename(&tmp_path, path).with_context(|| {
        format!(
            "replace recrawl plan output {} with {}",
            path.display(),
            tmp_path.display()
        )
    })?;
    Ok(())
}

fn write_recrawl_plan_entries<W: Write>(writer: &mut W, plan: &[RecrawlPlanEntry]) -> Result<()> {
    for line in recrawl_plan_entry_lines(plan)? {
        writer.write_all(&line)?;
    }
    writer.flush()?;
    Ok(())
}

fn recrawl_plan_entry_lines(plan: &[RecrawlPlanEntry]) -> Result<Vec<Vec<u8>>> {
    let mut lines = Vec::with_capacity(plan.len());
    for entry in plan {
        let mut line = Vec::new();
        serde_json::to_writer(
            &mut line,
            &serde_json::json!({
                "url": entry.url.as_str(),
                "priority": entry.priority,
                "recrawl_after": entry.recrawl_after.to_string(),
                "last_fetched_at": entry.last_fetched_at,
                "age_secs": entry.age_secs,
            }),
        )?;
        line.push(b'\n');
        lines.push(line);
    }
    Ok(lines)
}

fn recrawl_plan_output_max_bytes() -> u64 {
    web_storage_env_u64("BRUTAL_RECRAWL_PLAN_OUTPUT_MAX_BYTES")
        .unwrap_or(DEFAULT_RECRAWL_PLAN_OUTPUT_MAX_BYTES)
}

#[cfg(test)]
mod tests {
    use super::*;
    use url::Url;

    #[test]
    fn recrawl_plan_file_respects_output_byte_cap() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("recrawl-plan.jsonl");
        let plan = vec![RecrawlPlanEntry {
            url: format!("https://example.com/{}", "x".repeat(128)),
            priority: 100,
            recrawl_after: 10,
            last_fetched_at: 1,
            age_secs: 9,
        }];

        write_recrawl_plan_file(&path, &plan, 64).unwrap_err();
        assert!(!path.exists());

        write_recrawl_plan_file(&path, &plan, 0).unwrap();
        let contents = std::fs::read_to_string(path).unwrap();
        assert_eq!(contents.lines().count(), 1);
        assert!(contents.contains("https://example.com/"));
    }

    #[test]
    fn index_storage_stats_sum_known_artifacts() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("manifest.json"), b"12345").unwrap();
        std::fs::write(
            dir.path().join("web-cache.jsonl"),
            b"{\"fetched_at_unix\":100}\n{\"fetched_at_unix\":120}\n",
        )
        .unwrap();
        let frontier_path = dir.path().join("frontier.bin");
        let mut frontier = FrontierStore::open(&frontier_path).unwrap();
        frontier
            .discover(Url::parse("https://example.com/queued").unwrap(), 0, 100)
            .then_some(())
            .unwrap();
        let claim = frontier.claim_next(110, 10).unwrap();
        frontier.record_failed(&claim.url, "timeout".to_owned(), 30, 120);
        frontier
            .discover(Url::parse("https://example.com/fetched").unwrap(), 0, 130)
            .then_some(())
            .unwrap();
        let claim = frontier.claim_next(140, 10).unwrap();
        frontier.record_fetched(&claim.url, 200, None, None, 150);
        frontier
            .discover(Url::parse("https://example.com/deferred").unwrap(), 0, 160)
            .then_some(())
            .unwrap();
        let claim = frontier.claim_next(170, 10).unwrap();
        frontier.record_failed(&claim.url, "retry later".to_owned(), 60, 180);
        frontier
            .discover(
                Url::parse("https://example.com/queued-next").unwrap(),
                0,
                190,
            )
            .then_some(())
            .unwrap();
        frontier.save().unwrap();
        std::fs::write(
            dir.path().join("crawl-docs.jsonl"),
            b"{\"url\":\"https://example.com/1\"}\n\n{\"url\":\"https://example.com/2\"}\n",
        )
        .unwrap();
        std::fs::write(
            dir.path().join("brave-results.jsonl"),
            b"{\"fetched_at_unix\":130}\n",
        )
        .unwrap();
        std::fs::write(dir.path().join("untracked.tmp"), b"ignore me").unwrap();

        let stats = collect_index_storage_stats(dir.path()).unwrap();

        let frontier_bytes = std::fs::metadata(&frontier_path).unwrap().len();
        assert_eq!(stats.total_bytes, 142 + frontier_bytes);
        assert_eq!(
            stats.artifacts,
            vec![
                IndexStorageArtifact {
                    name: "manifest.json",
                    bytes: 5
                },
                IndexStorageArtifact {
                    name: "frontier.bin",
                    bytes: frontier_bytes
                },
                IndexStorageArtifact {
                    name: "crawl-docs.jsonl",
                    bytes: 65
                },
                IndexStorageArtifact {
                    name: "web-cache.jsonl",
                    bytes: 48
                },
                IndexStorageArtifact {
                    name: "brave-results.jsonl",
                    bytes: 24
                }
            ]
        );
        assert_eq!(stats.crawl_frontier_bytes, frontier_bytes);
        assert_eq!(
            stats.crawl_frontier_stats,
            Some(FrontierStats {
                queued: 1,
                fetching: 0,
                fetched: 1,
                failed: 1,
                deferred: 1,
                total: 4,
            })
        );
        assert_eq!(stats.crawl_snapshot_bytes, 65);
        assert_eq!(stats.crawl_snapshot_entries, 2);
        assert_eq!(stats.crawl_snapshot_unique_entries, 2);
        assert_eq!(stats.crawl_snapshot_duplicate_entries, 0);
        assert_eq!(
            stats.web_artifacts,
            vec![
                WebStorageArtifactStats {
                    name: "web-cache.jsonl",
                    bytes: 48,
                    entries: 2,
                    result_rows: 0,
                    durable_result_rows: 0,
                    incomplete_result_rows: 0,
                    unique_entries: 0,
                    duplicate_entries: 0,
                    unique_row_bytes: 0,
                    duplicate_row_bytes: 0,
                    query_count: 0,
                    query_examples: Vec::new(),
                    provider_count: 0,
                    provider_growth: Vec::new(),
                    max_entries_per_query: 0,
                    oldest_fetched_at_unix: Some(100),
                    newest_fetched_at_unix: Some(120),
                },
                WebStorageArtifactStats {
                    name: "brave-results.jsonl",
                    bytes: 24,
                    entries: 1,
                    result_rows: 1,
                    durable_result_rows: 0,
                    incomplete_result_rows: 1,
                    unique_entries: 0,
                    duplicate_entries: 0,
                    unique_row_bytes: 0,
                    duplicate_row_bytes: 0,
                    query_count: 0,
                    query_examples: Vec::new(),
                    provider_count: 0,
                    provider_growth: Vec::new(),
                    max_entries_per_query: 0,
                    oldest_fetched_at_unix: Some(130),
                    newest_fetched_at_unix: Some(130),
                },
            ]
        );
    }

    #[test]
    fn index_storage_stats_skip_missing_artifacts() {
        let dir = tempfile::tempdir().unwrap();

        let stats = collect_index_storage_stats(dir.path()).unwrap();

        assert_eq!(stats.total_bytes, 0);
        assert!(stats.artifacts.is_empty());
        assert!(stats.web_artifacts.is_empty());
        assert_eq!(stats.browser_document_bytes, 0);
        assert_eq!(stats.browser_document_rows, 0);
        assert_eq!(stats.browser_document_unique_rows, 0);
        assert_eq!(stats.browser_document_duplicate_rows, 0);
        assert_eq!(stats.browser_document_unique_row_bytes, 0);
        assert_eq!(stats.browser_document_duplicate_row_bytes, 0);
        assert_eq!(stats.crawl_frontier_bytes, 0);
        assert!(stats.crawl_frontier_stats.is_none());
        assert_eq!(stats.crawl_snapshot_bytes, 0);
        assert_eq!(stats.crawl_snapshot_entries, 0);
        assert_eq!(stats.crawl_snapshot_unique_entries, 0);
        assert_eq!(stats.crawl_snapshot_duplicate_entries, 0);
    }

    #[test]
    fn crawl_storage_pressure_summary_reports_frontier_and_snapshots() {
        let stats = IndexStorageStats {
            total_bytes: 170,
            artifacts: Vec::new(),
            web_artifacts: Vec::new(),
            browser_document_bytes: 0,
            browser_document_rows: 0,
            browser_document_unique_rows: 0,
            browser_document_duplicate_rows: 0,
            browser_document_unique_row_bytes: 0,
            browser_document_duplicate_row_bytes: 0,
            crawl_frontier_bytes: 50,
            crawl_frontier_stats: Some(FrontierStats {
                queued: 1,
                fetching: 2,
                fetched: 3,
                failed: DEFAULT_MAX_FAILED_FRONTIER_RECORDS + 2,
                deferred: 5,
                total: DEFAULT_MAX_FAILED_FRONTIER_RECORDS + 13,
            }),
            crawl_snapshot_bytes: 120,
            crawl_snapshot_entries: 4,
            crawl_snapshot_unique_entries: 3,
            crawl_snapshot_duplicate_entries: 1,
        };

        let lines = crawl_storage_pressure_summary_lines(&stats);

        assert!(lines.contains(
            &"crawl_storage_pressure_summary: retained_bytes=170 frontier_bytes=50 snapshot_bytes=120 snapshot_entries=4".to_owned()
        ));
        assert!(lines.contains(&"crawl_storage_retained_bytes: 170".to_owned()));
        assert!(lines.contains(&"crawl_storage_frontier_bytes: 50".to_owned()));
        assert!(lines.contains(&format!(
            "crawl_storage_frontier_records: {}",
            DEFAULT_MAX_FAILED_FRONTIER_RECORDS + 13
        )));
        assert!(lines.contains(&"crawl_storage_frontier_queued: 1".to_owned()));
        assert!(lines.contains(&"crawl_storage_frontier_fetching: 2".to_owned()));
        assert!(lines.contains(&"crawl_storage_frontier_fetched: 3".to_owned()));
        assert!(lines.contains(&format!(
            "crawl_storage_frontier_failed: {}",
            DEFAULT_MAX_FAILED_FRONTIER_RECORDS + 2
        )));
        assert!(lines.contains(&"crawl_storage_frontier_deferred: 5".to_owned()));
        assert!(lines.contains(&format!(
            "crawl_storage_frontier_failed_record_cap: {DEFAULT_MAX_FAILED_FRONTIER_RECORDS}"
        )));
        assert!(lines.contains(&format!(
            "crawl_storage_frontier_failed_projected_after: {DEFAULT_MAX_FAILED_FRONTIER_RECORDS}"
        )));
        assert!(lines.contains(&"crawl_storage_frontier_failed_projected_removed: 2".to_owned()));
        assert!(lines.contains(&"crawl_storage_frontier_failed_zero_removal: false".to_owned()));
        assert!(lines.contains(&"crawl_storage_frontier_dry_run_note: failed frontier records exceed retention cap and are removable by frontier compaction without deleting fetched documents".to_owned()));
        assert!(lines.contains(&"crawl_storage_frontier_apply_guard: report-only; stats does not mutate frontier.bin, run a dedicated frontier compaction apply path only after projected_removed is nonzero".to_owned()));
        assert!(lines.contains(&"crawl_storage_snapshot_bytes: 120".to_owned()));
        assert!(lines.contains(&"crawl_storage_snapshot_entries: 4".to_owned()));
        assert!(lines.contains(&"crawl_storage_snapshot_unique_entries: 3".to_owned()));
        assert!(lines.contains(&"crawl_storage_snapshot_duplicate_entries: 1".to_owned()));
        assert!(lines.contains(&"crawl_storage_snapshot_projected_entries_after: 3".to_owned()));
        assert!(lines.contains(&"crawl_storage_snapshot_projected_entries_removed: 1".to_owned()));
        assert!(lines.contains(&"crawl_storage_snapshot_dry_run_note: duplicate crawl-docs rows are retained until crawl snapshot compaction rewrites latest unique docs".to_owned()));
    }

    #[test]
    fn crawl_storage_frontier_apply_guard_reports_zero_removal() {
        let stats = IndexStorageStats {
            total_bytes: 80,
            artifacts: Vec::new(),
            web_artifacts: Vec::new(),
            browser_document_bytes: 0,
            browser_document_rows: 0,
            browser_document_unique_rows: 0,
            browser_document_duplicate_rows: 0,
            browser_document_unique_row_bytes: 0,
            browser_document_duplicate_row_bytes: 0,
            crawl_frontier_bytes: 80,
            crawl_frontier_stats: Some(FrontierStats {
                queued: 1,
                fetching: 0,
                fetched: 2,
                failed: DEFAULT_MAX_FAILED_FRONTIER_RECORDS,
                deferred: 0,
                total: DEFAULT_MAX_FAILED_FRONTIER_RECORDS + 3,
            }),
            crawl_snapshot_bytes: 0,
            crawl_snapshot_entries: 0,
            crawl_snapshot_unique_entries: 0,
            crawl_snapshot_duplicate_entries: 0,
        };

        let lines = crawl_storage_pressure_summary_lines(&stats);

        assert!(lines.contains(&format!(
            "crawl_storage_frontier_failed_projected_after: {DEFAULT_MAX_FAILED_FRONTIER_RECORDS}"
        )));
        assert!(lines.contains(&"crawl_storage_frontier_failed_projected_removed: 0".to_owned()));
        assert!(lines.contains(&"crawl_storage_frontier_failed_zero_removal: true".to_owned()));
        assert!(lines.contains(&"crawl_storage_frontier_apply_guard: zero-removal; frontier compaction apply would be pointless because all failed records are retained".to_owned()));
        assert!(!lines.iter().any(|line| {
            line == "crawl_storage_frontier_dry_run_note: failed frontier records exceed retention cap and are removable by frontier compaction without deleting fetched documents"
        }));
    }

    #[test]
    fn storage_pressure_rollup_separates_core_web_and_crawl_bytes() {
        let stats = IndexStorageStats {
            total_bytes: 1_000,
            artifacts: Vec::new(),
            web_artifacts: Vec::new(),
            browser_document_bytes: 50,
            browser_document_rows: 3,
            browser_document_unique_rows: 3,
            browser_document_duplicate_rows: 0,
            browser_document_unique_row_bytes: 50,
            browser_document_duplicate_row_bytes: 0,
            crawl_frontier_bytes: 100,
            crawl_frontier_stats: Some(FrontierStats {
                queued: 1,
                fetching: 0,
                fetched: 2,
                failed: 3,
                deferred: 4,
                total: 10,
            }),
            crawl_snapshot_bytes: 200,
            crawl_snapshot_entries: 5,
            crawl_snapshot_unique_entries: 5,
            crawl_snapshot_duplicate_entries: 0,
        };
        let web_summary = WebStoragePressureSummary {
            artifact_count: 2,
            bytes: 300,
            entries: 30,
            result_rows: 45,
            durable_result_rows: 40,
            incomplete_result_rows: 5,
            unique_entries: 26,
            duplicate_entries: 4,
            unique_row_bytes: 260,
            duplicate_row_bytes: 40,
            max_entries_per_query: 3,
            stale_artifacts: 1,
            suggested_dry_runs: 1,
        };

        let lines = storage_pressure_rollup_lines(&stats, &web_summary);

        assert!(lines.contains(&"storage_pressure_total_bytes: 1000".to_owned()));
        assert!(lines.contains(&"storage_pressure_core_index_bytes: 350".to_owned()));
        assert!(lines.contains(&"storage_pressure_web_bytes: 300".to_owned()));
        assert!(lines.contains(&"storage_pressure_browser_document_bytes: 50".to_owned()));
        assert!(lines.contains(&"storage_pressure_crawl_bytes: 300".to_owned()));
        assert!(lines.contains(&"storage_pressure_summary: total_bytes=1000 core_index_bytes=350 web_bytes=300 browser_document_bytes=50 crawl_bytes=300 web_entries=30 web_duplicates=4 snapshot_entries=5 frontier_records=10".to_owned()));
    }

    #[test]
    fn storage_budget_pressure_reports_component_bytes_without_mutation() {
        let stats = IndexStorageStats {
            total_bytes: 1_000,
            artifacts: Vec::new(),
            web_artifacts: Vec::new(),
            browser_document_bytes: 50,
            browser_document_rows: 3,
            browser_document_unique_rows: 3,
            browser_document_duplicate_rows: 0,
            browser_document_unique_row_bytes: 50,
            browser_document_duplicate_row_bytes: 0,
            crawl_frontier_bytes: 100,
            crawl_frontier_stats: Some(FrontierStats {
                queued: 1,
                fetching: 0,
                fetched: 2,
                failed: 3,
                deferred: 4,
                total: 10,
            }),
            crawl_snapshot_bytes: 200,
            crawl_snapshot_entries: 5,
            crawl_snapshot_unique_entries: 5,
            crawl_snapshot_duplicate_entries: 0,
        };
        let web_summary = WebStoragePressureSummary {
            artifact_count: 2,
            bytes: 300,
            entries: 30,
            result_rows: 45,
            durable_result_rows: 40,
            incomplete_result_rows: 5,
            unique_entries: 26,
            duplicate_entries: 4,
            unique_row_bytes: 260,
            duplicate_row_bytes: 40,
            max_entries_per_query: 3,
            stale_artifacts: 1,
            suggested_dry_runs: 1,
        };
        let retention_config = WebStorageRetentionConfig {
            cache_max_entries: 10,
            cache_max_bytes: 200,
            result_log_max_entries: 20,
            result_log_max_bytes: 250,
            result_log_max_entries_per_query: 0,
        };

        let lines = storage_budget_pressure_lines(&stats, &web_summary, retention_config);

        assert!(lines.contains(&format!(
            "storage_budget_summary: status=within-budget total_bytes=1000 budget_bytes={} remaining_bytes={} core_index_bytes=350 web_bytes=300 web_budget_bytes=450 browser_document_bytes=50 crawl_bytes=300",
            DEFAULT_INDEX_STORAGE_BUDGET_BYTES,
            DEFAULT_INDEX_STORAGE_BUDGET_BYTES - 1_000
        )));
        assert!(lines.contains(&"storage_budget_status: within-budget".to_owned()));
        assert!(lines.contains(&"storage_budget_core_index_bytes: 350".to_owned()));
        assert!(lines.contains(&"storage_budget_web_bytes: 300".to_owned()));
        assert!(lines.contains(&"storage_budget_web_budget_bytes: 450".to_owned()));
        assert!(lines.contains(&"storage_budget_browser_document_bytes: 50".to_owned()));
        assert!(lines.contains(&"storage_budget_crawl_bytes: 300".to_owned()));
        assert!(lines.contains(&"storage_budget_report_mode: report-only".to_owned()));
        assert!(lines.contains(&"storage_budget_apply_guard: report-only; stats does not mutate .brutal-index, run dry-run compaction commands only when removable bytes are nonzero".to_owned()));
    }

    #[test]
    fn storage_cleanup_readiness_reports_safe_and_pointless_cleanup() {
        let stats = IndexStorageStats {
            total_bytes: 1_000,
            artifacts: Vec::new(),
            web_artifacts: Vec::new(),
            browser_document_bytes: 50,
            browser_document_rows: 3,
            browser_document_unique_rows: 2,
            browser_document_duplicate_rows: 1,
            browser_document_unique_row_bytes: 35,
            browser_document_duplicate_row_bytes: 15,
            crawl_frontier_bytes: 100,
            crawl_frontier_stats: Some(FrontierStats {
                queued: 1,
                fetching: 0,
                fetched: 2,
                failed: DEFAULT_MAX_FAILED_FRONTIER_RECORDS + 2,
                deferred: 4,
                total: DEFAULT_MAX_FAILED_FRONTIER_RECORDS + 9,
            }),
            crawl_snapshot_bytes: 200,
            crawl_snapshot_entries: 5,
            crawl_snapshot_unique_entries: 4,
            crawl_snapshot_duplicate_entries: 1,
        };
        let web_summary = WebStoragePressureSummary {
            artifact_count: 2,
            bytes: 300,
            entries: 30,
            result_rows: 45,
            durable_result_rows: 40,
            incomplete_result_rows: 5,
            unique_entries: 26,
            duplicate_entries: 4,
            unique_row_bytes: 260,
            duplicate_row_bytes: 40,
            max_entries_per_query: 3,
            stale_artifacts: 1,
            suggested_dry_runs: 1,
        };

        let lines = storage_cleanup_readiness_lines(&stats, &web_summary);

        assert!(lines.contains(&"storage_cleanup_readiness: status=cleanup-available report_mode=report-only retained_bytes=945 known_removable_row_bytes=55 removable_rows=8 web_removable_rows=4 browser_document_removable_rows=1 snapshot_removable_rows=1 frontier_failed_removable_records=2".to_owned()));
        assert!(lines.contains(&"storage_cleanup_safe_to_clean: true".to_owned()));
        assert!(lines.contains(&"storage_cleanup_pointless: false".to_owned()));
        assert!(lines.contains(&"storage_cleanup_web_removable_rows: 4".to_owned()));
        assert!(lines.contains(&"storage_cleanup_browser_document_removable_rows: 1".to_owned()));
        assert!(lines.contains(&"storage_cleanup_snapshot_removable_rows: 1".to_owned()));
        assert!(lines.contains(&"storage_cleanup_frontier_failed_removable_records: 2".to_owned()));
        assert!(lines.contains(&"storage_cleanup_apply_guard: report-only; stats does not mutate .brutal-index, run dry-run/apply cleanup only when storage_cleanup_safe_to_clean is true".to_owned()));

        let clean_stats = IndexStorageStats {
            total_bytes: 700,
            artifacts: Vec::new(),
            web_artifacts: Vec::new(),
            browser_document_bytes: 25,
            browser_document_rows: 1,
            browser_document_unique_rows: 1,
            browser_document_duplicate_rows: 0,
            browser_document_unique_row_bytes: 25,
            browser_document_duplicate_row_bytes: 0,
            crawl_frontier_bytes: 40,
            crawl_frontier_stats: Some(FrontierStats {
                queued: 1,
                fetching: 0,
                fetched: 1,
                failed: DEFAULT_MAX_FAILED_FRONTIER_RECORDS,
                deferred: 0,
                total: DEFAULT_MAX_FAILED_FRONTIER_RECORDS + 2,
            }),
            crawl_snapshot_bytes: 60,
            crawl_snapshot_entries: 2,
            crawl_snapshot_unique_entries: 2,
            crawl_snapshot_duplicate_entries: 0,
        };
        let clean_web_summary = WebStoragePressureSummary {
            artifact_count: 2,
            bytes: 100,
            entries: 10,
            result_rows: 12,
            durable_result_rows: 12,
            incomplete_result_rows: 0,
            unique_entries: 10,
            duplicate_entries: 0,
            unique_row_bytes: 100,
            duplicate_row_bytes: 0,
            max_entries_per_query: 1,
            stale_artifacts: 0,
            suggested_dry_runs: 0,
        };
        let clean_lines = storage_cleanup_readiness_lines(&clean_stats, &clean_web_summary);

        assert!(clean_lines.contains(&"storage_cleanup_readiness: status=zero-removal report_mode=report-only retained_bytes=700 known_removable_row_bytes=0 removable_rows=0 web_removable_rows=0 browser_document_removable_rows=0 snapshot_removable_rows=0 frontier_failed_removable_records=0".to_owned()));
        assert!(clean_lines.contains(&"storage_cleanup_safe_to_clean: false".to_owned()));
        assert!(clean_lines.contains(&"storage_cleanup_pointless: true".to_owned()));
        assert!(clean_lines.contains(&"storage_cleanup_note: cleanup would be pointless because all tracked storage rows are retained".to_owned()));
    }

    #[test]
    fn storage_snapshot_readiness_reports_web_compaction_pressure() {
        let stats = IndexStorageStats {
            total_bytes: 1_000,
            artifacts: Vec::new(),
            web_artifacts: Vec::new(),
            browser_document_bytes: 50,
            browser_document_rows: 3,
            browser_document_unique_rows: 2,
            browser_document_duplicate_rows: 1,
            browser_document_unique_row_bytes: 35,
            browser_document_duplicate_row_bytes: 15,
            crawl_frontier_bytes: 80,
            crawl_frontier_stats: Some(FrontierStats {
                queued: 1,
                fetching: 0,
                fetched: 2,
                failed: 3,
                deferred: 4,
                total: 10,
            }),
            crawl_snapshot_bytes: 120,
            crawl_snapshot_entries: 5,
            crawl_snapshot_unique_entries: 5,
            crawl_snapshot_duplicate_entries: 0,
        };
        let web_summary = WebStoragePressureSummary {
            artifact_count: 2,
            bytes: 300,
            entries: 30,
            result_rows: 45,
            durable_result_rows: 40,
            incomplete_result_rows: 5,
            unique_entries: 26,
            duplicate_entries: 4,
            unique_row_bytes: 260,
            duplicate_row_bytes: 40,
            max_entries_per_query: 3,
            stale_artifacts: 1,
            suggested_dry_runs: 1,
        };

        let lines = storage_snapshot_readiness_lines(&stats, &web_summary);

        assert!(lines.contains(&"storage_snapshot_readiness: status=needs-web-compaction total_bytes=1000 web_bytes=300 browser_document_bytes=50 browser_document_rows=3 browser_document_duplicates=1 crawl_bytes=200 web_entries=30 web_result_rows=45 web_unique_entries=26 web_duplicates=4 web_duplicate_row_bytes=40 web_suggested_dry_runs=1 snapshot_entries=5 frontier_records=10".to_owned()));
        assert!(lines.contains(&"storage_snapshot_status: needs-web-compaction".to_owned()));
        assert!(lines.contains(&"storage_snapshot_browser_document_rows: 3".to_owned()));
        assert!(lines.contains(&"storage_snapshot_browser_document_duplicates: 1".to_owned()));
        assert!(lines.contains(&"storage_snapshot_web_suggested_dry_runs: 1".to_owned()));
        assert!(lines.contains(&"storage_snapshot_frontier_records: 10".to_owned()));
        assert!(lines.contains(
            &"storage_snapshot_cleanup_hint: brutal-search compact-web-cache --dry-run --min-entries 30".to_owned()
        ));
    }

    #[test]
    fn storage_snapshot_readiness_reports_ready_when_web_pressure_is_clean() {
        let stats = IndexStorageStats {
            total_bytes: 700,
            artifacts: Vec::new(),
            web_artifacts: Vec::new(),
            browser_document_bytes: 25,
            browser_document_rows: 1,
            browser_document_unique_rows: 1,
            browser_document_duplicate_rows: 0,
            browser_document_unique_row_bytes: 25,
            browser_document_duplicate_row_bytes: 0,
            crawl_frontier_bytes: 40,
            crawl_frontier_stats: None,
            crawl_snapshot_bytes: 60,
            crawl_snapshot_entries: 2,
            crawl_snapshot_unique_entries: 2,
            crawl_snapshot_duplicate_entries: 0,
        };
        let web_summary = WebStoragePressureSummary {
            artifact_count: 2,
            bytes: 100,
            entries: 10,
            result_rows: 12,
            durable_result_rows: 12,
            incomplete_result_rows: 0,
            unique_entries: 10,
            duplicate_entries: 0,
            unique_row_bytes: 100,
            duplicate_row_bytes: 0,
            max_entries_per_query: 1,
            stale_artifacts: 0,
            suggested_dry_runs: 0,
        };

        let lines = storage_snapshot_readiness_lines(&stats, &web_summary);

        assert!(lines.contains(&"storage_snapshot_readiness: status=ready total_bytes=700 web_bytes=100 browser_document_bytes=25 browser_document_rows=1 browser_document_duplicates=0 crawl_bytes=100 web_entries=10 web_result_rows=12 web_unique_entries=10 web_duplicates=0 web_duplicate_row_bytes=0 web_suggested_dry_runs=0 snapshot_entries=2 frontier_records=0".to_owned()));
        assert!(lines.contains(&"storage_snapshot_status: ready".to_owned()));
        assert!(lines.contains(&"storage_snapshot_browser_document_rows: 1".to_owned()));
        assert!(lines.contains(&"storage_snapshot_browser_document_duplicates: 0".to_owned()));
        assert!(lines.contains(&"storage_snapshot_web_suggested_dry_runs: 0".to_owned()));
        assert!(
            !lines
                .iter()
                .any(|line| line.starts_with("storage_snapshot_cleanup_hint:"))
        );
    }

    #[test]
    fn browser_document_storage_pressure_reports_zero_removal() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("browser-documents.jsonl");
        std::fs::write(
            &path,
            b"{\"session_id\":\"s1\",\"url\":\"https://example.com/a\"}\n{\"session_id\":\"s1\",\"url\":\"https://example.com/b\"}\n",
        )
        .unwrap();

        let stats = collect_index_storage_stats(dir.path()).unwrap();
        let lines = browser_document_storage_pressure_summary_lines(&stats);

        assert_eq!(stats.browser_document_rows, 2);
        assert_eq!(stats.browser_document_unique_rows, 2);
        assert_eq!(stats.browser_document_duplicate_rows, 0);
        assert!(lines.contains(&format!(
            "browser_document_storage_bytes: {}",
            stats.browser_document_bytes
        )));
        assert!(lines.contains(&"browser_document_storage_rows: 2".to_owned()));
        assert!(lines.contains(&"browser_document_storage_unique_rows: 2".to_owned()));
        assert!(lines.contains(&"browser_document_storage_duplicate_rows: 0".to_owned()));
        assert!(lines.contains(&"browser_document_storage_projected_rows_after: 2".to_owned()));
        assert!(lines.contains(&"browser_document_storage_projected_rows_removed: 0".to_owned()));
        assert!(lines.contains(&"browser_document_storage_zero_removal: true".to_owned()));
        assert!(lines.contains(&"browser_document_storage_dry_run_note: all browser document rows are retained; cleanup would remove nothing".to_owned()));
    }

    #[test]
    fn web_storage_stats_counts_unique_and_duplicate_entries() {
        let dir = tempfile::tempdir().unwrap();
        let cache_path = dir.path().join("web-cache.jsonl");
        std::fs::write(
            &cache_path,
            b"{\"normalized_query\":\"one\",\"fetched_at_unix\":100,\"results\":[{\"url\":\"https://example.com/a\"},{\"url\":\"https://example.com/b\"}]}\n{\"normalized_query\":\"one\",\"fetched_at_unix\":120,\"results\":[{\"url\":\"https://example.com/c\"}]}\n{\"normalized_query\":\"two\",\"fetched_at_unix\":130,\"results\":[{\"url\":\"https://example.com/d\"}]}\n",
        )
        .unwrap();
        let result_log_path = dir.path().join("brave-results.jsonl");
        std::fs::write(
            &result_log_path,
            b"{\"normalized_query\":\"one\",\"provider\":\"brave\",\"rank\":1,\"url\":\"https://example.com/a\",\"fetched_at_unix\":100}\n{\"normalized_query\":\"one\",\"provider\":\"brave\",\"rank\":2,\"url\":\"https://example.com/a\",\"fetched_at_unix\":120}\n{\"normalized_query\":\"one\",\"provider\":\"brave\",\"rank\":3,\"url\":\"https://example.com/b\",\"fetched_at_unix\":130}\n",
        )
        .unwrap();

        let cache_stats =
            collect_web_storage_artifact_stats("web-cache.jsonl", &cache_path).unwrap();
        let log_stats =
            collect_web_storage_artifact_stats("brave-results.jsonl", &result_log_path).unwrap();

        assert_eq!(cache_stats.entries, 3);
        assert!(cache_stats.bytes > 147);
        assert_eq!(cache_stats.result_rows, 4);
        assert_eq!(cache_stats.durable_result_rows, 0);
        assert_eq!(cache_stats.incomplete_result_rows, 4);
        assert_eq!(cache_stats.unique_entries, 2);
        assert_eq!(cache_stats.duplicate_entries, 1);
        assert_eq!(cache_stats.query_count, 2);
        assert_eq!(
            cache_stats.query_examples,
            vec!["one".to_owned(), "two".to_owned()]
        );
        assert_eq!(cache_stats.provider_count, 0);
        assert_eq!(cache_stats.max_entries_per_query, 2);
        assert_eq!(log_stats.entries, 3);
        assert_eq!(log_stats.result_rows, 3);
        assert_eq!(log_stats.durable_result_rows, 0);
        assert_eq!(log_stats.incomplete_result_rows, 3);
        assert_eq!(log_stats.bytes, 321);
        assert_eq!(log_stats.unique_entries, 2);
        assert_eq!(log_stats.duplicate_entries, 1);
        assert_eq!(log_stats.query_count, 1);
        assert_eq!(log_stats.query_examples, vec!["one".to_owned()]);
        assert_eq!(log_stats.provider_count, 1);
        assert_eq!(log_stats.max_entries_per_query, 3);
    }

    #[test]
    fn web_storage_compaction_suggestion_points_to_dry_run() {
        let duplicate_artifact = WebStorageArtifactStats {
            name: "web-cache.jsonl",
            bytes: 120,
            entries: 3,
            result_rows: 4,
            durable_result_rows: 4,
            incomplete_result_rows: 0,
            unique_entries: 2,
            duplicate_entries: 1,
            unique_row_bytes: 80,
            duplicate_row_bytes: 40,
            query_count: 2,
            query_examples: Vec::new(),
            provider_count: 1,
            provider_growth: Vec::new(),
            max_entries_per_query: 2,
            oldest_fetched_at_unix: Some(100),
            newest_fetched_at_unix: Some(120),
        };
        let large_artifact = WebStorageArtifactStats {
            name: "brave-results.jsonl",
            bytes: 4096,
            entries: WEB_STORAGE_COMPACT_SUGGEST_MIN_ENTRIES,
            result_rows: WEB_STORAGE_COMPACT_SUGGEST_MIN_ENTRIES,
            durable_result_rows: WEB_STORAGE_COMPACT_SUGGEST_MIN_ENTRIES,
            incomplete_result_rows: 0,
            unique_entries: WEB_STORAGE_COMPACT_SUGGEST_MIN_ENTRIES,
            duplicate_entries: 0,
            unique_row_bytes: 4096,
            duplicate_row_bytes: 0,
            query_count: WEB_STORAGE_COMPACT_SUGGEST_MIN_ENTRIES,
            query_examples: Vec::new(),
            provider_count: 1,
            provider_growth: Vec::new(),
            max_entries_per_query: 1,
            oldest_fetched_at_unix: Some(100),
            newest_fetched_at_unix: Some(120),
        };
        let small_clean_artifact = WebStorageArtifactStats {
            name: "web-cache.jsonl",
            bytes: 80,
            entries: 2,
            result_rows: 2,
            durable_result_rows: 2,
            incomplete_result_rows: 0,
            unique_entries: 2,
            duplicate_entries: 0,
            unique_row_bytes: 80,
            duplicate_row_bytes: 0,
            query_count: 2,
            query_examples: Vec::new(),
            provider_count: 1,
            provider_growth: Vec::new(),
            max_entries_per_query: 1,
            oldest_fetched_at_unix: Some(100),
            newest_fetched_at_unix: Some(120),
        };

        assert_eq!(
            web_storage_compaction_suggestion(
                &duplicate_artifact,
                200,
                DEFAULT_WEB_STORAGE_STALE_SECS
            )
            .as_deref(),
            Some("brutal-search compact-web-cache --dry-run --min-entries 3")
        );
        assert_eq!(
            web_storage_compaction_suggestion(&large_artifact, 200, DEFAULT_WEB_STORAGE_STALE_SECS)
                .as_deref(),
            Some("brutal-search compact-web-cache --dry-run --min-entries 1024")
        );
        assert!(
            web_storage_compaction_suggestion(
                &small_clean_artifact,
                200,
                DEFAULT_WEB_STORAGE_STALE_SECS
            )
            .is_none()
        );
    }

    #[test]
    fn web_storage_compaction_suggestion_flags_stale_artifacts() {
        let stale_artifact = WebStorageArtifactStats {
            name: "web-cache.jsonl",
            bytes: 120,
            entries: 2,
            result_rows: 2,
            durable_result_rows: 2,
            incomplete_result_rows: 0,
            unique_entries: 2,
            duplicate_entries: 0,
            unique_row_bytes: 120,
            duplicate_row_bytes: 0,
            query_count: 2,
            query_examples: Vec::new(),
            provider_count: 1,
            provider_growth: Vec::new(),
            max_entries_per_query: 1,
            oldest_fetched_at_unix: Some(100),
            newest_fetched_at_unix: Some(120),
        };
        let fresh_artifact = WebStorageArtifactStats {
            name: "brave-results.jsonl",
            bytes: 90,
            entries: 2,
            result_rows: 2,
            durable_result_rows: 2,
            incomplete_result_rows: 0,
            unique_entries: 2,
            duplicate_entries: 0,
            unique_row_bytes: 90,
            duplicate_row_bytes: 0,
            query_count: 2,
            query_examples: Vec::new(),
            provider_count: 1,
            provider_growth: Vec::new(),
            max_entries_per_query: 1,
            oldest_fetched_at_unix: Some(190),
            newest_fetched_at_unix: Some(200),
        };

        assert_eq!(web_storage_oldest_age_secs(&stale_artifact, 200), Some(100));
        assert_eq!(
            web_storage_compaction_suggestion(&stale_artifact, 200, 60).as_deref(),
            Some("brutal-search compact-web-cache --dry-run --min-entries 1")
        );
        assert!(web_storage_compaction_suggestion(&fresh_artifact, 200, 60).is_none());
    }

    #[test]
    fn web_storage_pressure_summary_lines_report_aggregate_pressure() {
        let lines = web_storage_pressure_summary_lines(
            &[
                WebStorageArtifactStats {
                    name: "web-cache.jsonl",
                    bytes: 120,
                    entries: 3,
                    result_rows: 4,
                    durable_result_rows: 4,
                    incomplete_result_rows: 0,
                    unique_entries: 2,
                    duplicate_entries: 1,
                    unique_row_bytes: 80,
                    duplicate_row_bytes: 40,
                    query_count: 2,
                    query_examples: Vec::new(),
                    provider_count: 1,
                    provider_growth: vec!["brave:entries=3:bytes=120:result_rows=4".to_owned()],
                    max_entries_per_query: 2,
                    oldest_fetched_at_unix: Some(100),
                    newest_fetched_at_unix: Some(120),
                },
                WebStorageArtifactStats {
                    name: "brave-results.jsonl",
                    bytes: 90,
                    entries: 2,
                    result_rows: 2,
                    durable_result_rows: 2,
                    incomplete_result_rows: 0,
                    unique_entries: 2,
                    duplicate_entries: 0,
                    unique_row_bytes: 90,
                    duplicate_row_bytes: 0,
                    query_count: 2,
                    query_examples: Vec::new(),
                    provider_count: 1,
                    provider_growth: vec!["brave:entries=2:bytes=90:result_rows=2".to_owned()],
                    max_entries_per_query: 1,
                    oldest_fetched_at_unix: Some(190),
                    newest_fetched_at_unix: Some(200),
                },
            ],
            200,
            60,
        );

        assert!(lines.contains(
            &"web_storage_pressure_summary: artifacts=2 bytes=210 entries=5 result_rows=6 durable_result_rows=6 incomplete_result_rows=0 unique_entries=4 duplicates=1 duplicate_row_bytes=40 stale_artifacts=1 suggested_dry_runs=1".to_owned()
        ));
        assert!(lines.contains(&"web_storage_pressure_bytes: 210".to_owned()));
        assert!(lines.contains(&"web_storage_pressure_result_rows: 6".to_owned()));
        assert!(lines.contains(&"web_storage_pressure_durable_result_rows: 6".to_owned()));
        assert!(lines.contains(&"web_storage_pressure_incomplete_result_rows: 0".to_owned()));
        assert!(lines.contains(&"web_storage_pressure_unique_entries: 4".to_owned()));
        assert!(lines.contains(&"web_storage_pressure_projected_entries_after: 4".to_owned()));
        assert!(lines.contains(&"web_storage_pressure_projected_entries_removed: 1".to_owned()));
        assert!(lines.contains(&"web_storage_pressure_projected_row_bytes_after: 170".to_owned()));
        assert!(lines.contains(&"web_storage_pressure_projected_row_bytes_removed: 40".to_owned()));
        assert!(lines.contains(&"web_storage_pressure_retained_result_rows: 6".to_owned()));
        assert!(lines.contains(&"web_storage_pressure_removable_row_bytes: 40".to_owned()));
        assert!(lines.contains(&"web_storage_pressure_zero_removal: false".to_owned()));
        assert!(lines.contains(&"web_storage_pressure_duplicate_row_bytes: 40".to_owned()));
        assert!(lines.contains(&"web_storage_pressure_duplicate_entries: 1".to_owned()));
        assert!(lines.contains(&"web_storage_pressure_max_entries_per_query: 2".to_owned()));
        assert!(lines.contains(&"web_storage_export_cache_query_buckets: 2".to_owned()));
        assert!(lines.contains(
            &"web_storage_replay_readiness: status=ready report_only=true cache_query_buckets=2 replayable_result_rows=4 result_log_unique_urls=2".to_owned()
        ));
        assert!(lines.contains(&"web_storage_replayable_result_rows: 4".to_owned()));
        assert!(lines.contains(&"web_storage_export_unique_result_urls: 2".to_owned()));
        assert!(lines.contains(&"web_storage_export_durable_result_rows: 6".to_owned()));
        assert!(lines.contains(&"web_storage_export_incomplete_result_rows: 0".to_owned()));
        assert!(lines.contains(&"web_storage_export_duplicate_rows: 1".to_owned()));
        assert!(lines.contains(
            &"web_storage_pressure_suggestion: brutal-search compact-web-cache --dry-run --min-entries 5".to_owned()
        ));
    }

    #[test]
    fn web_storage_export_readiness_lines_explain_report_only_manifest() {
        let ready_summary = WebStoragePressureSummary {
            artifact_count: 2,
            bytes: 210,
            entries: 5,
            result_rows: 6,
            durable_result_rows: 6,
            incomplete_result_rows: 0,
            unique_entries: 4,
            duplicate_entries: 1,
            unique_row_bytes: 170,
            duplicate_row_bytes: 40,
            max_entries_per_query: 2,
            stale_artifacts: 0,
            suggested_dry_runs: 0,
        };
        let ready_lines = web_storage_export_readiness_lines(
            &[
                WebStorageArtifactStats {
                    name: "web-cache.jsonl",
                    bytes: 120,
                    entries: 3,
                    result_rows: 4,
                    durable_result_rows: 4,
                    incomplete_result_rows: 0,
                    unique_entries: 2,
                    duplicate_entries: 1,
                    unique_row_bytes: 80,
                    duplicate_row_bytes: 40,
                    query_count: 2,
                    query_examples: vec!["one".to_owned(), "two".to_owned()],
                    provider_count: 1,
                    provider_growth: vec!["brave:entries=3:bytes=120:result_rows=4".to_owned()],
                    max_entries_per_query: 2,
                    oldest_fetched_at_unix: Some(100),
                    newest_fetched_at_unix: Some(120),
                },
                WebStorageArtifactStats {
                    name: "brave-results.jsonl",
                    bytes: 90,
                    entries: 2,
                    result_rows: 2,
                    durable_result_rows: 2,
                    incomplete_result_rows: 0,
                    unique_entries: 2,
                    duplicate_entries: 0,
                    unique_row_bytes: 90,
                    duplicate_row_bytes: 0,
                    query_count: 2,
                    query_examples: vec!["one".to_owned(), "two".to_owned()],
                    provider_count: 1,
                    provider_growth: vec!["brave:entries=2:bytes=90:result_rows=2".to_owned()],
                    max_entries_per_query: 1,
                    oldest_fetched_at_unix: Some(190),
                    newest_fetched_at_unix: Some(200),
                },
            ],
            &ready_summary,
            200,
            60,
        );

        assert!(ready_lines.contains(
            &"web_storage_export_readiness: status=ready report_only=true cache_query_buckets=2 unique_result_urls=2 durable_result_rows=6 incomplete_result_rows=0 duplicate_rows=1".to_owned()
        ));
        assert!(ready_lines.contains(
            &"web_storage_replay_readiness: status=ready report_only=true cache_query_buckets=2 replayable_result_rows=4 result_log_unique_urls=2".to_owned()
        ));
        assert!(ready_lines.contains(
            &"web_storage_export_manifest: report_only=true export_status=ready replay_status=ready staleness_status=fresh newest_age_secs=0 stale_after_secs=60 retained_bytes=170 removable_bytes=40 retained_rows=4 removable_rows=1 cache_query_buckets=2 unique_result_urls=2".to_owned()
        ));
        assert!(ready_lines.contains(
            &"web_storage_replay_query_coverage: report_only=true cache_query_buckets=2 result_log_query_buckets=2 missing_query_buckets=0".to_owned()
        ));
        assert!(ready_lines.contains(
            &"web_storage_replay_missing_query_examples: report_only=true limit=3 examples=none".to_owned()
        ));
        assert!(ready_lines.contains(
            &"web_storage_provider_growth: report_only=true limit=3 web-cache.jsonl=brave:entries=3:bytes=120:result_rows=4 brave-results.jsonl=brave:entries=2:bytes=90:result_rows=2".to_owned()
        ));
        assert!(ready_lines.contains(
            &"web_storage_replay_staleness: status=fresh report_only=true newest_age_secs=0 oldest_age_secs=100 stale_after_secs=60".to_owned()
        ));
        assert!(ready_lines.contains(
            &"web_storage_compaction_decision: report_only=true reason=duplicate-bytes duplicate_row_bytes=40 missing_query_buckets=0 provider_buckets=1 staleness_status=fresh".to_owned()
        ));
        assert!(ready_lines.contains(&"web_storage_provider_buckets: 1".to_owned()));
        assert!(ready_lines.contains(&"web_storage_replay_missing_query_buckets: 0".to_owned()));
        assert!(ready_lines.contains(&"web_storage_replayable_result_rows: 4".to_owned()));
        assert!(ready_lines.contains(
            &"web_storage_export_note: report-only; does not rewrite .brutal-index or cached web artifacts".to_owned()
        ));

        let mut partial_summary = ready_summary.clone();
        partial_summary.incomplete_result_rows = 1;
        partial_summary.duplicate_entries = 0;
        partial_summary.duplicate_row_bytes = 0;
        let partial_lines = web_storage_export_readiness_lines(&[], &partial_summary, 200, 60);
        assert!(partial_lines.contains(
            &"web_storage_export_readiness: status=partial report_only=true cache_query_buckets=0 unique_result_urls=0 durable_result_rows=6 incomplete_result_rows=1 duplicate_rows=0".to_owned()
        ));
        assert!(partial_lines.contains(
            &"web_storage_replay_readiness: status=empty report_only=true cache_query_buckets=0 replayable_result_rows=0 result_log_unique_urls=0".to_owned()
        ));
        assert!(partial_lines.contains(
            &"web_storage_replay_staleness: status=unknown report_only=true newest_age_secs=unknown oldest_age_secs=unknown stale_after_secs=60".to_owned()
        ));
        assert!(partial_lines.contains(
            &"web_storage_replay_missing_query_examples: report_only=true limit=3 examples=none".to_owned()
        ));
        assert!(partial_lines.contains(
            &"web_storage_compaction_decision: report_only=true reason=zero-removal duplicate_row_bytes=0 missing_query_buckets=0 provider_buckets=0 staleness_status=unknown".to_owned()
        ));

        let mut empty_summary = ready_summary;
        empty_summary.result_rows = 0;
        empty_summary.durable_result_rows = 0;
        empty_summary.incomplete_result_rows = 0;
        let empty_lines = web_storage_export_readiness_lines(&[], &empty_summary, 200, 60);
        assert!(empty_lines.contains(
            &"web_storage_export_readiness: status=empty report_only=true cache_query_buckets=0 unique_result_urls=0 durable_result_rows=0 incomplete_result_rows=0 duplicate_rows=1".to_owned()
        ));
    }

    #[test]
    fn web_storage_replay_readiness_flags_result_log_only_state() {
        let summary = WebStoragePressureSummary {
            artifact_count: 1,
            bytes: 90,
            entries: 2,
            result_rows: 2,
            durable_result_rows: 2,
            incomplete_result_rows: 0,
            unique_entries: 2,
            duplicate_entries: 0,
            unique_row_bytes: 90,
            duplicate_row_bytes: 0,
            max_entries_per_query: 1,
            stale_artifacts: 0,
            suggested_dry_runs: 0,
        };
        let lines = web_storage_export_readiness_lines(
            &[WebStorageArtifactStats {
                name: "brave-results.jsonl",
                bytes: 90,
                entries: 2,
                result_rows: 2,
                durable_result_rows: 2,
                incomplete_result_rows: 0,
                unique_entries: 2,
                duplicate_entries: 0,
                unique_row_bytes: 90,
                duplicate_row_bytes: 0,
                query_count: 2,
                query_examples: vec!["cached only".to_owned(), "result only".to_owned()],
                provider_count: 1,
                provider_growth: vec!["brave:entries=2:bytes=90:result_rows=2".to_owned()],
                max_entries_per_query: 1,
                oldest_fetched_at_unix: Some(190),
                newest_fetched_at_unix: Some(200),
            }],
            &summary,
            300,
            60,
        );

        assert!(lines.contains(
            &"web_storage_replay_readiness: status=miss-risk report_only=true cache_query_buckets=0 replayable_result_rows=0 result_log_unique_urls=2".to_owned()
        ));
        assert!(lines.contains(
            &"web_storage_replay_query_coverage: report_only=true cache_query_buckets=0 result_log_query_buckets=2 missing_query_buckets=2".to_owned()
        ));
        assert!(lines.contains(
            &"web_storage_replay_missing_query_examples: report_only=true limit=3 examples=cached_only,result_only".to_owned()
        ));
        assert!(lines.contains(
            &"web_storage_provider_growth: report_only=true limit=3 brave-results.jsonl=brave:entries=2:bytes=90:result_rows=2".to_owned()
        ));
        assert!(lines.contains(
            &"web_storage_replay_staleness: status=stale report_only=true newest_age_secs=100 oldest_age_secs=110 stale_after_secs=60".to_owned()
        ));
        assert!(lines.contains(
            &"web_storage_compaction_decision: report_only=true reason=replay-misses duplicate_row_bytes=0 missing_query_buckets=2 provider_buckets=1 staleness_status=stale".to_owned()
        ));
        assert!(lines.contains(&"web_storage_provider_buckets: 1".to_owned()));
        assert!(lines.contains(&"web_storage_replay_missing_query_buckets: 2".to_owned()));
        assert!(lines.contains(
            &"web_storage_export_manifest: report_only=true export_status=ready replay_status=miss-risk staleness_status=stale newest_age_secs=100 stale_after_secs=60 retained_bytes=90 removable_bytes=0 retained_rows=2 removable_rows=0 cache_query_buckets=0 unique_result_urls=2".to_owned()
        ));
    }

    #[test]
    fn web_storage_compaction_decision_reports_provider_and_staleness_reasons() {
        let summary = WebStoragePressureSummary {
            artifact_count: 2,
            bytes: 210,
            entries: 5,
            result_rows: 6,
            durable_result_rows: 6,
            incomplete_result_rows: 0,
            unique_entries: 5,
            duplicate_entries: 0,
            unique_row_bytes: 210,
            duplicate_row_bytes: 0,
            max_entries_per_query: 2,
            stale_artifacts: 0,
            suggested_dry_runs: 0,
        };
        let provider_lines = web_storage_export_readiness_lines(
            &[WebStorageArtifactStats {
                name: "web-cache.jsonl",
                bytes: 120,
                entries: 3,
                result_rows: 4,
                durable_result_rows: 4,
                incomplete_result_rows: 0,
                unique_entries: 3,
                duplicate_entries: 0,
                unique_row_bytes: 120,
                duplicate_row_bytes: 0,
                query_count: 2,
                query_examples: Vec::new(),
                provider_count: 2,
                provider_growth: vec![
                    "brave:entries=2:bytes=80:result_rows=2".to_owned(),
                    "cache:entries=1:bytes=40:result_rows=2".to_owned(),
                ],
                max_entries_per_query: 2,
                oldest_fetched_at_unix: Some(100),
                newest_fetched_at_unix: Some(120),
            }],
            &summary,
            130,
            60,
        );
        assert!(provider_lines.contains(
            &"web_storage_compaction_decision: report_only=true reason=multiple-providers duplicate_row_bytes=0 missing_query_buckets=0 provider_buckets=2 staleness_status=fresh".to_owned()
        ));

        let stale_lines = web_storage_export_readiness_lines(
            &[WebStorageArtifactStats {
                name: "web-cache.jsonl",
                bytes: 120,
                entries: 3,
                result_rows: 4,
                durable_result_rows: 4,
                incomplete_result_rows: 0,
                unique_entries: 3,
                duplicate_entries: 0,
                unique_row_bytes: 120,
                duplicate_row_bytes: 0,
                query_count: 2,
                query_examples: Vec::new(),
                provider_count: 1,
                provider_growth: vec!["brave:entries=3:bytes=120:result_rows=4".to_owned()],
                max_entries_per_query: 2,
                oldest_fetched_at_unix: Some(100),
                newest_fetched_at_unix: Some(120),
            }],
            &summary,
            300,
            60,
        );
        assert!(stale_lines.contains(
            &"web_storage_compaction_decision: report_only=true reason=stale-cache duplicate_row_bytes=0 missing_query_buckets=0 provider_buckets=1 staleness_status=stale".to_owned()
        ));
    }

    #[test]
    fn web_storage_stats_report_durable_and_incomplete_result_rows() {
        let dir = tempfile::tempdir().unwrap();
        let cache_path = dir.path().join("web-cache.jsonl");
        std::fs::write(
            &cache_path,
            b"{\"normalized_query\":\"complete\",\"provider\":\"brave\",\"fetched_at_unix\":100,\"results\":[{\"url\":\"https://example.com/a\",\"title\":\"A\",\"snippet\":\"Snippet\"}]}\n{\"normalized_query\":\"partial\",\"provider\":\"brave\",\"fetched_at_unix\":110,\"results\":[{\"url\":\"https://example.com/b\",\"title\":\"B\"}]}\n",
        )
        .unwrap();
        let result_log_path = dir.path().join("brave-results.jsonl");
        std::fs::write(
            &result_log_path,
            b"{\"normalized_query\":\"complete\",\"provider\":\"brave\",\"rank\":1,\"url\":\"https://example.com/a\",\"title\":\"A\",\"snippet\":\"Snippet\",\"fetched_at_unix\":100}\n{\"normalized_query\":\"partial\",\"provider\":\"brave\",\"rank\":2,\"url\":\"https://example.com/b\",\"title\":\"B\",\"fetched_at_unix\":110}\n",
        )
        .unwrap();

        let cache_stats =
            collect_web_storage_artifact_stats("web-cache.jsonl", &cache_path).unwrap();
        let log_stats =
            collect_web_storage_artifact_stats("brave-results.jsonl", &result_log_path).unwrap();
        let lines =
            web_storage_pressure_summary_lines(&[cache_stats.clone(), log_stats.clone()], 120, 60);

        assert_eq!(cache_stats.result_rows, 2);
        assert_eq!(cache_stats.durable_result_rows, 1);
        assert_eq!(cache_stats.incomplete_result_rows, 1);
        assert_eq!(log_stats.result_rows, 2);
        assert_eq!(log_stats.durable_result_rows, 1);
        assert_eq!(log_stats.incomplete_result_rows, 1);
        assert!(lines.contains(&"web_storage_pressure_result_rows: 4".to_owned()));
        assert!(lines.contains(&"web_storage_pressure_durable_result_rows: 2".to_owned()));
        assert!(lines.contains(&"web_storage_pressure_incomplete_result_rows: 2".to_owned()));
    }

    #[test]
    fn web_storage_compaction_artifact_lines_include_projected_savings() {
        let lines = web_storage_compaction_artifact_lines(
            "web-cache",
            Path::new("/tmp/web-cache.jsonl"),
            WebSearchStorageArtifactState {
                bytes: 120,
                entries: 6,
                unique_entries: 4,
                duplicate_entries: 2,
            },
            WebSearchStorageArtifactState {
                bytes: 120,
                entries: 6,
                unique_entries: 4,
                duplicate_entries: 2,
            },
            WebSearchStorageArtifactState {
                bytes: 80,
                entries: 4,
                unique_entries: 4,
                duplicate_entries: 0,
            },
        );

        assert!(lines.contains(&"web-cache_bytes_projected_after: 80".to_owned()));
        assert!(lines.contains(&"web-cache_bytes_projected_retained: 80".to_owned()));
        assert!(lines.contains(&"web-cache_bytes_projected_removed: 40".to_owned()));
        assert!(lines.contains(&"web-cache_entries_projected_after: 4".to_owned()));
        assert!(lines.contains(&"web-cache_entries_projected_retained: 4".to_owned()));
        assert!(lines.contains(&"web-cache_entries_projected_removed: 2".to_owned()));
        assert!(lines.contains(&"web-cache_unique_entries_projected_after: 4".to_owned()));
        assert!(lines.contains(&"web-cache_duplicate_entries_before: 2".to_owned()));
        assert!(lines.contains(&"web-cache_duplicate_entries_projected_after: 0".to_owned()));
        assert!(lines.contains(&"web-cache_duplicate_entries_projected_removed: 2".to_owned()));
        assert!(lines.contains(&"web-cache_bytes_removed: 0".to_owned()));
        assert!(lines.contains(&"web-cache_entries_removed: 0".to_owned()));
    }

    #[test]
    fn web_storage_compaction_snapshot_readiness_reports_projected_pressure() {
        let report = WebSearchStorageCompactionReport {
            cache_path: PathBuf::from("web-cache.jsonl"),
            result_log_path: PathBuf::from("brave-results.jsonl"),
            cache_before: WebSearchStorageArtifactState {
                bytes: 120,
                entries: 4,
                unique_entries: 2,
                duplicate_entries: 2,
            },
            cache_after: WebSearchStorageArtifactState {
                bytes: 120,
                entries: 4,
                unique_entries: 2,
                duplicate_entries: 2,
            },
            cache_projected_after: WebSearchStorageArtifactState {
                bytes: 80,
                entries: 2,
                unique_entries: 2,
                duplicate_entries: 0,
            },
            result_log_before: WebSearchStorageArtifactState {
                bytes: 90,
                entries: 3,
                unique_entries: 2,
                duplicate_entries: 1,
            },
            result_log_after: WebSearchStorageArtifactState {
                bytes: 90,
                entries: 3,
                unique_entries: 2,
                duplicate_entries: 1,
            },
            result_log_projected_after: WebSearchStorageArtifactState {
                bytes: 70,
                entries: 2,
                unique_entries: 2,
                duplicate_entries: 0,
            },
            skipped: false,
            dry_run: true,
        };

        let lines = web_storage_compaction_snapshot_readiness_lines(&report);

        assert!(lines.contains(&"web_storage_snapshot_readiness: status=ready projected_bytes=150 projected_removed_bytes=60 projected_duplicates=0".to_owned()));
        assert!(lines.contains(&"web_storage_snapshot_projected_bytes: 150".to_owned()));
        assert!(lines.contains(&"web_storage_snapshot_projected_removed_bytes: 60".to_owned()));
        assert!(lines.contains(&"web_storage_snapshot_projected_duplicates: 0".to_owned()));
        assert!(lines.contains(&"web_storage_snapshot_retained_bytes: 150".to_owned()));
        assert!(lines.contains(&"web_storage_snapshot_removable_bytes: 60".to_owned()));
        assert!(lines.contains(&"web_storage_snapshot_removable_duplicates: 3".to_owned()));
        assert!(lines.contains(&"web_storage_snapshot_zero_removal: false".to_owned()));
    }

    #[test]
    fn web_storage_compaction_snapshot_readiness_reports_artifact_cleanup_split() {
        let report = WebSearchStorageCompactionReport {
            cache_path: PathBuf::from("web-cache.jsonl"),
            result_log_path: PathBuf::from("brave-results.jsonl"),
            cache_before: WebSearchStorageArtifactState {
                bytes: 120,
                entries: 4,
                unique_entries: 2,
                duplicate_entries: 2,
            },
            cache_after: WebSearchStorageArtifactState {
                bytes: 120,
                entries: 4,
                unique_entries: 2,
                duplicate_entries: 2,
            },
            cache_projected_after: WebSearchStorageArtifactState {
                bytes: 80,
                entries: 2,
                unique_entries: 2,
                duplicate_entries: 0,
            },
            result_log_before: WebSearchStorageArtifactState {
                bytes: 90,
                entries: 3,
                unique_entries: 2,
                duplicate_entries: 1,
            },
            result_log_after: WebSearchStorageArtifactState {
                bytes: 90,
                entries: 3,
                unique_entries: 2,
                duplicate_entries: 1,
            },
            result_log_projected_after: WebSearchStorageArtifactState {
                bytes: 70,
                entries: 2,
                unique_entries: 2,
                duplicate_entries: 0,
            },
            skipped: false,
            dry_run: true,
        };

        let lines = web_storage_compaction_snapshot_readiness_lines(&report);

        assert!(lines.contains(&"web_storage_snapshot_cache_retained_bytes: 80".to_owned()));
        assert!(lines.contains(&"web_storage_snapshot_cache_removable_bytes: 40".to_owned()));
        assert!(lines.contains(&"web_storage_snapshot_cache_removable_duplicates: 2".to_owned()));
        assert!(lines.contains(&"web_storage_snapshot_result_log_retained_bytes: 70".to_owned()));
        assert!(lines.contains(&"web_storage_snapshot_result_log_removable_bytes: 20".to_owned()));
        assert!(
            lines.contains(&"web_storage_snapshot_result_log_removable_duplicates: 1".to_owned())
        );
        assert!(lines.contains(&"web_storage_snapshot_cleanup_scope: report-only dry_run=true cache_path=web-cache.jsonl result_log_path=brave-results.jsonl".to_owned()));
    }

    #[test]
    fn web_storage_compaction_snapshot_readiness_flags_zero_removal() {
        let report = WebSearchStorageCompactionReport {
            cache_path: PathBuf::from("web-cache.jsonl"),
            result_log_path: PathBuf::from("brave-results.jsonl"),
            cache_before: WebSearchStorageArtifactState {
                bytes: 120,
                entries: 4,
                unique_entries: 4,
                duplicate_entries: 0,
            },
            cache_after: WebSearchStorageArtifactState {
                bytes: 120,
                entries: 4,
                unique_entries: 4,
                duplicate_entries: 0,
            },
            cache_projected_after: WebSearchStorageArtifactState {
                bytes: 120,
                entries: 4,
                unique_entries: 4,
                duplicate_entries: 0,
            },
            result_log_before: WebSearchStorageArtifactState {
                bytes: 90,
                entries: 3,
                unique_entries: 3,
                duplicate_entries: 0,
            },
            result_log_after: WebSearchStorageArtifactState {
                bytes: 90,
                entries: 3,
                unique_entries: 3,
                duplicate_entries: 0,
            },
            result_log_projected_after: WebSearchStorageArtifactState {
                bytes: 90,
                entries: 3,
                unique_entries: 3,
                duplicate_entries: 0,
            },
            skipped: false,
            dry_run: true,
        };

        let lines = web_storage_compaction_snapshot_readiness_lines(&report);

        assert!(lines.contains(&"web_storage_snapshot_readiness: status=zero-removal projected_bytes=210 projected_removed_bytes=0 projected_duplicates=0".to_owned()));
        assert!(lines.contains(&"web_storage_snapshot_retained_bytes: 210".to_owned()));
        assert!(lines.contains(&"web_storage_snapshot_removable_bytes: 0".to_owned()));
        assert!(lines.contains(&"web_storage_snapshot_removable_duplicates: 0".to_owned()));
        assert!(lines.contains(&"web_storage_snapshot_zero_removal: true".to_owned()));
        assert!(!web_storage_compaction_apply_is_justified(&report, false));
    }

    #[test]
    fn web_storage_compaction_apply_guard_requires_projected_pressure() {
        let unchanged = WebSearchStorageCompactionReport {
            cache_path: PathBuf::from("web-cache.jsonl"),
            result_log_path: PathBuf::from("brave-results.jsonl"),
            cache_before: WebSearchStorageArtifactState {
                bytes: 120,
                entries: 2,
                unique_entries: 2,
                duplicate_entries: 0,
            },
            cache_after: WebSearchStorageArtifactState {
                bytes: 120,
                entries: 2,
                unique_entries: 2,
                duplicate_entries: 0,
            },
            cache_projected_after: WebSearchStorageArtifactState {
                bytes: 120,
                entries: 2,
                unique_entries: 2,
                duplicate_entries: 0,
            },
            result_log_before: WebSearchStorageArtifactState {
                bytes: 80,
                entries: 1,
                unique_entries: 1,
                duplicate_entries: 0,
            },
            result_log_after: WebSearchStorageArtifactState {
                bytes: 80,
                entries: 1,
                unique_entries: 1,
                duplicate_entries: 0,
            },
            result_log_projected_after: WebSearchStorageArtifactState {
                bytes: 80,
                entries: 1,
                unique_entries: 1,
                duplicate_entries: 0,
            },
            skipped: false,
            dry_run: true,
        };
        let duplicate_pressure = WebSearchStorageCompactionReport {
            cache_before: WebSearchStorageArtifactState {
                bytes: 120,
                entries: 2,
                unique_entries: 1,
                duplicate_entries: 1,
            },
            cache_projected_after: WebSearchStorageArtifactState {
                bytes: 120,
                entries: 1,
                unique_entries: 1,
                duplicate_entries: 0,
            },
            ..unchanged.clone()
        };
        let byte_pressure = WebSearchStorageCompactionReport {
            result_log_projected_after: WebSearchStorageArtifactState {
                bytes: 40,
                entries: 1,
                unique_entries: 1,
                duplicate_entries: 0,
            },
            ..unchanged.clone()
        };
        let skipped = WebSearchStorageCompactionReport {
            skipped: true,
            ..unchanged.clone()
        };

        assert!(!web_storage_compaction_apply_is_justified(
            &unchanged, false
        ));
        assert!(web_storage_compaction_apply_is_justified(&unchanged, true));
        assert!(web_storage_compaction_apply_is_justified(
            &duplicate_pressure,
            false
        ));
        assert!(web_storage_compaction_apply_is_justified(
            &byte_pressure,
            false
        ));
        assert!(!web_storage_compaction_apply_is_justified(&skipped, false));
    }

    #[test]
    fn web_storage_result_log_query_cap_lines_explain_compaction_scope() {
        assert!(web_storage_result_log_query_cap_lines(0).is_empty());

        let lines = web_storage_result_log_query_cap_lines(2);

        assert!(lines.contains(&"brave-results_entries_per_query_cap: 2".to_owned()));
        assert!(lines.contains(
            &"brave-results_entries_per_query_cap_note: applies only to compact-web-cache and dry-run projection; normal search append is unchanged".to_owned()
        ));
    }

    #[test]
    fn web_storage_retention_config_lines_explain_effective_caps() {
        let lines = web_storage_retention_config_lines(WebStorageRetentionConfig {
            cache_max_entries: 11,
            cache_max_bytes: 44,
            result_log_max_entries: 22,
            result_log_max_bytes: 33,
            result_log_max_entries_per_query: 3,
        });

        assert!(lines.contains(
            &"web_storage_retention_summary: web-cache_max_entries=11 web-cache_max_bytes=44 brave-results_max_entries=22 brave-results_max_bytes=33 brave-results_max_entries_per_query=3".to_owned()
        ));
        assert!(lines.contains(&"web_storage_cache_max_entries: 11".to_owned()));
        assert!(lines.contains(&"web_storage_cache_max_bytes: 44".to_owned()));
        assert!(lines.contains(&"web_storage_result_log_max_entries: 22".to_owned()));
        assert!(lines.contains(&"web_storage_result_log_max_bytes: 33".to_owned()));
        assert!(lines.contains(&"web_storage_result_log_max_entries_per_query: 3".to_owned()));
        assert!(lines.contains(
            &"web_storage_retention_note: normal search preserves durable web-cache and brave-results rows while enforcing global entry/byte caps; per-query caps apply during compact-web-cache dry-run/compaction when configured".to_owned()
        ));
    }
}
