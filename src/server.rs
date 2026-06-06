mod browser_sessions;

use std::cmp::Ordering;
use std::collections::HashSet;
use std::fmt::Write as _;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use reqwest::header::{ACCEPT, ACCEPT_LANGUAGE, USER_AGENT};
use serde::Serialize;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use url::form_urlencoded;

use crate::bench::{BenchStatusReport, read_bench_status};
use crate::frontier::{FrontierFailure, FrontierStats, FrontierStore, HostStats};
use crate::index::{PreloadMode, SearchIndex};
use crate::query::SearchResult;
use crate::render::render_target;
use crate::search_provider::{
    BackgroundIndexer, LocalSearchProvider, ProviderSearchResult, SearchProvider,
};
use crate::web_search::{WebSearchResult, WebSearchService};

const MAX_REQUEST_BYTES: usize = 16 * 1024;
const DEFAULT_CHROME_SEARCH_TIMING_ENDPOINT: &str = "https://www.google.com/search";
const CHROME_SEARCH_TIMING_TIMEOUT_SECS: u64 = 8;

pub async fn run_search_server(
    index_dir: PathBuf,
    addr: SocketAddr,
    preload: PreloadMode,
) -> Result<()> {
    let local = LocalSearchProvider::open(index_dir.clone(), preload)?;
    let web = WebSearchService::from_env(local.root())?;
    if let Some(web) = &web {
        eprintln!(
            "brutal-search web fallback: provider={} cache={}",
            web.provider_name(),
            web.cache_path().display()
        );
    }
    let background = web
        .as_ref()
        .and_then(|_| BackgroundIndexer::from_env(index_dir, local.clone()));
    if background.is_some() {
        eprintln!("brutal-search background crawl/index: enabled");
    }
    let state = Arc::new(ServerState {
        local,
        web,
        background,
        browser_sessions: browser_sessions::BrowserSessionRegistry::from_env(),
    });
    let listener = TcpListener::bind(addr)
        .await
        .with_context(|| format!("bind HTTP search server at {addr}"))?;
    eprintln!("brutal-search serve: http://{addr}");

    loop {
        let (stream, _) = listener.accept().await?;
        let state = Arc::clone(&state);
        tokio::spawn(async move {
            if let Err(error) = handle_connection(stream, state).await {
                eprintln!("serve connection error: {error:#}");
            }
        });
    }
}

struct ServerState {
    local: LocalSearchProvider,
    web: Option<WebSearchService>,
    background: Option<BackgroundIndexer>,
    browser_sessions: browser_sessions::BrowserSessionRegistry,
}

async fn handle_connection(mut stream: TcpStream, state: Arc<ServerState>) -> Result<()> {
    let mut buffer = vec![0; MAX_REQUEST_BYTES];
    let mut read = 0usize;

    loop {
        let n = stream.read(&mut buffer[read..]).await?;
        if n == 0 {
            return Ok(());
        }
        read += n;
        if read >= 4
            && buffer[..read]
                .windows(4)
                .any(|window| window == b"\r\n\r\n")
        {
            break;
        }
        if read == buffer.len() {
            write_response(
                &mut stream,
                431,
                "Request Header Fields Too Large",
                "text/plain; charset=utf-8",
                b"request too large",
            )
            .await?;
            return Ok(());
        }
    }

    let request = std::str::from_utf8(&buffer[..read]).context("request was not UTF-8")?;
    let response = route_request(request, &state).await;
    write_response(
        &mut stream,
        response.status,
        response.reason,
        response.content_type,
        response.body.as_bytes(),
    )
    .await?;
    Ok(())
}

async fn route_request(request: &str, state: &ServerState) -> HttpResponse {
    match parse_request_target(request) {
        Ok(target) => route_target(&target, state).await,
        Err(error) => text_response(400, "Bad Request", &error.to_string()),
    }
}

async fn route_target(target: &RequestTarget, state: &ServerState) -> HttpResponse {
    let index = state.local.current();
    match target.path.as_str() {
        "/" | "/search" => html_response(search_page()),
        "/render" => render_page(target, index.as_ref()),
        "/browser" => browser_sessions::browser_page(target, state).await,
        "/crawl" => crawl_status_page(index.as_ref()),
        "/bench" => bench_status_page(index.as_ref()),
        "/api/search" => api_search(target, state).await,
        "/api/browser-session" => browser_sessions::api_browser_session(target, state).await,
        "/api/chrome-search-timing" => api_chrome_search_timing(target).await,
        "/api/suggest" => api_suggest(target, index.as_ref()),
        "/api/spell" => api_spell(target, index.as_ref()),
        "/api/render" => api_render(target, index.as_ref()),
        "/api/stats" => api_stats(index.as_ref()),
        "/api/crawl-status" => api_crawl_status(index.as_ref()),
        "/api/bench-status" => api_bench_status(index.as_ref()),
        _ => text_response(404, "Not Found", "not found"),
    }
}

fn render_page(target: &RequestTarget, index: &SearchIndex) -> HttpResponse {
    let target_id = target
        .param("id")
        .or_else(|| target.param("target"))
        .unwrap_or_default();
    let doc_id = if let Ok(doc_id) = target_id.parse::<u32>() {
        doc_id
    } else if let Some(doc_id) = index.doc_id_for_url(&target_id) {
        doc_id
    } else {
        return text_response(404, "Not Found", "unknown document id or URL");
    };

    let Some(doc) = index.doc(doc_id) else {
        return text_response(404, "Not Found", "document not found");
    };
    let Some(text) = index.text(doc_id) else {
        return text_response(404, "Not Found", "document text not found");
    };

    let back_href = sanitized_search_return_href(target.param("from").as_deref());
    html_response(render_document_page(
        doc_id, &doc.url, &doc.title, text, &back_href,
    ))
}

async fn api_search(target: &RequestTarget, state: &ServerState) -> HttpResponse {
    let query = target.param("q").unwrap_or_default();
    let limit = target
        .param("limit")
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(20)
        .min(100);

    match state.local.search(&query, limit).await {
        Ok(local_response) => {
            let local_results: Vec<SearchResult> = local_response
                .results
                .into_iter()
                .filter_map(|result| match result {
                    ProviderSearchResult::Local(result) => Some(result),
                    ProviderSearchResult::Web(_) => None,
                })
                .collect();
            let strong_local_count = strong_local_match_count(&query, &local_results);
            let mut sources = SearchSourcesPayload {
                local_count: local_results.len(),
                web_enabled: state.web.is_some(),
                web_provider: state.web.as_ref().map(|web| web.provider_name().to_owned()),
                web_count: 0,
                web_cache_hit: false,
                web_fetched: false,
                web_error: None,
                background_crawl_enabled: state.background.is_some(),
                background_crawl_enqueued: 0,
            };
            let mut seen_urls = HashSet::new();
            for result in &local_results {
                seen_urls.insert(result.url.clone());
            }
            let mut web_results = Vec::new();

            if let Some(web) = &state.web {
                if web.should_search_web(strong_local_count, limit) {
                    let needed = limit.saturating_sub(strong_local_count).max(1);
                    match SearchProvider::search(web, &query, needed).await {
                        Ok(response) => {
                            sources.web_provider = Some(response.provider.to_owned());
                            sources.web_cache_hit = response.cache_hit;
                            sources.web_fetched = response.fetched;
                            let mut background_urls = Vec::new();
                            for result in response.results {
                                if let ProviderSearchResult::Web(result) = result {
                                    background_urls.push(result.url.clone());
                                    if seen_urls.insert(result.url.clone()) {
                                        sources.web_count += 1;
                                        web_results.push(result);
                                    }
                                }
                            }
                            if let Some(background) = &state.background {
                                sources.background_crawl_enqueued =
                                    background.enqueue_top_urls(background_urls).await;
                            }
                        }
                        Err(error) => {
                            sources.web_error = Some(error.to_string());
                        }
                    }
                }
            }
            let results = rank_search_results(
                &query,
                local_results,
                web_results,
                sources.web_cache_hit,
                limit,
            );

            json_response(
                200,
                "OK",
                &SearchPayload {
                    query,
                    sources,
                    results,
                },
            )
        }
        Err(error) => json_response(500, "Internal Server Error", &ErrorPayload::new(error)),
    }
}

async fn api_chrome_search_timing(target: &RequestTarget) -> HttpResponse {
    let query = target.param("q").unwrap_or_default();
    let payload = chrome_search_timing(&query).await;
    json_response(200, "OK", &payload)
}

async fn chrome_search_timing(query: &str) -> ChromeSearchTimingPayload {
    let normalized_query = query.trim();
    let endpoint = std::env::var("BRUTAL_CHROME_SEARCH_TIMING_URL")
        .unwrap_or_else(|_| DEFAULT_CHROME_SEARCH_TIMING_ENDPOINT.to_owned());
    let url = chrome_search_timing_url(&endpoint, normalized_query);
    if normalized_query.is_empty() {
        return ChromeSearchTimingPayload::error(
            "chrome-like-google-search",
            url,
            0.0,
            "empty query",
        );
    }

    let started = Instant::now();
    let client = match reqwest::Client::builder()
        .timeout(Duration::from_secs(CHROME_SEARCH_TIMING_TIMEOUT_SECS))
        .redirect(reqwest::redirect::Policy::limited(5))
        .build()
    {
        Ok(client) => client,
        Err(error) => {
            return ChromeSearchTimingPayload::error(
                "chrome-like-google-search",
                url,
                elapsed_ms(started),
                error,
            );
        }
    };

    let response = match client
        .get(&url)
        .header(
            USER_AGENT,
            "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 \
             (KHTML, like Gecko) Chrome/136.0.0.0 Safari/537.36",
        )
        .header(
            ACCEPT,
            "text/html,application/xhtml+xml,application/xml;q=0.9,image/avif,image/webp,*/*;q=0.8",
        )
        .header(ACCEPT_LANGUAGE, "en-US,en;q=0.9")
        .send()
        .await
    {
        Ok(response) => response,
        Err(error) => {
            return ChromeSearchTimingPayload::error(
                "chrome-like-google-search",
                url,
                elapsed_ms(started),
                error,
            );
        }
    };

    let status = response.status();
    match response.bytes().await {
        Ok(bytes) => ChromeSearchTimingPayload {
            provider: "chrome-like-google-search".to_owned(),
            url,
            elapsed_ms: elapsed_ms(started),
            ok: status.is_success(),
            status: Some(status.as_u16()),
            bytes: Some(bytes.len()),
            error: None,
        },
        Err(error) => ChromeSearchTimingPayload::error(
            "chrome-like-google-search",
            url,
            elapsed_ms(started),
            error,
        )
        .with_status(status.as_u16()),
    }
}

fn chrome_search_timing_url(endpoint: &str, query: &str) -> String {
    if endpoint.contains("{q}") {
        let encoded_query: String = form_urlencoded::byte_serialize(query.as_bytes()).collect();
        return endpoint.replace("{q}", &encoded_query);
    }
    let mut query_params = form_urlencoded::Serializer::new(String::new());
    query_params.append_pair("q", query);
    let separator = if endpoint.contains('?') {
        if endpoint.ends_with('?') || endpoint.ends_with('&') {
            ""
        } else {
            "&"
        }
    } else {
        "?"
    };
    format!("{endpoint}{separator}{}", query_params.finish())
}

fn elapsed_ms(started: Instant) -> f64 {
    started.elapsed().as_secs_f64() * 1000.0
}

fn api_suggest(target: &RequestTarget, index: &SearchIndex) -> HttpResponse {
    let prefix = target.param("q").unwrap_or_default();
    let limit = target
        .param("limit")
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(10)
        .min(50);
    json_response(
        200,
        "OK",
        &SuggestPayload {
            prefix: prefix.clone(),
            suggestions: index.suggest(&prefix, limit),
        },
    )
}

fn api_spell(target: &RequestTarget, index: &SearchIndex) -> HttpResponse {
    let term = target.param("q").unwrap_or_default();
    let limit = target
        .param("limit")
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(5)
        .min(20);
    json_response(
        200,
        "OK",
        &SpellPayload {
            term: term.clone(),
            corrections: index.spellcheck(&term, limit),
        },
    )
}

fn api_render(target: &RequestTarget, index: &SearchIndex) -> HttpResponse {
    let target_id = target
        .param("id")
        .or_else(|| target.param("target"))
        .unwrap_or_default();
    match render_target(index, &target_id) {
        Ok(text) => json_response(200, "OK", &RenderPayload { text }),
        Err(error) => json_response(404, "Not Found", &ErrorPayload::new(error)),
    }
}

fn api_stats(index: &SearchIndex) -> HttpResponse {
    let manifest = index.manifest();
    json_response(
        200,
        "OK",
        &StatsPayload {
            doc_count: manifest.doc_count,
            term_count: manifest.term_count,
            total_terms: manifest.total_terms,
            avg_doc_len: manifest.avg_doc_len,
            duplicate_cluster_count: manifest.duplicate_cluster_count,
            duplicate_doc_count: manifest.duplicate_doc_count,
            skipped_noindex_count: manifest.skipped_noindex_count,
            skipped_thin_count: manifest.skipped_thin_count,
            max_authority_score: manifest.max_authority_score,
            corpus_hash: manifest.corpus_hash.clone(),
        },
    )
}

fn api_crawl_status(index: &SearchIndex) -> HttpResponse {
    match crawl_status(index) {
        Ok(payload) => json_response(200, "OK", &payload),
        Err(error) => json_response(500, "Internal Server Error", &ErrorPayload::new(error)),
    }
}

fn crawl_status_page(index: &SearchIndex) -> HttpResponse {
    match crawl_status(index) {
        Ok(payload) => html_response(render_crawl_status_page(&payload)),
        Err(error) => text_response(500, "Internal Server Error", &error.to_string()),
    }
}

fn api_bench_status(index: &SearchIndex) -> HttpResponse {
    match bench_status(index) {
        Ok(payload) => json_response(200, "OK", &payload),
        Err(error) => json_response(500, "Internal Server Error", &ErrorPayload::new(error)),
    }
}

fn bench_status_page(index: &SearchIndex) -> HttpResponse {
    match bench_status(index) {
        Ok(payload) => html_response(render_bench_status_page(&payload)),
        Err(error) => text_response(500, "Internal Server Error", &error.to_string()),
    }
}

fn bench_status(index: &SearchIndex) -> Result<BenchStatusPayload> {
    let report = read_bench_status(index.root())?;
    Ok(BenchStatusPayload {
        report_exists: report.is_some(),
        report,
    })
}

fn crawl_status(index: &SearchIndex) -> Result<CrawlStatusPayload> {
    let frontier_path = index.root().join("frontier.bin");
    let snapshot_path = index.root().join("crawl-docs.jsonl");
    let frontier_exists = frontier_path.exists();
    let snapshot_exists = snapshot_path.exists();
    let snapshot_doc_count = count_snapshot_documents(&snapshot_path)?;

    let (frontier, mut hosts, failures) = if frontier_exists {
        let frontier = FrontierStore::open(&frontier_path)?;
        (
            frontier.stats(),
            frontier.host_stats(),
            frontier.failure_samples(25),
        )
    } else {
        (FrontierStats::default(), Vec::new(), Vec::new())
    };

    hosts.sort_by(|left, right| {
        right
            .total
            .cmp(&left.total)
            .then_with(|| left.host.cmp(&right.host))
    });
    hosts.truncate(20);

    Ok(CrawlStatusPayload {
        frontier_exists,
        snapshot_exists,
        snapshot_doc_count,
        frontier,
        hosts,
        failures,
    })
}

fn count_snapshot_documents(path: &std::path::Path) -> Result<usize> {
    if !path.exists() {
        return Ok(0);
    }

    let file = File::open(path).with_context(|| format!("open {}", path.display()))?;
    let reader = BufReader::new(file);
    let mut count = 0usize;
    for line in reader.lines() {
        if !line?.trim().is_empty() {
            count += 1;
        }
    }
    Ok(count)
}

async fn write_response(
    stream: &mut TcpStream,
    status: u16,
    reason: &str,
    content_type: &str,
    body: &[u8],
) -> Result<()> {
    let header = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(header.as_bytes()).await?;
    stream.write_all(body).await?;
    stream.flush().await?;
    Ok(())
}

#[derive(Debug)]
struct HttpResponse {
    status: u16,
    reason: &'static str,
    content_type: &'static str,
    body: String,
}

#[derive(Debug, PartialEq, Eq)]
struct RequestTarget {
    path: String,
    params: Vec<(String, String)>,
}

impl RequestTarget {
    fn param(&self, key: &str) -> Option<String> {
        self.params
            .iter()
            .find_map(|(candidate, value)| (candidate == key).then(|| value.clone()))
    }
}

fn parse_request_target(request: &str) -> Result<RequestTarget> {
    let request_line = request.lines().next().context("missing request line")?;
    let mut parts = request_line.split_ascii_whitespace();
    let method = parts.next().context("missing method")?;
    let target = parts.next().context("missing request target")?;
    let version = parts.next().context("missing HTTP version")?;

    if method != "GET" {
        bail!("only GET is supported");
    }
    if !version.starts_with("HTTP/") {
        bail!("invalid HTTP version");
    }

    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    let path = if path.is_empty() { "/" } else { path };
    let params = form_urlencoded::parse(query.as_bytes())
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect();

    Ok(RequestTarget {
        path: path.to_owned(),
        params,
    })
}

fn json_response<T: Serialize>(status: u16, reason: &'static str, value: &T) -> HttpResponse {
    match serde_json::to_string(value) {
        Ok(body) => HttpResponse {
            status,
            reason,
            content_type: "application/json; charset=utf-8",
            body,
        },
        Err(error) => text_response(500, "Internal Server Error", &error.to_string()),
    }
}

fn html_response(body: String) -> HttpResponse {
    HttpResponse {
        status: 200,
        reason: "OK",
        content_type: "text/html; charset=utf-8",
        body,
    }
}

fn text_response(status: u16, reason: &'static str, body: &str) -> HttpResponse {
    HttpResponse {
        status,
        reason,
        content_type: "text/plain; charset=utf-8",
        body: body.to_owned(),
    }
}

#[derive(Debug, Serialize)]
struct SearchPayload {
    query: String,
    sources: SearchSourcesPayload,
    results: Vec<SearchApiResult>,
}

#[derive(Debug, Serialize)]
struct SearchSourcesPayload {
    local_count: usize,
    web_enabled: bool,
    web_provider: Option<String>,
    web_count: usize,
    web_cache_hit: bool,
    web_fetched: bool,
    web_error: Option<String>,
    background_crawl_enabled: bool,
    background_crawl_enqueued: usize,
}

#[derive(Debug, Serialize)]
struct ChromeSearchTimingPayload {
    provider: String,
    url: String,
    elapsed_ms: f64,
    ok: bool,
    status: Option<u16>,
    bytes: Option<usize>,
    error: Option<String>,
}

impl ChromeSearchTimingPayload {
    fn error(
        provider: impl Into<String>,
        url: impl Into<String>,
        elapsed_ms: f64,
        error: impl std::fmt::Display,
    ) -> Self {
        Self {
            provider: provider.into(),
            url: url.into(),
            elapsed_ms,
            ok: false,
            status: None,
            bytes: None,
            error: Some(error.to_string()),
        }
    }

    fn with_status(mut self, status: u16) -> Self {
        self.status = Some(status);
        self
    }
}

#[derive(Debug, Serialize)]
struct SearchApiResult {
    doc_id: Option<u32>,
    url: String,
    canonical_url: Option<String>,
    title: String,
    language: Option<String>,
    fetched_at_unix: Option<u64>,
    score: f32,
    authority_score: f32,
    snippet: String,
    duplicate_of: Option<u32>,
    duplicate_count: u32,
    source: String,
    render_url: Option<String>,
}

impl SearchApiResult {
    fn from_local(result: SearchResult) -> Self {
        let render_url = Some(browser_render_url(&result.url));
        Self {
            render_url,
            doc_id: Some(result.doc_id),
            url: result.url,
            canonical_url: result.canonical_url,
            title: result.title,
            language: result.language,
            fetched_at_unix: result.fetched_at_unix,
            score: result.score,
            authority_score: result.authority_score,
            snippet: result.snippet,
            duplicate_of: Some(result.duplicate_of),
            duplicate_count: result.duplicate_count,
            source: "local".to_owned(),
        }
    }

    fn from_web(result: WebSearchResult, cache_hit: bool) -> Self {
        let source = if cache_hit {
            format!("{} cache", result.provider)
        } else {
            result.provider.clone()
        };
        let render_url = Some(browser_render_url(&result.url));
        Self {
            doc_id: None,
            url: result.url,
            canonical_url: None,
            title: result.title,
            language: None,
            fetched_at_unix: Some(result.fetched_at_unix),
            score: result.score,
            authority_score: 0.0,
            snippet: result.snippet,
            duplicate_of: None,
            duplicate_count: 1,
            source,
            render_url,
        }
    }
}

fn browser_render_url(target_url: &str) -> String {
    browser_render_url_with_return(target_url, None)
}

fn browser_render_url_with_return(target_url: &str, return_href: Option<&str>) -> String {
    let mut query = form_urlencoded::Serializer::new(String::new());
    query.append_pair("url", target_url);
    if let Some(return_href) = return_href {
        query.append_pair("from", return_href);
    }
    format!("/browser?{}", query.finish())
}

fn sanitized_search_return_href(return_href: Option<&str>) -> String {
    let Some(return_href) = return_href else {
        return "/search".to_owned();
    };
    let clean = return_href.trim();
    if clean == "/search" || clean.starts_with("/search?") {
        clean.to_owned()
    } else {
        "/search".to_owned()
    }
}

#[derive(Debug)]
struct RankedSearchResult {
    result: SearchApiResult,
    score: f32,
    provider_rank: usize,
    source_priority: u8,
}

fn rank_search_results(
    query: &str,
    local_results: Vec<SearchResult>,
    web_results: Vec<WebSearchResult>,
    web_cache_hit: bool,
    limit: usize,
) -> Vec<SearchApiResult> {
    if limit == 0 {
        return Vec::new();
    }
    let terms = ranking_query_terms(query);
    let phrase = terms.join(" ");
    let mut ranked = Vec::with_capacity(local_results.len() + web_results.len());

    for (provider_rank, result) in local_results.into_iter().enumerate() {
        let score = relevance_score(
            &terms,
            &phrase,
            &result.title,
            &result.url,
            &result.snippet,
            result.score,
            result.authority_score,
            provider_rank,
        );
        ranked.push(RankedSearchResult {
            result: SearchApiResult::from_local(result),
            score,
            provider_rank,
            source_priority: 0,
        });
    }

    for (provider_rank, result) in web_results.into_iter().enumerate() {
        let score = relevance_score(
            &terms,
            &phrase,
            &result.title,
            &result.url,
            &result.snippet,
            0.0,
            0.0,
            provider_rank,
        );
        ranked.push(RankedSearchResult {
            result: SearchApiResult::from_web(result, web_cache_hit),
            score,
            provider_rank,
            source_priority: 1,
        });
    }

    ranked.sort_by(|left, right| {
        right
            .score
            .partial_cmp(&left.score)
            .unwrap_or(Ordering::Equal)
            .then_with(|| left.source_priority.cmp(&right.source_priority))
            .then_with(|| left.provider_rank.cmp(&right.provider_rank))
            .then_with(|| left.result.url.cmp(&right.result.url))
    });
    ranked
        .into_iter()
        .take(limit)
        .map(|ranked| ranked.result)
        .collect()
}

fn strong_local_match_count(query: &str, results: &[SearchResult]) -> usize {
    let terms = ranking_query_terms(query);
    if terms.is_empty() {
        return 0;
    }
    results
        .iter()
        .filter(|result| {
            field_contains_all_terms(&result.title, &terms)
                || field_contains_all_terms(&result.url, &terms)
        })
        .count()
}

fn relevance_score(
    terms: &[String],
    phrase: &str,
    title: &str,
    url: &str,
    snippet: &str,
    raw_score: f32,
    authority_score: f32,
    provider_rank: usize,
) -> f32 {
    if terms.is_empty() {
        return 1.0 / (60.0 + provider_rank as f32);
    }
    let title_match = term_match_fraction(title, terms);
    let url_match = term_match_fraction(url, terms);
    let snippet_match = term_match_fraction(snippet, terms);
    let phrase_boost = if !phrase.is_empty()
        && (contains_case_insensitive(title, phrase) || contains_case_insensitive(url, phrase))
    {
        0.08
    } else {
        0.0
    };
    let local_score_boost = raw_score.clamp(0.0, 6.0) / 100.0;
    let authority_boost = authority_score.clamp(0.0, 1.0) * 0.02;
    let provider_rank_score = 1.0 / (60.0 + provider_rank as f32);

    provider_rank_score
        + title_match * 0.12
        + url_match * 0.05
        + snippet_match * 0.02
        + phrase_boost
        + local_score_boost
        + authority_boost
}

fn ranking_query_terms(query: &str) -> Vec<String> {
    let mut terms = Vec::new();
    for token in query.split_whitespace() {
        let lower = token.to_ascii_lowercase();
        if lower.starts_with("site:")
            || lower.starts_with("type:")
            || lower.starts_with("filetype:")
            || lower.starts_with("lang:")
            || lower.starts_with("after:")
            || lower.starts_with("before:")
        {
            continue;
        }
        for term in lower
            .split(|ch: char| !ch.is_ascii_alphanumeric())
            .filter(|term| !term.is_empty())
        {
            terms.push(term.to_owned());
        }
    }
    terms.sort();
    terms.dedup();
    terms
}

fn term_match_fraction(field: &str, terms: &[String]) -> f32 {
    if terms.is_empty() {
        return 0.0;
    }
    let tokens = field_terms(field);
    if tokens.is_empty() {
        return 0.0;
    }
    let matches = terms
        .iter()
        .filter(|term| tokens.iter().any(|token| token == *term))
        .count();
    matches as f32 / terms.len() as f32
}

fn field_contains_all_terms(field: &str, terms: &[String]) -> bool {
    let tokens = field_terms(field);
    !tokens.is_empty()
        && terms
            .iter()
            .all(|term| tokens.iter().any(|token| token == term))
}

fn field_terms(field: &str) -> Vec<String> {
    field
        .to_ascii_lowercase()
        .split(|ch: char| !ch.is_ascii_alphanumeric())
        .filter(|term| !term.is_empty())
        .map(str::to_owned)
        .collect()
}

fn contains_case_insensitive(field: &str, needle: &str) -> bool {
    field
        .to_ascii_lowercase()
        .contains(&needle.to_ascii_lowercase())
}

#[derive(Debug, Serialize)]
struct SuggestPayload {
    prefix: String,
    suggestions: Vec<crate::index::TermSuggestion>,
}

#[derive(Debug, Serialize)]
struct SpellPayload {
    term: String,
    corrections: Vec<crate::index::TermCorrection>,
}

#[derive(Debug, Serialize)]
struct RenderPayload {
    text: String,
}

#[derive(Debug, Serialize)]
struct StatsPayload {
    doc_count: u32,
    term_count: u32,
    total_terms: u64,
    avg_doc_len: f32,
    duplicate_cluster_count: u32,
    duplicate_doc_count: u32,
    skipped_noindex_count: u32,
    skipped_thin_count: u32,
    max_authority_score: f32,
    corpus_hash: String,
}

#[derive(Debug, Serialize)]
struct CrawlStatusPayload {
    frontier_exists: bool,
    snapshot_exists: bool,
    snapshot_doc_count: usize,
    frontier: FrontierStats,
    hosts: Vec<HostStats>,
    failures: Vec<FrontierFailure>,
}

#[derive(Debug, Serialize)]
struct BenchStatusPayload {
    report_exists: bool,
    report: Option<BenchStatusReport>,
}

#[derive(Debug, Serialize)]
struct ErrorPayload {
    error: String,
}

impl ErrorPayload {
    fn new(error: impl std::fmt::Display) -> Self {
        Self {
            error: error.to_string(),
        }
    }
}

fn render_document_page(
    doc_id: u32,
    url: &str,
    title: &str,
    text: &str,
    back_href: &str,
) -> String {
    let display_title = if title.trim().is_empty() { url } else { title };
    format!(
        r#"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>{title}</title>
<style>
:root {{ color-scheme: light; font-family: ui-sans-serif, system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif; }}
body {{ margin: 0; background: #f7f7f5; color: #191a1c; }}
main {{ max-width: 980px; margin: 0 auto; padding: 28px 18px 56px; }}
a {{ color: #123fae; text-decoration: none; font-weight: 700; }}
a:hover {{ text-decoration: underline; }}
h1 {{ margin: 16px 0 6px; font-size: 24px; letter-spacing: 0; }}
.meta {{ color: #5d636b; font-size: 13px; overflow-wrap: anywhere; }}
pre {{ white-space: pre-wrap; background: #fff; border: 1px solid #dfe2e6; border-radius: 6px; padding: 16px; line-height: 1.48; overflow: auto; }}
</style>
</head>
<body>
<main>
<a href="{back_href}">Back to search</a>
<h1>{heading}</h1>
<div class="meta">doc {doc_id} · {url}</div>
<pre>{text}</pre>
</main>
</body>
</html>"#,
        title = html_escape::encode_text(display_title),
        heading = html_escape::encode_text(display_title),
        back_href = html_escape::encode_double_quoted_attribute(back_href),
        url = html_escape::encode_text(url),
        text = html_escape::encode_text(text),
    )
}

fn render_crawl_status_page(payload: &CrawlStatusPayload) -> String {
    let frontier = &payload.frontier;
    let frontier_file = if payload.frontier_exists {
        "present"
    } else {
        "missing"
    };
    let snapshot_file = if payload.snapshot_exists {
        "present"
    } else {
        "missing"
    };
    let mut host_rows = String::new();
    let mut failure_rows = String::new();

    if payload.hosts.is_empty() {
        host_rows.push_str(
            r#"<tr><td colspan="7" class="empty">No host frontier records yet.</td></tr>"#,
        );
    } else {
        for host in &payload.hosts {
            let _ = write!(
                host_rows,
                r#"<tr><td>{host}</td><td>{total}</td><td>{queued}</td><td>{fetching}</td><td>{fetched}</td><td>{failed}</td><td>{deferred}</td></tr>"#,
                host = html_escape::encode_text(&host.host),
                total = host.total,
                queued = host.queued,
                fetching = host.fetching,
                fetched = host.fetched,
                failed = host.failed,
                deferred = host.deferred,
            );
        }
    }

    if payload.failures.is_empty() {
        failure_rows
            .push_str(r#"<tr><td colspan="4" class="empty">No failed frontier records.</td></tr>"#);
    } else {
        for failure in &payload.failures {
            let reason = failure.reason.as_deref().unwrap_or("unknown");
            let status = failure
                .status_code
                .map(|status| status.to_string())
                .unwrap_or_else(|| "-".to_owned());
            let _ = write!(
                failure_rows,
                r#"<tr><td>{host}</td><td>{status}</td><td>{reason}</td><td>{url}</td></tr>"#,
                host = html_escape::encode_text(&failure.host),
                status = html_escape::encode_text(&status),
                reason = html_escape::encode_text(reason),
                url = html_escape::encode_text(&failure.url),
            );
        }
    }

    format!(
        r#"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Blackium Starium✴ Crawl Status</title>
<style>
:root {{ color-scheme: light; font-family: ui-sans-serif, system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif; }}
body {{ margin: 0; background: #f7f7f5; color: #191a1c; }}
main {{ max-width: 1080px; margin: 0 auto; padding: 28px 18px 56px; }}
a {{ color: #123fae; text-decoration: none; font-weight: 700; }}
a:hover {{ text-decoration: underline; }}
header {{ display: flex; align-items: baseline; justify-content: space-between; gap: 16px; margin-bottom: 18px; }}
h1 {{ margin: 0; font-size: 26px; font-weight: 750; letter-spacing: 0; }}
.status {{ color: #5d636b; font-size: 13px; }}
.summary {{ display: grid; grid-template-columns: repeat(auto-fit, minmax(140px, 1fr)); gap: 10px; margin: 18px 0 22px; }}
.metric {{ background: #fff; border: 1px solid #dfe2e6; border-radius: 6px; padding: 12px; min-width: 0; }}
.metric strong {{ display: block; font-size: 22px; line-height: 1.1; overflow-wrap: anywhere; }}
.metric span {{ display: block; margin-top: 5px; color: #5d636b; font-size: 12px; text-transform: uppercase; letter-spacing: .04em; }}
table {{ width: 100%; border-collapse: collapse; table-layout: fixed; background: #fff; border: 1px solid #dfe2e6; }}
th, td {{ border-bottom: 1px solid #e7e9ed; padding: 10px 9px; font-size: 13px; text-align: right; }}
th {{ color: #505760; background: #f0f2f5; font-weight: 750; }}
th:first-child, td:first-child {{ text-align: left; width: 34%; overflow-wrap: anywhere; }}
tr:last-child td {{ border-bottom: 0; }}
.section-heading {{ margin: 24px 0 8px; font-size: 15px; }}
.failure-table th, .failure-table td {{ text-align: left; overflow-wrap: anywhere; }}
.failure-table th:nth-child(2), .failure-table td:nth-child(2) {{ width: 80px; text-align: right; }}
.failure-table th:nth-child(3), .failure-table td:nth-child(3) {{ width: 28%; }}
.empty {{ color: #5d636b; text-align: left; }}
</style>
</head>
<body>
<main>
<header><h1>Blackium Starium✴ Crawl Status</h1><a href="/search">Back to search</a></header>
<div class="status">frontier file: {frontier_file} | snapshot file: {snapshot_file}</div>
<section class="summary" aria-label="Crawl totals">
<div class="metric"><strong>{total}</strong><span>Total URLs</span></div>
<div class="metric"><strong>{fetched}</strong><span>Fetched</span></div>
<div class="metric"><strong>{queued}</strong><span>Queued</span></div>
<div class="metric"><strong>{deferred}</strong><span>Deferred</span></div>
<div class="metric"><strong>{failed}</strong><span>Failed</span></div>
<div class="metric"><strong>{snapshot_docs}</strong><span>Snapshotted Docs</span></div>
</section>
<table aria-label="Host crawl status">
<thead><tr><th>Host</th><th>Total</th><th>Queued</th><th>Fetching</th><th>Fetched</th><th>Failed</th><th>Deferred</th></tr></thead>
<tbody>{host_rows}</tbody>
</table>
<h2 class="section-heading">Recent Failures</h2>
<table class="failure-table" aria-label="Recent crawl failures">
<thead><tr><th>Host</th><th>Status</th><th>Reason</th><th>URL</th></tr></thead>
<tbody>{failure_rows}</tbody>
</table>
</main>
</body>
</html>"#,
        frontier_file = frontier_file,
        snapshot_file = snapshot_file,
        total = frontier.total,
        fetched = frontier.fetched,
        queued = frontier.queued,
        deferred = frontier.deferred,
        failed = frontier.failed,
        snapshot_docs = payload.snapshot_doc_count,
        host_rows = host_rows,
        failure_rows = failure_rows,
    )
}

fn render_bench_status_page(payload: &BenchStatusPayload) -> String {
    let mut report_html = String::new();
    match &payload.report {
        Some(BenchStatusReport::Search(report)) => {
            report_html.push_str(&bench_report_table("Rust Search Benchmark", report));
        }
        Some(BenchStatusReport::Comparison(comparison)) => {
            let gate_text = match comparison.passed {
                Some(true) => "passed",
                Some(false) => "failed",
                None => "not required",
            };
            let required = comparison
                .required_p95_speedup
                .map(|value| format!("{value:.2}x"))
                .unwrap_or_else(|| "none".to_owned());
            let _ = write!(
                report_html,
                r#"<section class="summary" aria-label="Benchmark gate">
<div class="metric"><strong>{speedup:.2}x</strong><span>p95 speedup</span></div>
<div class="metric"><strong>{gate}</strong><span>Gate</span></div>
<div class="metric"><strong>{required}</strong><span>Required</span></div>
</section>"#,
                speedup = comparison.p95_speedup,
                gate = html_escape::encode_text(gate_text),
                required = html_escape::encode_text(&required),
            );
            report_html.push_str(&bench_report_table(
                "Rust Search Benchmark",
                &comparison.rust,
            ));
            report_html.push_str(&bench_report_table(
                "Chromium JS Baseline",
                &comparison.chromium,
            ));
        }
        Some(BenchStatusReport::Smoke(report)) => {
            let _ = write!(
                report_html,
                r#"<section class="summary" aria-label="Smoke benchmark">
<div class="metric"><strong>{docs}</strong><span>Docs</span></div>
<div class="metric"><strong>{terms}</strong><span>Terms</span></div>
<div class="metric"><strong>{results}</strong><span>Results</span></div>
<div class="metric"><strong>{bytes}</strong><span>Rendered Bytes</span></div>
</section>
<table aria-label="Smoke inputs">
<tbody>
<tr><th>corpus</th><td>{corpus}</td></tr>
<tr><th>index</th><td>{index}</td></tr>
<tr><th>queries</th><td>{queries}</td></tr>
<tr><th>query</th><td>{query}</td></tr>
<tr><th>top_doc_id</th><td>{top_doc_id}</td></tr>
</tbody>
</table>"#,
                docs = report.build.doc_count,
                terms = report.build.term_count,
                results = report.result_count,
                bytes = report.rendered_bytes,
                corpus = html_escape::encode_text(&report.corpus),
                index = html_escape::encode_text(&report.index),
                queries = html_escape::encode_text(&report.queries),
                query = html_escape::encode_text(&report.query),
                top_doc_id = report.top_doc_id,
            );
            report_html.push_str(&bench_report_table("Smoke Search Benchmark", &report.bench));
        }
        Some(BenchStatusReport::Eval(report)) => {
            let gate_text = match report.passed {
                Some(true) => "passed",
                Some(false) => "failed",
                None => "not required",
            };
            let _ = write!(
                report_html,
                r#"<section class="summary" aria-label="Relevance evaluation">
<div class="metric"><strong>{queries}</strong><span>Judged Queries</span></div>
<div class="metric"><strong>{gate}</strong><span>Gate</span></div>
<div class="metric"><strong>{mrr:.4}</strong><span>MRR</span></div>
<div class="metric"><strong>{ndcg:.4}</strong><span>NDCG@K</span></div>
<div class="metric"><strong>{recall:.4}</strong><span>Recall@K</span></div>
<div class="metric"><strong>{precision:.4}</strong><span>Precision@K</span></div>
</section>"#,
                queries = report.evaluated_query_count,
                gate = html_escape::encode_text(gate_text),
                mrr = report.mean_reciprocal_rank,
                ndcg = report.mean_ndcg_at_k,
                recall = report.mean_recall_at_k,
                precision = report.mean_precision_at_k,
            );
            report_html.push_str(&eval_report_table(report));
        }
        Some(BenchStatusReport::BrowserPerf(report)) => {
            let gate_text = match report.passed {
                Some(true) => "passed",
                Some(false) => "failed",
                None => "not required",
            };
            let _ = write!(
                report_html,
                r#"<section class="summary" aria-label="Browser performance">
<div class="metric"><strong>{fixtures}</strong><span>Fixtures</span></div>
<div class="metric"><strong>{p95}</strong><span>p95 us</span></div>
<div class="metric"><strong>{raster_p95}</strong><span>Raster p95 us</span></div>
<div class="metric"><strong>{layers}</strong><span>Layers</span></div>
<div class="metric"><strong>{throughput:.2}</strong><span>Pages/Sec</span></div>
<div class="metric"><strong>{gate}</strong><span>Gate</span></div>
</section>"#,
                fixtures = report.fixture_count,
                p95 = report.p95_us,
                raster_p95 = report.raster_p95_us,
                layers = report.total_layers,
                throughput = report.throughput_pages_per_sec,
                gate = html_escape::encode_text(gate_text),
            );
            report_html.push_str(&browser_perf_table(report));
        }
        Some(BenchStatusReport::BrowserCompat(report)) => {
            let gate_text = match report.passed {
                Some(true) => "passed",
                Some(false) => "failed",
                None => "not required",
            };
            let _ = write!(
                report_html,
                r#"<section class="summary" aria-label="Browser compatibility">
<div class="metric"><strong>{selected}</strong><span>Selected Tests</span></div>
<div class="metric"><strong>{pass_rate:.4}</strong><span>Pass Rate</span></div>
<div class="metric"><strong>{unexpected}</strong><span>Unexpected</span></div>
<div class="metric"><strong>{gate}</strong><span>Gate</span></div>
</section>"#,
                selected = report.selected_count,
                pass_rate = report.pass_rate,
                unexpected = report.unexpected_count,
                gate = html_escape::encode_text(gate_text),
            );
            report_html.push_str(&browser_compat_table(report));
        }
        Some(BenchStatusReport::Gate(report)) => {
            let gate_text = if report.passed { "passed" } else { "failed" };
            let speedup = report
                .search_comparison
                .as_ref()
                .map(|comparison| format!("{:.2}x", comparison.p95_speedup))
                .unwrap_or_else(|| "skipped".to_owned());
            let parity = report
                .browser_chromium_parity
                .as_ref()
                .map(|parity| format!("{}/{}", parity.passed, parity.fixture_count))
                .unwrap_or_else(|| "skipped".to_owned());
            let compat = report
                .browser_compat
                .as_ref()
                .map(|compat| format!("{}/{}", compat.pass_count, compat.runnable_count))
                .unwrap_or_else(|| "skipped".to_owned());
            let failures = if report.failures.is_empty() {
                "none".to_owned()
            } else {
                report.failures.join("; ")
            };
            let _ = write!(
                report_html,
                r#"<section class="summary" aria-label="Competition gate">
<div class="metric"><strong>{gate}</strong><span>Gate</span></div>
<div class="metric"><strong>{speedup}</strong><span>p95 Speedup</span></div>
<div class="metric"><strong>{ndcg:.4}</strong><span>NDCG@K</span></div>
<div class="metric"><strong>{coverage:.4}</strong><span>Browser Coverage</span></div>
<div class="metric"><strong>{parity}</strong><span>Chromium Parity</span></div>
<div class="metric"><strong>{compat}</strong><span>Browser Compat</span></div>
</section>
<table aria-label="Gate failures"><tbody><tr><th>failures</th><td>{failures}</td></tr></tbody></table>"#,
                gate = html_escape::encode_text(gate_text),
                speedup = html_escape::encode_text(&speedup),
                ndcg = report.eval.mean_ndcg_at_k,
                coverage = report.browser_coverage.implemented_ratio,
                parity = html_escape::encode_text(&parity),
                compat = html_escape::encode_text(&compat),
                failures = html_escape::encode_text(&failures),
            );
            if let Some(comparison) = &report.search_comparison {
                report_html.push_str(&bench_report_table(
                    "Rust Search Benchmark",
                    &comparison.rust,
                ));
                report_html.push_str(&bench_report_table(
                    "Chromium JS Baseline",
                    &comparison.chromium,
                ));
            } else {
                report_html.push_str(&bench_report_table(
                    "Smoke Search Benchmark",
                    &report.smoke.bench,
                ));
            }
            report_html.push_str(&eval_report_table(&report.eval));
            report_html.push_str(&browser_coverage_table(&report.browser_coverage));
            if let Some(parity) = &report.browser_chromium_parity {
                report_html.push_str(&browser_chromium_parity_table(parity));
            }
            if let Some(compat) = &report.browser_compat {
                report_html.push_str(&browser_compat_table(compat));
            }
        }
        None => {
            report_html.push_str(
                r#"<section class="empty-state">
<h2>No Benchmark Report</h2>
<p>Run brutal-bench with --save-report to persist the latest benchmark beside this index.</p>
</section>"#,
            );
        }
    }

    format!(
        r#"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Blackium Starium✴ Benchmark Status</title>
<style>
:root {{ color-scheme: light; font-family: ui-sans-serif, system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif; }}
body {{ margin: 0; background: #f7f7f5; color: #191a1c; }}
main {{ max-width: 1080px; margin: 0 auto; padding: 28px 18px 56px; }}
a {{ color: #123fae; text-decoration: none; font-weight: 700; }}
a:hover {{ text-decoration: underline; }}
header {{ display: flex; align-items: baseline; justify-content: space-between; gap: 16px; margin-bottom: 18px; }}
h1 {{ margin: 0; font-size: 26px; font-weight: 750; letter-spacing: 0; }}
h2 {{ margin: 24px 0 10px; font-size: 18px; letter-spacing: 0; }}
.summary {{ display: grid; grid-template-columns: repeat(auto-fit, minmax(150px, 1fr)); gap: 10px; margin: 18px 0 22px; }}
.metric {{ background: #fff; border: 1px solid #dfe2e6; border-radius: 6px; padding: 12px; min-width: 0; }}
.metric strong {{ display: block; font-size: 22px; line-height: 1.1; overflow-wrap: anywhere; }}
.metric span {{ display: block; margin-top: 5px; color: #5d636b; font-size: 12px; text-transform: uppercase; letter-spacing: .04em; }}
table {{ width: 100%; border-collapse: collapse; table-layout: fixed; background: #fff; border: 1px solid #dfe2e6; margin-bottom: 18px; }}
th, td {{ border-bottom: 1px solid #e7e9ed; padding: 10px 9px; font-size: 13px; vertical-align: top; }}
th {{ width: 190px; color: #505760; background: #f0f2f5; text-align: left; font-weight: 750; }}
td {{ overflow-wrap: anywhere; }}
tr:last-child th, tr:last-child td {{ border-bottom: 0; }}
.query-table th {{ width: auto; text-align: right; }}
.query-table th:first-child, .query-table td:first-child {{ text-align: left; width: 30%; }}
.query-table td {{ text-align: right; }}
.empty-state {{ background: #fff; border: 1px solid #dfe2e6; border-radius: 6px; padding: 16px; }}
.empty-state p {{ color: #5d636b; margin: 8px 0 0; }}
</style>
</head>
<body>
<main>
<header><h1>Blackium Starium✴ Benchmark Status</h1><a href="/search">Back to search</a></header>
{report_html}
</main>
</body>
</html>"#,
        report_html = report_html,
    )
}

fn bench_report_table(title: &str, report: &crate::bench::BenchReport) -> String {
    let mut rows = String::new();
    push_bench_row(&mut rows, "engine", &report.engine);
    push_bench_row(&mut rows, "queries", report.query_count);
    push_bench_row(&mut rows, "limit", report.limit);
    push_bench_row(&mut rows, "p50_us", report.p50_us);
    push_bench_row(&mut rows, "p95_us", report.p95_us);
    push_bench_row(&mut rows, "p99_us", report.p99_us);
    push_bench_row(
        &mut rows,
        "throughput_qps",
        format!("{:.2}", report.throughput_qps),
    );
    push_bench_row(&mut rows, "total_ms", report.total_ms);
    push_bench_row(&mut rows, "corpus_hash", &report.corpus_hash);
    push_bench_row(&mut rows, "index_hash", &report.index_hash);
    if let Some(rustc) = &report.rustc {
        push_bench_row(&mut rows, "rustc", rustc);
    }
    if let Some(chrome) = &report.chrome {
        push_bench_row(&mut rows, "chrome", chrome);
    }
    if let Some(os) = &report.os {
        push_bench_row(&mut rows, "os", os);
    }
    if let Some(hardware) = &report.hardware {
        push_bench_row(&mut rows, "hardware", hardware);
    }

    format!(
        r#"<section>
<h2>{title}</h2>
<table aria-label="{title}">
<tbody>{rows}</tbody>
</table>
</section>"#,
        title = html_escape::encode_text(title),
        rows = rows,
    )
}

fn eval_report_table(report: &crate::bench::EvalReport) -> String {
    let mut rows = String::new();
    push_bench_row(&mut rows, "query_count", report.query_count);
    push_bench_row(
        &mut rows,
        "evaluated_query_count",
        report.evaluated_query_count,
    );
    push_bench_row(&mut rows, "limit", report.limit);
    push_bench_row(
        &mut rows,
        "mean_reciprocal_rank",
        format!("{:.4}", report.mean_reciprocal_rank),
    );
    push_bench_row(
        &mut rows,
        "mean_ndcg_at_k",
        format!("{:.4}", report.mean_ndcg_at_k),
    );
    push_bench_row(
        &mut rows,
        "mean_recall_at_k",
        format!("{:.4}", report.mean_recall_at_k),
    );
    push_bench_row(
        &mut rows,
        "mean_precision_at_k",
        format!("{:.4}", report.mean_precision_at_k),
    );
    push_bench_row(
        &mut rows,
        "unresolved_judgment_count",
        report.unresolved_judgment_count,
    );
    push_optional_f64_row(&mut rows, "required_mrr", report.required_mrr);
    push_optional_f64_row(&mut rows, "required_ndcg_at_k", report.required_ndcg_at_k);
    push_optional_f64_row(
        &mut rows,
        "required_recall_at_k",
        report.required_recall_at_k,
    );
    push_optional_f64_row(
        &mut rows,
        "required_precision_at_k",
        report.required_precision_at_k,
    );
    push_bench_row(
        &mut rows,
        "max_unresolved_judgment_count",
        report
            .max_unresolved_judgment_count
            .map(|value| value.to_string())
            .unwrap_or_else(|| "none".to_owned()),
    );
    push_bench_row(
        &mut rows,
        "gate_passed",
        report
            .passed
            .map(|value| value.to_string())
            .unwrap_or_else(|| "none".to_owned()),
    );
    push_bench_row(&mut rows, "corpus_hash", &report.corpus_hash);
    push_bench_row(&mut rows, "index_hash", &report.index_hash);

    let mut query_rows = String::new();
    for query in &report.queries {
        let _ = write!(
            query_rows,
            r#"<tr><td>{query}</td><td>{relevant}</td><td>{found}</td><td>{rr:.4}</td><td>{ndcg:.4}</td><td>{recall:.4}</td><td>{precision:.4}</td><td>{unresolved}</td></tr>"#,
            query = html_escape::encode_text(&query.query),
            relevant = query.relevant_count,
            found = query.retrieved_relevant,
            rr = query.reciprocal_rank,
            ndcg = query.ndcg_at_k,
            recall = query.recall_at_k,
            precision = query.precision_at_k,
            unresolved = query.unresolved_judgment_count,
        );
    }

    format!(
        r#"<section>
<h2>Relevance Metrics</h2>
<table aria-label="Relevance metrics"><tbody>{rows}</tbody></table>
<h2>Query Diagnostics</h2>
<table class="query-table" aria-label="Query diagnostics">
<thead><tr><th>Query</th><th>Relevant</th><th>Found</th><th>RR</th><th>NDCG</th><th>Recall</th><th>Precision</th><th>Unresolved</th></tr></thead>
<tbody>{query_rows}</tbody>
</table>
</section>"#,
        rows = rows,
        query_rows = query_rows,
    )
}

fn browser_perf_table(report: &crate::bench::BrowserPerfReport) -> String {
    let mut rows = String::new();
    push_bench_row(&mut rows, "engine", &report.engine);
    push_bench_row(&mut rows, "manifest", &report.manifest);
    push_bench_row(&mut rows, "fixture_count", report.fixture_count);
    push_bench_row(&mut rows, "iteration_count", report.iteration_count);
    push_bench_row(&mut rows, "warmup", report.warmup);
    push_bench_row(&mut rows, "sample_count", report.sample_count);
    push_bench_row(&mut rows, "p50_us", report.p50_us);
    push_bench_row(&mut rows, "p95_us", report.p95_us);
    push_bench_row(&mut rows, "p99_us", report.p99_us);
    push_bench_row(&mut rows, "raster_p50_us", report.raster_p50_us);
    push_bench_row(&mut rows, "raster_p95_us", report.raster_p95_us);
    push_bench_row(&mut rows, "raster_p99_us", report.raster_p99_us);
    push_bench_row(
        &mut rows,
        "throughput_pages_per_sec",
        format!("{:.2}", report.throughput_pages_per_sec),
    );
    push_bench_row(&mut rows, "total_ms", report.total_ms);
    push_bench_row(
        &mut rows,
        "total_rendered_bytes",
        report.total_rendered_bytes,
    );
    push_bench_row(&mut rows, "total_dom_nodes", report.total_dom_nodes);
    push_bench_row(&mut rows, "total_css_rules", report.total_css_rules);
    push_bench_row(&mut rows, "total_layout_boxes", report.total_layout_boxes);
    push_bench_row(
        &mut rows,
        "total_paint_commands",
        report.total_paint_commands,
    );
    push_bench_row(&mut rows, "total_layers", report.total_layers);
    push_bench_row(&mut rows, "total_image_layers", report.total_image_layers);
    push_bench_row(&mut rows, "max_layer_count", report.max_layer_count);
    push_bench_row(
        &mut rows,
        "max_image_layer_count",
        report.max_image_layer_count,
    );
    push_bench_row(
        &mut rows,
        "max_root_layer",
        format!(
            "{}x{}",
            report.max_root_layer_width, report.max_root_layer_height
        ),
    );
    push_bench_row(&mut rows, "max_layer_area", report.max_layer_area);
    push_bench_row(&mut rows, "total_layer_area", report.total_layer_area);
    push_bench_row(
        &mut rows,
        "layer_metrics_p50_us",
        report.layer_metrics_p50_us,
    );
    push_bench_row(
        &mut rows,
        "layer_metrics_p95_us",
        report.layer_metrics_p95_us,
    );
    push_bench_row(
        &mut rows,
        "layer_metrics_p99_us",
        report.layer_metrics_p99_us,
    );
    push_bench_row(
        &mut rows,
        "total_layer_metrics_us",
        report.total_layer_metrics_us,
    );
    push_bench_row(&mut rows, "total_raster_us", report.total_raster_us);
    push_bench_row(&mut rows, "total_raster_pixels", report.total_raster_pixels);
    push_bench_row(
        &mut rows,
        "total_raster_non_background_pixels",
        report.total_raster_non_background_pixels,
    );
    push_bench_row(
        &mut rows,
        "total_raster_visible_commands",
        report.total_raster_visible_commands,
    );
    push_bench_row(
        &mut rows,
        "total_raster_culled_commands",
        report.total_raster_culled_commands,
    );
    push_browser_timing_rows(&mut rows, "phase", &report.phase_totals);
    push_bench_row(&mut rows, "suite_hash", &report.suite_hash);
    push_bench_row(
        &mut rows,
        "required_max_p95_us",
        report
            .required_max_p95_us
            .map(|value| value.to_string())
            .unwrap_or_else(|| "none".to_owned()),
    );
    push_optional_f64_row(
        &mut rows,
        "required_min_throughput_pages_per_sec",
        report.required_min_throughput_pages_per_sec,
    );
    push_optional_f64_row(
        &mut rows,
        "required_min_chromium_p95_speedup",
        report.required_min_chromium_p95_speedup,
    );
    push_bench_row(
        &mut rows,
        "required_max_chromium_text_mismatches",
        report
            .required_max_chromium_text_mismatches
            .map(|value| value.to_string())
            .unwrap_or_else(|| "none".to_owned()),
    );
    push_bench_row(
        &mut rows,
        "required_max_layer_metrics_p95_us",
        report
            .required_max_layer_metrics_p95_us
            .map(|value| value.to_string())
            .unwrap_or_else(|| "none".to_owned()),
    );
    push_bench_row(
        &mut rows,
        "required_min_total_layers",
        report
            .required_min_total_layers
            .map(|value| value.to_string())
            .unwrap_or_else(|| "none".to_owned()),
    );
    push_bench_row(
        &mut rows,
        "required_min_total_image_layers",
        report
            .required_min_total_image_layers
            .map(|value| value.to_string())
            .unwrap_or_else(|| "none".to_owned()),
    );
    push_bench_row(
        &mut rows,
        "required_max_layer_count",
        report
            .required_max_layer_count
            .map(|value| value.to_string())
            .unwrap_or_else(|| "none".to_owned()),
    );
    push_bench_row(
        &mut rows,
        "required_max_image_layer_count",
        report
            .required_max_image_layer_count
            .map(|value| value.to_string())
            .unwrap_or_else(|| "none".to_owned()),
    );
    push_bench_row(
        &mut rows,
        "gate_passed",
        report
            .passed
            .map(|value| value.to_string())
            .unwrap_or_else(|| "none".to_owned()),
    );
    if let Some(rustc) = &report.rustc {
        push_bench_row(&mut rows, "rustc", rustc);
    }
    if let Some(chrome) = &report.chrome {
        push_bench_row(&mut rows, "chrome", chrome);
    }
    if let Some(os) = &report.os {
        push_bench_row(&mut rows, "os", os);
    }
    if let Some(hardware) = &report.hardware {
        push_bench_row(&mut rows, "hardware", hardware);
    }
    if let Some(chromium) = &report.chromium_baseline {
        push_bench_row(&mut rows, "chromium_engine", &chromium.engine);
        push_bench_row(&mut rows, "chromium_sample_count", chromium.sample_count);
        push_bench_row(
            &mut rows,
            "chromium_text_match_count",
            chromium.text_match_count,
        );
        push_bench_row(
            &mut rows,
            "chromium_text_mismatch_count",
            chromium.text_mismatch_count,
        );
        push_bench_row(&mut rows, "chromium_p95_us", chromium.p95_us);
    }
    push_optional_f64_row(
        &mut rows,
        "chromium_p95_speedup",
        report.chromium_p95_speedup,
    );

    let mut fixture_rows = String::new();
    for fixture in &report.fixtures {
        let _ = write!(
            fixture_rows,
            r#"<tr><td>{name}</td><td>{p50}</td><td>{p95}</td><td>{p99}</td><td>{raster_p50}</td><td>{raster_p95}</td><td>{raster_p99}</td><td>{raster_total}</td><td>{layer_metrics_p95}</td><td>{layer_metrics_total}</td><td>{parse}</td><td>{script}</td><td>{style}</td><td>{layout_time}</td><td>{bytes}</td><td>{nodes}</td><td>{layout}</td><td>{paint}</td><td>{layers}</td><td>{image_layers}</td><td>{root_width}x{root_height}</td><td>{max_layer_area}</td><td>{total_layer_area}</td><td>{raster_pixels}</td><td>{raster_non_background}</td><td>{raster_visible}</td><td>{raster_culled}</td></tr>"#,
            name = html_escape::encode_text(&fixture.name),
            p50 = fixture.p50_us,
            p95 = fixture.p95_us,
            p99 = fixture.p99_us,
            raster_p50 = fixture.raster_p50_us,
            raster_p95 = fixture.raster_p95_us,
            raster_p99 = fixture.raster_p99_us,
            raster_total = fixture.raster_total_us,
            layer_metrics_p95 = fixture.layer_metrics_p95_us,
            layer_metrics_total = fixture.layer_metrics_total_us,
            parse = fixture.phase_totals.parse_us,
            script = fixture.phase_totals.script_us,
            style = fixture.phase_totals.style_us,
            layout_time = fixture.phase_totals.layout_us,
            bytes = fixture.rendered_bytes,
            nodes = fixture.dom_node_count,
            layout = fixture.layout_box_count,
            paint = fixture.paint_command_count,
            layers = fixture.layer_count,
            image_layers = fixture.image_layer_count,
            root_width = fixture.root_layer_width,
            root_height = fixture.root_layer_height,
            max_layer_area = fixture.max_layer_area,
            total_layer_area = fixture.total_layer_area,
            raster_pixels = fixture.raster_pixels,
            raster_non_background = fixture.raster_non_background_pixels,
            raster_visible = fixture.raster_visible_command_count,
            raster_culled = fixture.raster_culled_command_count,
        );
    }

    format!(
        r#"<section>
<h2>Browser Performance Metrics</h2>
<table aria-label="Browser performance metrics"><tbody>{rows}</tbody></table>
<h2>Fixture Timings</h2>
<table class="query-table" aria-label="Browser fixture timings">
<thead><tr><th>Fixture</th><th>p50 us</th><th>p95 us</th><th>p99 us</th><th>Raster p50 us</th><th>Raster p95 us</th><th>Raster p99 us</th><th>Raster total us</th><th>Layer metrics p95 us</th><th>Layer metrics total us</th><th>Parse us</th><th>Script us</th><th>Style us</th><th>Layout us</th><th>Bytes</th><th>DOM Nodes</th><th>Layout Boxes</th><th>Paint Commands</th><th>Layers</th><th>Image Layers</th><th>Root Layer</th><th>Max Layer Area</th><th>Total Layer Area</th><th>Raster Pixels</th><th>Raster Non-BG Pixels</th><th>Raster Visible Commands</th><th>Raster Culled Commands</th></tr></thead>
<tbody>{fixture_rows}</tbody>
</table>
</section>"#,
        rows = rows,
        fixture_rows = fixture_rows,
    )
}

fn push_browser_timing_rows(
    rows: &mut String,
    prefix: &str,
    timings: &crate::browser::BrowserRenderTimings,
) {
    push_bench_row(rows, &format!("{prefix}_parse_us"), timings.parse_us);
    push_bench_row(rows, &format!("{prefix}_script_us"), timings.script_us);
    push_bench_row(rows, &format!("{prefix}_style_us"), timings.style_us);
    push_bench_row(rows, &format!("{prefix}_collect_us"), timings.collect_us);
    push_bench_row(rows, &format!("{prefix}_layout_us"), timings.layout_us);
    push_bench_row(rows, &format!("{prefix}_total_us"), timings.total_us);
}

fn browser_coverage_table(report: &crate::browser::BrowserCoverageReport) -> String {
    let mut rows = String::new();
    push_bench_row(&mut rows, "feature_count", report.feature_count);
    push_bench_row(&mut rows, "implemented_count", report.implemented_count);
    push_bench_row(&mut rows, "partial_count", report.partial_count);
    push_bench_row(&mut rows, "missing_count", report.missing_count);
    push_bench_row(
        &mut rows,
        "implemented_ratio",
        format!("{:.4}", report.implemented_ratio),
    );
    push_bench_row(
        &mut rows,
        "required_features",
        if report.required_features.is_empty() {
            "none".to_owned()
        } else {
            report.required_features.join(", ")
        },
    );
    push_bench_row(
        &mut rows,
        "missing_required_features",
        if report.missing_required_features.is_empty() {
            "none".to_owned()
        } else {
            report.missing_required_features.join(", ")
        },
    );
    push_optional_f64_row(
        &mut rows,
        "min_implemented_ratio",
        report.min_implemented_ratio,
    );
    push_bench_row(
        &mut rows,
        "max_missing_features",
        report
            .max_missing_features
            .map(|value| value.to_string())
            .unwrap_or_else(|| "none".to_owned()),
    );
    push_bench_row(
        &mut rows,
        "gate_passed",
        report
            .passed
            .map(|value| value.to_string())
            .unwrap_or_else(|| "none".to_owned()),
    );

    format!(
        r#"<section>
<h2>Browser Coverage</h2>
<table aria-label="Browser coverage"><tbody>{rows}</tbody></table>
</section>"#,
        rows = rows,
    )
}

fn browser_compat_table(report: &crate::browser_compat::BrowserCompatReport) -> String {
    let mut rows = String::new();
    push_bench_row(&mut rows, "engine", &report.engine);
    push_bench_row(&mut rows, "suite", &report.suite);
    push_bench_row(&mut rows, "manifest", &report.manifest);
    push_bench_row(&mut rows, "manifest_hash", &report.manifest_hash);
    push_bench_row(&mut rows, "suite_hash", &report.suite_hash);
    push_bench_row(
        &mut rows,
        "expectation_file",
        report.expectation_file.as_deref().unwrap_or("none"),
    );
    push_bench_row(
        &mut rows,
        "expectation_hash",
        report.expectation_hash.as_deref().unwrap_or("none"),
    );
    push_bench_row(&mut rows, "suite_count", report.suite_count);
    push_bench_row(&mut rows, "selected_count", report.selected_count);
    push_bench_row(&mut rows, "run_count", report.run_count);
    push_bench_row(&mut rows, "repeat", report.repeat);
    push_bench_row(
        &mut rows,
        "subsets",
        if report.subsets.is_empty() {
            "all".to_owned()
        } else {
            report.subsets.join(",")
        },
    );
    push_bench_row(
        &mut rows,
        "timeout_ms",
        report
            .timeout_ms
            .map(|value| value.to_string())
            .unwrap_or_else(|| "none".to_owned()),
    );
    push_bench_row(&mut rows, "subsystem_count", report.subsystem_count);
    push_bench_row(&mut rows, "runnable_count", report.runnable_count);
    push_bench_row(&mut rows, "pass_count", report.pass_count);
    push_bench_row(&mut rows, "fail_count", report.fail_count);
    push_bench_row(&mut rows, "timeout_count", report.timeout_count);
    push_bench_row(&mut rows, "crash_count", report.crash_count);
    push_bench_row(&mut rows, "skipped_count", report.skipped_count);
    push_bench_row(&mut rows, "unsupported_count", report.unsupported_count);
    push_bench_row(&mut rows, "flaky_count", report.flaky_count);
    push_bench_row(&mut rows, "expected_count", report.expected_count);
    push_bench_row(&mut rows, "unexpected_count", report.unexpected_count);
    push_bench_row(&mut rows, "pass_rate", format!("{:.4}", report.pass_rate));
    push_bench_row(
        &mut rows,
        "gate_passed",
        report
            .passed
            .map(|value| value.to_string())
            .unwrap_or_else(|| "none".to_owned()),
    );
    push_bench_row(
        &mut rows,
        "gate_failures",
        if report.gate_failures.is_empty() {
            "none".to_owned()
        } else {
            report.gate_failures.join("; ")
        },
    );

    let mut subsystem_rows = String::new();
    for subsystem in &report.subsystems {
        let _ = write!(
            subsystem_rows,
            r#"<tr><td>{name}</td><td>{selected}</td><td>{runnable}</td><td>{pass}</td><td>{fail}</td><td>{timeout}</td><td>{crash}</td><td>{skipped}</td><td>{unsupported}</td><td>{unexpected}</td><td>{rate:.4}</td></tr>"#,
            name = html_escape::encode_text(&subsystem.subsystem),
            selected = subsystem.suite_count,
            runnable = subsystem.runnable_count,
            pass = subsystem.pass_count,
            fail = subsystem.fail_count,
            timeout = subsystem.timeout_count,
            crash = subsystem.crash_count,
            skipped = subsystem.skipped_count,
            unsupported = subsystem.unsupported_count,
            unexpected = subsystem.unexpected_count,
            rate = subsystem.pass_rate,
        );
    }

    let mut test_rows = String::new();
    for test in &report.tests {
        let error = test.error.as_deref().unwrap_or("");
        let _ = write!(
            test_rows,
            r#"<tr><td>{id}</td><td>{subsystem}</td><td>{status}</td><td>{expected_status}</td><td>{expected}</td><td>{attempt}/{repeat}</td><td>{duration}</td><td>{error}</td></tr>"#,
            id = html_escape::encode_text(&test.id),
            subsystem = html_escape::encode_text(&test.subsystem),
            status = html_escape::encode_text(&test.status),
            expected_status = html_escape::encode_text(&test.expected_status),
            expected = test.expected,
            attempt = test.attempt,
            repeat = test.repeat_count,
            duration = test.duration_us,
            error = html_escape::encode_text(error),
        );
    }

    format!(
        r#"<h2>Browser Compatibility Metrics</h2>
<table aria-label="Browser compatibility metrics"><tbody>{rows}</tbody></table>
<h2>Browser Compatibility Subsystems</h2>
<table class="query-table" aria-label="Browser compatibility subsystems">
<thead><tr><th>Subsystem</th><th>Selected</th><th>Runnable</th><th>Pass</th><th>Fail</th><th>Timeout</th><th>Crash</th><th>Skipped</th><th>Unsupported</th><th>Unexpected</th><th>Pass Rate</th></tr></thead>
<tbody>{subsystem_rows}</tbody>
</table>
<h2>Browser Compatibility Tests</h2>
<table class="query-table" aria-label="Browser compatibility tests">
<thead><tr><th>Test</th><th>Subsystem</th><th>Status</th><th>Expected Status</th><th>Expected</th><th>Attempt</th><th>Duration Us</th><th>Error</th></tr></thead>
<tbody>{test_rows}</tbody>
</table>"#,
        rows = rows,
        subsystem_rows = subsystem_rows,
        test_rows = test_rows,
    )
}

fn browser_chromium_parity_table(report: &crate::browser::BrowserChromiumParityReport) -> String {
    let mut rows = String::new();
    push_bench_row(&mut rows, "fixture_count", report.fixture_count);
    push_bench_row(&mut rows, "passed", report.passed);
    push_bench_row(&mut rows, "failed", report.failed);
    push_bench_row(
        &mut rows,
        "chrome",
        report.chrome.as_deref().unwrap_or("unknown"),
    );

    let mut failure_rows = String::new();
    for failure in &report.failures {
        let _ = write!(
            failure_rows,
            r#"<tr><td>{name}</td><td>{path}</td><td>{reason}</td></tr>"#,
            name = html_escape::encode_text(&failure.name),
            path = html_escape::encode_text(&failure.path),
            reason = html_escape::encode_text(&failure.reason),
        );
    }
    if failure_rows.is_empty() {
        failure_rows.push_str(r#"<tr><td colspan="3">none</td></tr>"#);
    }

    format!(
        r#"<section>
<h2>Browser Chromium Parity</h2>
<table aria-label="Browser Chromium parity"><tbody>{rows}</tbody></table>
<table class="query-table" aria-label="Browser Chromium parity failures">
<thead><tr><th>Fixture</th><th>Path</th><th>Reason</th></tr></thead>
<tbody>{failure_rows}</tbody>
</table>
</section>"#,
        rows = rows,
        failure_rows = failure_rows,
    )
}

fn push_bench_row(rows: &mut String, label: &str, value: impl std::fmt::Display) {
    let _ = write!(
        rows,
        r#"<tr><th>{label}</th><td>{value}</td></tr>"#,
        label = html_escape::encode_text(label),
        value = html_escape::encode_text(&value.to_string()),
    );
}

fn push_optional_f64_row(rows: &mut String, label: &str, value: Option<f64>) {
    let value = value
        .map(|value| format!("{value:.4}"))
        .unwrap_or_else(|| "none".to_owned());
    push_bench_row(rows, label, value);
}

fn search_page() -> String {
    r#"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Blackium Starium✴</title>
<style>
:root { color-scheme: light; font-family: ui-sans-serif, system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif; }
body { margin: 0; background: #f7f7f5; color: #191a1c; }
main { max-width: 980px; margin: 0 auto; padding: 28px 18px 56px; }
header { display: flex; align-items: center; justify-content: space-between; gap: 16px; margin-bottom: 18px; }
h1 { margin: 0; font-size: 26px; font-weight: 750; letter-spacing: 0; }
#stats { color: #5d636b; font-size: 13px; white-space: nowrap; }
#status-links { display: flex; align-items: center; justify-content: flex-end; flex-wrap: wrap; gap: 10px; }
#status-links a { font-size: 13px; }
#crawl, #bench { color: #5d636b; font-size: 13px; min-height: 18px; }
#crawl { margin: -8px 0 2px; }
#bench { margin: 0 0 16px; }
form { margin-bottom: 8px; }
#search-row { display: flex; gap: 8px; margin-bottom: 8px; }
input { min-width: 0; height: 44px; border: 1px solid #b7bdc5; border-radius: 6px; padding: 0 13px; font-size: 17px; background: #fff; }
#q { flex: 1; }
button { height: 44px; border: 0; border-radius: 6px; padding: 0 18px; font-size: 15px; font-weight: 700; background: #2457d6; color: #fff; cursor: pointer; }
button:disabled { opacity: .6; cursor: wait; }
#filters { display: grid; grid-template-columns: repeat(auto-fit, minmax(112px, 1fr)); gap: 8px; }
#filters input { height: 34px; padding: 0 9px; font-size: 13px; }
#filters button { height: 34px; border: 1px solid #c6cbd2; background: #fff; color: #20242a; font-size: 13px; }
#suggestions { min-height: 28px; display: flex; flex-wrap: wrap; gap: 6px; margin-bottom: 12px; }
#suggestions button { height: 28px; border: 1px solid #c6cbd2; border-radius: 999px; padding: 0 10px; background: #fff; color: #20242a; font-size: 13px; font-weight: 650; cursor: pointer; }
#spell { min-height: 18px; color: #5d636b; font-size: 13px; margin: -4px 0 10px; }
#spell button { border: 0; background: transparent; color: #123fae; padding: 0; height: auto; font-size: 13px; font-weight: 700; cursor: pointer; }
#meta { min-height: 20px; color: #5d636b; font-size: 13px; margin-bottom: 8px; }
ol { list-style: none; margin: 0; padding: 0; }
li { padding: 16px 0; border-top: 1px solid #dfe2e6; }
a { color: #123fae; text-decoration: none; font-size: 18px; font-weight: 700; overflow-wrap: anywhere; }
a:hover { text-decoration: underline; }
.url { color: #207044; font-size: 13px; margin: 3px 0 6px; overflow-wrap: anywhere; }
.snippet { color: #24272b; font-size: 14px; line-height: 1.45; }
.score { color: #6b717a; font-size: 12px; margin-top: 6px; }
pre { white-space: pre-wrap; background: #fff; border: 1px solid #dfe2e6; border-radius: 6px; padding: 14px; overflow: auto; }
</style>
</head>
<body>
<main>
<header><h1>Blackium Starium✴</h1><div id="status-links"><div id="stats"></div><a href="/bench">Benchmarks</a></div></header>
<form id="form">
<div id="search-row"><input id="q" name="q" autocomplete="off" autofocus><button id="go">Search</button></div>
<div id="filters">
<input id="site" name="site" autocomplete="off" placeholder="site" aria-label="Site filter">
<input id="filetype" name="filetype" autocomplete="off" placeholder="type" aria-label="File type filter">
<input id="lang" name="lang" autocomplete="off" placeholder="lang" aria-label="Language filter">
<input id="after" name="after" type="date" aria-label="After date filter">
<input id="before" name="before" type="date" aria-label="Before date filter">
<button id="clearFilters" type="button">Clear</button>
</div>
</form>
<div id="suggestions"></div>
<div id="spell"></div>
<div id="crawl"></div>
<div id="bench"></div>
<div id="meta"></div>
<ol id="results"></ol>
</main>
<script>
const form = document.getElementById("form");
const input = document.getElementById("q");
const site = document.getElementById("site");
const filetype = document.getElementById("filetype");
const lang = document.getElementById("lang");
const after = document.getElementById("after");
const before = document.getElementById("before");
const clearFilters = document.getElementById("clearFilters");
const button = document.getElementById("go");
const meta = document.getElementById("meta");
const results = document.getElementById("results");
const stats = document.getElementById("stats");
const crawl = document.getElementById("crawl");
const bench = document.getElementById("bench");
const suggestions = document.getElementById("suggestions");
const spell = document.getElementById("spell");
let suggestTimer = 0;
let searchGeneration = 0;

function text(value) { return document.createTextNode(value || ""); }

function browserRenderHref(url) {
  const params = new URLSearchParams();
  params.set("url", url || "");
  return `/browser?${params.toString()}`;
}

function currentSearchHref() {
  return `${location.pathname}${location.search}` || "/search";
}

function withReturnHref(href) {
  const url = new URL(href, location.origin);
  url.searchParams.set("from", currentSearchHref());
  return `${url.pathname}${url.search}${url.hash}`;
}

function roundedMs(value) {
  return Math.round(value * 10) / 10;
}

function chromeTimingText(timing, localMs) {
  if (!timing) return "";
  if (typeof timing.elapsed_ms !== "number") return " · chrome error";
  const chromeMs = roundedMs(timing.elapsed_ms);
  const delta = roundedMs(timing.elapsed_ms - localMs);
  const sign = delta >= 0 ? "+" : "";
  const status = timing.status ? ` ${timing.status}` : "";
  const error = timing.error ? " error" : "";
  return ` · chrome ${chromeMs} ms${status}${error} · Δ ${sign}${delta} ms`;
}

async function loadChromeSearchTiming(query) {
  try {
    const res = await fetch(`/api/chrome-search-timing?q=${encodeURIComponent(query)}`);
    if (!res.ok) return { error: "chrome timing failed" };
    return await res.json();
  } catch (_error) {
    return { error: "chrome timing failed" };
  }
}

async function loadStats() {
  const res = await fetch("/api/stats");
  if (!res.ok) return;
  const data = await res.json();
  stats.textContent = `${data.doc_count} docs · ${data.term_count} terms · ${data.duplicate_doc_count} duplicates · ${data.skipped_noindex_count + data.skipped_thin_count} skipped`;
}

async function loadCrawlStatus() {
  const res = await fetch("/api/crawl-status");
  if (!res.ok) return;
  const data = await res.json();
  if (!data.frontier_exists && !data.snapshot_exists) {
    crawl.textContent = "No crawl frontier found for this index.";
    return;
  }
  const f = data.frontier;
  const parts = [
    `${f.fetched}/${f.total} urls fetched`,
    `${f.queued} queued`,
    `${f.deferred} deferred`,
    `${f.failed} failed`,
    `${data.snapshot_doc_count} docs snapshotted`
  ];
  if (data.hosts.length > 0) {
    parts.push(`${data.hosts.length} hosts tracked`);
  }
  crawl.replaceChildren(text(`crawl: ${parts.join(" · ")} `));
  const link = document.createElement("a");
  link.href = "/crawl";
  link.appendChild(text("details"));
  crawl.appendChild(link);
}

function attachStatusLink(node, href, label) {
  const link = document.createElement("a");
  link.href = href;
  link.appendChild(text(label));
  node.appendChild(link);
}

async function loadBenchStatus() {
  const res = await fetch("/api/bench-status");
  if (!res.ok) return;
  const data = await res.json();
  if (!data.report_exists || !data.report) {
    bench.replaceChildren(text("bench: no saved report "));
    attachStatusLink(bench, "/bench", "details");
    return;
  }
  const report = data.report;
  if (report.kind === "comparison") {
    const comparison = report.report;
    const gate = comparison.passed === true ? "gate passed" : comparison.passed === false ? "gate failed" : "no gate";
    bench.replaceChildren(text(`bench: ${comparison.p95_speedup.toFixed(2)}x p95 speedup · ${gate} `));
  } else if (report.kind === "eval") {
    const evalReport = report.report;
    const gate = evalReport.passed === true ? "gate passed" : evalReport.passed === false ? "gate failed" : "no gate";
    bench.replaceChildren(text(`quality: NDCG ${evalReport.mean_ndcg_at_k.toFixed(4)} · MRR ${evalReport.mean_reciprocal_rank.toFixed(4)} · ${gate} `));
  } else if (report.kind === "gate") {
    const gateReport = report.report;
    const status = gateReport.passed ? "gate passed" : "gate failed";
    const speed = gateReport.search_comparison ? ` · ${gateReport.search_comparison.p95_speedup.toFixed(2)}x p95` : "";
    bench.replaceChildren(text(`gate: ${status}${speed} · NDCG ${gateReport.eval.mean_ndcg_at_k.toFixed(4)} · browser ${(gateReport.browser_coverage.implemented_ratio * 100).toFixed(1)}% `));
  } else if (report.kind === "browser_perf") {
    const perf = report.report;
    const gate = perf.passed === true ? "gate passed" : perf.passed === false ? "gate failed" : "no gate";
    bench.replaceChildren(text(`browser perf: p95 ${perf.p95_us} us · raster p95 ${perf.raster_p95_us || 0} us · layers ${perf.total_layers || 0} · ${Math.round(perf.throughput_pages_per_sec)} pages/s · ${gate} `));
  } else if (report.kind === "browser_compat") {
    const compat = report.report;
    const gate = compat.passed === true ? "gate passed" : compat.passed === false ? "gate failed" : "no gate";
    bench.replaceChildren(text(`browser compat: ${compat.pass_count}/${compat.runnable_count} runnable pass · ${compat.unexpected_count} unexpected · ${gate} `));
  } else {
    const benchReport = report.kind === "smoke" ? report.report.bench : report.report;
    bench.replaceChildren(text(`bench: ${benchReport.engine} p95 ${benchReport.p95_us} us · ${Math.round(benchReport.throughput_qps)} qps `));
  }
  attachStatusLink(bench, "/bench", "details");
}

function compactFilterValue(value) {
  return (value || "").trim().replace(/\s+/g, "");
}

function appendFilter(parts, operator, value, normalizer = compactFilterValue) {
  const clean = normalizer(value);
  if (clean) parts.push(`${operator}:${clean}`);
}

function normalizeFiletype(value) {
  return compactFilterValue(value).replace(/^\./, "").toLowerCase();
}

function normalizeLanguage(value) {
  return compactFilterValue(value).toLowerCase();
}

function buildSearchQuery() {
  const parts = [];
  const base = input.value.trim();
  if (base) parts.push(base);
  appendFilter(parts, "site", site.value);
  appendFilter(parts, "filetype", filetype.value, normalizeFiletype);
  appendFilter(parts, "lang", lang.value, normalizeLanguage);
  appendFilter(parts, "after", after.value);
  appendFilter(parts, "before", before.value);
  return parts.join(" ");
}

function syncUrl() {
  const params = new URLSearchParams();
  const values = [
    ["q", input.value.trim()],
    ["site", compactFilterValue(site.value)],
    ["filetype", normalizeFiletype(filetype.value)],
    ["lang", normalizeLanguage(lang.value)],
    ["after", compactFilterValue(after.value)],
    ["before", compactFilterValue(before.value)]
  ];
  for (const [key, value] of values) {
    if (value) params.set(key, value);
  }
  history.replaceState(null, "", params.toString() ? `/search?${params}` : "/search");
}

async function search() {
  const query = buildSearchQuery();
  if (!query) return;
  const generation = ++searchGeneration;
  button.disabled = true;
  meta.textContent = "";
  spell.replaceChildren();
  results.replaceChildren();
  syncUrl();
  const started = performance.now();
  const chromeTimingPromise = loadChromeSearchTiming(query);
  const res = await fetch(`/api/search?q=${encodeURIComponent(query)}&limit=20`);
  const data = await res.json();
  button.disabled = false;
  if (generation !== searchGeneration) return;
  if (!res.ok) {
    meta.textContent = data.error || "Search failed";
    return;
  }
  const sourceParts = [];
  if (data.sources) {
    sourceParts.push(`${data.sources.local_count} local`);
    if (data.sources.web_enabled) {
      const webLabel = data.sources.web_provider || "web";
      const cacheLabel = data.sources.web_cache_hit ? " cache" : "";
      const fetchLabel = data.sources.web_fetched ? " fetched" : "";
      sourceParts.push(`${data.sources.web_count} ${webLabel}${cacheLabel}${fetchLabel}`);
      if (data.sources.background_crawl_enqueued) sourceParts.push(`${data.sources.background_crawl_enqueued} crawl queued`);
      if (data.sources.web_error) sourceParts.push(`web error: ${data.sources.web_error}`);
    }
  }
  const sourceText = sourceParts.length ? ` · ${sourceParts.join(" · ")}` : "";
  const localMs = roundedMs(performance.now() - started);
  const renderMeta = chromeTiming => {
    if (generation !== searchGeneration) return;
    meta.textContent = `${data.results.length} results · ${localMs} ms${sourceText}${chromeTimingText(chromeTiming, localMs)}`;
  };
  renderMeta(null);
  chromeTimingPromise.then(renderMeta);
  if (data.results.length === 0) {
    loadSpellCorrection(input.value.trim());
  }
  for (const result of data.results) {
    const item = document.createElement("li");
    const title = document.createElement("a");
    title.href = withReturnHref(result.render_url || browserRenderHref(result.url));
    title.appendChild(text(result.title || result.url));
    const url = document.createElement("div");
    url.className = "url";
    url.appendChild(text(result.url));
    const snippet = document.createElement("div");
    snippet.className = "snippet";
    snippet.appendChild(text(result.snippet));
    const score = document.createElement("div");
    score.className = "score";
    const duplicateText = result.duplicate_count > 1 ? ` · ${result.duplicate_count} duplicates` : "";
    const authorityText = result.authority_score ? ` · authority ${result.authority_score.toFixed(4)}` : "";
    const languageText = result.language ? ` · ${result.language}` : "";
    const fetchedText = result.fetched_at_unix ? ` · fetched ${result.fetched_at_unix}` : "";
    const idText = result.doc_id === null || result.doc_id === undefined ? result.source : `doc ${result.doc_id}`;
    score.appendChild(text(`${idText} · score ${result.score.toFixed(4)}${authorityText}${languageText}${fetchedText}${duplicateText}`));
    item.append(title, url, snippet, score);
    results.appendChild(item);
  }
}

async function loadSpellCorrection(query) {
  const res = await fetch(`/api/spell?q=${encodeURIComponent(query)}&limit=1`);
  if (!res.ok) return;
  const data = await res.json();
  if (!data.corrections.length) return;
  const correction = data.corrections[0];
  spell.replaceChildren(text("Did you mean "));
  const button = document.createElement("button");
  button.type = "button";
  button.appendChild(text(correction.term));
  button.addEventListener("click", () => {
    input.value = completeLastTerm(input.value, correction.term);
    search();
  });
  spell.appendChild(button);
  spell.appendChild(text("?"));
}

async function loadSuggestions(query) {
  const trimmed = query.trimEnd();
  if (!trimmed) {
    suggestions.replaceChildren();
    return;
  }
  const res = await fetch(`/api/suggest?q=${encodeURIComponent(trimmed)}&limit=8`);
  if (!res.ok) return;
  const data = await res.json();
  suggestions.replaceChildren();
  for (const suggestion of data.suggestions) {
    const chip = document.createElement("button");
    chip.type = "button";
    chip.appendChild(text(suggestion.term));
    chip.addEventListener("click", () => {
      input.value = completeLastTerm(input.value, suggestion.term);
      input.focus();
      search();
    });
    suggestions.appendChild(chip);
  }
}

function completeLastTerm(query, term) {
  const match = query.match(/^(.*?)([A-Za-z0-9]*)\s*$/);
  if (!match) return term;
  return `${match[1]}${term}`;
}

input.addEventListener("input", () => {
  clearTimeout(suggestTimer);
  suggestTimer = setTimeout(() => loadSuggestions(input.value), 80);
});

form.addEventListener("submit", event => {
  event.preventDefault();
  search();
});

clearFilters.addEventListener("click", () => {
  site.value = "";
  filetype.value = "";
  lang.value = "";
  after.value = "";
  before.value = "";
  syncUrl();
  if (input.value.trim()) search();
});

const params = new URLSearchParams(location.search);
const initial = params.get("q");
site.value = params.get("site") || "";
filetype.value = params.get("filetype") || "";
lang.value = params.get("lang") || "";
after.value = params.get("after") || "";
before.value = params.get("before") || "";
if (initial) {
  input.value = initial;
  search();
}
loadStats();
loadCrawlStatus();
loadBenchStatus();
</script>
</body>
</html>"#
        .to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_get_target_and_query_params() {
        let target =
            parse_request_target("GET /api/search?q=fast+rust&limit=3 HTTP/1.1\r\n\r\n").unwrap();
        assert_eq!(target.path, "/api/search");
        assert_eq!(target.param("q").as_deref(), Some("fast rust"));
        assert_eq!(target.param("limit").as_deref(), Some("3"));
    }

    #[test]
    fn rejects_non_get_requests() {
        let error = parse_request_target("POST /api/search HTTP/1.1\r\n\r\n").unwrap_err();
        assert!(error.to_string().contains("only GET"));
    }

    #[test]
    fn search_page_contains_api_hooks() {
        let html = search_page();
        assert!(html.contains("Blackium Starium✴"));
        assert!(html.contains("/api/search"));
        assert!(html.contains("/api/chrome-search-timing"));
        assert!(html.contains("/api/suggest"));
        assert!(html.contains("/api/spell"));
        assert!(html.contains("/api/stats"));
        assert!(html.contains("/api/crawl-status"));
        assert!(html.contains("/api/bench-status"));
        assert!(html.contains("/crawl"));
        assert!(html.contains("/bench"));
        assert!(html.contains("browserRenderHref(result.url)"));
        assert!(
            html.contains("withReturnHref(result.render_url || browserRenderHref(result.url))")
        );
        assert!(html.contains("chromeTimingText(chromeTiming, localMs)"));
        assert!(html.contains("id=\"filters\""));
        assert!(html.contains("buildSearchQuery"));
        assert!(html.contains("appendFilter(parts, \"site\""));
        assert!(html.contains("appendFilter(parts, \"filetype\""));
        assert!(html.contains("appendFilter(parts, \"lang\""));
        assert!(html.contains("appendFilter(parts, \"after\""));
        assert!(html.contains("appendFilter(parts, \"before\""));
    }

    #[test]
    fn browser_render_url_round_trips_external_targets() {
        let href = browser_render_url("https://example.com/a path?q=cat&x=1");
        let target = parse_request_target(&format!("GET {href} HTTP/1.1\r\n\r\n")).unwrap();

        assert_eq!(target.path, "/browser");
        assert_eq!(
            target.param("url").as_deref(),
            Some("https://example.com/a path?q=cat&x=1")
        );
    }

    #[test]
    fn local_search_results_open_browser_as_primary_target() {
        let result = SearchApiResult::from_local(SearchResult {
            doc_id: 37,
            url: "https://example.com/cats".to_owned(),
            canonical_url: None,
            title: "Cat result".to_owned(),
            language: Some("en".to_owned()),
            fetched_at_unix: Some(1),
            score: 12.0,
            authority_score: 0.25,
            snippet: "snippet".to_owned(),
            duplicate_of: 37,
            duplicate_count: 1,
        });

        assert_eq!(
            result.render_url.as_deref(),
            Some("/browser?url=https%3A%2F%2Fexample.com%2Fcats")
        );
        assert_eq!(result.doc_id, Some(37));
    }

    #[test]
    fn chrome_search_timing_url_appends_encoded_query() {
        assert_eq!(
            chrome_search_timing_url("https://www.google.com/search", "Daniel Edrisian"),
            "https://www.google.com/search?q=Daniel+Edrisian"
        );
        assert_eq!(
            chrome_search_timing_url("https://example.com/search?source=test", "a&b"),
            "https://example.com/search?source=test&q=a%26b"
        );
        assert_eq!(
            chrome_search_timing_url("https://example.com/search?q={q}&src=chrome", "a b"),
            "https://example.com/search?q=a+b&src=chrome"
        );
    }

    #[test]
    fn browser_render_url_preserves_search_return_href() {
        let href = browser_render_url_with_return("https://example.com/cat", Some("/search?q=cat"));
        let target = parse_request_target(&format!("GET {href} HTTP/1.1\r\n\r\n")).unwrap();

        assert_eq!(target.path, "/browser");
        assert_eq!(
            target.param("url").as_deref(),
            Some("https://example.com/cat")
        );
        assert_eq!(target.param("from").as_deref(), Some("/search?q=cat"));
    }

    #[test]
    fn search_return_href_accepts_only_search_routes() {
        assert_eq!(
            sanitized_search_return_href(Some("/search?q=cat")),
            "/search?q=cat"
        );
        assert_eq!(
            sanitized_search_return_href(Some("https://evil.example/")),
            "/search"
        );
        assert_eq!(sanitized_search_return_href(Some("/bench")), "/search");
    }

    #[test]
    fn web_search_results_link_to_rust_browser_route() {
        let result = SearchApiResult::from_web(
            WebSearchResult {
                title: "Domestic cat".to_owned(),
                url: "https://example.com/cat".to_owned(),
                snippet: "Cat page".to_owned(),
                score: 0.5,
                fetched_at_unix: 20,
                provider: "brave".to_owned(),
            },
            true,
        );

        assert_eq!(
            result.render_url.as_deref(),
            Some("/browser?url=https%3A%2F%2Fexample.com%2Fcat")
        );
        assert_eq!(result.source, "brave cache");
    }

    #[test]
    fn search_ranking_fuses_web_results_above_weak_local_body_hits() {
        let local = SearchResult {
            doc_id: 1,
            url: "https://en.wikipedia.org/wiki/Dog".to_owned(),
            canonical_url: Some("https://en.wikipedia.org/wiki/Dog".to_owned()),
            title: "Dog - Wikipedia".to_owned(),
            language: Some("en".to_owned()),
            fetched_at_unix: Some(10),
            score: 3.7,
            authority_score: 0.2,
            snippet: "Studies mention pet-dog or -cat guardians once.".to_owned(),
            duplicate_of: 1,
            duplicate_count: 1,
        };
        let web = WebSearchResult {
            title: "Domestic cat | National Geographic".to_owned(),
            url: "https://www.nationalgeographic.com/animals/mammals/facts/domestic-cat".to_owned(),
            snippet: "Domestic cats are small carnivorous mammals.".to_owned(),
            score: 0.5,
            fetched_at_unix: 20,
            provider: "brave".to_owned(),
        };

        let results = rank_search_results("cat", vec![local], vec![web], true, 10);

        assert_eq!(results[0].title, "Domestic cat | National Geographic");
        assert_eq!(results[0].source, "brave cache");
    }

    #[test]
    fn search_ranking_keeps_strong_local_matches_high() {
        let local = SearchResult {
            doc_id: 2,
            url: "https://en.wikipedia.org/wiki/Cat".to_owned(),
            canonical_url: Some("https://en.wikipedia.org/wiki/Cat".to_owned()),
            title: "Cat - Wikipedia".to_owned(),
            language: Some("en".to_owned()),
            fetched_at_unix: Some(10),
            score: 3.0,
            authority_score: 0.2,
            snippet: "The cat is a domestic species.".to_owned(),
            duplicate_of: 2,
            duplicate_count: 1,
        };
        let web = WebSearchResult {
            title: "Cat facts".to_owned(),
            url: "https://example.com/cat-facts".to_owned(),
            snippet: "Facts about cats.".to_owned(),
            score: 0.5,
            fetched_at_unix: 20,
            provider: "brave".to_owned(),
        };

        let results = rank_search_results("cat", vec![local], vec![web], false, 10);

        assert_eq!(results[0].doc_id, Some(2));
        assert_eq!(results[0].source, "local");
    }

    #[test]
    fn strong_local_match_count_ignores_body_only_matches() {
        let results = vec![
            SearchResult {
                doc_id: 1,
                url: "https://en.wikipedia.org/wiki/Dog".to_owned(),
                canonical_url: None,
                title: "Dog - Wikipedia".to_owned(),
                language: None,
                fetched_at_unix: None,
                score: 3.7,
                authority_score: 0.0,
                snippet: "The body mentions cat once.".to_owned(),
                duplicate_of: 1,
                duplicate_count: 1,
            },
            SearchResult {
                doc_id: 2,
                url: "https://en.wikipedia.org/wiki/Cat".to_owned(),
                canonical_url: None,
                title: "Cat - Wikipedia".to_owned(),
                language: None,
                fetched_at_unix: None,
                score: 3.0,
                authority_score: 0.0,
                snippet: "The body mentions cat many times.".to_owned(),
                duplicate_of: 2,
                duplicate_count: 1,
            },
        ];

        assert_eq!(strong_local_match_count("cat", &results), 1);
    }

    #[test]
    fn render_document_page_escapes_document_content() {
        let html = render_document_page(
            7,
            "https://example.com/?a=<b>",
            "Title <unsafe>",
            "body <script>alert(1)</script>",
            "/search?q=cat",
        );
        assert!(html.contains("Title &lt;unsafe&gt;"));
        assert!(html.contains(r#"<a href="/search?q=cat">Back to search</a>"#));
        assert!(html.contains("https://example.com/?a=&lt;b&gt;"));
        assert!(html.contains("body &lt;script&gt;alert(1)&lt;/script&gt;"));
        assert!(!html.contains("<script>alert(1)</script>"));
    }

    #[test]
    fn counts_non_empty_snapshot_lines() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("crawl-docs.jsonl");
        std::fs::write(&path, "\n{\"url\":\"a\"}\n\n{\"url\":\"b\"}\n").unwrap();

        assert_eq!(count_snapshot_documents(&path).unwrap(), 2);
    }

    #[test]
    fn crawl_status_page_escapes_host_names() {
        let payload = CrawlStatusPayload {
            frontier_exists: true,
            snapshot_exists: true,
            snapshot_doc_count: 3,
            frontier: FrontierStats {
                queued: 1,
                fetching: 2,
                fetched: 3,
                failed: 4,
                deferred: 5,
                total: 15,
            },
            hosts: vec![HostStats {
                host: "bad.example\"><script>".to_owned(),
                queued: 1,
                fetching: 0,
                fetched: 2,
                failed: 0,
                deferred: 0,
                total: 3,
            }],
            failures: vec![FrontierFailure {
                host: "bad.example\"><script>".to_owned(),
                status_code: Some(500),
                reason: Some("bad <reason>".to_owned()),
                url: "https://bad.example/\"><script>".to_owned(),
            }],
        };

        let html = render_crawl_status_page(&payload);
        assert!(html.contains("bad.example\"&gt;&lt;script&gt;"));
        assert!(html.contains("bad &lt;reason&gt;"));
        assert!(html.contains("<strong>15</strong>"));
        assert!(!html.contains("bad.example\"><script>"));
    }

    #[test]
    fn bench_status_page_escapes_report_metadata() {
        let payload = BenchStatusPayload {
            report_exists: true,
            report: Some(BenchStatusReport::Search(crate::bench::BenchReport {
                engine: "fast<script>".to_owned(),
                query_count: 3,
                limit: 20,
                p50_us: 1,
                p95_us: 2,
                p99_us: 3,
                throughput_qps: 1000.0,
                total_ms: 4,
                rustc: Some("rustc <nightly>".to_owned()),
                chrome: Some("chrome".to_owned()),
                os: Some("os".to_owned()),
                hardware: Some("m4".to_owned()),
                corpus_hash: "corpus".to_owned(),
                index_hash: "index".to_owned(),
            })),
        };

        let html = render_bench_status_page(&payload);
        assert!(html.contains("fast&lt;script&gt;"));
        assert!(html.contains("rustc &lt;nightly&gt;"));
        assert!(html.contains("p95_us"));
        assert!(!html.contains("fast<script>"));
    }

    #[test]
    fn bench_status_page_renders_browser_perf_raster_metrics() {
        let payload = BenchStatusPayload {
            report_exists: true,
            report: Some(BenchStatusReport::BrowserPerf(Box::new(
                crate::bench::BrowserPerfReport {
                    engine: "browser".to_owned(),
                    manifest: "manifest.json".to_owned(),
                    fixture_count: 1,
                    iteration_count: 1,
                    warmup: 0,
                    sample_count: 1,
                    p50_us: 10,
                    p95_us: 12,
                    p99_us: 12,
                    raster_p50_us: 3,
                    raster_p95_us: 4,
                    raster_p99_us: 4,
                    layer_metrics_p50_us: 1,
                    layer_metrics_p95_us: 2,
                    layer_metrics_p99_us: 2,
                    throughput_pages_per_sec: 100.0,
                    total_ms: 1,
                    total_rendered_bytes: 5,
                    total_dom_nodes: 2,
                    total_css_rules: 1,
                    total_layout_boxes: 1,
                    total_paint_commands: 2,
                    total_layers: 2,
                    total_image_layers: 1,
                    max_layer_count: 2,
                    max_image_layer_count: 1,
                    max_root_layer_width: 10,
                    max_root_layer_height: 8,
                    max_layer_area: 80,
                    total_layer_area: 100,
                    total_layer_metrics_us: 2,
                    total_raster_us: 3,
                    total_raster_pixels: 80,
                    total_raster_non_background_pixels: 9,
                    total_raster_visible_commands: 2,
                    total_raster_culled_commands: 1,
                    chromium_baseline: None,
                    chromium_p95_speedup: None,
                    phase_totals: crate::browser::BrowserRenderTimings {
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
                    fixtures: vec![crate::bench::BrowserPerfFixtureReport {
                        name: "fixture<script>".to_owned(),
                        path: "fixture.html".to_owned(),
                        sample_count: 1,
                        p50_us: 10,
                        p95_us: 12,
                        p99_us: 12,
                        raster_p50_us: 3,
                        raster_p95_us: 4,
                        raster_p99_us: 4,
                        raster_total_us: 3,
                        layer_metrics_p50_us: 1,
                        layer_metrics_p95_us: 2,
                        layer_metrics_p99_us: 2,
                        layer_metrics_total_us: 2,
                        rendered_bytes: 5,
                        dom_node_count: 2,
                        css_rule_count: 1,
                        layout_box_count: 1,
                        paint_command_count: 2,
                        layer_count: 2,
                        image_layer_count: 1,
                        root_layer_width: 10,
                        root_layer_height: 8,
                        max_layer_area: 80,
                        total_layer_area: 100,
                        raster_width: 10,
                        raster_height: 8,
                        raster_pixels: 80,
                        raster_non_background_pixels: 9,
                        raster_visible_command_count: 2,
                        raster_culled_command_count: 1,
                        phase_totals: crate::browser::BrowserRenderTimings {
                            parse_us: 1,
                            script_us: 1,
                            style_us: 1,
                            collect_us: 1,
                            layout_us: 1,
                            total_us: 5,
                        },
                    }],
                },
            ))),
        };

        let html = render_bench_status_page(&payload);
        assert!(html.contains("raster_p95_us"));
        assert!(html.contains("total_raster_non_background_pixels"));
        assert!(html.contains("total_raster_culled_commands"));
        assert!(html.contains("total_layers"));
        assert!(html.contains("total_layer_area"));
        assert!(html.contains("layer_metrics_p95_us"));
        assert!(html.contains("Image Layers"));
        assert!(html.contains("Raster Non-BG Pixels"));
        assert!(html.contains("fixture&lt;script&gt;"));
        assert!(!html.contains("fixture<script>"));
    }

    #[test]
    fn bench_status_page_renders_browser_compat_metrics() {
        let payload = BenchStatusPayload {
            report_exists: true,
            report: Some(BenchStatusReport::BrowserCompat(
                crate::browser_compat::BrowserCompatReport {
                    engine: "compat<script>".to_owned(),
                    suite: "local scaffold".to_owned(),
                    manifest: "manifest.json".to_owned(),
                    manifest_hash: "manifest-hash".to_owned(),
                    suite_hash: "suite-hash".to_owned(),
                    expectation_file: Some("expectations.jsonl".to_owned()),
                    expectation_hash: Some("expectation-hash".to_owned()),
                    suite_count: 1,
                    selected_count: 1,
                    run_count: 1,
                    repeat: 1,
                    subsets: vec!["html".to_owned()],
                    timeout_ms: Some(100),
                    subsystem_count: 1,
                    runnable_count: 1,
                    pass_count: 1,
                    fail_count: 0,
                    timeout_count: 0,
                    crash_count: 0,
                    skipped_count: 0,
                    unsupported_count: 0,
                    flaky_count: 0,
                    expected_count: 1,
                    unexpected_count: 0,
                    pass_rate: 1.0,
                    gate: Some(crate::browser_compat::BrowserCompatGate {
                        min_pass_rate: Some(1.0),
                        max_unexpected_failures: Some(0),
                        ..crate::browser_compat::BrowserCompatGate::default()
                    }),
                    gate_failures: Vec::new(),
                    passed: Some(true),
                    subsystems: vec![crate::browser_compat::BrowserCompatSubsystemReport {
                        subsystem: "html".to_owned(),
                        suite_count: 1,
                        runnable_count: 1,
                        pass_count: 1,
                        fail_count: 0,
                        timeout_count: 0,
                        crash_count: 0,
                        skipped_count: 0,
                        unsupported_count: 0,
                        flaky_count: 0,
                        expected_count: 1,
                        unexpected_count: 0,
                        pass_rate: 1.0,
                    }],
                    tests: vec![crate::browser_compat::BrowserCompatTestReport {
                        id: "compat<script>".to_owned(),
                        name: "compat name".to_owned(),
                        path: "fixture.html".to_owned(),
                        subsystem: "html".to_owned(),
                        status: "pass".to_owned(),
                        expected_status: "pass".to_owned(),
                        expected: true,
                        flaky: false,
                        reason: None,
                        error: Some("escaped <reason>".to_owned()),
                        duration_us: 7,
                        attempt: 1,
                        repeat_count: 1,
                        rendered_bytes: 12,
                        dom_node_count: 2,
                        css_rule_count: 0,
                        layout_box_count: 1,
                        paint_command_count: 1,
                        expected_title: None,
                        actual_title: None,
                        expected_text: Some("hello".to_owned()),
                        actual_text: Some("hello".to_owned()),
                        expected_raster_hash: None,
                        actual_raster_hash: None,
                    }],
                },
            )),
        };

        let html = render_bench_status_page(&payload);
        assert!(html.contains("Browser Compatibility Metrics"));
        assert!(html.contains("pass_rate"));
        assert!(html.contains("compat&lt;script&gt;"));
        assert!(html.contains("escaped &lt;reason&gt;"));
        assert!(!html.contains("compat<script>"));
        assert!(!html.contains("escaped <reason>"));
    }
}
