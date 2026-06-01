use std::collections::VecDeque;
use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result, ensure};
use futures::stream::{FuturesUnordered, StreamExt};
use reqwest::header::{CONTENT_TYPE, HeaderMap, HeaderValue, USER_AGENT};
use rustc_hash::{FxHashMap, FxHashSet};
use url::Url;

use crate::document::FieldedDocument;
use crate::extract::extract_html;
use crate::frontier::{FrontierStore, unix_now};
use crate::robots::{RobotsTxt, robots_origin_key};
use crate::urlcanon::{canonicalize_url, parse_seed, same_host};

#[derive(Debug, Clone)]
pub struct CrawlOptions {
    pub max_pages: usize,
    pub max_depth: usize,
    pub concurrency: usize,
    pub max_bytes: usize,
    pub ignore_robots: bool,
    pub boundary: CrawlBoundary,
    pub frontier_path: Option<PathBuf>,
    pub document_snapshot_path: Option<PathBuf>,
    pub max_fetching_per_host: usize,
    pub recrawl_seeds: Vec<String>,
}

impl Default for CrawlOptions {
    fn default() -> Self {
        Self {
            max_pages: 50_000,
            max_depth: 6,
            concurrency: 64,
            max_bytes: 4 * 1024 * 1024,
            ignore_robots: false,
            boundary: CrawlBoundary::SameHost,
            frontier_path: None,
            document_snapshot_path: None,
            max_fetching_per_host: 4,
            recrawl_seeds: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CrawlBoundary {
    SameHost,
    AnyDomain,
}

#[derive(Debug)]
struct FetchJob {
    url: Url,
    depth: usize,
}

#[derive(Debug)]
struct FetchedPage {
    requested: Url,
    final_url: Url,
    depth: usize,
    bytes: Vec<u8>,
}

pub async fn crawl(seed: &str, options: CrawlOptions) -> Result<Vec<FieldedDocument>> {
    crawl_many([seed], options).await
}

pub async fn crawl_many<I, S>(seeds: I, options: CrawlOptions) -> Result<Vec<FieldedDocument>>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let seeds = parse_seed_urls(seeds)?;
    let recrawl_seeds =
        parse_seed_urls_allow_empty(options.recrawl_seeds.iter().map(String::as_str))?;

    if let Some(frontier_path) = options.frontier_path.clone() {
        return crawl_with_frontier(seeds, recrawl_seeds, options, frontier_path).await;
    }

    crawl_in_memory(seeds, options).await
}

pub fn load_seed_file(path: &Path) -> Result<Vec<String>> {
    let text = fs::read_to_string(path)
        .with_context(|| format!("read seed URL file {}", path.display()))?;
    parse_seed_lines(&text)
}

pub fn load_domain_file(path: &Path) -> Result<Vec<String>> {
    let text = fs::read_to_string(path)
        .with_context(|| format!("read domain seed file {}", path.display()))?;
    parse_domain_lines(&text)
}

pub fn parse_seed_lines(text: &str) -> Result<Vec<String>> {
    let mut seeds = Vec::new();
    let mut seen = FxHashSet::default();

    for (line_no, line) in text.lines().enumerate() {
        let trimmed = line.split('#').next().unwrap_or("").trim();
        if trimmed.is_empty() {
            continue;
        }

        let url = parse_seed(trimmed)
            .with_context(|| format!("invalid seed URL on line {}", line_no + 1))?;
        let key = url.to_string();
        if seen.insert(key.clone()) {
            seeds.push(key);
        }
    }

    Ok(seeds)
}

pub fn parse_domain_lines(text: &str) -> Result<Vec<String>> {
    let mut seeds = Vec::new();
    let mut seen = FxHashSet::default();

    for (line_no, line) in text.lines().enumerate() {
        let trimmed = line.split('#').next().unwrap_or("").trim();
        if trimmed.is_empty() {
            continue;
        }

        let seed = domain_to_seed(trimmed)
            .with_context(|| format!("invalid domain seed on line {}", line_no + 1))?;
        if seen.insert(seed.clone()) {
            seeds.push(seed);
        }
    }

    Ok(seeds)
}

pub fn domain_to_seed(raw: &str) -> Result<String> {
    let raw = raw.trim();
    ensure!(!raw.is_empty(), "domain seed cannot be empty");

    let candidate = if raw.contains("://") {
        raw.to_owned()
    } else {
        format!("https://{raw}")
    };
    let mut url = Url::parse(&candidate).with_context(|| format!("invalid domain seed: {raw}"))?;
    ensure!(
        matches!(url.scheme(), "http" | "https"),
        "domain seed must use http or https"
    );
    ensure!(url.host_str().is_some(), "domain seed must include a host");

    let _ = url.set_username("");
    let _ = url.set_password(None);
    url.set_path("/");
    url.set_query(None);
    url.set_fragment(None);

    Ok(canonicalize_url(url).to_string())
}

fn parse_seed_urls<I, S>(seeds: I) -> Result<Vec<Url>>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let parsed = parse_seed_urls_allow_empty(seeds)?;
    ensure!(
        !parsed.is_empty(),
        "provide at least one seed URL, domain, seed file, domain file, or sitemap"
    );
    Ok(parsed)
}

fn parse_seed_urls_allow_empty<I, S>(seeds: I) -> Result<Vec<Url>>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut parsed = Vec::new();
    let mut seen = FxHashSet::default();

    for raw in seeds {
        let url = parse_seed(raw.as_ref())?;
        let key = url.to_string();
        if seen.insert(key) {
            parsed.push(url);
        }
    }

    Ok(parsed)
}

async fn crawl_in_memory(seeds: Vec<Url>, options: CrawlOptions) -> Result<Vec<FieldedDocument>> {
    let robots = fetch_seed_robots(&seeds, options.ignore_robots).await;
    let client = build_client()?;
    let mut seen = FxHashSet::default();
    let mut queued = VecDeque::new();
    let mut active = FuturesUnordered::new();
    let mut docs = Vec::with_capacity(options.max_pages.min(4096));

    for seed in &seeds {
        if seen.insert(seed.as_str().to_owned()) {
            queued.push_back(FetchJob {
                url: seed.clone(),
                depth: 0,
            });
        }
    }

    while docs.len() < options.max_pages && (!queued.is_empty() || !active.is_empty()) {
        while active.len() < options.concurrency && docs.len() + active.len() < options.max_pages {
            let Some(job) = queued.pop_front() else {
                break;
            };
            if !robots_allowed(&robots, &job.url) {
                continue;
            }
            let client = client.clone();
            let max_bytes = options.max_bytes;
            active.push(async move { fetch_page(client, job, max_bytes).await });
        }

        let Some(result) = active.next().await else {
            continue;
        };

        let fetched = match result {
            Ok(Some(fetched)) => fetched,
            Ok(None) => continue,
            Err(error) => {
                eprintln!("crawl fetch error: {error:#}");
                continue;
            }
        };

        let final_url = canonicalize_url(fetched.final_url);
        if !within_boundary(options.boundary, &seeds, &final_url) {
            continue;
        }

        let extracted = extract_html(&final_url, &fetched.bytes);
        if extracted.body.is_empty() {
            continue;
        }

        if fetched.depth < options.max_depth {
            for link in &extracted.outbound_links {
                let Ok(link) = Url::parse(link) else {
                    continue;
                };
                if !within_boundary(options.boundary, &seeds, &link) {
                    continue;
                }
                let key = link.as_str().to_owned();
                if seen.insert(key) {
                    queued.push_back(FetchJob {
                        url: link,
                        depth: fetched.depth + 1,
                    });
                }
            }
        }

        docs.push(FieldedDocument::from_extracted(
            &final_url,
            extracted,
            Some(blake3::hash(&fetched.bytes).to_hex().to_string()),
            Some(unix_now()),
        ));
        seen.insert(fetched.requested.as_str().to_owned());
    }

    Ok(docs)
}

async fn crawl_with_frontier(
    seeds: Vec<Url>,
    recrawl_seeds: Vec<Url>,
    options: CrawlOptions,
    frontier_path: PathBuf,
) -> Result<Vec<FieldedDocument>> {
    let robots = fetch_seed_robots(&seeds, options.ignore_robots).await;
    let client = build_client()?;
    let mut frontier = FrontierStore::open(frontier_path)?;
    let now = unix_now();
    frontier.reset_fetching(now);
    for seed in &seeds {
        frontier.discover(seed.clone(), 0, now);
    }
    for seed in &recrawl_seeds {
        frontier.schedule_recrawl(seed.clone(), 0, now);
    }
    frontier.save()?;

    let mut docs = if let Some(path) = options.document_snapshot_path.as_deref() {
        load_document_snapshot(path)?
    } else {
        Vec::new()
    };
    let mut doc_positions = docs
        .iter()
        .enumerate()
        .map(|(index, doc)| (doc.url.clone(), index))
        .collect::<FxHashMap<_, _>>();
    let mut active = FuturesUnordered::new();
    let mut fetched_this_run = 0usize;
    let run_fetch_budget = if recrawl_seeds.is_empty() {
        options.max_pages.saturating_sub(docs.len())
    } else {
        options.max_pages
    };

    while fetched_this_run < run_fetch_budget {
        while active.len() < options.concurrency
            && fetched_this_run + active.len() < run_fetch_budget
        {
            let Some(claim) = frontier.claim_next(unix_now(), options.max_fetching_per_host.max(1))
            else {
                break;
            };

            let url = match Url::parse(&claim.url) {
                Ok(url) => url,
                Err(error) => {
                    frontier.record_failed(&claim.url, error.to_string(), 0, unix_now());
                    continue;
                }
            };

            if !within_boundary(options.boundary, &seeds, &url) {
                frontier.record_failed(
                    &claim.url,
                    "outside crawl boundary".to_owned(),
                    0,
                    unix_now(),
                );
                continue;
            }

            if claim.depth as usize > options.max_depth {
                frontier.record_failed(&claim.url, "beyond max depth".to_owned(), 0, unix_now());
                continue;
            }

            if !robots_allowed(&robots, &url) {
                frontier.record_failed(
                    &claim.url,
                    "blocked by robots.txt".to_owned(),
                    0,
                    unix_now(),
                );
                continue;
            }

            let client = client.clone();
            let max_bytes = options.max_bytes;
            let attempts = claim.attempts;
            let claim_url = claim.url.clone();
            active.push(async move {
                let result = fetch_page(
                    client,
                    FetchJob {
                        url,
                        depth: claim.depth as usize,
                    },
                    max_bytes,
                )
                .await;
                (claim_url, attempts, result)
            });
        }

        frontier.save()?;

        let Some((claimed_url, attempts, result)) = active.next().await else {
            break;
        };
        let now = unix_now();

        let fetched = match result {
            Ok(Some(fetched)) => fetched,
            Ok(None) => {
                frontier.record_failed(&claimed_url, "skipped response".to_owned(), 0, now);
                frontier.save()?;
                continue;
            }
            Err(error) => {
                frontier.record_failed(
                    &claimed_url,
                    error.to_string(),
                    retry_after_seconds(attempts),
                    now,
                );
                frontier.save()?;
                continue;
            }
        };

        let final_url = canonicalize_url(fetched.final_url);
        if !within_boundary(options.boundary, &seeds, &final_url) {
            frontier.record_failed(
                &claimed_url,
                "redirected outside crawl boundary".to_owned(),
                0,
                now,
            );
            frontier.save()?;
            continue;
        }

        let content_hash = blake3::hash(&fetched.bytes).to_hex().to_string();
        let extracted = extract_html(&final_url, &fetched.bytes);
        fetched_this_run += 1;

        if fetched.depth < options.max_depth {
            for link in &extracted.outbound_links {
                let Ok(link) = Url::parse(link) else {
                    continue;
                };
                if within_boundary(options.boundary, &seeds, &link) {
                    frontier.discover(link, fetched.depth as u32 + 1, now);
                }
            }
        }

        if !extracted.body.is_empty() {
            let doc = FieldedDocument::from_extracted(
                &final_url,
                extracted,
                Some(content_hash.clone()),
                Some(now),
            );

            if let Some(path) = options.document_snapshot_path.as_deref() {
                append_document_snapshot(path, &doc)?;
            }

            if let Some(index) = doc_positions.get(&doc.url).copied() {
                docs[index] = doc;
            } else {
                doc_positions.insert(doc.url.clone(), docs.len());
                docs.push(doc);
            }
        }

        frontier.record_fetched(
            &claimed_url,
            200,
            Some(final_url.to_string()),
            Some(content_hash),
            now,
        );
        frontier.save()?;
    }

    Ok(docs)
}

fn within_boundary(boundary: CrawlBoundary, seeds: &[Url], candidate: &Url) -> bool {
    match boundary {
        CrawlBoundary::SameHost => seeds.iter().any(|seed| same_host(seed, candidate)),
        CrawlBoundary::AnyDomain => matches!(candidate.scheme(), "http" | "https"),
    }
}

type RobotsByOrigin = FxHashMap<String, RobotsTxt>;

async fn fetch_seed_robots(seeds: &[Url], ignore_robots: bool) -> RobotsByOrigin {
    let mut robots = RobotsByOrigin::default();

    for seed in seeds {
        let Some(origin) = robots_origin_key(seed) else {
            continue;
        };
        if robots.contains_key(&origin) {
            continue;
        }

        let policy = if ignore_robots {
            RobotsTxt::allow_all()
        } else {
            RobotsTxt::fetch(seed, 1024 * 1024)
                .await
                .unwrap_or_else(|_| RobotsTxt::allow_all())
        };
        robots.insert(origin, policy);
    }

    robots
}

fn robots_allowed(robots: &RobotsByOrigin, url: &Url) -> bool {
    robots_origin_key(url)
        .as_deref()
        .and_then(|origin| robots.get(origin))
        .is_none_or(|robots| robots.allowed(url.path()))
}

fn retry_after_seconds(attempts: u32) -> u64 {
    if attempts >= 4 {
        0
    } else {
        30u64.saturating_mul(1u64 << attempts.saturating_sub(1))
    }
}

fn load_document_snapshot(path: &Path) -> Result<Vec<FieldedDocument>> {
    if !path.exists() {
        return Ok(Vec::new());
    }

    let file = fs::File::open(path)
        .with_context(|| format!("open crawl document snapshot {}", path.display()))?;
    let reader = BufReader::new(file);
    let mut docs = Vec::new();
    let mut positions = FxHashMap::default();

    for (line_no, line) in reader.lines().enumerate() {
        let line = line.with_context(|| {
            format!(
                "read line {} from crawl document snapshot {}",
                line_no + 1,
                path.display()
            )
        })?;
        if line.trim().is_empty() {
            continue;
        }
        let doc: FieldedDocument = serde_json::from_str(&line).with_context(|| {
            format!(
                "decode line {} from crawl document snapshot {}",
                line_no + 1,
                path.display()
            )
        })?;
        if let Some(index) = positions.get(&doc.url).copied() {
            docs[index] = doc;
        } else {
            positions.insert(doc.url.clone(), docs.len());
            docs.push(doc);
        }
    }

    Ok(docs)
}

fn append_document_snapshot(path: &Path, doc: &FieldedDocument) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("create crawl snapshot parent {}", parent.display()))?;
    }

    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .with_context(|| format!("open crawl document snapshot {}", path.display()))?;
    serde_json::to_writer(&mut file, doc)?;
    file.write_all(b"\n")?;
    file.flush()?;
    Ok(())
}

fn build_client() -> Result<reqwest::Client> {
    let mut headers = HeaderMap::new();
    headers.insert(
        USER_AGENT,
        HeaderValue::from_static("brutal-search/0.1 static-text-crawler"),
    );

    Ok(reqwest::Client::builder()
        .default_headers(headers)
        .pool_max_idle_per_host(128)
        .tcp_nodelay(true)
        .timeout(Duration::from_secs(15))
        .redirect(reqwest::redirect::Policy::limited(5))
        .build()?)
}

async fn fetch_page(
    client: reqwest::Client,
    job: FetchJob,
    max_bytes: usize,
) -> Result<Option<FetchedPage>> {
    let response = client
        .get(job.url.clone())
        .send()
        .await
        .with_context(|| format!("fetch {}", job.url))?;
    if !response.status().is_success() {
        return Ok(None);
    }

    if let Some(content_type) = response.headers().get(CONTENT_TYPE) {
        let content_type = content_type.to_str().unwrap_or("");
        if !content_type.contains("text/html") && !content_type.contains("application/xhtml") {
            return Ok(None);
        }
    }

    let final_url = response.url().clone();
    let bytes = response.bytes().await?;
    if bytes.len() > max_bytes {
        return Ok(None);
    }

    Ok(Some(FetchedPage {
        requested: job.url,
        final_url,
        depth: job.depth,
        bytes: bytes.to_vec(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn document_snapshot_round_trips_and_dedupes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("crawl-docs.jsonl");
        let old_doc = FieldedDocument::from_plain_text(
            "https://example.com/a".to_owned(),
            "A".to_owned(),
            "alpha beta".to_owned(),
            None,
        );
        let new_doc = FieldedDocument::from_plain_text(
            "https://example.com/a".to_owned(),
            "A updated".to_owned(),
            "gamma delta".to_owned(),
            None,
        );

        append_document_snapshot(&path, &old_doc).unwrap();
        append_document_snapshot(&path, &new_doc).unwrap();

        let docs = load_document_snapshot(&path).unwrap();
        assert_eq!(docs.len(), 1);
        assert_eq!(docs[0].url, new_doc.url);
        assert_eq!(docs[0].body, new_doc.body);
    }

    #[test]
    fn seed_file_lines_ignore_comments_and_dedupe() {
        let seeds = parse_seed_lines(
            r#"
            # crawl seeds
            https://example.com/a#fragment
            https://example.com/a
            https://other.test/root
            "#,
        )
        .unwrap();

        assert_eq!(
            seeds,
            vec![
                "https://example.com/a".to_owned(),
                "https://other.test/root".to_owned()
            ]
        );
    }

    #[test]
    fn same_host_boundary_allows_any_seed_host() {
        let seeds =
            parse_seed_urls(["https://example.com/start", "https://other.test/root"]).unwrap();

        assert!(within_boundary(
            CrawlBoundary::SameHost,
            &seeds,
            &Url::parse("https://example.com/a").unwrap()
        ));
        assert!(within_boundary(
            CrawlBoundary::SameHost,
            &seeds,
            &Url::parse("https://other.test/b").unwrap()
        ));
        assert!(!within_boundary(
            CrawlBoundary::SameHost,
            &seeds,
            &Url::parse("https://elsewhere.test/c").unwrap()
        ));
    }

    #[test]
    fn domain_lines_normalize_to_root_url_seeds() {
        let seeds = parse_domain_lines(
            r#"
            # domain seeds
            example.com/path?ignored=1
            https://Example.com:443/other
            http://127.0.0.1:8080/start
            "#,
        )
        .unwrap();

        assert_eq!(
            seeds,
            vec![
                "https://example.com/".to_owned(),
                "http://127.0.0.1:8080/".to_owned()
            ]
        );
    }

    #[test]
    fn domain_seed_rejects_non_web_schemes() {
        let error = domain_to_seed("ftp://example.com").unwrap_err();
        assert!(error.to_string().contains("http or https"));
    }

    #[test]
    fn retry_delay_eventually_stops_retrying() {
        assert_eq!(retry_after_seconds(1), 30);
        assert_eq!(retry_after_seconds(2), 60);
        assert_eq!(retry_after_seconds(3), 120);
        assert_eq!(retry_after_seconds(4), 0);
    }
}
