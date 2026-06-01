use std::fs;
use std::path::Path;

use anyhow::{Context, Result, ensure};
use chrono::DateTime;
use rustc_hash::FxHashSet;
use serde::Deserialize;

use crate::crawler::domain_to_seed;
use crate::urlcanon::parse_seed;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RecrawlManifest {
    pub seeds: Vec<String>,
    pub sitemaps: Vec<String>,
    pub skipped_future: usize,
    pub total_entries: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RecrawlScheduleOptions {
    pub now: u64,
    pub include_future: bool,
}

impl RecrawlScheduleOptions {
    pub fn include_all() -> Self {
        Self {
            now: u64::MAX,
            include_future: true,
        }
    }

    pub fn due_at(now: u64) -> Self {
        Self {
            now,
            include_future: false,
        }
    }
}

#[derive(Debug, Deserialize)]
struct RecrawlEntry {
    url: Option<String>,
    domain: Option<String>,
    sitemap: Option<String>,
    priority: Option<i64>,
    recrawl_after: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RecrawlInputKind {
    Seed,
    Sitemap,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ScheduledInput {
    value: String,
    kind: RecrawlInputKind,
    priority: i64,
    order: usize,
    recrawl_after: Option<u64>,
}

pub fn load_recrawl_manifest(path: &Path) -> Result<RecrawlManifest> {
    load_recrawl_manifest_with_options(path, RecrawlScheduleOptions::include_all())
}

pub fn load_recrawl_manifest_with_options(
    path: &Path,
    options: RecrawlScheduleOptions,
) -> Result<RecrawlManifest> {
    let text = fs::read_to_string(path)
        .with_context(|| format!("read recrawl manifest {}", path.display()))?;
    parse_recrawl_manifest_with_options(&text, options)
        .with_context(|| format!("parse recrawl manifest {}", path.display()))
}

pub fn parse_recrawl_manifest(text: &str) -> Result<RecrawlManifest> {
    parse_recrawl_manifest_with_options(text, RecrawlScheduleOptions::include_all())
}

pub fn parse_recrawl_manifest_with_options(
    text: &str,
    options: RecrawlScheduleOptions,
) -> Result<RecrawlManifest> {
    let mut manifest = RecrawlManifest::default();
    let mut seen_seeds = FxHashSet::default();
    let mut seen_sitemaps = FxHashSet::default();
    let mut scheduled = Vec::new();

    for (line_no, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        let entry: RecrawlEntry = serde_json::from_str(line)
            .with_context(|| format!("decode recrawl manifest line {}", line_no + 1))?;

        manifest.total_entries += 1;
        let mut has_input = false;
        let priority = entry.priority.unwrap_or(0);
        let recrawl_after = entry
            .recrawl_after
            .as_deref()
            .map(|value| parse_recrawl_after(value, line_no + 1))
            .transpose()?;

        if let Some(url) = entry.url.as_deref() {
            let seed = parse_seed(url)
                .with_context(|| format!("invalid url on recrawl manifest line {}", line_no + 1))?
                .to_string();
            scheduled.push(ScheduledInput {
                value: seed,
                kind: RecrawlInputKind::Seed,
                priority,
                order: line_no,
                recrawl_after,
            });
            has_input = true;
        }

        if let Some(domain) = entry.domain.as_deref() {
            let seed = domain_to_seed(domain).with_context(|| {
                format!("invalid domain on recrawl manifest line {}", line_no + 1)
            })?;
            scheduled.push(ScheduledInput {
                value: seed,
                kind: RecrawlInputKind::Seed,
                priority,
                order: line_no,
                recrawl_after,
            });
            has_input = true;
        }

        if let Some(sitemap) = entry.sitemap.as_deref() {
            let sitemap = sitemap.trim();
            ensure!(
                !sitemap.is_empty(),
                "empty sitemap on recrawl manifest line {}",
                line_no + 1
            );
            scheduled.push(ScheduledInput {
                value: sitemap.to_owned(),
                kind: RecrawlInputKind::Sitemap,
                priority,
                order: line_no,
                recrawl_after,
            });
            has_input = true;
        }

        ensure!(
            has_input,
            "recrawl manifest line {} must include url, domain, or sitemap",
            line_no + 1
        );
    }

    scheduled.sort_by(|left, right| {
        right
            .priority
            .cmp(&left.priority)
            .then_with(|| left.order.cmp(&right.order))
            .then_with(|| left.value.cmp(&right.value))
    });

    for input in scheduled {
        if !options.include_future
            && input
                .recrawl_after
                .is_some_and(|recrawl_after| recrawl_after > options.now)
        {
            manifest.skipped_future += 1;
            continue;
        }

        match input.kind {
            RecrawlInputKind::Seed => {
                if seen_seeds.insert(input.value.clone()) {
                    manifest.seeds.push(input.value);
                }
            }
            RecrawlInputKind::Sitemap => {
                if seen_sitemaps.insert(input.value.clone()) {
                    manifest.sitemaps.push(input.value);
                }
            }
        }
    }

    Ok(manifest)
}

fn parse_recrawl_after(value: &str, line_no: usize) -> Result<u64> {
    let value = value.trim();
    ensure!(
        !value.is_empty(),
        "empty recrawl_after on recrawl manifest line {}",
        line_no
    );

    if let Ok(timestamp) = value.parse::<u64>() {
        return Ok(timestamp);
    }

    let parsed = DateTime::parse_from_rfc3339(value)
        .with_context(|| format!("invalid recrawl_after on recrawl manifest line {}", line_no))?;
    Ok(parsed.timestamp().max(0) as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_jsonl_manifest_and_dedupes_inputs() {
        let manifest = parse_recrawl_manifest(
            r#"
            # future fields are allowed and ignored
            {"url":"https://example.com/a#fragment","priority":10}
            {"url":"https://example.com/a"}
            {"domain":"example.org/path?x=1","recrawl_after":"2026-05-29T00:00:00Z"}
            {"sitemap":"https://example.net/sitemap.xml"}
            {"sitemap":"https://example.net/sitemap.xml"}
            "#,
        )
        .unwrap();

        assert_eq!(
            manifest.seeds,
            vec![
                "https://example.com/a".to_owned(),
                "https://example.org/".to_owned()
            ]
        );
        assert_eq!(
            manifest.sitemaps,
            vec!["https://example.net/sitemap.xml".to_owned()]
        );
        assert_eq!(manifest.skipped_future, 0);
        assert_eq!(manifest.total_entries, 5);
    }

    #[test]
    fn filters_future_recrawl_entries_and_sorts_due_work_by_priority() {
        let manifest = parse_recrawl_manifest_with_options(
            r#"
            {"url":"https://example.com/low","priority":1,"recrawl_after":"2000-01-01T00:00:00Z"}
            {"url":"https://example.com/future","priority":100,"recrawl_after":"2999-01-01T00:00:00Z"}
            {"url":"https://example.com/high","priority":50,"recrawl_after":"946684800"}
            {"sitemap":"https://example.com/sitemap.xml","priority":25}
            "#,
            RecrawlScheduleOptions::due_at(1_700_000_000),
        )
        .unwrap();

        assert_eq!(
            manifest.seeds,
            vec![
                "https://example.com/high".to_owned(),
                "https://example.com/low".to_owned()
            ]
        );
        assert_eq!(
            manifest.sitemaps,
            vec!["https://example.com/sitemap.xml".to_owned()]
        );
        assert_eq!(manifest.skipped_future, 1);
        assert_eq!(manifest.total_entries, 4);
    }

    #[test]
    fn include_all_keeps_future_recrawl_entries() {
        let manifest = parse_recrawl_manifest_with_options(
            r#"{"url":"https://example.com/future","recrawl_after":"2999-01-01T00:00:00Z"}"#,
            RecrawlScheduleOptions::include_all(),
        )
        .unwrap();

        assert_eq!(
            manifest.seeds,
            vec!["https://example.com/future".to_owned()]
        );
        assert_eq!(manifest.skipped_future, 0);
    }

    #[test]
    fn rejects_entries_without_importable_inputs() {
        let error = parse_recrawl_manifest(r#"{"priority":1}"#).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("must include url, domain, or sitemap")
        );
    }
}
