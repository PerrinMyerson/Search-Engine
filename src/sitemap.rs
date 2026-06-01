use std::collections::VecDeque;
use std::fs;
use std::io::Read;
use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result, bail, ensure};
use flate2::read::GzDecoder;
use quick_xml::Reader;
use quick_xml::events::Event;
use reqwest::header::{HeaderMap, HeaderValue, USER_AGENT};
use rustc_hash::FxHashSet;
use url::Url;

use crate::robots::{RobotsTxt, robots_origin_key};
use crate::urlcanon::parse_seed;

#[derive(Debug, Clone, Copy)]
pub struct SitemapLoadOptions {
    pub max_sitemaps: usize,
    pub max_urls: usize,
    pub max_bytes: usize,
}

impl Default for SitemapLoadOptions {
    fn default() -> Self {
        Self {
            max_sitemaps: 1024,
            max_urls: 200_000,
            max_bytes: 64 * 1024 * 1024,
        }
    }
}

#[derive(Debug, Default, PartialEq, Eq)]
struct ParsedSitemap {
    page_urls: Vec<String>,
    nested_sitemaps: Vec<String>,
}

pub async fn load_sitemap_seeds<I, S>(
    sources: I,
    options: SitemapLoadOptions,
) -> Result<Vec<String>>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    ensure!(
        options.max_sitemaps > 0,
        "max_sitemaps must be greater than 0"
    );
    ensure!(options.max_urls > 0, "max_urls must be greater than 0");
    ensure!(options.max_bytes > 0, "max_bytes must be greater than 0");

    let client = build_client()?;
    let mut queue = VecDeque::new();
    let mut seen_sitemaps = FxHashSet::default();

    for source in sources {
        let source = source.as_ref().trim();
        if source.is_empty() {
            continue;
        }
        let key = sitemap_source_key(source);
        if seen_sitemaps.insert(key.clone()) {
            queue.push_back(key);
        }
    }

    let mut loaded_sitemaps = 0usize;
    let mut seeds = Vec::new();
    let mut seen_urls = FxHashSet::default();

    while let Some(source) = queue.pop_front() {
        if loaded_sitemaps >= options.max_sitemaps {
            bail!("sitemap limit exceeded: {}", options.max_sitemaps);
        }
        if seeds.len() >= options.max_urls {
            break;
        }

        loaded_sitemaps += 1;
        let bytes = load_sitemap_bytes(&client, &source, options.max_bytes).await?;
        let xml = decode_sitemap_bytes(&source, &bytes, options.max_bytes)?;
        let parsed = parse_sitemap_xml(&xml)
            .with_context(|| format!("parse sitemap {}", source_display(&source)))?;

        for nested in parsed.nested_sitemaps {
            let key = sitemap_source_key(&nested);
            if seen_sitemaps.insert(key.clone()) {
                queue.push_back(key);
            }
        }

        for page in parsed.page_urls {
            let page = parse_seed(&page)
                .with_context(|| format!("invalid sitemap URL {page:?} in {source}"))?
                .to_string();
            if seen_urls.insert(page.clone()) {
                seeds.push(page);
                if seeds.len() >= options.max_urls {
                    break;
                }
            }
        }
    }

    Ok(seeds)
}

pub async fn discover_sitemap_sources_from_robots<I, S>(
    seeds: I,
    max_robots_bytes: usize,
) -> Result<Vec<String>>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    ensure!(
        max_robots_bytes > 0,
        "max_robots_bytes must be greater than 0"
    );

    let mut seen_origins = FxHashSet::default();
    let mut seen_sitemaps = FxHashSet::default();
    let mut sitemaps = Vec::new();

    for seed in seeds {
        let seed = parse_seed(seed.as_ref())?;
        let Some(origin) = robots_origin_key(&seed) else {
            continue;
        };
        if !seen_origins.insert(origin) {
            continue;
        }

        let robots = RobotsTxt::fetch(&seed, max_robots_bytes)
            .await
            .unwrap_or_else(|_| RobotsTxt::allow_all());
        for sitemap in robots.sitemaps() {
            if seen_sitemaps.insert(sitemap.clone()) {
                sitemaps.push(sitemap.clone());
            }
        }
    }

    Ok(sitemaps)
}

fn parse_sitemap_xml(xml: &str) -> Result<ParsedSitemap> {
    let mut reader = Reader::from_reader(xml.as_bytes());
    reader.config_mut().trim_text(true);

    let mut buf = Vec::new();
    let mut parsed = ParsedSitemap::default();
    let mut in_loc = false;
    let mut sitemap_depth = 0usize;

    loop {
        match reader.read_event_into(&mut buf)? {
            Event::Start(event) => match local_name(event.name().as_ref()) {
                b"sitemap" => sitemap_depth += 1,
                b"loc" => in_loc = true,
                _ => {}
            },
            Event::End(event) => match local_name(event.name().as_ref()) {
                b"sitemap" => sitemap_depth = sitemap_depth.saturating_sub(1),
                b"loc" => in_loc = false,
                _ => {}
            },
            Event::Text(text) if in_loc => {
                let value = text.unescape()?.trim().to_owned();
                push_loc(&mut parsed, sitemap_depth, value);
            }
            Event::CData(text) if in_loc => {
                let value = std::str::from_utf8(text.as_ref())?.trim().to_owned();
                push_loc(&mut parsed, sitemap_depth, value);
            }
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }

    Ok(parsed)
}

fn push_loc(parsed: &mut ParsedSitemap, sitemap_depth: usize, value: String) {
    if value.is_empty() {
        return;
    }
    if sitemap_depth > 0 {
        parsed.nested_sitemaps.push(value);
    } else {
        parsed.page_urls.push(value);
    }
}

fn local_name(name: &[u8]) -> &[u8] {
    name.rsplit(|byte| *byte == b':').next().unwrap_or(name)
}

async fn load_sitemap_bytes(
    client: &reqwest::Client,
    source: &str,
    max_bytes: usize,
) -> Result<Vec<u8>> {
    if is_http_source(source) {
        let response = client
            .get(source)
            .send()
            .await
            .with_context(|| format!("fetch sitemap {source}"))?
            .error_for_status()
            .with_context(|| format!("fetch sitemap {source}"))?;

        if let Some(length) = response.content_length() {
            ensure!(
                length <= max_bytes as u64,
                "sitemap {} exceeds byte cap: {} > {}",
                source,
                length,
                max_bytes
            );
        }

        let bytes = response.bytes().await?;
        ensure!(
            bytes.len() <= max_bytes,
            "sitemap {} exceeds byte cap: {} > {}",
            source,
            bytes.len(),
            max_bytes
        );
        return Ok(bytes.to_vec());
    }

    let path = Path::new(source);
    let bytes = fs::read(path).with_context(|| format!("read sitemap {}", path.display()))?;
    ensure!(
        bytes.len() <= max_bytes,
        "sitemap {} exceeds byte cap: {} > {}",
        path.display(),
        bytes.len(),
        max_bytes
    );
    Ok(bytes)
}

fn decode_sitemap_bytes(source: &str, bytes: &[u8], max_bytes: usize) -> Result<String> {
    let mut decoded = Vec::new();
    if bytes.starts_with(&[0x1f, 0x8b]) {
        let mut decoder = GzDecoder::new(bytes);
        decoder
            .by_ref()
            .take(max_bytes as u64 + 1)
            .read_to_end(&mut decoded)
            .with_context(|| format!("decompress sitemap {}", source_display(source)))?;
        ensure!(
            decoded.len() <= max_bytes,
            "decompressed sitemap {} exceeds byte cap: {} > {}",
            source_display(source),
            decoded.len(),
            max_bytes
        );
    } else {
        decoded.extend_from_slice(bytes);
    }

    String::from_utf8(decoded)
        .with_context(|| format!("sitemap {} is not valid UTF-8 XML", source_display(source)))
}

fn build_client() -> Result<reqwest::Client> {
    let mut headers = HeaderMap::new();
    headers.insert(
        USER_AGENT,
        HeaderValue::from_static("brutal-search/0.1 sitemap-loader"),
    );

    Ok(reqwest::Client::builder()
        .default_headers(headers)
        .timeout(Duration::from_secs(15))
        .redirect(reqwest::redirect::Policy::limited(5))
        .build()?)
}

fn sitemap_source_key(source: &str) -> String {
    Url::parse(source)
        .ok()
        .filter(|url| matches!(url.scheme(), "http" | "https"))
        .map(|url| url.to_string())
        .unwrap_or_else(|| source.to_owned())
}

fn is_http_source(source: &str) -> bool {
    Url::parse(source)
        .map(|url| matches!(url.scheme(), "http" | "https"))
        .unwrap_or(false)
}

fn source_display(source: &str) -> String {
    if is_http_source(source) {
        source.to_owned()
    } else {
        Path::new(source).display().to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::Compression;
    use flate2::write::GzEncoder;
    use std::io::Write;

    #[test]
    fn parses_urlset_and_unescapes_locations() {
        let parsed = parse_sitemap_xml(
            r#"
            <urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">
              <url><loc>https://example.com/a?x=1&amp;y=2</loc></url>
              <url><loc><![CDATA[https://example.com/b]]></loc></url>
            </urlset>
            "#,
        )
        .unwrap();

        assert_eq!(parsed.nested_sitemaps, Vec::<String>::new());
        assert_eq!(
            parsed.page_urls,
            vec![
                "https://example.com/a?x=1&y=2".to_owned(),
                "https://example.com/b".to_owned()
            ]
        );
    }

    #[test]
    fn parses_sitemap_index_locations() {
        let parsed = parse_sitemap_xml(
            r#"
            <sitemapindex>
              <sitemap><loc>https://example.com/sitemap-a.xml</loc></sitemap>
              <sitemap><loc>https://example.com/sitemap-b.xml.gz</loc></sitemap>
            </sitemapindex>
            "#,
        )
        .unwrap();

        assert_eq!(parsed.page_urls, Vec::<String>::new());
        assert_eq!(
            parsed.nested_sitemaps,
            vec![
                "https://example.com/sitemap-a.xml".to_owned(),
                "https://example.com/sitemap-b.xml.gz".to_owned()
            ]
        );
    }

    #[test]
    fn decodes_gzip_sitemaps() {
        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        encoder
            .write_all(b"<urlset><url><loc>https://example.com/a</loc></url></urlset>")
            .unwrap();
        let compressed = encoder.finish().unwrap();

        let decoded = decode_sitemap_bytes("sitemap.xml.gz", &compressed, 1024).unwrap();
        assert!(decoded.contains("https://example.com/a"));
    }

    #[tokio::test]
    async fn loads_local_sitemap_file_and_dedupes_urls() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sitemap.xml");
        fs::write(
            &path,
            r#"
            <urlset>
              <url><loc>https://example.com/a#fragment</loc></url>
              <url><loc>https://example.com/a</loc></url>
              <url><loc>https://example.com/b</loc></url>
            </urlset>
            "#,
        )
        .unwrap();

        let seeds = load_sitemap_seeds(
            [path.to_string_lossy().to_string()],
            SitemapLoadOptions {
                max_sitemaps: 1,
                max_urls: 10,
                max_bytes: 4096,
            },
        )
        .await
        .unwrap();

        assert_eq!(
            seeds,
            vec![
                "https://example.com/a".to_owned(),
                "https://example.com/b".to_owned()
            ]
        );
    }
}
