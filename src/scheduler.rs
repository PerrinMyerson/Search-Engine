use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result};
use serde::Serialize;

use crate::BuildStats;
use crate::crawler::{CrawlBoundary, CrawlOptions, crawl_many};
use crate::frontier::{FrontierStore, RecrawlPlanEntry, unix_now};
use crate::index::{IndexBuildOptions, build_from_fielded_documents};

#[derive(Debug, Clone)]
pub struct RecrawlSchedulerOptions {
    pub index: PathBuf,
    pub interval_secs: u64,
    pub batch_size: usize,
    pub poll_secs: u64,
    pub max_rounds: Option<usize>,
    pub crawl: RecrawlCrawlOptions,
}

#[derive(Debug, Clone, Copy)]
pub struct RecrawlCrawlOptions {
    pub max_depth: usize,
    pub concurrency: usize,
    pub max_bytes: usize,
    pub ignore_robots: bool,
    pub boundary: CrawlBoundary,
    pub max_fetching_per_host: usize,
}

impl Default for RecrawlCrawlOptions {
    fn default() -> Self {
        Self {
            max_depth: 0,
            concurrency: 64,
            max_bytes: 4 * 1024 * 1024,
            ignore_robots: false,
            boundary: CrawlBoundary::SameHost,
            max_fetching_per_host: 4,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct RecrawlRoundReport {
    pub round: usize,
    pub due_count: usize,
    pub changed_count: usize,
    pub unchanged_count: usize,
    pub missing_count: usize,
    pub indexed_docs: Option<u32>,
    pub indexed_terms: Option<u32>,
    pub corpus_hash: Option<String>,
}

impl RecrawlRoundReport {
    fn idle(round: usize) -> Self {
        Self {
            round,
            due_count: 0,
            changed_count: 0,
            unchanged_count: 0,
            missing_count: 0,
            indexed_docs: None,
            indexed_terms: None,
            corpus_hash: None,
        }
    }

    fn indexed(
        round: usize,
        due_count: usize,
        changes: RecrawlChangeCounts,
        stats: BuildStats,
    ) -> Self {
        Self {
            round,
            due_count,
            changed_count: changes.changed,
            unchanged_count: changes.unchanged,
            missing_count: changes.missing,
            indexed_docs: Some(stats.doc_count),
            indexed_terms: Some(stats.term_count),
            corpus_hash: Some(stats.corpus_hash),
        }
    }
}

pub async fn run_recrawl_scheduler(
    options: RecrawlSchedulerOptions,
) -> Result<Vec<RecrawlRoundReport>> {
    if options.max_rounds == Some(0) {
        return Ok(Vec::new());
    }

    let mut reports = Vec::new();
    let mut round = 0usize;

    loop {
        round += 1;
        let report = run_recrawl_round(&options, round).await?;
        reports.push(report);

        if options
            .max_rounds
            .is_some_and(|max_rounds| reports.len() >= max_rounds)
        {
            break;
        }

        if options.poll_secs > 0 {
            tokio::time::sleep(Duration::from_secs(options.poll_secs)).await;
        }
    }

    Ok(reports)
}

pub async fn run_recrawl_round(
    options: &RecrawlSchedulerOptions,
    round: usize,
) -> Result<RecrawlRoundReport> {
    let frontier_path = options.index.join("frontier.bin");
    let frontier = FrontierStore::open(&frontier_path)
        .with_context(|| format!("open crawl frontier {}", frontier_path.display()))?;
    let plan = frontier.recrawl_plan(unix_now(), options.interval_secs, options.batch_size);
    if plan.is_empty() {
        return Ok(RecrawlRoundReport::idle(round));
    }

    let seeds = recrawl_urls(&plan);
    let previous_hashes = plan_hashes(&frontier, &plan);
    let docs = crawl_many(
        seeds.iter().map(String::as_str),
        CrawlOptions {
            max_pages: plan.len(),
            max_depth: options.crawl.max_depth,
            concurrency: options.crawl.concurrency,
            max_bytes: options.crawl.max_bytes,
            ignore_robots: options.crawl.ignore_robots,
            boundary: options.crawl.boundary,
            frontier_path: Some(frontier_path),
            document_snapshot_path: Some(options.index.join("crawl-docs.jsonl")),
            max_fetching_per_host: options.crawl.max_fetching_per_host,
            recrawl_seeds: seeds.clone(),
        },
    )
    .await?;
    let frontier = FrontierStore::open(options.index.join("frontier.bin"))?;
    let changes = count_recrawl_changes(&frontier, &previous_hashes);
    let stats = build_from_fielded_documents(docs, &options.index, IndexBuildOptions::default())?;

    Ok(RecrawlRoundReport::indexed(
        round,
        plan.len(),
        changes,
        stats,
    ))
}

fn recrawl_urls(plan: &[RecrawlPlanEntry]) -> Vec<String> {
    plan.iter().map(|entry| entry.url.clone()).collect()
}

fn plan_hashes(
    frontier: &FrontierStore,
    plan: &[RecrawlPlanEntry],
) -> Vec<(String, Option<String>)> {
    plan.iter()
        .map(|entry| {
            (
                entry.url.clone(),
                frontier
                    .get(&entry.url)
                    .and_then(|record| record.content_hash.clone()),
            )
        })
        .collect()
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct RecrawlChangeCounts {
    changed: usize,
    unchanged: usize,
    missing: usize,
}

fn count_recrawl_changes(
    frontier: &FrontierStore,
    previous_hashes: &[(String, Option<String>)],
) -> RecrawlChangeCounts {
    let mut counts = RecrawlChangeCounts::default();

    for (url, previous_hash) in previous_hashes {
        let Some(record) = frontier.get(url) else {
            counts.missing += 1;
            continue;
        };
        let Some(current_hash) = &record.content_hash else {
            counts.missing += 1;
            continue;
        };
        if previous_hash.as_ref() == Some(current_hash) {
            counts.unchanged += 1;
        } else {
            counts.changed += 1;
        }
    }

    counts
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use url::Url;

    #[test]
    fn recrawl_urls_preserve_plan_order() {
        let plan = vec![
            RecrawlPlanEntry {
                url: "https://example.com/old".to_owned(),
                priority: 100,
                recrawl_after: 10,
                last_fetched_at: 1,
                age_secs: 100,
            },
            RecrawlPlanEntry {
                url: "https://example.com/newer".to_owned(),
                priority: 50,
                recrawl_after: 20,
                last_fetched_at: 2,
                age_secs: 50,
            },
        ];

        assert_eq!(
            recrawl_urls(&plan),
            vec![
                "https://example.com/old".to_owned(),
                "https://example.com/newer".to_owned()
            ]
        );
    }

    #[test]
    fn count_recrawl_changes_compares_previous_hashes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("frontier.bin");
        let mut frontier = FrontierStore::open(path).unwrap();

        frontier.discover(Url::parse("https://example.com/changed").unwrap(), 0, 1);
        frontier.discover(Url::parse("https://example.com/same").unwrap(), 0, 1);
        let changed = frontier.claim_next(2, 10).unwrap();
        frontier.record_fetched(&changed.url, 200, None, Some("new".to_owned()), 3);
        let same = frontier.claim_next(4, 10).unwrap();
        frontier.record_fetched(&same.url, 200, None, Some("same".to_owned()), 5);

        let counts = count_recrawl_changes(
            &frontier,
            &[
                (
                    "https://example.com/changed".to_owned(),
                    Some("old".to_owned()),
                ),
                (
                    "https://example.com/same".to_owned(),
                    Some("same".to_owned()),
                ),
                (
                    "https://example.com/missing".to_owned(),
                    Some("gone".to_owned()),
                ),
            ],
        );

        assert_eq!(
            counts,
            RecrawlChangeCounts {
                changed: 1,
                unchanged: 1,
                missing: 1,
            }
        );
    }

    #[tokio::test]
    async fn max_rounds_zero_returns_without_work() {
        let reports = run_recrawl_scheduler(RecrawlSchedulerOptions {
            index: PathBuf::from("/no/such/index"),
            interval_secs: 0,
            batch_size: 1,
            poll_secs: 0,
            max_rounds: Some(0),
            crawl: RecrawlCrawlOptions::default(),
        })
        .await
        .unwrap();

        assert!(reports.is_empty());
    }
}
