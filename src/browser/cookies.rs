use serde::{Deserialize, Serialize};
use url::Url;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BrowserCookieJar {
    cookies: Vec<BrowserCookie>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BrowserCookie {
    pub name: String,
    pub value: String,
    pub domain: String,
    pub path: String,
    pub secure: bool,
    pub http_only: bool,
    pub host_only: bool,
}

impl BrowserCookieJar {
    pub fn from_cookies(cookies: Vec<BrowserCookie>) -> Self {
        Self { cookies }
    }

    pub fn cookie_header(&self, target: &str) -> Option<String> {
        let url = Url::parse(target).ok()?;
        let host = url.host_str()?.to_ascii_lowercase();
        let path = url.path();
        let secure_request = url.scheme() == "https";
        let pairs = self
            .cookies
            .iter()
            .filter(|cookie| cookie_matches(cookie, &host, path, secure_request))
            .map(|cookie| format!("{}={}", cookie.name, cookie.value))
            .collect::<Vec<_>>();
        (!pairs.is_empty()).then(|| pairs.join("; "))
    }

    pub fn snapshot(&self) -> Vec<BrowserCookie> {
        self.cookies.clone()
    }

    pub fn clear(&mut self) {
        self.cookies.clear();
    }

    pub(super) fn store_from_set_cookie_headers(&mut self, response_url: &str, headers: &[String]) {
        for header in headers {
            self.store_from_set_cookie(response_url, header);
        }
    }

    fn store_from_set_cookie(&mut self, response_url: &str, header: &str) {
        let Ok(url) = Url::parse(response_url) else {
            return;
        };
        let Some(host) = url.host_str().map(str::to_ascii_lowercase) else {
            return;
        };
        let Some((name_value, attributes)) = header.split_once(';').or(Some((header, ""))) else {
            return;
        };
        let Some((name, value)) = name_value.trim().split_once('=') else {
            return;
        };
        let name = name.trim();
        if name.is_empty() {
            return;
        }

        let mut domain = host.clone();
        let mut host_only = true;
        let mut path = default_cookie_path(url.path());
        let mut secure = false;
        let mut http_only = false;
        let mut delete_cookie = false;

        for attribute in attributes
            .split(';')
            .map(str::trim)
            .filter(|item| !item.is_empty())
        {
            if attribute.eq_ignore_ascii_case("secure") {
                secure = true;
            } else if attribute.eq_ignore_ascii_case("httponly") {
                http_only = true;
            } else if let Some((key, value)) = attribute.split_once('=') {
                if key.trim().eq_ignore_ascii_case("domain") {
                    let candidate = value.trim().trim_start_matches('.').to_ascii_lowercase();
                    if !candidate.is_empty() && domain_matches(&host, &candidate) {
                        domain = candidate;
                        host_only = false;
                    }
                } else if key.trim().eq_ignore_ascii_case("path") {
                    let candidate = value.trim();
                    if candidate.starts_with('/') {
                        path = candidate.to_owned();
                    }
                } else if key.trim().eq_ignore_ascii_case("max-age") {
                    delete_cookie = value
                        .trim()
                        .parse::<i64>()
                        .is_ok_and(|seconds| seconds <= 0);
                }
            }
        }

        let cookie = BrowserCookie {
            name: name.to_owned(),
            value: value.trim().to_owned(),
            domain,
            path,
            secure,
            http_only,
            host_only,
        };

        self.cookies.retain(|existing| {
            !(existing.name == cookie.name
                && existing.domain == cookie.domain
                && existing.path == cookie.path)
        });
        if !delete_cookie {
            self.cookies.push(cookie);
        }
    }
}

fn cookie_matches(cookie: &BrowserCookie, host: &str, path: &str, secure_request: bool) -> bool {
    if cookie.secure && !secure_request {
        return false;
    }
    let domain_match = if cookie.host_only {
        host == cookie.domain
    } else {
        domain_matches(host, &cookie.domain)
    };
    domain_match && path.starts_with(&cookie.path)
}

fn domain_matches(host: &str, domain: &str) -> bool {
    host == domain
        || host
            .strip_suffix(domain)
            .is_some_and(|prefix| prefix.ends_with('.'))
}

fn default_cookie_path(path: &str) -> String {
    if !path.starts_with('/') || path == "/" {
        return "/".to_owned();
    }
    match path.rfind('/') {
        Some(0) | None => "/".to_owned(),
        Some(index) => path[..index].to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cookie_jar_matches_domain_path_and_secure_rules() {
        let mut jar = BrowserCookieJar::default();
        jar.store_from_set_cookie(
            "http://example.com/account/page",
            "sid=abc; Path=/account; HttpOnly",
        );
        jar.store_from_set_cookie(
            "https://example.com/",
            "wide=1; Domain=example.com; Path=/; Secure",
        );
        jar.store_from_set_cookie("http://example.com/", "gone=1; Max-Age=0");
        jar.store_from_set_cookie("http://example.com/", "past=1; Max-Age=-1");

        assert_eq!(
            jar.cookie_header("http://example.com/account/settings"),
            Some("sid=abc".to_owned())
        );
        assert_eq!(jar.cookie_header("http://example.com/other"), None);
        assert_eq!(
            jar.cookie_header("https://sub.example.com/"),
            Some("wide=1".to_owned())
        );
        assert_eq!(jar.cookie_header("http://sub.example.com/"), None);
        assert!(jar.snapshot().iter().all(|cookie| cookie.name != "gone"));
        assert!(jar.snapshot().iter().all(|cookie| cookie.name != "past"));
    }
}
