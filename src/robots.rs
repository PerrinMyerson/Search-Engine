use std::time::Duration;

use anyhow::{Context, Result, ensure};
use reqwest::header::{HeaderMap, HeaderValue, USER_AGENT};
use rustc_hash::FxHashSet;
use url::Url;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RobotsTxt {
    disallow: Vec<String>,
    sitemaps: Vec<String>,
    crawl_delay_millis: Option<u64>,
}

impl RobotsTxt {
    pub fn allow_all() -> Self {
        Self {
            disallow: Vec::new(),
            sitemaps: Vec::new(),
            crawl_delay_millis: None,
        }
    }

    pub async fn fetch(seed: &Url, max_bytes: usize) -> Result<Self> {
        let robots_url = seed.join("/robots.txt")?;
        let client = build_client()?;
        let response = client
            .get(robots_url.clone())
            .send()
            .await
            .with_context(|| format!("fetch robots.txt {}", robots_url))?
            .error_for_status()
            .with_context(|| format!("fetch robots.txt {}", robots_url))?;

        if let Some(length) = response.content_length() {
            ensure!(
                length <= max_bytes as u64,
                "robots.txt {} exceeds byte cap: {} > {}",
                robots_url,
                length,
                max_bytes
            );
        }

        let bytes = response.bytes().await?;
        ensure!(
            bytes.len() <= max_bytes,
            "robots.txt {} exceeds byte cap: {} > {}",
            robots_url,
            bytes.len(),
            max_bytes
        );

        Ok(Self::parse(&String::from_utf8_lossy(&bytes)))
    }

    pub fn parse(text: &str) -> Self {
        let mut group_matches = false;
        let mut group_has_rules = false;
        let mut disallow = Vec::new();
        let mut sitemaps = Vec::new();
        let mut seen_sitemaps = FxHashSet::default();
        let mut crawl_delay_millis = None;

        for line in text.lines() {
            let line = line.split('#').next().unwrap_or("").trim();
            if line.is_empty() {
                if group_has_rules {
                    group_matches = false;
                    group_has_rules = false;
                }
                continue;
            }

            let Some((key, value)) = line.split_once(':') else {
                continue;
            };
            let key = key.trim().to_ascii_lowercase();
            let value = value.trim();

            match key.as_str() {
                "sitemap" => {
                    if let Ok(url) = Url::parse(value)
                        && matches!(url.scheme(), "http" | "https")
                    {
                        let url = url.to_string();
                        if seen_sitemaps.insert(url.clone()) {
                            sitemaps.push(url);
                        }
                    }
                }
                "user-agent" => {
                    if group_has_rules {
                        group_matches = false;
                        group_has_rules = false;
                    }
                    group_matches |= value == "*" || value.eq_ignore_ascii_case("brutal-search");
                }
                "disallow" => {
                    group_has_rules = true;
                    if group_matches && !value.is_empty() {
                        disallow.push(value.to_owned());
                    }
                }
                "crawl-delay" => {
                    group_has_rules = true;
                    if group_matches
                        && let Ok(seconds) = value.parse::<f64>()
                        && seconds.is_finite()
                        && seconds >= 0.0
                    {
                        crawl_delay_millis = Some((seconds * 1000.0).round() as u64);
                    }
                }
                _ => {}
            }
        }

        Self {
            disallow,
            sitemaps,
            crawl_delay_millis,
        }
    }

    pub fn allowed(&self, path: &str) -> bool {
        !self
            .disallow
            .iter()
            .any(|prefix| prefix == "/" || path.starts_with(prefix))
    }

    pub fn sitemaps(&self) -> &[String] {
        &self.sitemaps
    }

    pub fn crawl_delay(&self) -> Option<Duration> {
        self.crawl_delay_millis.map(Duration::from_millis)
    }
}

pub fn robots_origin_key(url: &Url) -> Option<String> {
    let host = url.host_str()?;
    let mut key = format!("{}://{}", url.scheme(), host);
    if let Some(port) = url.port() {
        key.push(':');
        key.push_str(&port.to_string());
    }
    Some(key)
}

fn build_client() -> Result<reqwest::Client> {
    let mut headers = HeaderMap::new();
    headers.insert(
        USER_AGENT,
        HeaderValue::from_static("brutal-search/0.1 robots-fetcher"),
    );

    Ok(reqwest::Client::builder()
        .default_headers(headers)
        .timeout(Duration::from_secs(15))
        .redirect(reqwest::redirect::Policy::limited(5))
        .build()?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_matching_user_agent_policy_and_metadata() {
        let robots = RobotsTxt::parse(
            r#"
            User-agent: otherbot
            Disallow: /

            User-agent: *
            Disallow: /private
            Crawl-delay: 1.5
            Sitemap: https://example.com/sitemap.xml
            Sitemap: https://example.com/sitemap.xml
            Sitemap: https://example.com/news.xml.gz
            "#,
        );

        assert!(robots.allowed("/public"));
        assert!(!robots.allowed("/private/page"));
        assert_eq!(robots.crawl_delay(), Some(Duration::from_millis(1500)));
        assert_eq!(
            robots.sitemaps(),
            &[
                "https://example.com/sitemap.xml".to_owned(),
                "https://example.com/news.xml.gz".to_owned()
            ]
        );
    }

    #[test]
    fn origin_key_includes_scheme_host_and_explicit_port() {
        let url = Url::parse("https://example.com:8443/path").unwrap();
        assert_eq!(
            robots_origin_key(&url).as_deref(),
            Some("https://example.com:8443")
        );
    }

    #[test]
    fn user_agent_lines_share_group_until_rules_start() {
        let robots = RobotsTxt::parse(
            r#"
            User-agent: brutal-search
            User-agent: otherbot
            Disallow: /shared

            User-agent: otherbot
            Disallow: /other-only
            "#,
        );

        assert!(!robots.allowed("/shared/doc"));
        assert!(robots.allowed("/other-only/doc"));
    }
}
