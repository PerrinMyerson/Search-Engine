use std::collections::{HashMap, HashSet};
use std::env;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use tokio::sync::{Mutex, mpsc};
use url::Url;

use crate::crawler::{CrawlBoundary, CrawlOptions, crawl_many};
use crate::document::FieldedDocument;
use crate::index::{IndexBuildOptions, PreloadMode, SearchIndex, build_from_fielded_documents};
use crate::query::{SearchOptions, SearchResult};
use crate::urlcanon::canonicalize_url;
use crate::web_search::{WebSearchResult, WebSearchService};

const DEFAULT_BACKGROUND_TOP_N: usize = 5;
const DEFAULT_BACKGROUND_MAX_DEPTH: usize = 0;
const DEFAULT_BACKGROUND_CONCURRENCY: usize = 4;
const DEFAULT_BACKGROUND_MAX_BYTES: usize = 4 * 1024 * 1024;
const DEFAULT_BACKGROUND_MAX_FETCHING_PER_HOST: usize = 2;
const DEFAULT_BACKGROUND_QUEUE_CAPACITY: usize = 64;
const DEFAULT_BACKGROUND_DEDUPE_SECS: u64 = 60 * 60;

pub type SearchProviderFuture<'a> =
    Pin<Box<dyn Future<Output = Result<ProviderSearchResponse>> + Send + 'a>>;

pub trait SearchProvider: Send + Sync {
    fn name(&self) -> &'static str;
    fn search<'a>(&'a self, query: &'a str, limit: usize) -> SearchProviderFuture<'a>;
}

#[derive(Debug, Clone)]
pub struct ProviderSearchResponse {
    pub provider: &'static str,
    pub cache_hit: bool,
    pub fetched: bool,
    pub results: Vec<ProviderSearchResult>,
}

#[derive(Debug, Clone)]
pub enum ProviderSearchResult {
    Local(SearchResult),
    Web(WebSearchResult),
}

#[derive(Debug, Clone)]
pub struct LocalSearchProvider {
    index_dir: PathBuf,
    preload: PreloadMode,
    index: Arc<RwLock<Arc<SearchIndex>>>,
}

impl LocalSearchProvider {
    pub fn open(index_dir: PathBuf, preload: PreloadMode) -> Result<Self> {
        let index = Arc::new(SearchIndex::open(&index_dir, preload)?);
        Ok(Self {
            index_dir,
            preload,
            index: Arc::new(RwLock::new(index)),
        })
    }

    pub fn root(&self) -> &Path {
        &self.index_dir
    }

    pub fn current(&self) -> Arc<SearchIndex> {
        self.index
            .read()
            .expect("local search provider lock poisoned")
            .clone()
    }

    pub fn reload(&self) -> Result<Arc<SearchIndex>> {
        let reloaded = Arc::new(SearchIndex::open(&self.index_dir, self.preload)?);
        *self
            .index
            .write()
            .expect("local search provider lock poisoned") = Arc::clone(&reloaded);
        Ok(reloaded)
    }
}

impl SearchProvider for LocalSearchProvider {
    fn name(&self) -> &'static str {
        "local"
    }

    fn search<'a>(&'a self, query: &'a str, limit: usize) -> SearchProviderFuture<'a> {
        Box::pin(async move {
            let index = self.current();
            let results = index
                .search(query, SearchOptions { limit })?
                .into_iter()
                .map(ProviderSearchResult::Local)
                .collect();
            Ok(ProviderSearchResponse {
                provider: self.name(),
                cache_hit: false,
                fetched: false,
                results,
            })
        })
    }
}

impl SearchProvider for WebSearchService {
    fn name(&self) -> &'static str {
        self.provider_name()
    }

    fn search<'a>(&'a self, query: &'a str, limit: usize) -> SearchProviderFuture<'a> {
        Box::pin(async move {
            let lookup = WebSearchService::search(self, query, limit).await?;
            Ok(ProviderSearchResponse {
                provider: lookup.provider,
                cache_hit: lookup.cache_hit,
                fetched: lookup.fetched,
                results: lookup
                    .results
                    .into_iter()
                    .map(ProviderSearchResult::Web)
                    .collect(),
            })
        })
    }
}

#[derive(Debug, Clone)]
pub struct BackgroundIndexer {
    tx: mpsc::Sender<Vec<String>>,
    recent: Arc<Mutex<HashMap<String, Instant>>>,
    dedupe_secs: u64,
    top_n: usize,
}

#[derive(Debug, Clone)]
struct BackgroundIndexConfig {
    index_dir: PathBuf,
    top_n: usize,
    max_pages: usize,
    max_depth: usize,
    concurrency: usize,
    max_bytes: usize,
    ignore_robots: bool,
    max_fetching_per_host: usize,
}

impl BackgroundIndexer {
    pub fn from_env(index_dir: PathBuf, local: LocalSearchProvider) -> Option<Self> {
        if env_flag_disabled("BRUTAL_BACKGROUND_CRAWL") {
            return None;
        }

        let top_n = env_usize("BRUTAL_BACKGROUND_CRAWL_TOP_N").unwrap_or(DEFAULT_BACKGROUND_TOP_N);
        if top_n == 0 {
            return None;
        }

        let max_pages = env_usize("BRUTAL_BACKGROUND_CRAWL_MAX_PAGES").unwrap_or(top_n);
        let config = BackgroundIndexConfig {
            index_dir,
            top_n,
            max_pages: max_pages.max(1),
            max_depth: env_usize("BRUTAL_BACKGROUND_CRAWL_MAX_DEPTH")
                .unwrap_or(DEFAULT_BACKGROUND_MAX_DEPTH),
            concurrency: env_usize("BRUTAL_BACKGROUND_CRAWL_CONCURRENCY")
                .unwrap_or(DEFAULT_BACKGROUND_CONCURRENCY)
                .max(1),
            max_bytes: env_usize("BRUTAL_BACKGROUND_CRAWL_MAX_BYTES")
                .unwrap_or(DEFAULT_BACKGROUND_MAX_BYTES),
            ignore_robots: env_flag_enabled("BRUTAL_BACKGROUND_CRAWL_IGNORE_ROBOTS"),
            max_fetching_per_host: env_usize("BRUTAL_BACKGROUND_CRAWL_MAX_FETCHING_PER_HOST")
                .unwrap_or(DEFAULT_BACKGROUND_MAX_FETCHING_PER_HOST)
                .max(1),
        };
        let queue_capacity = env_usize("BRUTAL_BACKGROUND_CRAWL_QUEUE")
            .unwrap_or(DEFAULT_BACKGROUND_QUEUE_CAPACITY)
            .max(1);
        let dedupe_secs = env_u64("BRUTAL_BACKGROUND_CRAWL_DEDUPE_SECS")
            .unwrap_or(DEFAULT_BACKGROUND_DEDUPE_SECS);
        Some(Self::start(local, config, queue_capacity, dedupe_secs))
    }

    fn start(
        local: LocalSearchProvider,
        config: BackgroundIndexConfig,
        queue_capacity: usize,
        dedupe_secs: u64,
    ) -> Self {
        let (tx, rx) = mpsc::channel(queue_capacity);
        let top_n = config.top_n();
        tokio::spawn(run_background_indexer(rx, local, config));

        Self {
            tx,
            recent: Arc::new(Mutex::new(HashMap::new())),
            dedupe_secs,
            top_n,
        }
    }

    pub async fn enqueue_top_urls<I>(&self, urls: I) -> usize
    where
        I: IntoIterator<Item = String>,
    {
        let mut batch = normalize_url_batch(urls, self.top_n);
        if batch.is_empty() {
            return 0;
        }

        if self.dedupe_secs > 0 {
            let now = Instant::now();
            let ttl = Duration::from_secs(self.dedupe_secs);
            let mut recent = self.recent.lock().await;
            recent.retain(|_, submitted_at| now.duration_since(*submitted_at) <= ttl);
            batch.retain(|url| {
                if recent.contains_key(url) {
                    false
                } else {
                    recent.insert(url.clone(), now);
                    true
                }
            });
        }

        let enqueued = batch.len();
        if enqueued == 0 {
            return 0;
        }

        match self.tx.try_send(batch) {
            Ok(()) => enqueued,
            Err(error) => {
                eprintln!("background crawl queue skipped URLs: {error}");
                0
            }
        }
    }
}

impl BackgroundIndexConfig {
    fn top_n(&self) -> usize {
        self.top_n.max(1)
    }
}

async fn run_background_indexer(
    mut rx: mpsc::Receiver<Vec<String>>,
    local: LocalSearchProvider,
    config: BackgroundIndexConfig,
) {
    while let Some(mut urls) = rx.recv().await {
        while let Ok(mut extra) = rx.try_recv() {
            urls.append(&mut extra);
        }
        urls = normalize_url_batch(urls, config.max_pages);
        filter_existing_urls(&mut urls, &local);
        if urls.is_empty() {
            continue;
        }

        if let Err(error) = crawl_and_reload(urls, &local, &config).await {
            eprintln!("background crawl/index failed: {error:#}");
        }
    }
}

async fn crawl_and_reload(
    urls: Vec<String>,
    local: &LocalSearchProvider,
    config: &BackgroundIndexConfig,
) -> Result<()> {
    eprintln!(
        "background crawl/index: fetching {} third-party result URLs",
        urls.len()
    );
    let docs = crawl_many(
        urls.iter().map(String::as_str),
        CrawlOptions {
            max_pages: config.max_pages,
            max_depth: config.max_depth,
            concurrency: config.concurrency.min(config.max_pages.max(1)),
            max_bytes: config.max_bytes,
            ignore_robots: config.ignore_robots,
            boundary: CrawlBoundary::SameHost,
            frontier_path: Some(config.index_dir.join("frontier.bin")),
            document_snapshot_path: Some(config.index_dir.join("crawl-docs.jsonl")),
            max_fetching_per_host: config.max_fetching_per_host,
            recrawl_seeds: urls.clone(),
        },
    )
    .await?;

    if docs.is_empty() {
        return Ok(());
    }

    let docs = merge_current_and_crawled_docs(local, docs);
    let index_dir = config.index_dir.clone();
    let local = local.clone();
    let stats = tokio::task::spawn_blocking(move || -> Result<_> {
        let stats = build_from_fielded_documents(docs, &index_dir, IndexBuildOptions::default())?;
        local.reload()?;
        Ok(stats)
    })
    .await
    .context("join background index rebuild task")??;

    eprintln!(
        "background crawl/index: reloaded local index with {} docs and {} terms",
        stats.doc_count, stats.term_count
    );
    Ok(())
}

fn filter_existing_urls(urls: &mut Vec<String>, local: &LocalSearchProvider) {
    let index = local.current();
    urls.retain(|url| index.doc_id_for_url(url).is_none());
}

fn merge_current_and_crawled_docs(
    local: &LocalSearchProvider,
    crawled_docs: Vec<FieldedDocument>,
) -> Vec<FieldedDocument> {
    let index = local.current();
    let mut docs = index
        .field_docs()
        .map_or_else(Vec::new, |docs| docs.to_vec());
    let mut positions = docs
        .iter()
        .enumerate()
        .map(|(position, doc)| (doc.url.clone(), position))
        .collect::<HashMap<_, _>>();

    for doc in crawled_docs {
        if let Some(position) = positions.get(&doc.url).copied() {
            docs[position] = doc;
        } else {
            positions.insert(doc.url.clone(), docs.len());
            docs.push(doc);
        }
    }

    docs
}

fn normalize_url_batch<I>(urls: I, limit: usize) -> Vec<String>
where
    I: IntoIterator<Item = String>,
{
    let mut seen = HashSet::new();
    let mut normalized = Vec::new();
    for url in urls {
        let Some(url) = normalize_http_url(&url) else {
            continue;
        };
        if seen.insert(url.clone()) {
            normalized.push(url);
        }
        if normalized.len() >= limit {
            break;
        }
    }
    normalized
}

fn normalize_http_url(raw: &str) -> Option<String> {
    let url = Url::parse(raw.trim()).ok()?;
    if !matches!(url.scheme(), "http" | "https") {
        return None;
    }
    Some(canonicalize_url(url).to_string())
}

fn env_flag_disabled(name: &str) -> bool {
    env::var(name)
        .ok()
        .map(|value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "0" | "false" | "no" | "off"
            )
        })
        .unwrap_or(false)
}

fn env_flag_enabled(name: &str) -> bool {
    env::var(name)
        .ok()
        .map(|value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(false)
}

fn env_usize(name: &str) -> Option<usize> {
    env::var(name)
        .ok()
        .and_then(|value| value.trim().parse::<usize>().ok())
}

fn env_u64(name: &str) -> Option<u64> {
    env::var(name)
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::FieldedDocument;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    #[test]
    fn normalizes_top_urls_for_background_crawl() {
        assert_eq!(
            normalize_url_batch(
                [
                    "https://example.com/a#frag".to_owned(),
                    "ftp://example.com/skip".to_owned(),
                    "https://example.com/a".to_owned(),
                    "http://example.com/b".to_owned(),
                ],
                10,
            ),
            vec!["https://example.com/a", "http://example.com/b"]
        );
    }

    #[tokio::test]
    async fn local_provider_reload_swaps_search_index() {
        let dir = tempfile::tempdir().unwrap();
        build_from_fielded_documents(
            vec![FieldedDocument::from_plain_text(
                "https://example.com/one".to_owned(),
                "One".to_owned(),
                "alpha only".to_owned(),
                None,
            )],
            dir.path(),
            IndexBuildOptions::default(),
        )
        .unwrap();

        let provider =
            LocalSearchProvider::open(dir.path().to_path_buf(), PreloadMode::Lazy).unwrap();
        let before = provider.search("alpha", 5).await.unwrap();
        assert_eq!(before.results.len(), 1);

        build_from_fielded_documents(
            vec![FieldedDocument::from_plain_text(
                "https://example.com/two".to_owned(),
                "Two".to_owned(),
                "beta only".to_owned(),
                None,
            )],
            dir.path(),
            IndexBuildOptions::default(),
        )
        .unwrap();
        provider.reload().unwrap();

        let after = provider.search("beta", 5).await.unwrap();
        assert_eq!(after.results.len(), 1);
        let old = provider.search("alpha", 5).await.unwrap();
        assert!(old.results.is_empty());
    }

    #[tokio::test]
    async fn background_indexer_crawls_indexes_and_reloads_top_urls() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            loop {
                let Ok((mut stream, _)) = listener.accept().await else {
                    break;
                };
                tokio::spawn(async move {
                    let mut request = [0u8; 1024];
                    let _ = stream.read(&mut request).await;
                    let body = br#"<!doctype html>
<html><head><title>Background Indexed Page</title></head>
<body><h1>Background Indexed Page</h1><p>needlebackground unique text</p></body></html>"#;
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len()
                    );
                    let _ = stream.write_all(response.as_bytes()).await;
                    let _ = stream.write_all(body).await;
                    let _ = stream.flush().await;
                });
            }
        });

        let dir = tempfile::tempdir().unwrap();
        build_from_fielded_documents(
            vec![FieldedDocument::from_plain_text(
                "https://example.com/seed".to_owned(),
                "Seed".to_owned(),
                "seed text".to_owned(),
                None,
            )],
            dir.path(),
            IndexBuildOptions::default(),
        )
        .unwrap();
        let local = LocalSearchProvider::open(dir.path().to_path_buf(), PreloadMode::Lazy).unwrap();
        let indexer = BackgroundIndexer::start(
            local.clone(),
            BackgroundIndexConfig {
                index_dir: dir.path().to_path_buf(),
                top_n: 1,
                max_pages: 1,
                max_depth: 0,
                concurrency: 1,
                max_bytes: 1024 * 1024,
                ignore_robots: true,
                max_fetching_per_host: 1,
            },
            4,
            0,
        );

        let url = format!("http://{addr}/page");
        assert_eq!(indexer.enqueue_top_urls([url]).await, 1);

        let mut found = false;
        for _ in 0..50 {
            let response = local.search("needlebackground", 5).await.unwrap();
            if !response.results.is_empty() {
                found = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }

        assert!(found, "background crawled page did not become searchable");
        let seed = local.search("seed", 5).await.unwrap();
        assert_eq!(
            seed.results.len(),
            1,
            "background rebuild should preserve the existing local corpus"
        );
    }
}
