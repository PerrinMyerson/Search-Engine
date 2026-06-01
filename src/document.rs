use serde::{Deserialize, Serialize};
use url::Url;

use crate::extract::ExtractedPage;
use crate::tokenizer::for_each_term;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ExtractionMode {
    StaticHtml,
    PlainText,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DocumentQuality {
    pub robots_noindex: bool,
    pub body_term_count: u32,
}

impl Default for DocumentQuality {
    fn default() -> Self {
        Self {
            robots_noindex: false,
            body_term_count: 0,
        }
    }
}

impl DocumentQuality {
    pub fn for_body(body: &str, robots_noindex: bool) -> Self {
        let mut body_term_count = 0u32;
        for_each_term(body, |_, _| {
            body_term_count = body_term_count.saturating_add(1)
        });
        Self {
            robots_noindex,
            body_term_count,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SearchDocument {
    pub url: String,
    pub title: String,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FieldedDocument {
    pub url: String,
    pub canonical_url: Option<String>,
    pub title: String,
    pub meta_description: Option<String>,
    pub language: Option<String>,
    pub quality: DocumentQuality,
    pub headings: Vec<String>,
    pub body: String,
    pub anchor_text: Vec<String>,
    pub outbound_links: Vec<String>,
    pub content_hash: Option<String>,
    pub fetched_at_unix: Option<u64>,
    pub extraction_mode: ExtractionMode,
}

impl FieldedDocument {
    pub fn from_plain_text(
        url: String,
        title: String,
        body: String,
        content_hash: Option<String>,
    ) -> Self {
        Self {
            url,
            canonical_url: None,
            title,
            meta_description: None,
            language: None,
            quality: DocumentQuality::for_body(&body, false),
            headings: Vec::new(),
            body,
            anchor_text: Vec::new(),
            outbound_links: Vec::new(),
            content_hash,
            fetched_at_unix: None,
            extraction_mode: ExtractionMode::PlainText,
        }
    }

    pub fn from_extracted(
        base: &Url,
        extracted: ExtractedPage,
        content_hash: Option<String>,
        fetched_at_unix: Option<u64>,
    ) -> Self {
        let body = extracted.body;
        Self {
            url: base.to_string(),
            canonical_url: extracted.canonical_url,
            title: extracted.title,
            meta_description: extracted.meta_description,
            language: extracted.language,
            quality: DocumentQuality::for_body(&body, extracted.robots_noindex),
            headings: extracted.headings,
            body,
            anchor_text: extracted.anchor_text,
            outbound_links: extracted.outbound_links,
            content_hash,
            fetched_at_unix,
            extraction_mode: ExtractionMode::StaticHtml,
        }
    }
}
