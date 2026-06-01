use url::Url;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ExtractedPage {
    pub title: String,
    pub meta_description: Option<String>,
    pub language: Option<String>,
    pub robots_noindex: bool,
    pub canonical_url: Option<String>,
    pub headings: Vec<String>,
    pub body: String,
    pub anchor_text: Vec<String>,
    pub outbound_links: Vec<String>,
}

pub fn extract_html(base: &Url, bytes: &[u8]) -> ExtractedPage {
    let html = String::from_utf8_lossy(bytes);
    let mut page = ExtractedPage {
        title: tag_text(&html, "title").unwrap_or_else(|| base.to_string()),
        meta_description: meta_content(&html, "description"),
        language: html_lang(&html),
        robots_noindex: meta_content(&html, "robots")
            .map(|value| value.to_ascii_lowercase().contains("noindex"))
            .unwrap_or(false),
        canonical_url: canonical_url(base, &html),
        headings: headings(&html),
        body: visible_text(&html),
        anchor_text: anchor_text(&html),
        outbound_links: outbound_links(base, &html),
    };
    if page.title.trim().is_empty() {
        page.title = base.to_string();
    }
    page
}

fn tag_text(html: &str, tag: &str) -> Option<String> {
    let lower = html.to_ascii_lowercase();
    let open = lower.find(&format!("<{tag}"))?;
    let after_open = lower[open..].find('>')? + open + 1;
    let close = lower[after_open..].find(&format!("</{tag}>"))? + after_open;
    Some(clean_text(&html[after_open..close]))
}

fn headings(html: &str) -> Vec<String> {
    let mut out = Vec::new();
    for tag in ["h1", "h2", "h3"] {
        let mut start = 0usize;
        let lower = html.to_ascii_lowercase();
        while let Some(pos) = lower[start..].find(&format!("<{tag}")) {
            let open = start + pos;
            let Some(gt_rel) = lower[open..].find('>') else {
                break;
            };
            let body_start = open + gt_rel + 1;
            let Some(close_rel) = lower[body_start..].find(&format!("</{tag}>")) else {
                break;
            };
            let body_end = body_start + close_rel;
            let text = clean_text(&html[body_start..body_end]);
            if !text.is_empty() {
                out.push(text);
            }
            start = body_end + tag.len() + 3;
        }
    }
    out
}

fn meta_content(html: &str, name: &str) -> Option<String> {
    for tag in tags_named(html, "meta") {
        let attrs = parse_attrs(tag);
        let attr_name = attrs
            .get("name")
            .or_else(|| attrs.get("property"))
            .map(|value| value.to_ascii_lowercase());
        if attr_name.as_deref() == Some(name) {
            return attrs.get("content").map(|value| clean_text(value));
        }
    }
    None
}

fn html_lang(html: &str) -> Option<String> {
    let tag = tags_named(html, "html").into_iter().next()?;
    parse_attrs(tag)
        .get("lang")
        .cloned()
        .filter(|value| !value.is_empty())
}

fn canonical_url(base: &Url, html: &str) -> Option<String> {
    for tag in tags_named(html, "link") {
        let attrs = parse_attrs(tag);
        if attrs
            .get("rel")
            .map(|value| {
                value
                    .to_ascii_lowercase()
                    .split_whitespace()
                    .any(|part| part == "canonical")
            })
            .unwrap_or(false)
        {
            let href = attrs.get("href")?;
            return base.join(href).ok().map(|url| url.to_string());
        }
    }
    None
}

fn anchor_text(html: &str) -> Vec<String> {
    let mut out = Vec::new();
    let lower = html.to_ascii_lowercase();
    let mut start = 0usize;
    while let Some(pos) = lower[start..].find("<a") {
        let open = start + pos;
        let Some(gt_rel) = lower[open..].find('>') else {
            break;
        };
        let body_start = open + gt_rel + 1;
        let Some(close_rel) = lower[body_start..].find("</a>") else {
            break;
        };
        let body_end = body_start + close_rel;
        let text = clean_text(&html[body_start..body_end]);
        if !text.is_empty() {
            out.push(text);
        }
        start = body_end + 4;
    }
    out
}

fn outbound_links(base: &Url, html: &str) -> Vec<String> {
    let mut out = Vec::new();
    for tag in tags_named(html, "a") {
        let Some(href) = parse_attrs(tag).get("href").cloned() else {
            continue;
        };
        if let Ok(mut url) = base.join(&href) {
            url.set_fragment(None);
            if matches!(url.scheme(), "http" | "https") {
                out.push(url.to_string());
            }
        }
    }
    out.sort();
    out.dedup();
    out
}

fn visible_text(html: &str) -> String {
    let without_scripts = remove_block(html, "script");
    let without_styles = remove_block(&without_scripts, "style");
    clean_text(&strip_tags(&without_styles))
}

fn remove_block(html: &str, tag: &str) -> String {
    let mut out = String::new();
    let mut start = 0usize;
    let lower = html.to_ascii_lowercase();
    while let Some(open_rel) = lower[start..].find(&format!("<{tag}")) {
        let open = start + open_rel;
        out.push_str(&html[start..open]);
        let Some(close_rel) = lower[open..].find(&format!("</{tag}>")) else {
            return out;
        };
        start = open + close_rel + tag.len() + 3;
    }
    out.push_str(&html[start..]);
    out
}

fn strip_tags(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut in_tag = false;
    for ch in html.chars() {
        match ch {
            '<' => {
                in_tag = true;
                out.push(' ');
            }
            '>' => {
                in_tag = false;
                out.push(' ');
            }
            _ if !in_tag => out.push(ch),
            _ => {}
        }
    }
    out
}

fn clean_text(text: &str) -> String {
    let decoded = html_escape::decode_html_entities(text);
    decoded.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn tags_named<'a>(html: &'a str, name: &str) -> Vec<&'a str> {
    let lower = html.to_ascii_lowercase();
    let mut tags = Vec::new();
    let mut start = 0usize;
    while let Some(pos) = lower[start..].find(&format!("<{name}")) {
        let open = start + pos;
        let Some(end_rel) = lower[open..].find('>') else {
            break;
        };
        let end = open + end_rel + 1;
        tags.push(&html[open..end]);
        start = end;
    }
    tags
}

fn parse_attrs(tag: &str) -> std::collections::HashMap<String, String> {
    let mut attrs = std::collections::HashMap::new();
    let bytes = tag.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        while i < bytes.len() && !bytes[i].is_ascii_alphabetic() {
            i += 1;
        }
        let key_start = i;
        while i < bytes.len()
            && (bytes[i].is_ascii_alphanumeric() || matches!(bytes[i], b'-' | b':' | b'_'))
        {
            i += 1;
        }
        if key_start == i {
            continue;
        }
        let key = tag[key_start..i].to_ascii_lowercase();
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= bytes.len() || bytes[i] != b'=' {
            continue;
        }
        i += 1;
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= bytes.len() {
            break;
        }
        let quote = bytes[i];
        let value = if quote == b'"' || quote == b'\'' {
            i += 1;
            let value_start = i;
            while i < bytes.len() && bytes[i] != quote {
                i += 1;
            }
            let value = tag[value_start..i].to_owned();
            i += usize::from(i < bytes.len());
            value
        } else {
            let value_start = i;
            while i < bytes.len() && !bytes[i].is_ascii_whitespace() && bytes[i] != b'>' {
                i += 1;
            }
            tag[value_start..i].to_owned()
        };
        attrs.insert(key, html_escape::decode_html_entities(&value).into_owned());
    }
    attrs
}
