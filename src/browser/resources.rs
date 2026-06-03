use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result, ensure};
use reqwest::StatusCode;
use reqwest::header::{
    CONTENT_TYPE, COOKIE, HeaderMap, HeaderValue, LOCATION, SET_COOKIE, USER_AGENT,
};
use serde::{Deserialize, Serialize};
use url::Url;

use super::images::{
    ImageDecodeDiagnostic, image_decode_diagnostic, image_render_source, selected_srcset_candidate,
    srcset_candidate_urls,
};
use super::{BrowserCookieJar, Dom, ElementData, NodeKind, resolve_browser_href};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BrowserResource {
    pub kind: String,
    pub initiator: String,
    pub url: String,
    pub resolved: String,
    pub rel: Option<String>,
    pub media: Option<String>,
    pub alt: Option<String>,
    pub type_hint: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrowserResourceFetchReport {
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
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image_decode_status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image_decode_error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decoded_width: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decoded_height: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decoded_hash: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrowserStylesheetRenderReport {
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

#[derive(Debug, Clone, Default)]
pub(super) struct BrowserResourceCache {
    entries: HashMap<String, BrowserCachedResource>,
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
}

impl BrowserResourceFetch {
    fn from_args(args: BrowserResourceFetchArgs, image_bytes: Option<&[u8]>) -> Self {
        let image_report = image_resource_fetch_decode_report(
            &args.resource,
            &args.status,
            args.content_type.as_deref(),
            image_bytes,
        );
        let (image_decode_status, image_decode_error, decoded_width, decoded_height, decoded_hash) =
            image_report
                .map(|report| {
                    (
                        Some(report.status.to_owned()),
                        report.error,
                        report.width,
                        report.height,
                        report.pixel_hash,
                    )
                })
                .unwrap_or((None, None, None, None, None));

        Self {
            resource: args.resource,
            status: args.status,
            source: args.source,
            bytes: args.bytes,
            content_type: args.content_type,
            error: args.error,
            image_decode_status,
            image_decode_error,
            decoded_width,
            decoded_height,
            decoded_hash,
        }
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

impl BrowserResourceCache {
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

    pub(super) fn cached_resources(&self) -> impl Iterator<Item = (&str, Option<&str>, &[u8])> {
        self.entries.iter().map(|(url, resource)| {
            (
                url.as_str(),
                resource.content_type.as_deref(),
                resource.bytes.as_slice(),
            )
        })
    }
}

pub(super) async fn load_target(target: &str, max_bytes: usize) -> Result<(String, Vec<u8>)> {
    load_target_with_cookie_jar(target, max_bytes, None).await
}

pub(super) async fn fetch_resource_with_cache(
    resource: BrowserResource,
    max_resource_bytes: usize,
    cookie_jar: &mut BrowserCookieJar,
    cache: &mut BrowserResourceCache,
) -> BrowserResourceFetch {
    if let Some(cached) = cache.entries.get(&resource.resolved) {
        return BrowserResourceFetch::from_args(
            BrowserResourceFetchArgs {
                resource,
                status: "cached".to_owned(),
                source: Some(cached.source.clone()),
                bytes: cached.bytes.len(),
                content_type: cached.content_type.clone(),
                error: None,
            },
            Some(cached.bytes.as_slice()),
        );
    }

    if resource.resolved.starts_with("data:") {
        return match load_data_url_resource(&resource.resolved, max_resource_bytes) {
            Ok((source, bytes, content_type)) => {
                let byte_len = bytes.len();
                let fetch = BrowserResourceFetch::from_args(
                    BrowserResourceFetchArgs {
                        resource: resource.clone(),
                        status: "cached".to_owned(),
                        source: Some(source.clone()),
                        bytes: byte_len,
                        content_type: Some(content_type.clone()),
                        error: None,
                    },
                    Some(bytes.as_slice()),
                );
                cache.entries.insert(
                    resource.resolved.clone(),
                    BrowserCachedResource {
                        source: source.clone(),
                        bytes,
                        content_type: Some(content_type.clone()),
                    },
                );
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
            },
            None,
        );
    }

    match load_resource_target(&resource.resolved, max_resource_bytes, cookie_jar).await {
        Ok((source, bytes, content_type)) => {
            let byte_len = bytes.len();
            let fetch = BrowserResourceFetch::from_args(
                BrowserResourceFetchArgs {
                    resource: resource.clone(),
                    status: "fetched".to_owned(),
                    source: Some(source.clone()),
                    bytes: byte_len,
                    content_type: content_type.clone(),
                    error: None,
                },
                Some(bytes.as_slice()),
            );
            cache.entries.insert(
                resource.resolved.clone(),
                BrowserCachedResource {
                    source: source.clone(),
                    bytes,
                    content_type: content_type.clone(),
                },
            );
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
        percent_decode_data_url_payload(data)?
    };
    ensure!(
        bytes.len() <= max_bytes,
        "resource exceeds byte cap: {} > {}",
        bytes.len(),
        max_bytes
    );
    Ok((target.to_owned(), bytes, content_type))
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

fn percent_decode_data_url_payload(input: &str) -> Result<Vec<u8>> {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0usize;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            let high = data_url_hex_value(
                *bytes
                    .get(index + 1)
                    .context("truncated percent escape in data URL")?,
            )?;
            let low = data_url_hex_value(
                *bytes
                    .get(index + 2)
                    .context("truncated percent escape in data URL")?,
            )?;
            out.push((high << 4) | low);
            index += 3;
        } else {
            out.push(bytes[index]);
            index += 1;
        }
    }
    Ok(out)
}

fn data_url_hex_value(byte: u8) -> Result<u8> {
    match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        b'A'..=b'F' => Ok(byte - b'A' + 10),
        _ => anyhow::bail!("invalid percent escape in data URL"),
    }
}

pub(super) async fn load_target_with_cookie_jar(
    target: &str,
    max_bytes: usize,
    cookie_jar: Option<&mut BrowserCookieJar>,
) -> Result<(String, Vec<u8>)> {
    if target.starts_with("http://") || target.starts_with("https://") {
        let (source, bytes, _) = load_http_bytes(target, max_bytes, cookie_jar, true, None).await?;
        Ok((source, bytes))
    } else if target.starts_with("file://") {
        let url = Url::parse(target).with_context(|| format!("parse file URL {target}"))?;
        let path = url.to_file_path().map_err(|_| {
            anyhow::anyhow!("file URL cannot be converted to a local path: {target}")
        })?;
        load_file_with_source(target, &path, max_bytes)
    } else {
        let path = local_path_without_url_parts(target);
        load_file_with_source(target, Path::new(path), max_bytes)
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
        .timeout(Duration::from_secs(15))
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
                push_src_resource(resources, source, element, "image");
                push_srcset_resources(resources, source, element, "image_candidate");
                push_image_alias_resources(resources, source, element);
            }
            "source" if parent_element_tag_is(dom, node_id, "picture") => {
                push_src_resource(resources, source, element, "image");
                push_srcset_resources(resources, source, element, "image_candidate");
                push_image_alias_resources(resources, source, element);
            }
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
    }

    for &child in &node.children {
        collect_resources_at(dom, child, source, resources);
    }
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
        for url in srcset_candidate_urls(srcset) {
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
    "data-original-url",
    "data-original",
    "data-original-src",
    "data-image",
    "data-image-src",
    "data-img-src",
    "data-current-src",
    "current-src",
    "currentsrc",
];

const IMAGE_SRCSET_ALIAS_ATTRS: &[&str] = &[
    "data-srcset",
    "data-lazy-srcset",
    "data-original-srcset",
    "data-originalset",
    "data-image-srcset",
    "data-img-srcset",
    "data-current-srcset",
    "current-srcset",
    "currentsrcset",
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
        && let Some(url) = selected_srcset_candidate(
            srcset,
            element.attrs.get("imagesizes").map(String::as_str),
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
    for url in srcset_candidate_urls(srcset) {
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
