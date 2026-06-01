use anyhow::{Context, Result};
use url::Url;

pub fn parse_seed(raw: &str) -> Result<Url> {
    let url = Url::parse(raw).with_context(|| format!("invalid URL: {raw}"))?;
    anyhow::ensure!(
        matches!(url.scheme(), "http" | "https"),
        "seed must be http or https"
    );
    Ok(canonicalize_url(url))
}

pub fn canonicalize_url(mut url: Url) -> Url {
    url.set_fragment(None);

    if (url.scheme() == "http" && url.port() == Some(80))
        || (url.scheme() == "https" && url.port() == Some(443))
    {
        let _ = url.set_port(None);
    }

    let path = url.path().to_owned();
    if path.len() > 1 && path.ends_with('/') {
        url.set_path(path.trim_end_matches('/'));
    }

    url
}

pub fn same_host(seed: &Url, candidate: &Url) -> bool {
    matches!(candidate.scheme(), "http" | "https") && seed.host_str() == candidate.host_str()
}

pub fn resolve_link(base: &Url, href: &str) -> Option<Url> {
    let trimmed = href.trim();
    if trimmed.is_empty()
        || trimmed.starts_with('#')
        || trimmed.starts_with("mailto:")
        || trimmed.starts_with("tel:")
        || trimmed.starts_with("javascript:")
        || trimmed.starts_with("data:")
    {
        return None;
    }

    base.join(trimmed).ok().map(canonicalize_url)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonicalization_drops_fragments_and_default_ports() {
        let url = Url::parse("https://example.com:443/path/#frag").unwrap();
        assert_eq!(canonicalize_url(url).as_str(), "https://example.com/path");
    }

    #[test]
    fn boundary_is_exact_host() {
        let seed = Url::parse("https://example.com").unwrap();
        let same = Url::parse("https://example.com/a").unwrap();
        let sub = Url::parse("https://www.example.com/a").unwrap();
        assert!(same_host(&seed, &same));
        assert!(!same_host(&seed, &sub));
    }
}
