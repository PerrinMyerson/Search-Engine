use std::collections::{HashMap, HashSet, VecDeque};
use std::fs;
use std::path::Path;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, ensure};
use reqwest::StatusCode;
use reqwest::header::{
    CONTENT_TYPE, COOKIE, HeaderMap, HeaderValue, LOCATION, SET_COOKIE, USER_AGENT,
};
use serde::{Deserialize, Serialize};
use url::Url;

use super::images::{
    ImageDecodeDiagnostic, background_image_sizes_attr, image_decode_diagnostic,
    image_mime_type_supported, image_render_source, image_sizes_attr,
    selected_supported_srcset_candidate, srcset_candidate_urls, supported_srcset_candidate_urls,
};
use super::{BrowserCookieJar, Dom, ElementData, NodeKind, resolve_browser_href};

const DEFAULT_RESOURCE_CACHE_MAX_ENTRIES: usize = 256;
const DEFAULT_RESOURCE_CACHE_MAX_BYTES: usize = 32 * 1024 * 1024;
const DOCUMENT_HTTP_TIMEOUT: Duration = Duration::from_secs(6);
const RESOURCE_HTTP_TIMEOUT: Duration = Duration::from_secs(15);
const DATA_URL_SOURCE_METADATA_MAX_CHARS: usize = 80;
const DATA_URL_REPORT_LABEL_MAX_CHARS: usize = 120;
const REPORT_PAGE_SOURCE_MAX_CHARS: usize = 16 * 1024;
const REPORT_ERROR_MAX_CHARS: usize = 2 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BrowserResource {
    pub kind: String,
    pub initiator: String,
    #[serde(serialize_with = "serialize_resource_url")]
    pub url: String,
    #[serde(serialize_with = "serialize_resource_url")]
    pub resolved: String,
    pub rel: Option<String>,
    pub media: Option<String>,
    pub alt: Option<String>,
    pub type_hint: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrowserResourceFetchReport {
    #[serde(serialize_with = "serialize_report_page_source")]
    pub page_source: String,
    pub total: usize,
    pub fetched: usize,
    pub cached: usize,
    pub failed: usize,
    pub skipped: usize,
    #[serde(default)]
    pub cached_resource_count: usize,
    #[serde(default)]
    pub cached_resource_bytes: usize,
    pub resources: Vec<BrowserResourceFetch>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrowserResourceFetch {
    pub resource: BrowserResource,
    pub status: String,
    pub source: Option<String>,
    pub bytes: usize,
    pub content_type: Option<String>,
    #[serde(serialize_with = "serialize_optional_report_error")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_scheme: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_host: Option<String>,
    #[serde(default)]
    pub timed_out: bool,
    #[serde(default)]
    pub retryable: bool,
    #[serde(default)]
    pub elapsed_ms: u128,
    #[serde(default)]
    pub request_timeout_ms: u128,
    #[serde(default)]
    pub cache_entries: usize,
    #[serde(default)]
    pub cache_bytes: usize,
    #[serde(default)]
    pub cache_evicted_entries: usize,
    #[serde(default)]
    pub cache_evicted_bytes: usize,
    #[serde(default)]
    pub cache_max_entries: usize,
    #[serde(default)]
    pub cache_max_bytes: usize,
    #[serde(default)]
    pub cache_remaining_entries: usize,
    #[serde(default)]
    pub cache_remaining_bytes: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_pressure_level: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_outcome: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diagnostic: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image_decode_status: Option<String>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        serialize_with = "serialize_optional_report_error"
    )]
    pub image_decode_error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decoded_width: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decoded_height: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decoded_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decoded_color_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decoded_color_bytes: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrowserStylesheetRenderReport {
    #[serde(serialize_with = "serialize_report_page_source")]
    pub page_source: String,
    pub stylesheet_count: usize,
    pub applied: usize,
    pub failed: usize,
    #[serde(default)]
    pub cached_resource_count: usize,
    #[serde(default)]
    pub cached_resource_bytes: usize,
    pub fetches: Vec<BrowserResourceFetch>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrowserScriptRenderReport {
    #[serde(serialize_with = "serialize_report_page_source")]
    pub page_source: String,
    pub script_count: usize,
    pub applied: usize,
    pub failed: usize,
    #[serde(default)]
    pub cached_resource_count: usize,
    #[serde(default)]
    pub cached_resource_bytes: usize,
    pub fetches: Vec<BrowserResourceFetch>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrowserImageRenderReport {
    #[serde(serialize_with = "serialize_report_page_source")]
    pub page_source: String,
    pub image_count: usize,
    pub decoded: usize,
    pub failed: usize,
    #[serde(default)]
    pub cached_resource_count: usize,
    #[serde(default)]
    pub cached_resource_bytes: usize,
    #[serde(default)]
    pub decoded_image_bytes: usize,
    pub fetches: Vec<BrowserResourceFetch>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[allow(dead_code)]
pub struct BrowserDocumentLoadReport {
    pub target: String,
    pub status: String,
    pub source: Option<String>,
    pub bytes: usize,
    pub content_type: Option<String>,
    #[serde(serialize_with = "serialize_optional_report_error")]
    pub error: Option<String>,
    pub error_kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_scheme: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_host: Option<String>,
    #[serde(default)]
    pub timed_out: bool,
    #[serde(default)]
    pub retryable: bool,
    pub elapsed_ms: u128,
    pub request_timeout_ms: u128,
    pub diagnostic: Option<String>,
}

#[derive(Debug, Clone)]
pub(super) struct BrowserResourceCache {
    entries: HashMap<String, BrowserCachedResource>,
    order: VecDeque<String>,
    max_entries: usize,
    max_bytes: usize,
    evicted_entries: usize,
    evicted_bytes: usize,
}

#[derive(Debug, Clone)]
struct BrowserCachedResource {
    source: String,
    bytes: Vec<u8>,
    content_type: Option<String>,
}

struct BrowserResourceFetchArgs {
    resource: BrowserResource,
    status: String,
    source: Option<String>,
    bytes: usize,
    content_type: Option<String>,
    error: Option<String>,
    error_kind: Option<String>,
    elapsed_ms: u128,
    request_timeout_ms: u128,
    cache_pressure: BrowserResourceCachePressure,
    cache_outcome: Option<String>,
}

#[derive(Debug, Clone, Copy, Default)]
struct BrowserResourceCachePressure {
    entries: usize,
    bytes: usize,
    evicted_entries: usize,
    evicted_bytes: usize,
    max_entries: usize,
    max_bytes: usize,
}

impl BrowserResourceCachePressure {
    fn remaining_entries(self) -> usize {
        self.max_entries.saturating_sub(self.entries)
    }

    fn remaining_bytes(self) -> usize {
        self.max_bytes.saturating_sub(self.bytes)
    }

    fn pressure_level(self) -> &'static str {
        if self.max_entries == 0 || self.max_bytes == 0 {
            return "disabled";
        }
        let entry_percent = cache_pressure_percent(self.entries, self.max_entries);
        let byte_percent = cache_pressure_percent(self.bytes, self.max_bytes);
        let percent = entry_percent.max(byte_percent);

        if percent >= 100 {
            "full"
        } else if percent >= 90 {
            "high"
        } else if percent >= 75 {
            "elevated"
        } else {
            "normal"
        }
    }
}

fn cache_pressure_percent(value: usize, max: usize) -> usize {
    if max == 0 {
        0
    } else {
        value.saturating_mul(100) / max
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BrowserResourceCacheStoreOutcome {
    Stored,
    Replaced,
    SkippedDisabled,
    SkippedOversize,
}

impl BrowserResourceCacheStoreOutcome {
    fn as_str(self) -> &'static str {
        match self {
            Self::Stored => "stored",
            Self::Replaced => "replaced",
            Self::SkippedDisabled => "skipped_disabled",
            Self::SkippedOversize => "skipped_oversize",
        }
    }
}

impl BrowserResourceFetch {
    fn from_args(args: BrowserResourceFetchArgs, image_bytes: Option<&[u8]>) -> Self {
        let target_diagnostics =
            resource_target_diagnostics(&args.resource.resolved, args.error_kind.as_deref());
        let image_report = image_resource_fetch_decode_report(
            &args.resource,
            &args.status,
            args.content_type.as_deref(),
            image_bytes,
        );
        let (
            image_decode_status,
            image_decode_error,
            decoded_width,
            decoded_height,
            decoded_hash,
            decoded_color_hash,
            decoded_color_bytes,
        ) = image_report
            .map(|report| {
                (
                    Some(report.status.to_owned()),
                    report.error,
                    report.width,
                    report.height,
                    report.pixel_hash,
                    report.color_pixel_hash,
                    report.color_bytes,
                )
            })
            .unwrap_or((None, None, None, None, None, None, None));
        let diagnostic = resource_fetch_diagnostic(
            &args.status,
            args.error_kind.as_deref(),
            args.cache_outcome.as_deref(),
            image_decode_status.as_deref(),
        );

        Self {
            resource: args.resource,
            status: args.status,
            source: args.source,
            bytes: args.bytes,
            content_type: args.content_type,
            error: args.error,
            error_kind: args.error_kind,
            target_scheme: target_diagnostics.scheme,
            target_host: target_diagnostics.host,
            timed_out: target_diagnostics.timed_out,
            retryable: target_diagnostics.retryable,
            elapsed_ms: args.elapsed_ms,
            request_timeout_ms: args.request_timeout_ms,
            cache_entries: args.cache_pressure.entries,
            cache_bytes: args.cache_pressure.bytes,
            cache_evicted_entries: args.cache_pressure.evicted_entries,
            cache_evicted_bytes: args.cache_pressure.evicted_bytes,
            cache_max_entries: args.cache_pressure.max_entries,
            cache_max_bytes: args.cache_pressure.max_bytes,
            cache_remaining_entries: args.cache_pressure.remaining_entries(),
            cache_remaining_bytes: args.cache_pressure.remaining_bytes(),
            cache_pressure_level: Some(args.cache_pressure.pressure_level().to_owned()),
            cache_outcome: args.cache_outcome,
            diagnostic,
            image_decode_status,
            image_decode_error,
            decoded_width,
            decoded_height,
            decoded_hash,
            decoded_color_hash,
            decoded_color_bytes,
        }
    }

    fn set_cache_pressure(&mut self, pressure: BrowserResourceCachePressure) {
        self.cache_entries = pressure.entries;
        self.cache_bytes = pressure.bytes;
        self.cache_evicted_entries = pressure.evicted_entries;
        self.cache_evicted_bytes = pressure.evicted_bytes;
        self.cache_max_entries = pressure.max_entries;
        self.cache_max_bytes = pressure.max_bytes;
        self.cache_remaining_entries = pressure.remaining_entries();
        self.cache_remaining_bytes = pressure.remaining_bytes();
        self.cache_pressure_level = Some(pressure.pressure_level().to_owned());
    }

    fn set_cache_outcome(&mut self, outcome: BrowserResourceCacheStoreOutcome) {
        self.cache_outcome = Some(outcome.as_str().to_owned());
        self.diagnostic = resource_fetch_diagnostic(
            &self.status,
            self.error_kind.as_deref(),
            self.cache_outcome.as_deref(),
            self.image_decode_status.as_deref(),
        );
    }
}

fn image_resource_fetch_decode_report(
    resource: &BrowserResource,
    status: &str,
    content_type: Option<&str>,
    bytes: Option<&[u8]>,
) -> Option<ImageDecodeDiagnostic> {
    if !resource_may_be_image(resource, content_type) {
        return None;
    }

    let Some(bytes) = bytes else {
        return Some(ImageDecodeDiagnostic {
            status: "not_fetched",
            error: (!matches!(status, "fetched" | "cached"))
                .then(|| format!("resource {status} before decode")),
            width: None,
            height: None,
            pixel_hash: None,
            color_pixel_hash: None,
            color_bytes: None,
        });
    };

    Some(image_decode_diagnostic(
        &resource.resolved,
        content_type,
        bytes,
    ))
}

fn resource_may_be_image(resource: &BrowserResource, content_type: Option<&str>) -> bool {
    matches!(
        resource.kind.as_str(),
        "image" | "image_candidate" | "background_image" | "poster" | "icon"
    ) || matches!(resource.initiator.as_str(), "img" | "source" | "picture")
        || resource
            .type_hint
            .as_deref()
            .is_some_and(media_type_declares_image)
        || content_type.is_some_and(media_type_declares_image)
        || url_likely_supported_image(&resource.url)
}

fn media_type_declares_image(media_type: &str) -> bool {
    let media_type = media_type.split(';').next().unwrap_or(media_type).trim();
    media_type
        .get(..6)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("image/"))
}

impl Default for BrowserResourceCache {
    fn default() -> Self {
        Self::with_limits(
            DEFAULT_RESOURCE_CACHE_MAX_ENTRIES,
            DEFAULT_RESOURCE_CACHE_MAX_BYTES,
        )
    }
}

impl BrowserResourceCache {
    fn with_limits(max_entries: usize, max_bytes: usize) -> Self {
        Self {
            entries: HashMap::new(),
            order: VecDeque::new(),
            max_entries,
            max_bytes,
            evicted_entries: 0,
            evicted_bytes: 0,
        }
    }

    pub(super) fn len(&self) -> usize {
        self.entries.len()
    }

    pub(super) fn total_bytes(&self) -> usize {
        self.entries
            .values()
            .map(|resource| resource.bytes.len())
            .sum()
    }

    pub(super) fn cached_bytes(&self, url: &str) -> Option<&[u8]> {
        self.entries
            .get(url)
            .map(|resource| resource.bytes.as_slice())
    }

    #[allow(dead_code)]
    pub(super) fn evicted_entries(&self) -> usize {
        self.evicted_entries
    }

    #[allow(dead_code)]
    pub(super) fn evicted_bytes(&self) -> usize {
        self.evicted_bytes
    }

    pub(super) fn cached_resources(&self) -> impl Iterator<Item = (&str, Option<&str>, &[u8])> {
        self.entries.iter().map(|(url, resource)| {
            (
                url.as_str(),
                resource.content_type.as_deref(),
                resource.bytes.as_slice(),
            )
        })
    }

    fn pressure(&self) -> BrowserResourceCachePressure {
        BrowserResourceCachePressure {
            entries: self.len(),
            bytes: self.total_bytes(),
            evicted_entries: self.evicted_entries,
            evicted_bytes: self.evicted_bytes,
            max_entries: self.max_entries,
            max_bytes: self.max_bytes,
        }
    }

    fn touch(&mut self, url: &str) {
        self.order.retain(|entry| entry != url);
        self.order.push_back(url.to_owned());
    }

    fn insert(
        &mut self,
        url: String,
        resource: BrowserCachedResource,
    ) -> BrowserResourceCacheStoreOutcome {
        if self.max_entries == 0 || self.max_bytes == 0 {
            return BrowserResourceCacheStoreOutcome::SkippedDisabled;
        }

        let byte_len = resource.bytes.len();
        if byte_len > self.max_bytes {
            return BrowserResourceCacheStoreOutcome::SkippedOversize;
        }

        let outcome = if let Some(previous) = self.entries.remove(&url) {
            self.order.retain(|entry| entry != &url);
            drop(previous);
            BrowserResourceCacheStoreOutcome::Replaced
        } else {
            BrowserResourceCacheStoreOutcome::Stored
        };

        self.order.push_back(url.clone());
        self.entries.insert(url, resource);
        self.evict_over_limits();
        outcome
    }

    fn evict_over_limits(&mut self) {
        while self.entries.len() > self.max_entries || self.total_bytes() > self.max_bytes {
            let Some(url) = self.order.pop_front() else {
                break;
            };
            if let Some(resource) = self.entries.remove(&url) {
                self.evicted_entries += 1;
                self.evicted_bytes += resource.bytes.len();
            }
        }
    }
}

pub(super) async fn load_target(target: &str, max_bytes: usize) -> Result<(String, Vec<u8>)> {
    load_target_with_cookie_jar(target, max_bytes, None).await
}

#[allow(dead_code)]
pub(super) async fn load_target_with_cookie_jar_report(
    target: &str,
    max_bytes: usize,
    cookie_jar: Option<&mut BrowserCookieJar>,
) -> BrowserDocumentLoadReport {
    let started = Instant::now();
    match load_target_with_cookie_jar_inner(target, max_bytes, cookie_jar).await {
        Ok((source, bytes, content_type)) => {
            let target_diagnostics = resource_target_diagnostics(target, None);
            BrowserDocumentLoadReport {
                target: target.to_owned(),
                status: "loaded".to_owned(),
                source: Some(source),
                bytes: bytes.len(),
                content_type,
                error: None,
                error_kind: None,
                target_scheme: target_diagnostics.scheme,
                target_host: target_diagnostics.host,
                timed_out: target_diagnostics.timed_out,
                retryable: target_diagnostics.retryable,
                elapsed_ms: started.elapsed().as_millis(),
                request_timeout_ms: request_timeout_ms_for_target(target, true),
                diagnostic: Some("document_loaded".to_owned()),
            }
        }
        Err(error) => {
            let error_kind = classify_load_error(&error);
            let target_diagnostics = resource_target_diagnostics(target, Some(&error_kind));
            BrowserDocumentLoadReport {
                target: target.to_owned(),
                status: "failed".to_owned(),
                source: None,
                bytes: 0,
                content_type: None,
                error: Some(error.to_string()),
                error_kind: Some(error_kind.clone()),
                target_scheme: target_diagnostics.scheme,
                target_host: target_diagnostics.host,
                timed_out: target_diagnostics.timed_out,
                retryable: target_diagnostics.retryable,
                elapsed_ms: started.elapsed().as_millis(),
                request_timeout_ms: request_timeout_ms_for_target(target, true),
                diagnostic: Some(document_load_failure_diagnostic(&error_kind)),
            }
        }
    }
}

pub(super) async fn fetch_resource_with_cache(
    resource: BrowserResource,
    max_resource_bytes: usize,
    cookie_jar: &mut BrowserCookieJar,
    cache: &mut BrowserResourceCache,
) -> BrowserResourceFetch {
    let started = Instant::now();
    let request_timeout_ms = request_timeout_ms_for_target(&resource.resolved, false);
    if cache.entries.contains_key(&resource.resolved) {
        cache.touch(&resource.resolved);
        let cached = cache
            .entries
            .get(&resource.resolved)
            .expect("cache entry must exist after touch");
        return BrowserResourceFetch::from_args(
            BrowserResourceFetchArgs {
                resource,
                status: "cached".to_owned(),
                source: Some(cached.source.clone()),
                bytes: cached.bytes.len(),
                content_type: cached.content_type.clone(),
                error: None,
                error_kind: None,
                elapsed_ms: started.elapsed().as_millis(),
                request_timeout_ms: 0,
                cache_pressure: cache.pressure(),
                cache_outcome: Some("hit".to_owned()),
            },
            Some(cached.bytes.as_slice()),
        );
    }

    if resource.resolved.starts_with("data:") {
        return match load_data_url_resource(&resource.resolved, max_resource_bytes) {
            Ok((source, bytes, content_type)) => {
                let byte_len = bytes.len();
                let mut fetch = BrowserResourceFetch::from_args(
                    BrowserResourceFetchArgs {
                        resource: resource.clone(),
                        status: "cached".to_owned(),
                        source: Some(source.clone()),
                        bytes: byte_len,
                        content_type: Some(content_type.clone()),
                        error: None,
                        error_kind: None,
                        elapsed_ms: started.elapsed().as_millis(),
                        request_timeout_ms: 0,
                        cache_pressure: BrowserResourceCachePressure::default(),
                        cache_outcome: None,
                    },
                    Some(bytes.as_slice()),
                );
                let cache_outcome = cache.insert(
                    resource.resolved.clone(),
                    BrowserCachedResource {
                        source: source.clone(),
                        bytes,
                        content_type: Some(content_type.clone()),
                    },
                );
                fetch.set_cache_pressure(cache.pressure());
                fetch.set_cache_outcome(cache_outcome);
                fetch
            }
            Err(error) => BrowserResourceFetch::from_args(
                BrowserResourceFetchArgs {
                    resource,
                    status: "failed".to_owned(),
                    source: None,
                    bytes: 0,
                    content_type: None,
                    error: Some(error.to_string()),
                    error_kind: Some(classify_load_error(&error)),
                    elapsed_ms: started.elapsed().as_millis(),
                    request_timeout_ms: 0,
                    cache_pressure: cache.pressure(),
                    cache_outcome: Some("miss_failed".to_owned()),
                },
                None,
            ),
        };
    }

    if unsupported_resource_target(&resource.resolved) {
        return BrowserResourceFetch::from_args(
            BrowserResourceFetchArgs {
                resource,
                status: "skipped".to_owned(),
                source: None,
                bytes: 0,
                content_type: None,
                error: Some("unsupported resource scheme".to_owned()),
                error_kind: Some("unsupported_scheme".to_owned()),
                elapsed_ms: started.elapsed().as_millis(),
                request_timeout_ms: 0,
                cache_pressure: cache.pressure(),
                cache_outcome: Some("skipped".to_owned()),
            },
            None,
        );
    }

    match load_resource_target(&resource.resolved, max_resource_bytes, cookie_jar).await {
        Ok((source, bytes, content_type)) => {
            let byte_len = bytes.len();
            let mut fetch = BrowserResourceFetch::from_args(
                BrowserResourceFetchArgs {
                    resource: resource.clone(),
                    status: "fetched".to_owned(),
                    source: Some(source.clone()),
                    bytes: byte_len,
                    content_type: content_type.clone(),
                    error: None,
                    error_kind: None,
                    elapsed_ms: started.elapsed().as_millis(),
                    request_timeout_ms,
                    cache_pressure: BrowserResourceCachePressure::default(),
                    cache_outcome: None,
                },
                Some(bytes.as_slice()),
            );
            let cache_outcome = cache.insert(
                resource.resolved.clone(),
                BrowserCachedResource {
                    source: source.clone(),
                    bytes,
                    content_type: content_type.clone(),
                },
            );
            fetch.set_cache_pressure(cache.pressure());
            fetch.set_cache_outcome(cache_outcome);
            fetch
        }
        Err(error) => BrowserResourceFetch::from_args(
            BrowserResourceFetchArgs {
                resource,
                status: "failed".to_owned(),
                source: None,
                bytes: 0,
                content_type: None,
                error: Some(error.to_string()),
                error_kind: Some(classify_load_error(&error)),
                elapsed_ms: started.elapsed().as_millis(),
                request_timeout_ms,
                cache_pressure: cache.pressure(),
                cache_outcome: Some("miss_failed".to_owned()),
            },
            None,
        ),
    }
}

fn load_data_url_resource(target: &str, max_bytes: usize) -> Result<(String, Vec<u8>, String)> {
    let payload = target
        .strip_prefix("data:")
        .with_context(|| format!("parse data URL {target}"))?;
    let (metadata, data) = payload
        .split_once(',')
        .with_context(|| format!("parse data URL metadata for {target}"))?;
    let mut content_type = "text/plain".to_owned();
    let mut base64 = false;
    for (index, part) in metadata.split(';').enumerate() {
        if index == 0 && !part.is_empty() {
            content_type = part.to_owned();
        } else if part.eq_ignore_ascii_case("base64") {
            base64 = true;
        }
    }
    let bytes = if base64 {
        decode_base64_data_url_payload(data)?
    } else {
        percent_decode_data_url_payload(data)
    };
    ensure!(
        bytes.len() <= max_bytes,
        "resource exceeds byte cap: {} > {}",
        bytes.len(),
        max_bytes
    );
    let source = data_url_source_label(target, metadata, bytes.len());
    Ok((source, bytes, content_type))
}

fn data_url_source_label(target: &str, metadata: &str, decoded_bytes: usize) -> String {
    let metadata = if metadata.chars().count() > DATA_URL_SOURCE_METADATA_MAX_CHARS {
        let preview: String = metadata
            .chars()
            .take(DATA_URL_SOURCE_METADATA_MAX_CHARS)
            .collect();
        format!("{preview}...")
    } else {
        metadata.to_owned()
    };
    format!(
        "data:{metadata},... decoded_bytes={decoded_bytes} source_chars={}",
        target.len()
    )
}

fn serialize_resource_url<S>(target: &str, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    serializer.serialize_str(&resource_url_report_label(target))
}

fn serialize_report_page_source<S>(source: &str, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    serializer.serialize_str(&report_page_source_label(source))
}

fn serialize_optional_report_error<S>(
    error: &Option<String>,
    serializer: S,
) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    match error.as_deref() {
        Some(error) => serializer.serialize_some(&report_error_label(error)),
        None => serializer.serialize_none(),
    }
}

fn resource_url_report_label(target: &str) -> String {
    if !target.starts_with("data:") || target.chars().count() <= DATA_URL_REPORT_LABEL_MAX_CHARS {
        return target.to_owned();
    }

    let payload = target.strip_prefix("data:").unwrap_or_default();
    let metadata = payload
        .split_once(',')
        .map(|(metadata, _)| metadata)
        .unwrap_or("");
    let metadata = if metadata.chars().count() > DATA_URL_SOURCE_METADATA_MAX_CHARS {
        let preview: String = metadata
            .chars()
            .take(DATA_URL_SOURCE_METADATA_MAX_CHARS)
            .collect();
        format!("{preview}...")
    } else {
        metadata.to_owned()
    };
    format!("data:{metadata},... source_chars={}", target.len())
}

fn report_page_source_label(source: &str) -> String {
    let source_chars = source.chars().count();
    if source_chars <= REPORT_PAGE_SOURCE_MAX_CHARS {
        return source.to_owned();
    }

    let preview: String = source.chars().take(REPORT_PAGE_SOURCE_MAX_CHARS).collect();
    format!(
        "{preview}\n<!-- brutal-report-page-source-truncated source_chars={source_chars} retained_chars={REPORT_PAGE_SOURCE_MAX_CHARS} -->"
    )
}

fn report_error_label(error: &str) -> String {
    let error_chars = error.chars().count();
    if error_chars <= REPORT_ERROR_MAX_CHARS {
        return error.to_owned();
    }

    let preview: String = error.chars().take(REPORT_ERROR_MAX_CHARS).collect();
    format!(
        "{preview}\n[brutal-report-error-truncated error_chars={error_chars} retained_chars={REPORT_ERROR_MAX_CHARS}]"
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct BrowserResourceTargetDiagnostics {
    scheme: Option<String>,
    host: Option<String>,
    timed_out: bool,
    retryable: bool,
}

fn resource_target_diagnostics(
    target: &str,
    error_kind: Option<&str>,
) -> BrowserResourceTargetDiagnostics {
    let parsed = Url::parse(target).ok();
    let scheme = parsed
        .as_ref()
        .map(|url| url.scheme().to_owned())
        .or_else(|| target.split_once(':').map(|(scheme, _)| scheme.to_owned()));
    let host = parsed
        .as_ref()
        .and_then(|url| url.host_str().map(str::to_owned));
    let timed_out = error_kind == Some("timeout");
    let retryable = matches!(error_kind, Some("timeout" | "dns" | "connect"));

    BrowserResourceTargetDiagnostics {
        scheme,
        host,
        timed_out,
        retryable,
    }
}

fn decode_base64_data_url_payload(input: &str) -> Result<Vec<u8>> {
    let mut out = Vec::with_capacity(input.len().saturating_mul(3) / 4);
    let mut block = [0u8; 4];
    let mut block_len = 0usize;
    let mut padding = 0usize;

    for byte in input.bytes().filter(|byte| !byte.is_ascii_whitespace()) {
        let value = match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            b'=' => {
                padding += 1;
                0
            }
            _ => anyhow::bail!("invalid base64 data URL byte: 0x{byte:02x}"),
        };
        ensure!(
            padding == 0 || byte == b'=',
            "invalid base64 data URL padding"
        );
        block[block_len] = value;
        block_len += 1;
        if block_len == 4 {
            out.push((block[0] << 2) | (block[1] >> 4));
            if padding < 2 {
                out.push((block[1] << 4) | (block[2] >> 2));
            }
            if padding == 0 {
                out.push((block[2] << 6) | block[3]);
            }
            block_len = 0;
            padding = 0;
        }
    }

    ensure!(block_len == 0, "truncated base64 data URL payload");
    Ok(out)
}

fn percent_decode_data_url_payload(input: &str) -> Vec<u8> {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0usize;
    while index < bytes.len() {
        if bytes[index] == b'%'
            && let (Some(high), Some(low)) = (
                bytes
                    .get(index + 1)
                    .and_then(|byte| data_url_hex_value(*byte)),
                bytes
                    .get(index + 2)
                    .and_then(|byte| data_url_hex_value(*byte)),
            )
        {
            out.push((high << 4) | low);
            index += 3;
        } else {
            out.push(bytes[index]);
            index += 1;
        }
    }
    out
}

fn data_url_hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn http_load_timeout(require_html: bool) -> Duration {
    if require_html {
        DOCUMENT_HTTP_TIMEOUT
    } else {
        RESOURCE_HTTP_TIMEOUT
    }
}

fn duration_millis(duration: Duration) -> u128 {
    duration.as_millis()
}

fn request_timeout_ms_for_target(target: &str, require_html: bool) -> u128 {
    if target.starts_with("http://") || target.starts_with("https://") {
        duration_millis(http_load_timeout(require_html))
    } else {
        0
    }
}

fn resource_fetch_diagnostic(
    status: &str,
    error_kind: Option<&str>,
    cache_outcome: Option<&str>,
    image_decode_status: Option<&str>,
) -> Option<String> {
    if matches!(
        image_decode_status,
        Some("unsupported_format" | "undecoded")
    ) {
        return Some("image_decode_failed".to_owned());
    }
    if image_decode_status == Some("decoded") {
        return Some("image_decoded".to_owned());
    }
    if image_decode_status == Some("not_fetched") {
        return Some("image_not_fetched".to_owned());
    }

    match (status, error_kind, cache_outcome) {
        ("cached", _, Some("hit")) => Some("cache_hit".to_owned()),
        ("cached" | "fetched", _, Some("stored")) => Some("resource_fetch_cached".to_owned()),
        ("cached" | "fetched", _, Some("replaced")) => {
            Some("resource_fetch_replaced_cache_entry".to_owned())
        }
        ("cached" | "fetched", _, Some("skipped_oversize")) => {
            Some("resource_fetch_uncached_oversize".to_owned())
        }
        ("cached" | "fetched", _, Some("skipped_disabled")) => {
            Some("resource_fetch_cache_disabled".to_owned())
        }
        ("failed", Some("timeout"), _) => Some("network_timeout".to_owned()),
        ("failed", Some(kind), _) => Some(format!("network_{kind}")),
        ("skipped", Some("unsupported_scheme"), _) => Some("unsupported_scheme".to_owned()),
        ("skipped", _, _) => Some("skipped_resource".to_owned()),
        _ => None,
    }
}

fn document_load_failure_diagnostic(error_kind: &str) -> String {
    match error_kind {
        "timeout" => "document_timeout".to_owned(),
        "unsupported_content_type" => "document_unsupported_content_type".to_owned(),
        "byte_cap" => "document_byte_cap".to_owned(),
        "redirect" => "document_redirect_failed".to_owned(),
        "dns" => "document_dns_failed".to_owned(),
        "connect" => "document_connect_failed".to_owned(),
        _ => format!("document_{error_kind}"),
    }
}

pub(super) async fn load_target_with_cookie_jar(
    target: &str,
    max_bytes: usize,
    cookie_jar: Option<&mut BrowserCookieJar>,
) -> Result<(String, Vec<u8>)> {
    let (source, bytes, _) =
        load_target_with_cookie_jar_inner(target, max_bytes, cookie_jar).await?;
    Ok((source, bytes))
}

async fn load_target_with_cookie_jar_inner(
    target: &str,
    max_bytes: usize,
    cookie_jar: Option<&mut BrowserCookieJar>,
) -> Result<(String, Vec<u8>, Option<String>)> {
    if target.starts_with("http://") || target.starts_with("https://") {
        load_http_bytes(target, max_bytes, cookie_jar, true, None).await
    } else if target.starts_with("file://") {
        let url = Url::parse(target).with_context(|| format!("parse file URL {target}"))?;
        let path = url.to_file_path().map_err(|_| {
            anyhow::anyhow!("file URL cannot be converted to a local path: {target}")
        })?;
        let content_type = content_type_for_path(&path).map(str::to_owned);
        let (source, bytes) = load_file_with_source(target, &path, max_bytes)?;
        Ok((source, bytes, content_type))
    } else {
        let path = local_path_without_url_parts(target);
        let content_type = content_type_for_path(Path::new(path)).map(str::to_owned);
        let (source, bytes) = load_file_with_source(target, Path::new(path), max_bytes)?;
        Ok((source, bytes, content_type))
    }
}

pub(super) async fn load_post_form_target_with_cookie_jar(
    target: &str,
    body: String,
    max_bytes: usize,
    cookie_jar: &mut BrowserCookieJar,
) -> Result<(String, Vec<u8>)> {
    ensure!(
        target.starts_with("http://") || target.starts_with("https://"),
        "POST form submission currently requires an HTTP(S) action target"
    );
    let (source, bytes, _) =
        load_http_bytes(target, max_bytes, Some(cookie_jar), true, Some(body)).await?;
    Ok((source, bytes))
}

pub(super) fn local_path_without_url_parts(target: &str) -> &str {
    let end = target.find(['?', '#']).unwrap_or(target.len());
    &target[..end]
}

async fn load_resource_target(
    target: &str,
    max_bytes: usize,
    cookie_jar: &mut BrowserCookieJar,
) -> Result<(String, Vec<u8>, Option<String>)> {
    if target.starts_with("http://") || target.starts_with("https://") {
        let (source, bytes, content_type) =
            load_http_bytes(target, max_bytes, Some(cookie_jar), false, None).await?;
        Ok((source, bytes, content_type))
    } else if target.starts_with("file://") {
        let url = Url::parse(target).with_context(|| format!("parse file URL {target}"))?;
        let path = url.to_file_path().map_err(|_| {
            anyhow::anyhow!("file URL cannot be converted to a local path: {target}")
        })?;
        let content_type = content_type_for_path(&path).map(str::to_owned);
        let (source, bytes) = load_file_with_source(target, &path, max_bytes)?;
        Ok((source, bytes, content_type))
    } else {
        let path = local_path_without_url_parts(target);
        let path = Path::new(path);
        let content_type = content_type_for_path(path).map(str::to_owned);
        let (source, bytes) = load_file_with_source(target, path, max_bytes)?;
        Ok((source, bytes, content_type))
    }
}

async fn load_http_bytes(
    target: &str,
    max_bytes: usize,
    mut cookie_jar: Option<&mut BrowserCookieJar>,
    require_html: bool,
    post_form_body: Option<String>,
) -> Result<(String, Vec<u8>, Option<String>)> {
    let mut headers = HeaderMap::new();
    headers.insert(
        USER_AGENT,
        HeaderValue::from_static("brutal-browser/0.1 static-engine"),
    );
    let client = reqwest::Client::builder()
        .default_headers(headers)
        .timeout(http_load_timeout(require_html))
        .redirect(reqwest::redirect::Policy::none())
        .build()?;
    let mut current_url = Url::parse(target).with_context(|| format!("parse URL {target}"))?;
    let mut method = if let Some(body) = post_form_body {
        BrowserHttpMethod::Post(body)
    } else {
        BrowserHttpMethod::Get
    };

    for redirect_count in 0..=5 {
        let mut request = match &method {
            BrowserHttpMethod::Get => client.get(current_url.clone()),
            BrowserHttpMethod::Post(body) => client
                .post(current_url.clone())
                .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(body.clone()),
        };
        if let Some(cookie_header) = cookie_jar
            .as_deref()
            .and_then(|cookie_jar| cookie_jar.cookie_header(current_url.as_str()))
        {
            request = request.header(
                COOKIE,
                HeaderValue::from_str(&cookie_header).context("build Cookie header")?,
            );
        }

        let response = request
            .send()
            .await
            .with_context(|| format!("load {}", current_url.as_str()))?;
        let response_url = response.url().to_string();
        let set_cookie_headers = response_set_cookie_headers(response.headers());
        if let Some(cookie_jar) = cookie_jar.as_deref_mut() {
            cookie_jar.store_from_set_cookie_headers(&response_url, &set_cookie_headers);
        }

        if response.status().is_redirection() {
            ensure!(
                redirect_count < 5,
                "load {target} exceeded redirect limit of 5"
            );
            let location = response
                .headers()
                .get(LOCATION)
                .and_then(|value| value.to_str().ok())
                .with_context(|| {
                    format!(
                        "load {} returned redirect without Location",
                        current_url.as_str()
                    )
                })?;
            current_url = response
                .url()
                .join(location)
                .with_context(|| format!("resolve redirect Location {location:?}"))?;
            ensure!(
                matches!(current_url.scheme(), "http" | "https"),
                "unsupported redirect target scheme: {}",
                current_url.scheme()
            );
            method = redirected_method(response.status(), method);
            continue;
        }

        let content_type = response
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        let status = response.status();
        let final_url = response.url().to_string();
        if !status.is_success() {
            if require_html
                && content_type
                    .as_deref()
                    .is_some_and(is_browser_renderable_content_type)
            {
                let bytes = response.bytes().await?;
                ensure!(
                    bytes.len() <= max_bytes,
                    "document exceeds byte cap: {} > {}",
                    bytes.len(),
                    max_bytes
                );
                return Ok((final_url, bytes.to_vec(), content_type));
            }
            ensure!(status.is_success(), "load {target} failed with {status}");
        }
        if require_html && let Some(content_type) = content_type.as_deref() {
            ensure!(
                is_browser_renderable_content_type(content_type),
                "unsupported content type for browser render: {content_type}"
            );
        }
        let bytes = response.bytes().await?;
        ensure!(
            bytes.len() <= max_bytes,
            "document exceeds byte cap: {} > {}",
            bytes.len(),
            max_bytes
        );
        return Ok((final_url, bytes.to_vec(), content_type));
    }

    unreachable!("redirect loop must return or fail at the configured limit")
}

fn is_browser_renderable_content_type(content_type: &str) -> bool {
    content_type.contains("text/html") || content_type.contains("application/xhtml")
}

#[derive(Debug, Clone)]
enum BrowserHttpMethod {
    Get,
    Post(String),
}

fn response_set_cookie_headers(headers: &HeaderMap) -> Vec<String> {
    headers
        .get_all(SET_COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok().map(str::to_owned))
        .collect()
}

fn redirected_method(status: StatusCode, method: BrowserHttpMethod) -> BrowserHttpMethod {
    match status {
        StatusCode::MOVED_PERMANENTLY | StatusCode::FOUND | StatusCode::SEE_OTHER => {
            BrowserHttpMethod::Get
        }
        StatusCode::TEMPORARY_REDIRECT | StatusCode::PERMANENT_REDIRECT => method,
        _ => BrowserHttpMethod::Get,
    }
}

fn unsupported_resource_target(target: &str) -> bool {
    Url::parse(target).is_ok_and(|url| {
        !matches!(url.scheme(), "http" | "https" | "file")
            && !target.starts_with('/')
            && !target.starts_with('.')
    })
}

fn load_file_with_source(source: &str, path: &Path, max_bytes: usize) -> Result<(String, Vec<u8>)> {
    let bytes = fs::read(path).with_context(|| format!("read {}", path.display()))?;
    ensure!(
        bytes.len() <= max_bytes,
        "document exceeds byte cap: {} > {}",
        bytes.len(),
        max_bytes
    );
    Ok((source.to_owned(), bytes))
}

fn classify_load_error(error: &anyhow::Error) -> String {
    let error_text = error.to_string().to_ascii_lowercase();
    if error.chain().any(|cause| {
        cause
            .downcast_ref::<reqwest::Error>()
            .is_some_and(reqwest::Error::is_timeout)
    }) || error_text.contains("operation timed out")
        || error_text.contains("deadline has elapsed")
        || error_text.contains("request timed out")
    {
        "timeout".to_owned()
    } else if error_text.contains("unsupported resource scheme") {
        "unsupported_scheme".to_owned()
    } else if error_text.contains("unsupported content type") {
        "unsupported_content_type".to_owned()
    } else if error_text.contains("exceeds byte cap") {
        "byte_cap".to_owned()
    } else if error_text.contains("redirect") {
        "redirect".to_owned()
    } else if error_text.contains("dns") || error_text.contains("resolve") {
        "dns".to_owned()
    } else if error_text.contains("connect") {
        "connect".to_owned()
    } else {
        "error".to_owned()
    }
}

fn content_type_for_path(path: &Path) -> Option<&'static str> {
    match path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("css") => Some("text/css"),
        Some("js") | Some("mjs") => Some("text/javascript"),
        Some("json") | Some("webmanifest") => Some("application/json"),
        Some("html") | Some("htm") | Some("xhtml") => Some("text/html"),
        Some("svg") => Some("image/svg+xml"),
        Some("png") => Some("image/png"),
        Some("jpg") | Some("jpeg") => Some("image/jpeg"),
        Some("gif") => Some("image/gif"),
        Some("webp") => Some("image/webp"),
        Some("ico") => Some("image/x-icon"),
        Some("mp4") => Some("video/mp4"),
        Some("webm") => Some("video/webm"),
        Some("mp3") => Some("audio/mpeg"),
        Some("wav") => Some("audio/wav"),
        Some("wasm") => Some("application/wasm"),
        _ => None,
    }
}

pub(super) fn collect_resources(dom: &Dom, source: &str) -> Vec<BrowserResource> {
    let mut resources = Vec::new();
    collect_resources_at(dom, 0, source, &mut resources);
    resources
}

pub(super) fn collect_selected_image_resources(
    dom: &Dom,
    source: &str,
    viewport_width_css_px: usize,
) -> Vec<BrowserResource> {
    let mut resources = Vec::new();
    collect_selected_image_resources_at(dom, 0, source, viewport_width_css_px, &mut resources);
    dedupe_resources_by_resolved(resources)
}

fn dedupe_resources_by_resolved(resources: Vec<BrowserResource>) -> Vec<BrowserResource> {
    let mut seen = HashSet::new();
    resources
        .into_iter()
        .filter(|resource| seen.insert(resource.resolved.clone()))
        .collect()
}

fn collect_selected_image_resources_at(
    dom: &Dom,
    node_id: usize,
    source: &str,
    viewport_width_css_px: usize,
    resources: &mut Vec<BrowserResource>,
) {
    let Some(node) = dom.nodes.get(node_id) else {
        return;
    };

    if let NodeKind::Element(element) = &node.kind {
        if element.tag == "img"
            && let Some(url) = image_render_source(dom, node_id, element, viewport_width_css_px)
        {
            push_resource(resources, source, element, "image", "img", &url);
        } else if element.tag == "link" && link_preloads_image(element) {
            push_link_image_resources(
                resources,
                source,
                element,
                true,
                Some(viewport_width_css_px),
            );
        } else if element.tag == "video"
            && let Some(poster) = element.poster.as_deref().map(str::trim)
            && !poster.is_empty()
        {
            push_resource(resources, source, element, "poster", "video", poster);
        } else if let Some(url) = selected_replaced_media_image_url(element) {
            push_resource(resources, source, element, "image", &element.tag, url);
        }
        push_selected_background_alias_resource(resources, source, element, viewport_width_css_px);
    }

    for &child in &node.children {
        collect_selected_image_resources_at(dom, child, source, viewport_width_css_px, resources);
    }
}

fn collect_resources_at(
    dom: &Dom,
    node_id: usize,
    source: &str,
    resources: &mut Vec<BrowserResource>,
) {
    let Some(node) = dom.nodes.get(node_id) else {
        return;
    };

    if let NodeKind::Element(element) = &node.kind {
        match element.tag.as_str() {
            "script" => push_src_resource(resources, source, element, "script"),
            "img" => {
                push_img_primary_resource(resources, source, dom, node_id, element);
                push_image_srcset_resources(resources, source, element, "image_candidate");
                push_image_alias_resources(resources, source, element);
            }
            "source"
                if parent_element_tag_is(dom, node_id, "picture")
                    && picture_source_resource_type_supported(element) =>
            {
                push_src_resource(resources, source, element, "image");
                push_image_srcset_resources(resources, source, element, "image_candidate");
                push_image_alias_resources(resources, source, element);
            }
            "source" if parent_element_tag_is(dom, node_id, "picture") => {}
            "source" => {
                push_src_resource(resources, source, element, "media_source");
                push_srcset_resources(resources, source, element, "media_candidate");
            }
            "video" | "audio" => push_src_resource(resources, source, element, "media"),
            "iframe" => push_src_resource(resources, source, element, "frame"),
            "embed" => push_src_resource(resources, source, element, "embed"),
            "object" => {
                if let Some(data) = element.data.as_deref() {
                    push_resource(resources, source, element, "object", "object", data);
                }
            }
            "link" => {
                if let Some(href) = element.href.as_deref().map(str::trim)
                    && !href.is_empty()
                {
                    let kind = link_resource_kind(element);
                    push_resource(resources, source, element, &kind, "link", href);
                }
                push_link_image_resources(resources, source, element, false, None);
            }
            _ => {}
        }

        if matches!(element.tag.as_str(), "video" | "audio")
            && let Some(poster) = element.poster.as_deref().map(str::trim)
            && !poster.is_empty()
        {
            push_resource(resources, source, element, "poster", &element.tag, poster);
        }

        push_background_alias_resources(resources, source, element);
    }

    for &child in &node.children {
        collect_resources_at(dom, child, source, resources);
    }
}

fn picture_source_resource_type_supported(element: &ElementData) -> bool {
    element
        .attrs
        .get("type")
        .is_none_or(|source_type| image_mime_type_supported(source_type))
}

fn parent_element_tag_is(dom: &Dom, node_id: usize, tag: &str) -> bool {
    let Some(parent) = dom.nodes.get(node_id).and_then(|node| node.parent) else {
        return false;
    };
    matches!(
        dom.nodes.get(parent).map(|node| &node.kind),
        Some(NodeKind::Element(element)) if element.tag == tag
    )
}

fn push_src_resource(
    resources: &mut Vec<BrowserResource>,
    source: &str,
    element: &ElementData,
    kind: &str,
) {
    if let Some(src) = element.src.as_deref().map(str::trim)
        && !src.is_empty()
    {
        push_resource(resources, source, element, kind, &element.tag, src);
    }
}

fn push_img_primary_resource(
    resources: &mut Vec<BrowserResource>,
    source: &str,
    dom: &Dom,
    node_id: usize,
    element: &ElementData,
) {
    let Some(src) = element
        .src
        .as_deref()
        .map(str::trim)
        .filter(|src| !src.is_empty())
    else {
        return;
    };
    let selected = image_render_source(dom, node_id, element, usize::MAX);
    if selected.as_deref().is_some_and(|selected| selected != src) {
        return;
    }
    push_resource(resources, source, element, "image", &element.tag, src);
}

fn push_srcset_resources(
    resources: &mut Vec<BrowserResource>,
    source: &str,
    element: &ElementData,
    kind: &str,
) {
    let Some(srcset) = element.srcset.as_deref() else {
        return;
    };
    for url in srcset_candidate_urls(srcset) {
        push_resource(resources, source, element, kind, &element.tag, &url);
    }
}

fn push_image_srcset_resources(
    resources: &mut Vec<BrowserResource>,
    source: &str,
    element: &ElementData,
    kind: &str,
) {
    let Some(srcset) = element.srcset.as_deref() else {
        return;
    };
    for url in supported_srcset_candidate_urls(srcset) {
        push_resource(resources, source, element, kind, &element.tag, &url);
    }
}

fn push_image_alias_resources(
    resources: &mut Vec<BrowserResource>,
    source: &str,
    element: &ElementData,
) {
    for attr_name in IMAGE_SRC_ALIAS_ATTRS {
        if let Some(url) = element.attrs.get(*attr_name).map(String::as_str)
            && !url.trim().is_empty()
        {
            push_resource(resources, source, element, "image", &element.tag, url);
        }
    }

    for attr_name in IMAGE_SRCSET_ALIAS_ATTRS {
        let Some(srcset) = element.attrs.get(*attr_name).map(String::as_str) else {
            continue;
        };
        for url in supported_srcset_candidate_urls(srcset) {
            push_resource(
                resources,
                source,
                element,
                "image_candidate",
                &element.tag,
                &url,
            );
        }
    }
}

const IMAGE_SRC_ALIAS_ATTRS: &[&str] = &[
    "data-src",
    "data-lazy-src",
    "data-lazysrc",
    "data-lazyload",
    "data-lazyload-src",
    "data-original-url",
    "data-original",
    "data-original-src",
    "data-originalsrc",
    "data-flickity-lazyload",
    "data-flickity-lazyload-src",
    "data-image",
    "data-image-src",
    "data-imagesrc",
    "data-img-src",
    "data-imgsrc",
    "data-current-src",
    "data-currentsrc",
    "current-src",
    "currentsrc",
];

const IMAGE_SRCSET_ALIAS_ATTRS: &[&str] = &[
    "data-srcset",
    "data-lazy-srcset",
    "data-lazysrcset",
    "data-lazyload-srcset",
    "data-original-srcset",
    "data-originalset",
    "data-originalsrcset",
    "data-flickity-lazyload-srcset",
    "data-image-srcset",
    "data-imagesrcset",
    "data-img-srcset",
    "data-imgsrcset",
    "data-current-srcset",
    "data-currentsrcset",
    "current-srcset",
    "currentsrcset",
];

fn push_selected_background_alias_resource(
    resources: &mut Vec<BrowserResource>,
    source: &str,
    element: &ElementData,
    viewport_width_css_px: usize,
) {
    for attr_name in BACKGROUND_IMAGE_SRCSET_ALIAS_ATTRS {
        if let Some(srcset) = element.attrs.get(*attr_name).map(String::as_str)
            && let Some(url) = selected_supported_srcset_candidate(
                srcset,
                background_image_sizes_attr(element),
                viewport_width_css_px,
            )
        {
            push_resource(
                resources,
                source,
                element,
                "background_image",
                &element.tag,
                &url,
            );
            return;
        }
    }

    for attr_name in BACKGROUND_IMAGE_SRC_ALIAS_ATTRS {
        if let Some(value) = element.attrs.get(*attr_name).map(String::as_str)
            && let Some(url) = background_image_urls_from_attr_value(value)
                .into_iter()
                .next()
        {
            push_resource(
                resources,
                source,
                element,
                "background_image",
                &element.tag,
                url,
            );
            return;
        }
    }
}

fn push_background_alias_resources(
    resources: &mut Vec<BrowserResource>,
    source: &str,
    element: &ElementData,
) {
    for attr_name in BACKGROUND_IMAGE_SRC_ALIAS_ATTRS {
        if let Some(value) = element.attrs.get(*attr_name).map(String::as_str) {
            for url in background_image_urls_from_attr_value(value) {
                push_resource(
                    resources,
                    source,
                    element,
                    "background_image",
                    &element.tag,
                    url,
                );
            }
        }
    }

    for attr_name in BACKGROUND_IMAGE_SRCSET_ALIAS_ATTRS {
        let Some(srcset) = element.attrs.get(*attr_name).map(String::as_str) else {
            continue;
        };
        for url in supported_srcset_candidate_urls(srcset) {
            push_resource(
                resources,
                source,
                element,
                "background_image",
                &element.tag,
                &url,
            );
        }
    }
}

fn background_image_urls_from_attr_value(value: &str) -> Vec<&str> {
    let value = value.trim();
    if value.is_empty() {
        return Vec::new();
    }
    if let Some(args) = css_function_args(value, &["image-set", "-webkit-image-set"]) {
        let urls = split_css_top_level_commas(args)
            .into_iter()
            .filter_map(background_image_set_candidate_url)
            .collect::<Vec<_>>();
        if urls
            .iter()
            .any(|url| !background_image_candidate_clearly_unsupported(url))
        {
            return urls
                .into_iter()
                .filter(|url| !background_image_candidate_clearly_unsupported(url))
                .collect();
        }
        return urls;
    }
    if let Some(inner) = value
        .strip_prefix("url(")
        .and_then(|value| value.strip_suffix(')'))
    {
        let url = inner.trim().trim_matches(['"', '\'']);
        return (!url.is_empty()).then_some(url).into_iter().collect();
    }
    if value.contains(';') || value.contains('{') || value.contains('}') {
        return Vec::new();
    }
    vec![value]
}

fn background_image_set_candidate_url(candidate: &str) -> Option<&str> {
    if background_image_set_candidate_type_unsupported(candidate) {
        return None;
    }
    css_function_args(candidate, &["url"])
        .and_then(css_url_token)
        .or_else(|| css_quoted_url(candidate))
}

fn background_image_set_candidate_type_unsupported(candidate: &str) -> bool {
    css_nested_function_args(candidate, &["type"])
        .and_then(css_url_token)
        .is_some_and(|image_type| !image_mime_type_supported(image_type))
}

fn css_nested_function_args<'a>(value: &'a str, names: &[&str]) -> Option<&'a str> {
    for (open, _) in value.match_indices('(') {
        let name_start = value[..open]
            .rfind(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '-'))
            .map(|index| index.saturating_add(1))
            .unwrap_or(0);
        let name = value[name_start..open].trim();
        if names
            .iter()
            .any(|expected| name.eq_ignore_ascii_case(expected))
        {
            let close = matching_closing_paren(value, open)?;
            return Some(&value[open + 1..close]);
        }
    }
    None
}

fn css_function_args<'a>(value: &'a str, names: &[&str]) -> Option<&'a str> {
    let value = value.trim();
    let open = value.find('(')?;
    let name = value[..open].trim();
    if !names
        .iter()
        .any(|expected| name.eq_ignore_ascii_case(expected))
    {
        return None;
    }
    let close = matching_closing_paren(value, open)?;
    Some(&value[open + 1..close])
}

fn matching_closing_paren(input: &str, open_index: usize) -> Option<usize> {
    if input.as_bytes().get(open_index) != Some(&b'(') {
        return None;
    }
    let mut depth = 0usize;
    let mut quote = None;
    for (index, byte) in input.as_bytes().iter().enumerate().skip(open_index) {
        if let Some(quote_byte) = quote {
            if *byte == quote_byte {
                quote = None;
            }
            continue;
        }
        match *byte {
            b'\'' | b'"' => quote = Some(*byte),
            b'(' => depth = depth.saturating_add(1),
            b')' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return Some(index);
                }
            }
            _ => {}
        }
    }
    None
}

fn split_css_top_level_commas(input: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut start = 0usize;
    let mut depth = 0usize;
    let mut quote = None;
    for (index, byte) in input.as_bytes().iter().enumerate() {
        if let Some(quote_byte) = quote {
            if *byte == quote_byte {
                quote = None;
            }
            continue;
        }
        match *byte {
            b'\'' | b'"' => quote = Some(*byte),
            b'(' => depth = depth.saturating_add(1),
            b')' => depth = depth.saturating_sub(1),
            b',' if depth == 0 => {
                parts.push(input[start..index].trim());
                start = index.saturating_add(1);
            }
            _ => {}
        }
    }
    parts.push(input[start..].trim());
    parts
}

fn css_url_token(value: &str) -> Option<&str> {
    let value = value.trim();
    if value.is_empty() {
        return None;
    }
    let bytes = value.as_bytes();
    let url = if bytes.len() >= 2
        && matches!(bytes[0], b'\'' | b'"')
        && bytes.last() == Some(&bytes[0])
    {
        &value[1..value.len().saturating_sub(1)]
    } else {
        value
    };
    (!url.is_empty()).then_some(url)
}

fn css_quoted_url(value: &str) -> Option<&str> {
    let value = value.trim_start();
    let quote = value.as_bytes().first().copied()?;
    if !matches!(quote, b'\'' | b'"') {
        return None;
    }
    let end = value[1..]
        .find(quote as char)
        .map(|offset| 1usize.saturating_add(offset))?;
    css_url_token(&value[..=end])
}

fn background_image_candidate_clearly_unsupported(url: &str) -> bool {
    let url = url.trim();
    let path = Url::parse(url)
        .ok()
        .map(|url| url.path().to_owned())
        .unwrap_or_else(|| url.split(['?', '#']).next().unwrap_or(url).to_owned());
    Path::new(&path)
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            matches!(
                extension.to_ascii_lowercase().as_str(),
                "avif" | "avifs" | "heic" | "heif" | "gif" | "bmp" | "ico" | "tif" | "tiff"
            )
        })
}

const BACKGROUND_IMAGE_SRC_ALIAS_ATTRS: &[&str] = &[
    "data-bg",
    "data-bg-src",
    "data-bgsrc",
    "data-background",
    "data-background-image",
    "data-backgroundimage",
    "data-background-src",
    "data-backgroundsrc",
    "data-lazy-bg",
    "data-lazybg",
    "data-lazy-background",
    "data-lazybackground",
    "data-lazy-background-image",
    "data-lazybackgroundimage",
];

const BACKGROUND_IMAGE_SRCSET_ALIAS_ATTRS: &[&str] = &[
    "data-bgset",
    "data-background-srcset",
    "data-backgroundsrcset",
    "data-lazy-bgset",
    "data-lazybgset",
    "data-lazy-background-srcset",
    "data-lazybackgroundsrcset",
];

fn push_link_image_resources(
    resources: &mut Vec<BrowserResource>,
    source: &str,
    element: &ElementData,
    include_href: bool,
    selected_viewport_width_css_px: Option<usize>,
) {
    if !link_preloads_image(element) {
        return;
    }
    if let Some(viewport_width_css_px) = selected_viewport_width_css_px
        && let Some(srcset) = element.attrs.get("imagesrcset").map(String::as_str)
        && let Some(url) = selected_supported_srcset_candidate(
            srcset,
            element
                .attrs
                .get("imagesizes")
                .map(String::as_str)
                .or_else(|| image_sizes_attr(element)),
            viewport_width_css_px,
        )
    {
        push_resource(resources, source, element, "image", "link", &url);
        return;
    }
    if include_href
        && let Some(href) = element.href.as_deref().map(str::trim)
        && !href.is_empty()
    {
        push_resource(resources, source, element, "image", "link", href);
    }
    let Some(srcset) = element.attrs.get("imagesrcset").map(String::as_str) else {
        return;
    };
    for url in supported_srcset_candidate_urls(srcset) {
        push_resource(resources, source, element, "image_candidate", "link", &url);
    }
}

fn push_resource(
    resources: &mut Vec<BrowserResource>,
    source: &str,
    element: &ElementData,
    kind: &str,
    initiator: &str,
    url: &str,
) {
    resources.push(BrowserResource {
        kind: kind.to_owned(),
        initiator: initiator.to_owned(),
        url: url.to_owned(),
        resolved: resolve_browser_href(source, url),
        rel: element.rel.clone(),
        media: element.media.clone(),
        alt: element.alt.clone(),
        type_hint: element.type_hint.clone(),
    });
}

fn link_resource_kind(element: &ElementData) -> String {
    if link_rel_contains(element.rel.as_deref(), "stylesheet") {
        "stylesheet".to_owned()
    } else if link_rel_contains(element.rel.as_deref(), "icon") {
        "icon".to_owned()
    } else if link_preloads_image(element) {
        "image".to_owned()
    } else if link_rel_contains(element.rel.as_deref(), "preload") {
        "preload".to_owned()
    } else if link_rel_contains(element.rel.as_deref(), "modulepreload") {
        "modulepreload".to_owned()
    } else if link_rel_contains(element.rel.as_deref(), "manifest") {
        "manifest".to_owned()
    } else {
        "link".to_owned()
    }
}

fn link_rel_contains(rel: Option<&str>, needle: &str) -> bool {
    rel.unwrap_or_default()
        .split_ascii_whitespace()
        .any(|item| item.eq_ignore_ascii_case(needle))
}

fn link_preloads_image(element: &ElementData) -> bool {
    link_rel_contains(element.rel.as_deref(), "preload")
        && element
            .attrs
            .get("as")
            .is_some_and(|as_attr| as_attr.trim().eq_ignore_ascii_case("image"))
}

fn selected_replaced_media_image_url(element: &ElementData) -> Option<&str> {
    let url = match element.tag.as_str() {
        "object" => element.data.as_deref(),
        "embed" => element.src.as_deref(),
        _ => None,
    }?
    .trim();
    (!url.is_empty() && url_likely_supported_image(url)).then_some(url)
}

fn url_likely_supported_image(url: &str) -> bool {
    let url = url.trim();
    if url
        .get(..11)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("data:image/"))
    {
        return true;
    }
    let path = Url::parse(url)
        .ok()
        .map(|url| url.path().to_owned())
        .unwrap_or_else(|| url.split(['?', '#']).next().unwrap_or(url).to_owned());
    Path::new(&path)
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            matches!(
                extension.to_ascii_lowercase().as_str(),
                "svg" | "png" | "jpg" | "jpeg" | "jpe" | "jfif" | "pjpeg" | "pjp" | "webp"
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::{TcpListener, TcpStream};

    fn test_resource(url: &str) -> BrowserResource {
        BrowserResource {
            kind: "stylesheet".to_owned(),
            initiator: "link".to_owned(),
            url: url.to_owned(),
            resolved: url.to_owned(),
            rel: None,
            media: None,
            alt: None,
            type_hint: None,
        }
    }

    async fn read_http_request(stream: &mut TcpStream) -> (String, String) {
        let mut request_bytes = Vec::new();
        let mut buf = [0u8; 4096];
        loop {
            let n = stream.read(&mut buf).await.unwrap();
            assert!(n > 0);
            request_bytes.extend_from_slice(&buf[..n]);
            let Some(header_end) = request_bytes.windows(4).position(|w| w == b"\r\n\r\n") else {
                continue;
            };
            let request_head = String::from_utf8_lossy(&request_bytes[..header_end]);
            let content_length = request_head
                .lines()
                .find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    name.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse::<usize>().ok())
                        .flatten()
                })
                .unwrap_or(0);
            if request_bytes.len() >= header_end + 4 + content_length {
                let body = String::from_utf8_lossy(&request_bytes[header_end + 4..]).to_string();
                return (request_head.to_string(), body);
            }
        }
    }

    #[tokio::test]
    async fn resource_cache_evicts_oldest_entries_and_tracks_pressure() {
        let mut cache = BrowserResourceCache::with_limits(2, 1024);
        let mut jar = BrowserCookieJar::default();
        let first = "data:text/css,one";
        let second = "data:text/css,two";
        let third = "data:text/css,three";

        let first_fetch =
            fetch_resource_with_cache(test_resource(first), 64, &mut jar, &mut cache).await;
        assert_eq!(first_fetch.status, "cached");
        assert_eq!(first_fetch.error_kind, None);
        assert_eq!(first_fetch.cache_entries, 1);
        assert_eq!(first_fetch.cache_bytes, 3);
        assert_eq!(first_fetch.cache_max_entries, 2);
        assert_eq!(first_fetch.cache_max_bytes, 1024);
        assert_eq!(first_fetch.cache_outcome.as_deref(), Some("stored"));
        assert_eq!(
            first_fetch.diagnostic.as_deref(),
            Some("resource_fetch_cached")
        );
        assert!(cache.cached_bytes(first).is_some());

        let second_fetch =
            fetch_resource_with_cache(test_resource(second), 64, &mut jar, &mut cache).await;
        assert_eq!(second_fetch.status, "cached");
        assert_eq!(second_fetch.cache_outcome.as_deref(), Some("stored"));
        assert_eq!(cache.len(), 2);

        let hit = fetch_resource_with_cache(test_resource(first), 64, &mut jar, &mut cache).await;
        assert_eq!(hit.status, "cached");
        assert_eq!(hit.bytes, 3);
        assert_eq!(hit.cache_entries, 2);
        assert_eq!(hit.cache_bytes, 6);
        assert_eq!(hit.cache_outcome.as_deref(), Some("hit"));
        assert_eq!(hit.diagnostic.as_deref(), Some("cache_hit"));

        let third_fetch =
            fetch_resource_with_cache(test_resource(third), 64, &mut jar, &mut cache).await;
        assert_eq!(third_fetch.status, "cached");
        assert_eq!(third_fetch.cache_entries, 2);
        assert_eq!(third_fetch.cache_bytes, 8);
        assert_eq!(third_fetch.cache_evicted_entries, 1);
        assert_eq!(third_fetch.cache_evicted_bytes, 3);
        assert_eq!(third_fetch.cache_outcome.as_deref(), Some("stored"));
        assert_eq!(
            third_fetch.diagnostic.as_deref(),
            Some("resource_fetch_cached")
        );

        assert_eq!(cache.len(), 2);
        assert!(cache.cached_bytes(first).is_some());
        assert!(cache.cached_bytes(second).is_none());
        assert!(cache.cached_bytes(third).is_some());
        assert_eq!(cache.evicted_entries(), 1);
        assert_eq!(cache.evicted_bytes(), 3);
    }

    #[tokio::test]
    async fn resource_cache_skips_entries_over_byte_cap() {
        let mut cache = BrowserResourceCache::with_limits(8, 4);
        let mut jar = BrowserCookieJar::default();
        let oversized = "data:text/css,large";

        let fetch =
            fetch_resource_with_cache(test_resource(oversized), 64, &mut jar, &mut cache).await;

        assert_eq!(fetch.status, "cached");
        assert_eq!(fetch.bytes, 5);
        assert_eq!(fetch.cache_entries, 0);
        assert_eq!(fetch.cache_bytes, 0);
        assert_eq!(fetch.cache_evicted_entries, 0);
        assert_eq!(fetch.cache_max_entries, 8);
        assert_eq!(fetch.cache_max_bytes, 4);
        assert_eq!(fetch.cache_outcome.as_deref(), Some("skipped_oversize"));
        assert_eq!(
            fetch.diagnostic.as_deref(),
            Some("resource_fetch_uncached_oversize")
        );
        assert_eq!(cache.len(), 0);
        assert_eq!(cache.total_bytes(), 0);
        assert_eq!(cache.evicted_entries(), 0);
    }

    #[tokio::test]
    async fn resource_fetch_reports_cache_pressure_remaining_capacity() {
        let mut cache = BrowserResourceCache::with_limits(4, 10);
        let mut jar = BrowserCookieJar::default();

        let first =
            fetch_resource_with_cache(test_resource("data:text/css,abc"), 64, &mut jar, &mut cache)
                .await;
        assert_eq!(first.cache_remaining_entries, 3);
        assert_eq!(first.cache_remaining_bytes, 7);
        assert_eq!(first.cache_pressure_level.as_deref(), Some("normal"));

        let second = fetch_resource_with_cache(
            test_resource("data:text/css,123456"),
            64,
            &mut jar,
            &mut cache,
        )
        .await;
        assert_eq!(second.cache_entries, 2);
        assert_eq!(second.cache_bytes, 9);
        assert_eq!(second.cache_remaining_entries, 2);
        assert_eq!(second.cache_remaining_bytes, 1);
        assert_eq!(second.cache_pressure_level.as_deref(), Some("high"));

        let third =
            fetch_resource_with_cache(test_resource("data:text/css,z"), 64, &mut jar, &mut cache)
                .await;
        assert_eq!(third.cache_bytes, 10);
        assert_eq!(third.cache_remaining_bytes, 0);
        assert_eq!(third.cache_pressure_level.as_deref(), Some("full"));
    }

    #[tokio::test]
    async fn data_image_fetch_reports_decode_status_and_diagnostic() {
        let mut cache = BrowserResourceCache::with_limits(8, 4096);
        let mut jar = BrowserCookieJar::default();
        let image = "data:image/svg+xml,%3Csvg%20xmlns='http://www.w3.org/2000/svg'%20width='3'%20height='2'%3E%3Crect%20width='3'%20height='2'%20fill='red'/%3E%3C/svg%3E";

        let fetch = fetch_resource_with_cache(
            BrowserResource {
                kind: "image".to_owned(),
                initiator: "img".to_owned(),
                url: image.to_owned(),
                resolved: image.to_owned(),
                rel: None,
                media: None,
                alt: None,
                type_hint: None,
            },
            4096,
            &mut jar,
            &mut cache,
        )
        .await;

        assert_eq!(fetch.status, "cached");
        assert_eq!(fetch.content_type.as_deref(), Some("image/svg+xml"));
        assert_eq!(fetch.image_decode_status.as_deref(), Some("decoded"));
        assert_eq!(fetch.decoded_width, Some(3));
        assert_eq!(fetch.decoded_height, Some(2));
        assert_eq!(fetch.diagnostic.as_deref(), Some("image_decoded"));
        assert_eq!(fetch.cache_outcome.as_deref(), Some("stored"));
        let source = fetch.source.as_deref().unwrap();
        assert_ne!(source, image);
        assert!(source.starts_with("data:image/svg+xml,..."));
        assert!(source.contains("decoded_bytes="));
        assert!(source.contains(&format!("source_chars={}", image.len())));

        let cached = fetch_resource_with_cache(
            BrowserResource {
                kind: "image".to_owned(),
                initiator: "img".to_owned(),
                url: image.to_owned(),
                resolved: image.to_owned(),
                rel: None,
                media: None,
                alt: None,
                type_hint: None,
            },
            4096,
            &mut jar,
            &mut cache,
        )
        .await;

        assert_eq!(cached.cache_outcome.as_deref(), Some("hit"));
        assert_eq!(cached.source.as_deref(), Some(source));
    }

    #[test]
    fn resource_report_serialization_bounds_large_data_urls() {
        let payload = "A".repeat(DATA_URL_REPORT_LABEL_MAX_CHARS + 16);
        let data_url = format!("data:image/svg+xml,{payload}");
        let resource = BrowserResource {
            kind: "image".to_owned(),
            initiator: "img".to_owned(),
            url: data_url.clone(),
            resolved: data_url.clone(),
            rel: None,
            media: None,
            alt: None,
            type_hint: None,
        };

        let serialized = serde_json::to_value(&resource).unwrap();
        let url = serialized["url"].as_str().unwrap();
        let resolved = serialized["resolved"].as_str().unwrap();

        assert_eq!(resource.url, data_url);
        assert_eq!(resource.resolved, data_url);
        assert_ne!(url, data_url);
        assert_eq!(url, resolved);
        assert_eq!(
            url,
            format!("data:image/svg+xml,... source_chars={}", resource.url.len())
        );
        assert!(!url.contains(&payload));
    }

    #[test]
    fn resource_report_serialization_bounds_large_page_source() {
        let body = "x".repeat(REPORT_PAGE_SOURCE_MAX_CHARS + 32);
        let page_source = format!("<html><body>{body}</body></html>");
        let report = BrowserResourceFetchReport {
            page_source: page_source.clone(),
            total: 0,
            fetched: 0,
            cached: 0,
            failed: 0,
            skipped: 0,
            cached_resource_count: 0,
            cached_resource_bytes: 0,
            resources: Vec::new(),
        };

        let serialized = serde_json::to_value(&report).unwrap();
        let serialized_source = serialized["page_source"].as_str().unwrap();

        assert_eq!(report.page_source, page_source);
        assert_ne!(serialized_source, page_source);
        assert!(serialized_source.starts_with("<html><body>"));
        assert!(serialized_source.contains("brutal-report-page-source-truncated"));
        assert!(
            serialized_source.contains(&format!("source_chars={}", page_source.chars().count()))
        );
        assert!(serialized_source.chars().count() <= REPORT_PAGE_SOURCE_MAX_CHARS + 128);
    }

    #[test]
    fn resource_report_serialization_bounds_large_errors() {
        let detail = " request timed out while loading a repeated diagnostic URL".repeat(64);
        let error = format!("operation timed out:{detail}");
        let fetch = BrowserResourceFetch {
            resource: test_resource("https://example.test/slow.png"),
            status: "failed".to_owned(),
            source: None,
            bytes: 0,
            content_type: None,
            error: Some(error.clone()),
            error_kind: Some("timeout".to_owned()),
            target_scheme: Some("https".to_owned()),
            target_host: Some("example.test".to_owned()),
            timed_out: true,
            retryable: true,
            elapsed_ms: 6000,
            request_timeout_ms: 6000,
            cache_entries: 0,
            cache_bytes: 0,
            cache_evicted_entries: 0,
            cache_evicted_bytes: 0,
            cache_max_entries: 256,
            cache_max_bytes: 32 * 1024 * 1024,
            cache_remaining_entries: 256,
            cache_remaining_bytes: 32 * 1024 * 1024,
            cache_pressure_level: Some("normal".to_owned()),
            cache_outcome: Some("miss_failed".to_owned()),
            diagnostic: Some("network_timeout".to_owned()),
            image_decode_status: None,
            image_decode_error: Some(error.clone()),
            decoded_width: None,
            decoded_height: None,
            decoded_hash: None,
            decoded_color_hash: None,
            decoded_color_bytes: None,
        };

        let serialized = serde_json::to_value(&fetch).unwrap();
        let serialized_error = serialized["error"].as_str().unwrap();
        let serialized_decode_error = serialized["image_decode_error"].as_str().unwrap();

        assert_eq!(fetch.error.as_deref(), Some(error.as_str()));
        assert_eq!(fetch.image_decode_error.as_deref(), Some(error.as_str()));
        assert_ne!(serialized_error, error);
        assert_eq!(serialized_error, serialized_decode_error);
        assert!(serialized_error.starts_with("operation timed out:"));
        assert!(serialized_error.contains("brutal-report-error-truncated"));
        assert!(serialized_error.contains(&format!("error_chars={}", error.chars().count())));
        assert!(serialized_error.chars().count() <= REPORT_ERROR_MAX_CHARS + 128);
    }

    #[test]
    fn resource_cache_replacement_does_not_count_as_eviction_pressure() {
        let mut cache = BrowserResourceCache::with_limits(2, 1024);
        let first_insert = cache.insert(
            "https://example.test/app.css".to_owned(),
            BrowserCachedResource {
                source: "https://example.test/app.css".to_owned(),
                bytes: b"one".to_vec(),
                content_type: Some("text/css".to_owned()),
            },
        );
        let replacement = cache.insert(
            "https://example.test/app.css".to_owned(),
            BrowserCachedResource {
                source: "https://example.test/app.css".to_owned(),
                bytes: b"replacement".to_vec(),
                content_type: Some("text/css".to_owned()),
            },
        );

        assert_eq!(first_insert, BrowserResourceCacheStoreOutcome::Stored);
        assert_eq!(replacement, BrowserResourceCacheStoreOutcome::Replaced);
        assert_eq!(cache.len(), 1);
        assert_eq!(cache.total_bytes(), 11);
        assert_eq!(cache.evicted_entries(), 0);
        assert_eq!(cache.evicted_bytes(), 0);
    }

    #[tokio::test]
    async fn resource_fetch_reports_error_kind_and_elapsed_time() {
        let mut cache = BrowserResourceCache::default();
        let mut jar = BrowserCookieJar::default();

        let fetch = fetch_resource_with_cache(
            test_resource("mailto:hello@example.com"),
            64,
            &mut jar,
            &mut cache,
        )
        .await;

        assert_eq!(fetch.status, "skipped");
        assert_eq!(fetch.error_kind.as_deref(), Some("unsupported_scheme"));
        assert_eq!(fetch.request_timeout_ms, 0);
        assert_eq!(fetch.cache_entries, 0);
        assert_eq!(fetch.cache_bytes, 0);
        assert_eq!(fetch.cache_outcome.as_deref(), Some("skipped"));
        assert_eq!(fetch.diagnostic.as_deref(), Some("unsupported_scheme"));
        assert!(fetch.error.is_some());
    }

    #[test]
    fn document_http_timeout_stays_inside_hosted_initial_render_window() {
        assert_eq!(duration_millis(http_load_timeout(true)), 6_000);
        assert_eq!(duration_millis(http_load_timeout(false)), 15_000);
        assert!(http_load_timeout(true) < Duration::from_millis(8_000));
        assert!(http_load_timeout(true) < http_load_timeout(false));
    }

    #[test]
    fn resource_diagnostic_labels_timeout_and_decode_failures() {
        assert_eq!(
            resource_fetch_diagnostic("failed", Some("timeout"), None, None).as_deref(),
            Some("network_timeout")
        );
        assert_eq!(
            resource_fetch_diagnostic("cached", None, Some("hit"), Some("undecoded")).as_deref(),
            Some("image_decode_failed")
        );
        assert_eq!(
            document_load_failure_diagnostic("timeout"),
            "document_timeout"
        );
    }

    #[tokio::test]
    async fn http_resource_fetch_reports_resource_timeout_budget() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let (request_head, _) = read_http_request(&mut stream).await;
            assert!(request_head.starts_with("GET /style.css "));
            let body = "body { color: #123456; }";
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/css\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream.write_all(response.as_bytes()).await.unwrap();
        });

        let mut cache = BrowserResourceCache::with_limits(8, 1024);
        let mut jar = BrowserCookieJar::default();
        let fetch = fetch_resource_with_cache(
            test_resource(&format!("http://{addr}/style.css")),
            1024,
            &mut jar,
            &mut cache,
        )
        .await;
        server.await.unwrap();

        assert_eq!(fetch.status, "fetched");
        assert_eq!(fetch.request_timeout_ms, 15_000);
        assert_eq!(fetch.cache_outcome.as_deref(), Some("stored"));
        assert_eq!(fetch.diagnostic.as_deref(), Some("resource_fetch_cached"));
        assert_eq!(fetch.cache_entries, 1);
    }

    #[test]
    fn load_error_classification_identifies_request_timeouts() {
        let error = anyhow::anyhow!(
            "load https://www.truveta.com/: error sending request for url \
             (https://www.truveta.com/): operation timed out"
        );

        assert_eq!(classify_load_error(&error), "timeout");
    }

    #[test]
    fn resource_timeout_diagnostics_include_host_and_retry_visibility() {
        let fetch = BrowserResourceFetch::from_args(
            BrowserResourceFetchArgs {
                resource: test_resource("https://www.truveta.com/"),
                status: "failed".to_owned(),
                source: None,
                bytes: 0,
                content_type: None,
                error: Some("operation timed out".to_owned()),
                error_kind: Some("timeout".to_owned()),
                elapsed_ms: 6001,
                request_timeout_ms: 6000,
                cache_pressure: BrowserResourceCachePressure::default(),
                cache_outcome: Some("miss_failed".to_owned()),
            },
            None,
        );

        assert_eq!(fetch.target_scheme.as_deref(), Some("https"));
        assert_eq!(fetch.target_host.as_deref(), Some("www.truveta.com"));
        assert!(fetch.timed_out);
        assert!(fetch.retryable);
        assert_eq!(fetch.diagnostic.as_deref(), Some("network_timeout"));

        let byte_cap = resource_target_diagnostics("data:text/plain,hello", Some("byte_cap"));
        assert_eq!(byte_cap.scheme.as_deref(), Some("data"));
        assert_eq!(byte_cap.host, None);
        assert!(!byte_cap.timed_out);
        assert!(!byte_cap.retryable);
    }

    #[tokio::test]
    async fn document_load_report_records_elapsed_bytes_and_error_kind() {
        let path = std::env::temp_dir().join(format!(
            "brutal-document-load-report-{}-{}.html",
            std::process::id(),
            "byte-cap"
        ));
        fs::write(&path, "<html><body>browser</body></html>").unwrap();

        let loaded = load_target_with_cookie_jar_report(path.to_str().unwrap(), 1024, None).await;
        assert_eq!(loaded.status, "loaded");
        assert_eq!(loaded.bytes, 33);
        assert_eq!(loaded.content_type.as_deref(), Some("text/html"));
        assert_eq!(loaded.error_kind, None);
        assert_eq!(loaded.request_timeout_ms, 0);
        assert_eq!(loaded.diagnostic.as_deref(), Some("document_loaded"));

        let failed = load_target_with_cookie_jar_report(path.to_str().unwrap(), 4, None).await;
        assert_eq!(failed.status, "failed");
        assert_eq!(failed.bytes, 0);
        assert_eq!(failed.error_kind.as_deref(), Some("byte_cap"));
        assert_eq!(failed.request_timeout_ms, 0);
        assert_eq!(failed.diagnostic.as_deref(), Some("document_byte_cap"));

        let _ = fs::remove_file(path);
    }

    #[tokio::test]
    async fn http_redirect_stores_cookie_before_following_location() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let (request_head, _) = read_http_request(&mut stream).await;
            assert!(request_head.starts_with("GET /start "));
            stream
                .write_all(
                    b"HTTP/1.1 302 Found\r\nLocation: /final\r\nSet-Cookie: sid=abc; Path=/; HttpOnly\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                )
                .await
                .unwrap();

            let (mut stream, _) = listener.accept().await.unwrap();
            let (request_head, _) = read_http_request(&mut stream).await;
            assert!(request_head.starts_with("GET /final "));
            assert!(
                request_head
                    .to_ascii_lowercase()
                    .contains("cookie: sid=abc")
            );
            let body = "<html><head><title>Final</title></head><body>redirected</body></html>";
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream.write_all(response.as_bytes()).await.unwrap();
        });

        let mut jar = BrowserCookieJar::default();
        let (source, bytes) =
            load_target_with_cookie_jar(&format!("http://{addr}/start"), 4096, Some(&mut jar))
                .await
                .unwrap();
        server.await.unwrap();

        assert_eq!(source, format!("http://{addr}/final"));
        assert!(String::from_utf8_lossy(&bytes).contains("redirected"));
        assert_eq!(
            jar.cookie_header(&format!("http://{addr}/final")),
            Some("sid=abc".to_owned())
        );
    }

    #[tokio::test]
    async fn post_redirect_see_other_replays_as_get_with_redirect_cookie() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let (request_head, request_body) = read_http_request(&mut stream).await;
            assert!(request_head.starts_with("POST /submit "));
            assert_eq!(request_body, "q=rust");
            stream
                .write_all(
                    b"HTTP/1.1 303 See Other\r\nLocation: /done\r\nSet-Cookie: posted=1; Path=/\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                )
                .await
                .unwrap();

            let (mut stream, _) = listener.accept().await.unwrap();
            let (request_head, request_body) = read_http_request(&mut stream).await;
            assert!(request_head.starts_with("GET /done "));
            assert!(request_body.is_empty());
            assert!(
                request_head
                    .to_ascii_lowercase()
                    .contains("cookie: posted=1")
            );
            let body = "<html><head><title>Done</title></head><body>done</body></html>";
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream.write_all(response.as_bytes()).await.unwrap();
        });

        let mut jar = BrowserCookieJar::default();
        let (source, bytes) = load_post_form_target_with_cookie_jar(
            &format!("http://{addr}/submit"),
            "q=rust".to_owned(),
            4096,
            &mut jar,
        )
        .await
        .unwrap();
        server.await.unwrap();

        assert_eq!(source, format!("http://{addr}/done"));
        assert!(String::from_utf8_lossy(&bytes).contains("done"));
        assert_eq!(
            jar.cookie_header(&format!("http://{addr}/done")),
            Some("posted=1".to_owned())
        );
    }

    #[tokio::test]
    async fn document_loader_renders_html_error_status_bodies() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let (request_head, _) = read_http_request(&mut stream).await;
            assert!(request_head.starts_with("GET /blocked "));
            let body = "<html><head><title>Blocked</title></head><body>Access denied</body></html>";
            let response = format!(
                "HTTP/1.1 403 Forbidden\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream.write_all(response.as_bytes()).await.unwrap();
        });

        let (source, bytes) =
            load_target_with_cookie_jar(&format!("http://{addr}/blocked"), 4096, None)
                .await
                .unwrap();
        server.await.unwrap();

        assert_eq!(source, format!("http://{addr}/blocked"));
        assert!(String::from_utf8_lossy(&bytes).contains("Access denied"));
    }
}
