use std::collections::{HashMap, HashSet};
use std::env;
use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

const BRAVE_WEB_SEARCH_ENDPOINT: &str = "https://api.search.brave.com/res/v1/web/search";
const BRAVE_MAX_COUNT: usize = 20;
const DEFAULT_CACHE_TTL_SECS: u64 = 30 * 24 * 60 * 60;
pub const DEFAULT_CACHE_MAX_ENTRIES: usize = 4096;
pub const DEFAULT_RESULT_LOG_MAX_ENTRIES: usize = 4096;
const DEFAULT_MAX_WEB_RESULTS: usize = 20;
const DEFAULT_MIN_LOCAL_RESULTS: usize = 20;

#[derive(Debug, Clone)]
pub struct WebSearchConfig {
    provider: ThirdPartySearchProvider,
    cache_path: PathBuf,
    result_log_path: PathBuf,
    result_log_max_entries: usize,
    cache_ttl_secs: u64,
    min_local_results: usize,
    max_results: usize,
    country: String,
    search_lang: String,
}

#[derive(Debug, Clone)]
enum ThirdPartySearchProvider {
    Brave { api_key: String },
    CacheOnly,
}

#[derive(Debug)]
pub struct WebSearchService {
    client: reqwest::Client,
    config: WebSearchConfig,
    cache: Arc<Mutex<WebResultCache>>,
}

#[derive(Debug, Clone)]
pub struct WebSearchLookup {
    pub provider: &'static str,
    pub cache_hit: bool,
    pub fetched: bool,
    pub results: Vec<WebSearchResult>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebSearchStorageCompactionReport {
    pub cache_path: PathBuf,
    pub result_log_path: PathBuf,
    pub cache_before: WebSearchStorageArtifactState,
    pub cache_after: WebSearchStorageArtifactState,
    pub cache_projected_after: WebSearchStorageArtifactState,
    pub result_log_before: WebSearchStorageArtifactState,
    pub result_log_after: WebSearchStorageArtifactState,
    pub result_log_projected_after: WebSearchStorageArtifactState,
    pub skipped: bool,
    pub dry_run: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WebSearchStorageArtifactState {
    pub bytes: u64,
    pub entries: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct WebSearchStorageCompactionOptions {
    pub dry_run: bool,
    pub min_entries: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebSearchResult {
    pub title: String,
    pub url: String,
    pub snippet: String,
    pub score: f32,
    pub fetched_at_unix: u64,
    pub provider: String,
}

#[derive(Debug)]
struct WebResultCache {
    path: PathBuf,
    ttl_secs: u64,
    max_entries: usize,
    entries: HashMap<String, CachedWebSearch>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CachedWebSearch {
    query: String,
    normalized_query: String,
    provider: String,
    fetched_at_unix: u64,
    results: Vec<WebSearchResult>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct WebSearchResultLogEntry {
    query: String,
    normalized_query: String,
    provider: String,
    fetched_at_unix: u64,
    rank: usize,
    title: String,
    url: String,
    snippet: String,
    score: f32,
}

#[derive(Debug, Deserialize)]
struct BraveSearchResponse {
    web: Option<BraveWebResults>,
}

#[derive(Debug, Deserialize)]
struct BraveWebResults {
    #[serde(default)]
    results: Vec<BraveSearchItem>,
}

#[derive(Debug, Deserialize)]
struct BraveSearchItem {
    #[serde(default)]
    title: String,
    url: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    extra_snippets: Vec<String>,
}

impl WebSearchService {
    pub fn from_env(index_dir: &Path) -> Result<Option<Self>> {
        if env_flag_disabled("BRUTAL_WEB_FALLBACK") {
            return Ok(None);
        }

        let api_key = env::var("BRAVE_SEARCH_API_KEY")
            .ok()
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty());

        let cache_path = env::var_os("BRUTAL_WEB_CACHE_PATH")
            .map(PathBuf::from)
            .unwrap_or_else(|| index_dir.join("web-cache.jsonl"));
        let result_log_path = env::var_os("BRUTAL_WEB_RESULT_LOG_PATH")
            .map(PathBuf::from)
            .unwrap_or_else(|| index_dir.join("brave-results.jsonl"));
        let result_log_max_entries = env_usize("BRUTAL_WEB_RESULT_LOG_MAX_ENTRIES")
            .unwrap_or(DEFAULT_RESULT_LOG_MAX_ENTRIES);
        let provider = match api_key {
            Some(api_key) => ThirdPartySearchProvider::Brave { api_key },
            None if cache_path.exists() => ThirdPartySearchProvider::CacheOnly,
            None => return Ok(None),
        };
        let cache_ttl_secs = env_u64("BRUTAL_WEB_CACHE_TTL_SECS").unwrap_or(DEFAULT_CACHE_TTL_SECS);
        let cache_max_entries =
            env_usize("BRUTAL_WEB_CACHE_MAX_ENTRIES").unwrap_or(DEFAULT_CACHE_MAX_ENTRIES);
        let min_local_results =
            env_usize("BRUTAL_WEB_FALLBACK_MIN_LOCAL_RESULTS").unwrap_or(DEFAULT_MIN_LOCAL_RESULTS);
        let max_results = env_usize("BRUTAL_WEB_FALLBACK_COUNT")
            .unwrap_or(DEFAULT_MAX_WEB_RESULTS)
            .clamp(1, BRAVE_MAX_COUNT);
        let country = env::var("BRAVE_SEARCH_COUNTRY").unwrap_or_else(|_| "us".to_owned());
        let search_lang = env::var("BRAVE_SEARCH_LANG").unwrap_or_else(|_| "en".to_owned());

        let cache = WebResultCache::load(cache_path.clone(), cache_ttl_secs, cache_max_entries)?;
        let client = reqwest::Client::builder()
            .user_agent("brutal-search/0.1 web-search-provider")
            .timeout(Duration::from_secs(8))
            .pool_max_idle_per_host(16)
            .tcp_nodelay(true)
            .build()?;

        Ok(Some(Self {
            client,
            config: WebSearchConfig {
                provider,
                cache_path,
                result_log_path,
                result_log_max_entries,
                cache_ttl_secs,
                min_local_results,
                max_results,
                country,
                search_lang,
            },
            cache: Arc::new(Mutex::new(cache)),
        }))
    }

    pub fn provider_name(&self) -> &'static str {
        match &self.config.provider {
            ThirdPartySearchProvider::Brave { .. } => "brave",
            ThirdPartySearchProvider::CacheOnly => "brave-cache",
        }
    }

    pub fn cache_path(&self) -> &Path {
        &self.config.cache_path
    }

    pub fn should_search_web(&self, local_count: usize, requested_limit: usize) -> bool {
        local_count < requested_limit && local_count < self.config.min_local_results
    }

    pub async fn search(&self, query: &str, requested_limit: usize) -> Result<WebSearchLookup> {
        let normalized_query = normalize_web_query(query);
        if normalized_query.is_empty() || requested_limit == 0 {
            return Ok(WebSearchLookup {
                provider: self.provider_name(),
                cache_hit: false,
                fetched: false,
                results: Vec::new(),
            });
        }

        let limit = requested_limit
            .min(self.config.max_results)
            .min(BRAVE_MAX_COUNT);
        if self.config.cache_ttl_secs > 0 {
            let cache = self.cache.lock().await;
            if let Some(results) = cache.lookup(&normalized_query, now_unix()) {
                return Ok(WebSearchLookup {
                    provider: self.provider_name(),
                    cache_hit: true,
                    fetched: false,
                    results: take_results(results, limit),
                });
            }
        }

        let results = match &self.config.provider {
            ThirdPartySearchProvider::Brave { api_key } => {
                fetch_brave_web_search(
                    &self.client,
                    api_key,
                    query,
                    limit,
                    &self.config.country,
                    &self.config.search_lang,
                )
                .await?
            }
            ThirdPartySearchProvider::CacheOnly => {
                return Ok(WebSearchLookup {
                    provider: self.provider_name(),
                    cache_hit: false,
                    fetched: false,
                    results: Vec::new(),
                });
            }
        };

        if self.config.cache_ttl_secs > 0 {
            let mut cache = self.cache.lock().await;
            cache.store(
                query.to_owned(),
                normalized_query.clone(),
                self.provider_name().to_owned(),
                results.clone(),
                now_unix(),
            )?;
        }
        append_result_log(
            &self.config.result_log_path,
            query,
            &normalized_query,
            self.provider_name(),
            &results,
            self.config.result_log_max_entries,
        )?;

        Ok(WebSearchLookup {
            provider: self.provider_name(),
            cache_hit: false,
            fetched: true,
            results,
        })
    }
}

pub fn compact_web_search_storage_from_env(
    index_dir: &Path,
    options: WebSearchStorageCompactionOptions,
) -> Result<WebSearchStorageCompactionReport> {
    let cache_path = env::var_os("BRUTAL_WEB_CACHE_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|| index_dir.join("web-cache.jsonl"));
    let result_log_path = env::var_os("BRUTAL_WEB_RESULT_LOG_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|| index_dir.join("brave-results.jsonl"));
    let cache_ttl_secs = env_u64("BRUTAL_WEB_CACHE_TTL_SECS").unwrap_or(DEFAULT_CACHE_TTL_SECS);
    let cache_max_entries =
        env_usize("BRUTAL_WEB_CACHE_MAX_ENTRIES").unwrap_or(DEFAULT_CACHE_MAX_ENTRIES);
    let result_log_max_entries =
        env_usize("BRUTAL_WEB_RESULT_LOG_MAX_ENTRIES").unwrap_or(DEFAULT_RESULT_LOG_MAX_ENTRIES);
    let result_log_max_entries_per_query =
        env_usize("BRUTAL_WEB_RESULT_LOG_MAX_ENTRIES_PER_QUERY").unwrap_or(0);

    compact_web_search_storage(
        cache_path,
        result_log_path,
        cache_ttl_secs,
        cache_max_entries,
        result_log_max_entries,
        result_log_max_entries_per_query,
        options,
    )
}

fn compact_web_search_storage(
    cache_path: PathBuf,
    result_log_path: PathBuf,
    cache_ttl_secs: u64,
    cache_max_entries: usize,
    result_log_max_entries: usize,
    result_log_max_entries_per_query: usize,
    options: WebSearchStorageCompactionOptions,
) -> Result<WebSearchStorageCompactionReport> {
    let cache_before = web_storage_artifact_state(&cache_path)?;
    let result_log_before = web_storage_artifact_state(&result_log_path)?;
    let max_entries_before = cache_before.entries.max(result_log_before.entries);
    let skipped = options.min_entries > 0 && max_entries_before < options.min_entries;
    let should_compact = !skipped && !options.dry_run;
    let should_project = !skipped && options.dry_run;

    let (cache_projected_after, result_log_projected_after) = if should_project {
        projected_web_search_storage_state(
            &cache_path,
            &result_log_path,
            cache_ttl_secs,
            cache_max_entries,
            result_log_max_entries,
            result_log_max_entries_per_query,
        )?
    } else {
        (cache_before, result_log_before)
    };

    if should_compact && cache_path.exists() {
        let _ = WebResultCache::load(cache_path.clone(), cache_ttl_secs, cache_max_entries)?;
    }
    if should_compact {
        enforce_result_log_retention_with_query_cap(
            &result_log_path,
            result_log_max_entries,
            result_log_max_entries_per_query,
        )?;
    }

    let cache_after = web_storage_artifact_state(&cache_path)?;
    let result_log_after = web_storage_artifact_state(&result_log_path)?;
    Ok(WebSearchStorageCompactionReport {
        cache_path,
        result_log_path,
        cache_before,
        cache_after,
        cache_projected_after,
        result_log_before,
        result_log_after,
        result_log_projected_after,
        skipped,
        dry_run: options.dry_run,
    })
}

fn projected_web_search_storage_state(
    cache_path: &Path,
    result_log_path: &Path,
    cache_ttl_secs: u64,
    cache_max_entries: usize,
    result_log_max_entries: usize,
    result_log_max_entries_per_query: usize,
) -> Result<(WebSearchStorageArtifactState, WebSearchStorageArtifactState)> {
    let temp_dir = WebStorageProjectionTempDir::create()?;
    let temp_cache_path = temp_dir.path().join("web-cache.jsonl");
    let temp_result_log_path = temp_dir.path().join("brave-results.jsonl");

    if cache_path.exists() {
        fs::copy(cache_path, &temp_cache_path).with_context(|| {
            format!(
                "copy web cache {} to dry-run temp {}",
                cache_path.display(),
                temp_cache_path.display()
            )
        })?;
        let _ = WebResultCache::load(temp_cache_path.clone(), cache_ttl_secs, cache_max_entries)?;
    }
    if result_log_path.exists() {
        fs::copy(result_log_path, &temp_result_log_path).with_context(|| {
            format!(
                "copy web result log {} to dry-run temp {}",
                result_log_path.display(),
                temp_result_log_path.display()
            )
        })?;
        enforce_result_log_retention_with_query_cap(
            &temp_result_log_path,
            result_log_max_entries,
            result_log_max_entries_per_query,
        )?;
    }

    Ok((
        web_storage_artifact_state(&temp_cache_path)?,
        web_storage_artifact_state(&temp_result_log_path)?,
    ))
}

struct WebStorageProjectionTempDir {
    path: PathBuf,
}

impl WebStorageProjectionTempDir {
    fn create() -> Result<Self> {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let path = env::temp_dir().join(format!(
            "brutal-web-storage-projection-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir(&path)
            .with_context(|| format!("create web storage dry-run temp dir {}", path.display()))?;
        Ok(Self { path })
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for WebStorageProjectionTempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn web_storage_artifact_state(path: &Path) -> Result<WebSearchStorageArtifactState> {
    let bytes = match fs::metadata(path) {
        Ok(metadata) if metadata.is_file() => metadata.len(),
        Ok(_) => 0,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => 0,
        Err(error) => {
            return Err(error)
                .with_context(|| format!("read web storage artifact {}", path.display()));
        }
    };
    Ok(WebSearchStorageArtifactState {
        bytes,
        entries: count_nonempty_lines(path)?,
    })
}

fn count_nonempty_lines(path: &Path) -> Result<usize> {
    let file = match fs::File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(error) => {
            return Err(error)
                .with_context(|| format!("open web storage artifact {}", path.display()));
        }
    };
    let mut entries = 0usize;
    for (line_no, line) in BufReader::new(file).lines().enumerate() {
        let line = line.with_context(|| {
            format!(
                "read line {} from web storage artifact {}",
                line_no + 1,
                path.display()
            )
        })?;
        if !line.trim().is_empty() {
            entries += 1;
        }
    }
    Ok(entries)
}

impl WebResultCache {
    fn load(path: PathBuf, ttl_secs: u64, max_entries: usize) -> Result<Self> {
        let mut entries = HashMap::new();
        let mut parsed_lines = 0usize;
        let mut skipped_lines = false;
        if path.exists() {
            let file = fs::File::open(&path)
                .with_context(|| format!("open web search cache {}", path.display()))?;
            for (line_no, line) in BufReader::new(file).lines().enumerate() {
                let line = line.with_context(|| {
                    format!(
                        "read line {} from web search cache {}",
                        line_no + 1,
                        path.display()
                    )
                })?;
                if line.trim().is_empty() {
                    skipped_lines = true;
                    continue;
                }
                match serde_json::from_str::<CachedWebSearch>(&line) {
                    Ok(entry) if !entry.normalized_query.is_empty() => {
                        parsed_lines += 1;
                        preserve_newest_cached_entry(&mut entries, entry);
                    }
                    Ok(_) => {
                        skipped_lines = true;
                    }
                    Err(error) => {
                        skipped_lines = true;
                        eprintln!(
                            "web search cache skipped invalid line {} in {}: {error}",
                            line_no + 1,
                            path.display()
                        );
                    }
                }
            }
        }

        let mut cache = Self {
            path,
            ttl_secs,
            max_entries,
            entries,
        };
        let pruned = cache.enforce_retention(now_unix());
        if cache.ttl_secs > 0
            && (pruned || skipped_lines || parsed_lines != cache.entries.len())
            && let Err(error) = cache.rewrite()
        {
            eprintln!("web search cache compaction failed: {error:#}");
        }

        Ok(cache)
    }

    fn lookup(&self, normalized_query: &str, now: u64) -> Option<Vec<WebSearchResult>> {
        let entry = self.entries.get(normalized_query)?;
        if now.saturating_sub(entry.fetched_at_unix) > self.ttl_secs {
            return None;
        }
        Some(entry.results.clone())
    }

    fn store(
        &mut self,
        query: String,
        normalized_query: String,
        provider: String,
        results: Vec<WebSearchResult>,
        fetched_at_unix: u64,
    ) -> Result<()> {
        let entry = CachedWebSearch {
            query,
            normalized_query,
            provider,
            fetched_at_unix,
            results,
        };
        let replaced_existing = self.entries.contains_key(&entry.normalized_query);
        append_cache_entry(&self.path, &entry)?;
        self.entries.insert(entry.normalized_query.clone(), entry);
        let pruned = self.enforce_retention(fetched_at_unix);
        if replaced_existing || pruned {
            self.rewrite()?;
        }
        Ok(())
    }

    fn enforce_retention(&mut self, now: u64) -> bool {
        let before = self.entries.len();
        if self.ttl_secs > 0 {
            let ttl_secs = self.ttl_secs;
            self.entries
                .retain(|_, entry| now.saturating_sub(entry.fetched_at_unix) <= ttl_secs);
        }

        if self.max_entries > 0 && self.entries.len() > self.max_entries {
            let remove_count = self.entries.len() - self.max_entries;
            let mut entries_by_age = self
                .entries
                .iter()
                .map(|(query, entry)| (entry.fetched_at_unix, query.clone()))
                .collect::<Vec<_>>();
            entries_by_age
                .sort_by(|left, right| left.0.cmp(&right.0).then_with(|| left.1.cmp(&right.1)));
            for (_, query) in entries_by_age.into_iter().take(remove_count) {
                self.entries.remove(&query);
            }
        }

        self.entries.len() != before
    }

    fn rewrite(&self) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("create web search cache parent {}", parent.display()))?;
        }

        let tmp_path = self.path.with_extension("tmp");
        {
            let mut file = fs::File::create(&tmp_path)
                .with_context(|| format!("create web search cache temp {}", tmp_path.display()))?;
            let mut entries = self.entries.values().collect::<Vec<_>>();
            entries.sort_by(|left, right| {
                left.fetched_at_unix
                    .cmp(&right.fetched_at_unix)
                    .then_with(|| left.normalized_query.cmp(&right.normalized_query))
            });
            for entry in entries {
                serde_json::to_writer(&mut file, entry)?;
                file.write_all(b"\n")?;
            }
            file.flush()?;
        }
        fs::rename(&tmp_path, &self.path).with_context(|| {
            format!(
                "replace web search cache {} with {}",
                self.path.display(),
                tmp_path.display()
            )
        })?;
        Ok(())
    }
}

fn preserve_newest_cached_entry(
    entries: &mut HashMap<String, CachedWebSearch>,
    entry: CachedWebSearch,
) {
    match entries.get(&entry.normalized_query) {
        Some(existing) if existing.fetched_at_unix > entry.fetched_at_unix => {}
        _ => {
            entries.insert(entry.normalized_query.clone(), entry);
        }
    }
}

async fn fetch_brave_web_search(
    client: &reqwest::Client,
    api_key: &str,
    query: &str,
    count: usize,
    country: &str,
    search_lang: &str,
) -> Result<Vec<WebSearchResult>> {
    let response = client
        .get(BRAVE_WEB_SEARCH_ENDPOINT)
        .header("Accept", "application/json")
        .header("Accept-Encoding", "gzip")
        .header("X-Subscription-Token", api_key)
        .query(&[
            ("q", query),
            ("count", &count.to_string()),
            ("country", country),
            ("search_lang", search_lang),
        ])
        .send()
        .await
        .context("send Brave Search request")?;

    let status = response.status();
    let body = response
        .text()
        .await
        .context("read Brave Search response")?;
    if !status.is_success() {
        anyhow::bail!(
            "Brave Search returned {status}: {}",
            compact_error_body(&body)
        );
    }

    let parsed: BraveSearchResponse =
        serde_json::from_str(&body).context("decode Brave Search response")?;
    Ok(brave_results_to_web_results(parsed, now_unix()))
}

fn brave_results_to_web_results(
    response: BraveSearchResponse,
    fetched_at_unix: u64,
) -> Vec<WebSearchResult> {
    let mut seen_urls = HashSet::new();
    let mut results = Vec::new();
    let Some(web) = response.web else {
        return results;
    };

    for item in web.results {
        if item.url.trim().is_empty() || !seen_urls.insert(item.url.clone()) {
            continue;
        }
        let rank = results.len() + 1;
        let snippet = brave_snippet(&item);
        results.push(WebSearchResult {
            title: item.title.trim().to_owned(),
            url: item.url,
            snippet,
            score: 1.0 / rank as f32,
            fetched_at_unix,
            provider: "brave".to_owned(),
        });
    }

    results
}

fn brave_snippet(item: &BraveSearchItem) -> String {
    let mut snippet = item.description.trim().to_owned();
    for extra in &item.extra_snippets {
        let extra = extra.trim();
        if extra.is_empty() {
            continue;
        }
        if !snippet.is_empty() {
            snippet.push(' ');
        }
        snippet.push_str(extra);
        if snippet.len() >= 360 {
            truncate_to_char_boundary(&mut snippet, 360);
            break;
        }
    }
    snippet
}

fn append_cache_entry(path: &Path, entry: &CachedWebSearch) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("create web search cache parent {}", parent.display()))?;
    }

    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .with_context(|| format!("open web search cache {}", path.display()))?;
    serde_json::to_writer(&mut file, entry)?;
    file.write_all(b"\n")?;
    file.flush()?;
    Ok(())
}

fn append_result_log(
    path: &Path,
    query: &str,
    normalized_query: &str,
    provider: &str,
    results: &[WebSearchResult],
    max_entries: usize,
) -> Result<()> {
    if results.is_empty() {
        return Ok(());
    }

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("create web search result log parent {}", parent.display()))?;
    }

    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .with_context(|| format!("open web search result log {}", path.display()))?;
    for (index, result) in results.iter().enumerate() {
        let entry = WebSearchResultLogEntry {
            query: query.to_owned(),
            normalized_query: normalized_query.to_owned(),
            provider: provider.to_owned(),
            fetched_at_unix: result.fetched_at_unix,
            rank: index + 1,
            title: result.title.clone(),
            url: result.url.clone(),
            snippet: result.snippet.clone(),
            score: result.score,
        };
        serde_json::to_writer(&mut file, &entry)?;
        file.write_all(b"\n")?;
    }
    file.flush()?;
    enforce_result_log_retention(path, max_entries)?;
    Ok(())
}

fn enforce_result_log_retention(path: &Path, max_entries: usize) -> Result<()> {
    enforce_result_log_retention_with_query_cap(path, max_entries, 0)
}

fn enforce_result_log_retention_with_query_cap(
    path: &Path,
    max_entries: usize,
    max_entries_per_query: usize,
) -> Result<()> {
    if max_entries == 0 || !path.exists() {
        return Ok(());
    }

    let file = fs::File::open(path)
        .with_context(|| format!("open web search result log {}", path.display()))?;
    let mut entries = Vec::new();
    let mut line_count = 0usize;
    for (line_no, line) in BufReader::new(file).lines().enumerate() {
        let line = line.with_context(|| {
            format!(
                "read line {} from web search result log {}",
                line_no + 1,
                path.display()
            )
        })?;
        if line.trim().is_empty() {
            line_count += 1;
            continue;
        }
        line_count += 1;
        match serde_json::from_str::<WebSearchResultLogEntry>(&line) {
            Ok(entry) => entries.push(entry),
            Err(error) => eprintln!(
                "web search result log skipped invalid line {} in {}: {error}",
                line_no + 1,
                path.display()
            ),
        }
    }

    let original_entries = entries.len();
    let mut seen = HashSet::new();
    let mut query_counts = HashMap::new();
    let mut retained = Vec::new();
    for entry in entries.into_iter().rev() {
        if !seen.insert(result_log_dedupe_key(&entry)) {
            continue;
        }
        let query_key = (entry.normalized_query.clone(), entry.provider.clone());
        if max_entries_per_query > 0 {
            let count = query_counts.entry(query_key).or_insert(0usize);
            if *count >= max_entries_per_query {
                continue;
            }
            *count += 1;
        }
        retained.push(entry);
        if retained.len() >= max_entries {
            break;
        }
    }
    retained.reverse();

    if retained.len() == original_entries && line_count == original_entries {
        return Ok(());
    }

    let tmp_path = path.with_extension("tmp");
    {
        let mut file = fs::File::create(&tmp_path)
            .with_context(|| format!("create web search result log temp {}", tmp_path.display()))?;
        for entry in &retained {
            serde_json::to_writer(&mut file, entry)?;
            file.write_all(b"\n")?;
        }
        file.flush()?;
    }
    fs::rename(&tmp_path, path).with_context(|| {
        format!(
            "replace web search result log {} with {}",
            path.display(),
            tmp_path.display()
        )
    })?;
    Ok(())
}

fn result_log_dedupe_key(entry: &WebSearchResultLogEntry) -> (String, String, usize, String) {
    (
        entry.normalized_query.clone(),
        entry.provider.clone(),
        entry.rank,
        entry.url.clone(),
    )
}

fn take_results(mut results: Vec<WebSearchResult>, limit: usize) -> Vec<WebSearchResult> {
    results.truncate(limit);
    results
}

fn normalize_web_query(query: &str) -> String {
    query
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

fn now_unix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
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

fn compact_error_body(body: &str) -> String {
    let compact = body.split_whitespace().collect::<Vec<_>>().join(" ");
    if compact.len() > 240 {
        let mut compact = compact;
        truncate_to_char_boundary(&mut compact, 240);
        compact.push_str("...");
        compact
    } else {
        compact
    }
}

fn truncate_to_char_boundary(value: &mut String, max_len: usize) {
    if value.len() <= max_len {
        return;
    }
    let mut end = max_len;
    while end > 0 && !value.is_char_boundary(end) {
        end -= 1;
    }
    value.truncate(end);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_web_queries_for_cache_keys() {
        assert_eq!(
            normalize_web_query("  Browser   Runtime "),
            "browser runtime"
        );
    }

    #[test]
    fn converts_brave_results_to_ranked_web_results() {
        let results = brave_results_to_web_results(
            BraveSearchResponse {
                web: Some(BraveWebResults {
                    results: vec![
                        BraveSearchItem {
                            title: "First".to_owned(),
                            url: "https://example.com/a".to_owned(),
                            description: "Primary snippet.".to_owned(),
                            extra_snippets: vec!["Extra context.".to_owned()],
                        },
                        BraveSearchItem {
                            title: "Duplicate".to_owned(),
                            url: "https://example.com/a".to_owned(),
                            description: "Duplicate snippet.".to_owned(),
                            extra_snippets: Vec::new(),
                        },
                        BraveSearchItem {
                            title: "Second".to_owned(),
                            url: "https://example.com/b".to_owned(),
                            description: String::new(),
                            extra_snippets: vec!["Only extra.".to_owned()],
                        },
                    ],
                }),
            },
            123,
        );

        assert_eq!(results.len(), 2);
        assert_eq!(results[0].title, "First");
        assert_eq!(results[0].snippet, "Primary snippet. Extra context.");
        assert_eq!(results[0].score, 1.0);
        assert_eq!(results[1].url, "https://example.com/b");
        assert_eq!(results[1].score, 0.5);
    }

    #[test]
    fn brave_snippet_truncates_at_utf8_boundary() {
        let snippet = brave_snippet(&BraveSearchItem {
            title: "Boundary".to_owned(),
            url: "https://example.com/boundary".to_owned(),
            description: "a".repeat(358),
            extra_snippets: vec!["é tail".to_owned()],
        });

        assert!(snippet.is_char_boundary(snippet.len()));
        assert!(snippet.len() <= 360);
    }

    #[test]
    fn compact_error_body_truncates_at_utf8_boundary() {
        let body = format!("{}é tail", "a".repeat(239));
        let compact = compact_error_body(&body);

        assert!(compact.is_char_boundary(compact.len()));
        assert!(compact.ends_with("..."));
    }

    #[test]
    fn cache_lookup_respects_ttl() {
        let mut cache = WebResultCache {
            path: PathBuf::from("unused"),
            ttl_secs: 10,
            max_entries: DEFAULT_CACHE_MAX_ENTRIES,
            entries: HashMap::new(),
        };
        cache.entries.insert(
            "query".to_owned(),
            CachedWebSearch {
                query: "query".to_owned(),
                normalized_query: "query".to_owned(),
                provider: "brave".to_owned(),
                fetched_at_unix: 100,
                results: vec![WebSearchResult {
                    title: "Title".to_owned(),
                    url: "https://example.com".to_owned(),
                    snippet: "Snippet".to_owned(),
                    score: 1.0,
                    fetched_at_unix: 100,
                    provider: "brave".to_owned(),
                }],
            },
        );

        assert!(cache.lookup("query", 110).is_some());
        assert!(cache.lookup("query", 111).is_none());
    }

    #[test]
    fn cache_store_compacts_replaced_query_on_disk() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("web-cache.jsonl");
        let mut cache = WebResultCache {
            path: path.clone(),
            ttl_secs: 100,
            max_entries: DEFAULT_CACHE_MAX_ENTRIES,
            entries: HashMap::new(),
        };

        cache
            .store(
                "query".to_owned(),
                "query".to_owned(),
                "brave".to_owned(),
                vec![web_result("https://example.com/old", 10)],
                10,
            )
            .unwrap();
        cache
            .store(
                "query".to_owned(),
                "query".to_owned(),
                "brave".to_owned(),
                vec![web_result("https://example.com/new", 20)],
                20,
            )
            .unwrap();

        let lines = fs::read_to_string(path).unwrap();
        assert_eq!(lines.lines().count(), 1);
        assert!(lines.contains("https://example.com/new"));
        assert!(!lines.contains("https://example.com/old"));
    }

    #[test]
    fn cache_store_keeps_only_newest_max_entries() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("web-cache.jsonl");
        let mut cache = WebResultCache {
            path: path.clone(),
            ttl_secs: 100,
            max_entries: 2,
            entries: HashMap::new(),
        };

        for fetched_at in 10..=12 {
            cache
                .store(
                    format!("query {fetched_at}"),
                    format!("query {fetched_at}"),
                    "brave".to_owned(),
                    vec![web_result(
                        &format!("https://example.com/{fetched_at}"),
                        fetched_at,
                    )],
                    fetched_at,
                )
                .unwrap();
        }

        let lines = fs::read_to_string(path).unwrap();
        assert_eq!(cache.entries.len(), 2);
        assert_eq!(lines.lines().count(), 2);
        assert!(!lines.contains("https://example.com/10"));
        assert!(lines.contains("https://example.com/11"));
        assert!(lines.contains("https://example.com/12"));
    }

    #[test]
    fn cache_load_compacts_stale_and_duplicate_entries() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("web-cache.jsonl");
        let now = now_unix();
        append_cache_entry(
            &path,
            &CachedWebSearch {
                query: "stale".to_owned(),
                normalized_query: "stale".to_owned(),
                provider: "brave".to_owned(),
                fetched_at_unix: now.saturating_sub(10),
                results: vec![web_result(
                    "https://example.com/stale",
                    now.saturating_sub(10),
                )],
            },
        )
        .unwrap();
        append_cache_entry(
            &path,
            &CachedWebSearch {
                query: "fresh".to_owned(),
                normalized_query: "fresh".to_owned(),
                provider: "brave".to_owned(),
                fetched_at_unix: now,
                results: vec![web_result("https://example.com/old-fresh", now)],
            },
        )
        .unwrap();
        append_cache_entry(
            &path,
            &CachedWebSearch {
                query: "fresh".to_owned(),
                normalized_query: "fresh".to_owned(),
                provider: "brave".to_owned(),
                fetched_at_unix: now,
                results: vec![web_result("https://example.com/new-fresh", now)],
            },
        )
        .unwrap();

        let cache = WebResultCache::load(path.clone(), 5, DEFAULT_CACHE_MAX_ENTRIES).unwrap();

        let lines = fs::read_to_string(path).unwrap();
        assert_eq!(cache.entries.len(), 1);
        assert_eq!(lines.lines().count(), 1);
        assert!(lines.contains("https://example.com/new-fresh"));
        assert!(!lines.contains("https://example.com/stale"));
        assert!(!lines.contains("https://example.com/old-fresh"));
    }

    #[test]
    fn cache_load_preserves_newest_duplicate_query_entry() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("web-cache.jsonl");
        let now = now_unix();
        append_cache_entry(
            &path,
            &CachedWebSearch {
                query: "query".to_owned(),
                normalized_query: "query".to_owned(),
                provider: "brave".to_owned(),
                fetched_at_unix: now,
                results: vec![web_result("https://example.com/new", now)],
            },
        )
        .unwrap();
        append_cache_entry(
            &path,
            &CachedWebSearch {
                query: "query".to_owned(),
                normalized_query: "query".to_owned(),
                provider: "brave".to_owned(),
                fetched_at_unix: now.saturating_sub(1),
                results: vec![web_result("https://example.com/old", now.saturating_sub(1))],
            },
        )
        .unwrap();

        let cache = WebResultCache::load(path.clone(), 1_000, DEFAULT_CACHE_MAX_ENTRIES).unwrap();

        let lines = fs::read_to_string(path).unwrap();
        assert_eq!(cache.entries.len(), 1);
        assert_eq!(lines.lines().count(), 1);
        assert!(lines.contains("https://example.com/new"));
        assert!(!lines.contains("https://example.com/old"));
    }

    #[test]
    fn compact_web_search_storage_reports_before_after_cleanup() {
        let dir = tempfile::tempdir().unwrap();
        let cache_path = dir.path().join("web-cache.jsonl");
        let result_log_path = dir.path().join("brave-results.jsonl");
        let now = now_unix();
        append_cache_entry(
            &cache_path,
            &CachedWebSearch {
                query: "query".to_owned(),
                normalized_query: "query".to_owned(),
                provider: "brave".to_owned(),
                fetched_at_unix: now,
                results: vec![web_result("https://example.com/new", now)],
            },
        )
        .unwrap();
        append_cache_entry(
            &cache_path,
            &CachedWebSearch {
                query: "query".to_owned(),
                normalized_query: "query".to_owned(),
                provider: "brave".to_owned(),
                fetched_at_unix: now.saturating_sub(1),
                results: vec![web_result("https://example.com/old", now.saturating_sub(1))],
            },
        )
        .unwrap();
        let duplicate_log_entry = WebSearchResultLogEntry {
            query: "Query".to_owned(),
            normalized_query: "query".to_owned(),
            provider: "brave".to_owned(),
            fetched_at_unix: now,
            rank: 1,
            title: "Title".to_owned(),
            url: "https://example.com/new".to_owned(),
            snippet: "Snippet".to_owned(),
            score: 1.0,
        };
        {
            let mut file = fs::File::create(&result_log_path).unwrap();
            serde_json::to_writer(&mut file, &duplicate_log_entry).unwrap();
            file.write_all(b"\n").unwrap();
            serde_json::to_writer(&mut file, &duplicate_log_entry).unwrap();
            file.write_all(b"\n").unwrap();
            file.flush().unwrap();
        }

        let report = compact_web_search_storage(
            cache_path.clone(),
            result_log_path.clone(),
            1_000,
            DEFAULT_CACHE_MAX_ENTRIES,
            DEFAULT_RESULT_LOG_MAX_ENTRIES,
            0,
            WebSearchStorageCompactionOptions::default(),
        )
        .unwrap();

        assert_eq!(report.cache_before.entries, 2);
        assert_eq!(report.cache_after.entries, 1);
        assert!(report.cache_after.bytes < report.cache_before.bytes);
        assert_eq!(report.result_log_before.entries, 2);
        assert_eq!(report.result_log_after.entries, 1);
        assert!(report.result_log_after.bytes < report.result_log_before.bytes);
        let cache = fs::read_to_string(cache_path).unwrap();
        assert!(cache.contains("https://example.com/new"));
        assert!(!cache.contains("https://example.com/old"));
    }

    #[test]
    fn compact_web_search_storage_dry_run_reports_without_rewriting() {
        let dir = tempfile::tempdir().unwrap();
        let cache_path = dir.path().join("web-cache.jsonl");
        let result_log_path = dir.path().join("brave-results.jsonl");
        let now = now_unix();
        append_cache_entry(
            &cache_path,
            &CachedWebSearch {
                query: "query".to_owned(),
                normalized_query: "query".to_owned(),
                provider: "brave".to_owned(),
                fetched_at_unix: now,
                results: vec![web_result("https://example.com/new", now)],
            },
        )
        .unwrap();
        append_cache_entry(
            &cache_path,
            &CachedWebSearch {
                query: "query".to_owned(),
                normalized_query: "query".to_owned(),
                provider: "brave".to_owned(),
                fetched_at_unix: now.saturating_sub(1),
                results: vec![web_result("https://example.com/old", now.saturating_sub(1))],
            },
        )
        .unwrap();
        let before = fs::read_to_string(&cache_path).unwrap();

        let report = compact_web_search_storage(
            cache_path.clone(),
            result_log_path,
            1_000,
            DEFAULT_CACHE_MAX_ENTRIES,
            DEFAULT_RESULT_LOG_MAX_ENTRIES,
            0,
            WebSearchStorageCompactionOptions {
                dry_run: true,
                min_entries: 0,
            },
        )
        .unwrap();

        assert!(report.dry_run);
        assert!(!report.skipped);
        assert_eq!(report.cache_before, report.cache_after);
        assert!(report.cache_projected_after.bytes < report.cache_before.bytes);
        assert!(report.cache_projected_after.entries < report.cache_before.entries);
        assert_eq!(fs::read_to_string(cache_path).unwrap(), before);
    }

    #[test]
    fn compact_web_search_storage_min_entries_skips_small_artifacts() {
        let dir = tempfile::tempdir().unwrap();
        let cache_path = dir.path().join("web-cache.jsonl");
        let result_log_path = dir.path().join("brave-results.jsonl");
        let now = now_unix();
        append_cache_entry(
            &cache_path,
            &CachedWebSearch {
                query: "query".to_owned(),
                normalized_query: "query".to_owned(),
                provider: "brave".to_owned(),
                fetched_at_unix: now,
                results: vec![web_result("https://example.com/new", now)],
            },
        )
        .unwrap();

        let report = compact_web_search_storage(
            cache_path,
            result_log_path,
            1_000,
            DEFAULT_CACHE_MAX_ENTRIES,
            DEFAULT_RESULT_LOG_MAX_ENTRIES,
            0,
            WebSearchStorageCompactionOptions {
                dry_run: false,
                min_entries: 2,
            },
        )
        .unwrap();

        assert!(!report.dry_run);
        assert!(report.skipped);
        assert_eq!(report.cache_before, report.cache_after);
        assert_eq!(report.cache_before, report.cache_projected_after);
    }

    #[test]
    fn compact_web_search_storage_caps_result_log_entries_per_query_when_configured() {
        let dir = tempfile::tempdir().unwrap();
        let cache_path = dir.path().join("web-cache.jsonl");
        let result_log_path = dir.path().join("brave-results.jsonl");
        {
            let mut file = fs::File::create(&result_log_path).unwrap();
            for (query, base, count) in [
                ("heavy query", "https://example.com/heavy", 4usize),
                ("light query", "https://example.com/light", 2usize),
            ] {
                for index in 0..count {
                    let entry = WebSearchResultLogEntry {
                        query: query.to_owned(),
                        normalized_query: query.to_owned(),
                        provider: "brave".to_owned(),
                        fetched_at_unix: 100 + index as u64,
                        rank: index + 1,
                        title: format!("{query} {index}"),
                        url: format!("{base}/{index}"),
                        snippet: "Snippet".to_owned(),
                        score: 1.0,
                    };
                    serde_json::to_writer(&mut file, &entry).unwrap();
                    file.write_all(b"\n").unwrap();
                }
            }
            file.flush().unwrap();
        }

        let report = compact_web_search_storage(
            cache_path,
            result_log_path.clone(),
            1_000,
            DEFAULT_CACHE_MAX_ENTRIES,
            DEFAULT_RESULT_LOG_MAX_ENTRIES,
            2,
            WebSearchStorageCompactionOptions::default(),
        )
        .unwrap();

        let lines = fs::read_to_string(result_log_path).unwrap();
        assert_eq!(report.result_log_before.entries, 6);
        assert_eq!(report.result_log_after.entries, 4);
        assert!(!lines.contains("https://example.com/heavy/0"));
        assert!(!lines.contains("https://example.com/heavy/1"));
        assert!(lines.contains("https://example.com/heavy/2"));
        assert!(lines.contains("https://example.com/heavy/3"));
        assert!(lines.contains("https://example.com/light/0"));
        assert!(lines.contains("https://example.com/light/1"));
    }

    #[test]
    fn append_result_log_writes_one_line_per_returned_result() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("brave-results.jsonl");
        append_result_log(
            &path,
            "Query",
            "query",
            "brave",
            &[
                web_result("https://example.com/one", 100),
                web_result("https://example.com/two", 101),
            ],
            DEFAULT_RESULT_LOG_MAX_ENTRIES,
        )
        .unwrap();

        let lines = fs::read_to_string(path).unwrap();
        assert_eq!(lines.lines().count(), 2);
        assert!(lines.contains(r#""query":"Query""#));
        assert!(lines.contains(r#""normalized_query":"query""#));
        assert!(lines.contains(r#""rank":1"#));
        assert!(lines.contains("https://example.com/one"));
        assert!(lines.contains(r#""rank":2"#));
        assert!(lines.contains("https://example.com/two"));
    }

    #[test]
    fn append_result_log_keeps_only_newest_max_entries() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("brave-results.jsonl");
        append_result_log(
            &path,
            "Old Query",
            "old query",
            "brave",
            &[
                web_result("https://example.com/one", 100),
                web_result("https://example.com/two", 101),
            ],
            3,
        )
        .unwrap();
        append_result_log(
            &path,
            "New Query",
            "new query",
            "brave",
            &[
                web_result("https://example.com/three", 102),
                web_result("https://example.com/four", 103),
            ],
            3,
        )
        .unwrap();

        let lines = fs::read_to_string(path).unwrap();
        assert_eq!(lines.lines().count(), 3);
        assert!(!lines.contains("https://example.com/one"));
        assert!(lines.contains("https://example.com/two"));
        assert!(lines.contains("https://example.com/three"));
        assert!(lines.contains("https://example.com/four"));
    }

    #[test]
    fn append_result_log_dedupes_repeated_returned_results() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("brave-results.jsonl");
        let results = [
            web_result("https://example.com/one", 100),
            web_result("https://example.com/two", 101),
        ];

        append_result_log(
            &path,
            "Repeated Query",
            "repeated query",
            "brave",
            &results,
            DEFAULT_RESULT_LOG_MAX_ENTRIES,
        )
        .unwrap();
        append_result_log(
            &path,
            "Repeated Query",
            "repeated query",
            "brave",
            &results,
            DEFAULT_RESULT_LOG_MAX_ENTRIES,
        )
        .unwrap();

        let lines = fs::read_to_string(path).unwrap();
        assert_eq!(lines.lines().count(), 2);
        assert_eq!(lines.matches("https://example.com/one").count(), 1);
        assert_eq!(lines.matches("https://example.com/two").count(), 1);
    }

    #[tokio::test]
    async fn cache_only_provider_serves_cached_results_without_fetching() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("web-cache.jsonl");
        let now = now_unix();
        append_cache_entry(
            &path,
            &CachedWebSearch {
                query: "Cached Query".to_owned(),
                normalized_query: "cached query".to_owned(),
                provider: "brave".to_owned(),
                fetched_at_unix: now,
                results: vec![web_result("https://example.com/cached", now)],
            },
        )
        .unwrap();

        let service = WebSearchService {
            client: reqwest::Client::builder().build().unwrap(),
            config: WebSearchConfig {
                provider: ThirdPartySearchProvider::CacheOnly,
                cache_path: path.clone(),
                result_log_path: dir.path().join("brave-results.jsonl"),
                result_log_max_entries: DEFAULT_RESULT_LOG_MAX_ENTRIES,
                cache_ttl_secs: 60,
                min_local_results: DEFAULT_MIN_LOCAL_RESULTS,
                max_results: DEFAULT_MAX_WEB_RESULTS,
                country: "us".to_owned(),
                search_lang: "en".to_owned(),
            },
            cache: std::sync::Arc::new(tokio::sync::Mutex::new(
                WebResultCache::load(path, 60, DEFAULT_CACHE_MAX_ENTRIES).unwrap(),
            )),
        };

        let hit = service.search("  Cached   Query ", 10).await.unwrap();
        assert_eq!(hit.provider, "brave-cache");
        assert!(hit.cache_hit);
        assert!(!hit.fetched);
        assert_eq!(hit.results.len(), 1);
        assert_eq!(hit.results[0].url, "https://example.com/cached");

        let miss = service.search("missing", 10).await.unwrap();
        assert_eq!(miss.provider, "brave-cache");
        assert!(!miss.cache_hit);
        assert!(!miss.fetched);
        assert!(miss.results.is_empty());
    }

    fn web_result(url: &str, fetched_at_unix: u64) -> WebSearchResult {
        WebSearchResult {
            title: "Title".to_owned(),
            url: url.to_owned(),
            snippet: "Snippet".to_owned(),
            score: 1.0,
            fetched_at_unix,
            provider: "brave".to_owned(),
        }
    }
}
