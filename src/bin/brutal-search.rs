use std::collections::HashMap;
use std::env;
use std::io::{BufRead, Write};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use brutal_search::crawler::{
    CrawlBoundary, CrawlOptions, crawl_many, domain_to_seed, load_domain_file, load_seed_file,
};
use brutal_search::daemon::{default_socket_path, send_request};
use brutal_search::frontier::{FrontierStore, RecrawlPlanEntry, unix_now};
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
    DEFAULT_CACHE_MAX_ENTRIES, DEFAULT_RESULT_LOG_MAX_ENTRIES, WebSearchStorageArtifactState,
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
    "bench-status.json",
];
const WEB_STORAGE_COMPACT_SUGGEST_DUPLICATES: usize = 1;
const WEB_STORAGE_COMPACT_SUGGEST_MIN_ENTRIES: usize = 1024;
const DEFAULT_WEB_STORAGE_STALE_SECS: u64 = 30 * 24 * 60 * 60;

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
            min_entries,
        } => {
            let report = compact_web_search_storage_from_env(
                &index,
                WebSearchStorageCompactionOptions {
                    dry_run,
                    min_entries,
                },
            )?;
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
    crawl_frontier_bytes: u64,
    crawl_snapshot_bytes: u64,
    crawl_snapshot_entries: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct WebStorageArtifactStats {
    name: &'static str,
    bytes: u64,
    entries: usize,
    unique_entries: usize,
    duplicate_entries: usize,
    query_count: usize,
    max_entries_per_query: usize,
    oldest_fetched_at_unix: Option<u64>,
    newest_fetched_at_unix: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct WebStoragePressureSummary {
    artifact_count: usize,
    bytes: u64,
    entries: usize,
    duplicate_entries: usize,
    max_entries_per_query: usize,
    stale_artifacts: usize,
    suggested_dry_runs: usize,
}

fn collect_index_storage_stats(index: &Path) -> Result<IndexStorageStats> {
    let mut stats = IndexStorageStats {
        total_bytes: 0,
        artifacts: Vec::new(),
        web_artifacts: Vec::new(),
        crawl_frontier_bytes: 0,
        crawl_snapshot_bytes: 0,
        crawl_snapshot_entries: 0,
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
            }
            "crawl-docs.jsonl" => {
                stats.crawl_snapshot_bytes = bytes;
                stats.crawl_snapshot_entries = count_jsonl_entries(&path)?;
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
    for line in web_storage_retention_config_lines(web_storage_retention_config()) {
        println!("{line}");
    }
    let now = unix_now();
    let stale_secs = web_storage_stale_threshold_secs();
    for line in web_storage_pressure_summary_lines(&stats.web_artifacts, now, stale_secs) {
        println!("{line}");
    }
    for artifact in stats.web_artifacts {
        println!(
            "web_storage_artifact_entries: {} {}",
            artifact.name, artifact.entries
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

    vec![
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
    ]
}

fn count_jsonl_entries(path: &Path) -> Result<usize> {
    let file = std::fs::File::open(path)
        .with_context(|| format!("open index storage jsonl artifact {}", path.display()))?;
    let reader = std::io::BufReader::new(file);
    let mut entries = 0usize;
    for line in reader.lines() {
        let line =
            line.with_context(|| format!("read index storage jsonl artifact {}", path.display()))?;
        if !line.trim().is_empty() {
            entries = entries.saturating_add(1);
        }
    }
    Ok(entries)
}

fn web_storage_pressure_summary_lines(
    artifacts: &[WebStorageArtifactStats],
    now: u64,
    stale_secs: u64,
) -> Vec<String> {
    let summary = web_storage_pressure_summary(artifacts, now, stale_secs);
    let mut lines = vec![
        format!(
            "web_storage_pressure_summary: artifacts={} bytes={} entries={} duplicates={} stale_artifacts={} suggested_dry_runs={}",
            summary.artifact_count,
            summary.bytes,
            summary.entries,
            summary.duplicate_entries,
            summary.stale_artifacts,
            summary.suggested_dry_runs
        ),
        format!("web_storage_pressure_artifacts: {}", summary.artifact_count),
        format!("web_storage_pressure_bytes: {}", summary.bytes),
        format!("web_storage_pressure_entries: {}", summary.entries),
        format!(
            "web_storage_pressure_duplicate_entries: {}",
            summary.duplicate_entries
        ),
        format!(
            "web_storage_pressure_max_entries_per_query: {}",
            summary.max_entries_per_query
        ),
        format!(
            "web_storage_pressure_stale_artifacts: {}",
            summary.stale_artifacts
        ),
    ];
    if summary.suggested_dry_runs > 0 {
        lines.push(format!(
            "web_storage_pressure_suggestion: brutal-search compact-web-cache --dry-run --min-entries {}",
            summary.entries.max(1)
        ));
    }
    lines
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
        duplicate_entries: artifacts
            .iter()
            .map(|artifact| artifact.duplicate_entries)
            .sum(),
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
    ]
}

fn web_storage_result_log_query_cap() -> usize {
    web_storage_env_usize("BRUTAL_WEB_RESULT_LOG_MAX_ENTRIES_PER_QUERY").unwrap_or(0)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct WebStorageRetentionConfig {
    cache_max_entries: usize,
    result_log_max_entries: usize,
    result_log_max_entries_per_query: usize,
}

fn web_storage_retention_config() -> WebStorageRetentionConfig {
    WebStorageRetentionConfig {
        cache_max_entries: web_storage_env_usize("BRUTAL_WEB_CACHE_MAX_ENTRIES")
            .unwrap_or(DEFAULT_CACHE_MAX_ENTRIES),
        result_log_max_entries: web_storage_env_usize("BRUTAL_WEB_RESULT_LOG_MAX_ENTRIES")
            .unwrap_or(DEFAULT_RESULT_LOG_MAX_ENTRIES),
        result_log_max_entries_per_query: web_storage_result_log_query_cap(),
    }
}

fn web_storage_env_usize(name: &str) -> Option<usize> {
    env::var(name)
        .ok()
        .and_then(|value| value.trim().parse::<usize>().ok())
}

fn web_storage_retention_config_lines(config: WebStorageRetentionConfig) -> Vec<String> {
    vec![
        format!(
            "web_storage_retention_summary: web-cache_max_entries={} brave-results_max_entries={} brave-results_max_entries_per_query={}",
            config.cache_max_entries,
            config.result_log_max_entries,
            config.result_log_max_entries_per_query
        ),
        format!(
            "web_storage_cache_max_entries: {}",
            config.cache_max_entries
        ),
        format!(
            "web_storage_result_log_max_entries: {}",
            config.result_log_max_entries
        ),
        format!(
            "web_storage_result_log_max_entries_per_query: {}",
            config.result_log_max_entries_per_query
        ),
        "web_storage_retention_note: normal search preserves durable web-cache and brave-results rows while enforcing global caps; per-query caps apply during compact-web-cache dry-run/compaction when configured".to_owned(),
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
        unique_entries: 0,
        duplicate_entries: 0,
        query_count: 0,
        max_entries_per_query: 0,
        oldest_fetched_at_unix: None,
        newest_fetched_at_unix: None,
    };
    let mut unique_keys = std::collections::HashSet::new();
    let mut query_counts = HashMap::new();

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
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&line) else {
            continue;
        };
        if let Some(key) = web_storage_unique_key(name, &value) {
            if unique_keys.insert(key) {
                stats.unique_entries += 1;
            } else {
                stats.duplicate_entries += 1;
            }
        }
        if let Some(query) = value
            .get("normalized_query")
            .and_then(|value| value.as_str())
            .filter(|query| !query.is_empty())
        {
            *query_counts.entry(query.to_owned()).or_insert(0usize) += 1;
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
    stats.query_count = query_counts.len();
    stats.max_entries_per_query = query_counts.into_values().max().unwrap_or(0);

    Ok(stats)
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
            let rank = value
                .get("rank")
                .and_then(|value| value.as_u64())
                .unwrap_or_default();
            let url = value
                .get("url")
                .and_then(|value| value.as_str())
                .filter(|url| !url.is_empty())?;
            Some(format!("{query}\t{provider}\t{rank}\t{url}"))
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
        let mut file = std::fs::File::create(path)?;
        write_recrawl_plan_entries(&mut file, plan)?;
    } else {
        let stdout = std::io::stdout();
        let mut stdout = stdout.lock();
        write_recrawl_plan_entries(&mut stdout, plan)?;
    }

    Ok(())
}

fn write_recrawl_plan_entries<W: Write>(writer: &mut W, plan: &[RecrawlPlanEntry]) -> Result<()> {
    for entry in plan {
        serde_json::to_writer(
            &mut *writer,
            &serde_json::json!({
                "url": entry.url.as_str(),
                "priority": entry.priority,
                "recrawl_after": entry.recrawl_after.to_string(),
                "last_fetched_at": entry.last_fetched_at,
                "age_secs": entry.age_secs,
            }),
        )?;
        (*writer).write_all(b"\n")?;
    }
    (*writer).flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn index_storage_stats_sum_known_artifacts() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("manifest.json"), b"12345").unwrap();
        std::fs::write(
            dir.path().join("web-cache.jsonl"),
            b"{\"fetched_at_unix\":100}\n{\"fetched_at_unix\":120}\n",
        )
        .unwrap();
        std::fs::write(dir.path().join("frontier.bin"), b"abcd").unwrap();
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

        assert_eq!(stats.total_bytes, 146);
        assert_eq!(
            stats.artifacts,
            vec![
                IndexStorageArtifact {
                    name: "manifest.json",
                    bytes: 5
                },
                IndexStorageArtifact {
                    name: "frontier.bin",
                    bytes: 4
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
        assert_eq!(stats.crawl_frontier_bytes, 4);
        assert_eq!(stats.crawl_snapshot_bytes, 65);
        assert_eq!(stats.crawl_snapshot_entries, 2);
        assert_eq!(
            stats.web_artifacts,
            vec![
                WebStorageArtifactStats {
                    name: "web-cache.jsonl",
                    bytes: 48,
                    entries: 2,
                    unique_entries: 0,
                    duplicate_entries: 0,
                    query_count: 0,
                    max_entries_per_query: 0,
                    oldest_fetched_at_unix: Some(100),
                    newest_fetched_at_unix: Some(120),
                },
                WebStorageArtifactStats {
                    name: "brave-results.jsonl",
                    bytes: 24,
                    entries: 1,
                    unique_entries: 0,
                    duplicate_entries: 0,
                    query_count: 0,
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
        assert_eq!(stats.crawl_frontier_bytes, 0);
        assert_eq!(stats.crawl_snapshot_bytes, 0);
        assert_eq!(stats.crawl_snapshot_entries, 0);
    }

    #[test]
    fn crawl_storage_pressure_summary_reports_frontier_and_snapshots() {
        let stats = IndexStorageStats {
            total_bytes: 170,
            artifacts: Vec::new(),
            web_artifacts: Vec::new(),
            crawl_frontier_bytes: 50,
            crawl_snapshot_bytes: 120,
            crawl_snapshot_entries: 3,
        };

        let lines = crawl_storage_pressure_summary_lines(&stats);

        assert!(lines.contains(
            &"crawl_storage_pressure_summary: retained_bytes=170 frontier_bytes=50 snapshot_bytes=120 snapshot_entries=3".to_owned()
        ));
        assert!(lines.contains(&"crawl_storage_retained_bytes: 170".to_owned()));
        assert!(lines.contains(&"crawl_storage_frontier_bytes: 50".to_owned()));
        assert!(lines.contains(&"crawl_storage_snapshot_bytes: 120".to_owned()));
        assert!(lines.contains(&"crawl_storage_snapshot_entries: 3".to_owned()));
    }

    #[test]
    fn web_storage_stats_counts_unique_and_duplicate_entries() {
        let dir = tempfile::tempdir().unwrap();
        let cache_path = dir.path().join("web-cache.jsonl");
        std::fs::write(
            &cache_path,
            b"{\"normalized_query\":\"one\",\"fetched_at_unix\":100}\n{\"normalized_query\":\"one\",\"fetched_at_unix\":120}\n{\"normalized_query\":\"two\",\"fetched_at_unix\":130}\n",
        )
        .unwrap();
        let result_log_path = dir.path().join("brave-results.jsonl");
        std::fs::write(
            &result_log_path,
            b"{\"normalized_query\":\"one\",\"provider\":\"brave\",\"rank\":1,\"url\":\"https://example.com/a\",\"fetched_at_unix\":100}\n{\"normalized_query\":\"one\",\"provider\":\"brave\",\"rank\":1,\"url\":\"https://example.com/a\",\"fetched_at_unix\":120}\n",
        )
        .unwrap();

        let cache_stats =
            collect_web_storage_artifact_stats("web-cache.jsonl", &cache_path).unwrap();
        let log_stats =
            collect_web_storage_artifact_stats("brave-results.jsonl", &result_log_path).unwrap();

        assert_eq!(cache_stats.entries, 3);
        assert_eq!(cache_stats.bytes, 147);
        assert_eq!(cache_stats.unique_entries, 2);
        assert_eq!(cache_stats.duplicate_entries, 1);
        assert_eq!(cache_stats.query_count, 2);
        assert_eq!(cache_stats.max_entries_per_query, 2);
        assert_eq!(log_stats.entries, 2);
        assert_eq!(log_stats.bytes, 214);
        assert_eq!(log_stats.unique_entries, 1);
        assert_eq!(log_stats.duplicate_entries, 1);
        assert_eq!(log_stats.query_count, 1);
        assert_eq!(log_stats.max_entries_per_query, 2);
    }

    #[test]
    fn web_storage_compaction_suggestion_points_to_dry_run() {
        let duplicate_artifact = WebStorageArtifactStats {
            name: "web-cache.jsonl",
            bytes: 120,
            entries: 3,
            unique_entries: 2,
            duplicate_entries: 1,
            query_count: 2,
            max_entries_per_query: 2,
            oldest_fetched_at_unix: Some(100),
            newest_fetched_at_unix: Some(120),
        };
        let large_artifact = WebStorageArtifactStats {
            name: "brave-results.jsonl",
            bytes: 4096,
            entries: WEB_STORAGE_COMPACT_SUGGEST_MIN_ENTRIES,
            unique_entries: WEB_STORAGE_COMPACT_SUGGEST_MIN_ENTRIES,
            duplicate_entries: 0,
            query_count: WEB_STORAGE_COMPACT_SUGGEST_MIN_ENTRIES,
            max_entries_per_query: 1,
            oldest_fetched_at_unix: Some(100),
            newest_fetched_at_unix: Some(120),
        };
        let small_clean_artifact = WebStorageArtifactStats {
            name: "web-cache.jsonl",
            bytes: 80,
            entries: 2,
            unique_entries: 2,
            duplicate_entries: 0,
            query_count: 2,
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
            unique_entries: 2,
            duplicate_entries: 0,
            query_count: 2,
            max_entries_per_query: 1,
            oldest_fetched_at_unix: Some(100),
            newest_fetched_at_unix: Some(120),
        };
        let fresh_artifact = WebStorageArtifactStats {
            name: "brave-results.jsonl",
            bytes: 90,
            entries: 2,
            unique_entries: 2,
            duplicate_entries: 0,
            query_count: 2,
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
                    unique_entries: 2,
                    duplicate_entries: 1,
                    query_count: 2,
                    max_entries_per_query: 2,
                    oldest_fetched_at_unix: Some(100),
                    newest_fetched_at_unix: Some(120),
                },
                WebStorageArtifactStats {
                    name: "brave-results.jsonl",
                    bytes: 90,
                    entries: 2,
                    unique_entries: 2,
                    duplicate_entries: 0,
                    query_count: 2,
                    max_entries_per_query: 1,
                    oldest_fetched_at_unix: Some(190),
                    newest_fetched_at_unix: Some(200),
                },
            ],
            200,
            60,
        );

        assert!(lines.contains(
            &"web_storage_pressure_summary: artifacts=2 bytes=210 entries=5 duplicates=1 stale_artifacts=1 suggested_dry_runs=1".to_owned()
        ));
        assert!(lines.contains(&"web_storage_pressure_bytes: 210".to_owned()));
        assert!(lines.contains(&"web_storage_pressure_duplicate_entries: 1".to_owned()));
        assert!(lines.contains(&"web_storage_pressure_max_entries_per_query: 2".to_owned()));
        assert!(lines.contains(
            &"web_storage_pressure_suggestion: brutal-search compact-web-cache --dry-run --min-entries 5".to_owned()
        ));
    }

    #[test]
    fn web_storage_compaction_artifact_lines_include_projected_savings() {
        let lines = web_storage_compaction_artifact_lines(
            "web-cache",
            Path::new("/tmp/web-cache.jsonl"),
            WebSearchStorageArtifactState {
                bytes: 120,
                entries: 6,
            },
            WebSearchStorageArtifactState {
                bytes: 120,
                entries: 6,
            },
            WebSearchStorageArtifactState {
                bytes: 80,
                entries: 4,
            },
        );

        assert!(lines.contains(&"web-cache_bytes_projected_after: 80".to_owned()));
        assert!(lines.contains(&"web-cache_bytes_projected_retained: 80".to_owned()));
        assert!(lines.contains(&"web-cache_bytes_projected_removed: 40".to_owned()));
        assert!(lines.contains(&"web-cache_entries_projected_after: 4".to_owned()));
        assert!(lines.contains(&"web-cache_entries_projected_retained: 4".to_owned()));
        assert!(lines.contains(&"web-cache_entries_projected_removed: 2".to_owned()));
        assert!(lines.contains(&"web-cache_bytes_removed: 0".to_owned()));
        assert!(lines.contains(&"web-cache_entries_removed: 0".to_owned()));
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
            result_log_max_entries: 22,
            result_log_max_entries_per_query: 3,
        });

        assert!(lines.contains(
            &"web_storage_retention_summary: web-cache_max_entries=11 brave-results_max_entries=22 brave-results_max_entries_per_query=3".to_owned()
        ));
        assert!(lines.contains(&"web_storage_cache_max_entries: 11".to_owned()));
        assert!(lines.contains(&"web_storage_result_log_max_entries: 22".to_owned()));
        assert!(lines.contains(&"web_storage_result_log_max_entries_per_query: 3".to_owned()));
        assert!(lines.contains(
            &"web_storage_retention_note: normal search preserves durable web-cache and brave-results rows while enforcing global caps; per-query caps apply during compact-web-cache dry-run/compaction when configured".to_owned()
        ));
    }
}
