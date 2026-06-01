use std::cmp::Ordering;

use anyhow::Result;
use rustc_hash::{FxHashMap, FxHashSet};
use serde::{Deserialize, Serialize};

use crate::index::{Posting, SearchIndex};
use crate::tokenizer::for_each_term;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SearchOptions {
    pub limit: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SearchResult {
    pub doc_id: u32,
    pub url: String,
    pub canonical_url: Option<String>,
    pub title: String,
    pub language: Option<String>,
    pub fetched_at_unix: Option<u64>,
    pub score: f32,
    pub authority_score: f32,
    pub snippet: String,
    pub duplicate_of: u32,
    pub duplicate_count: u32,
}

#[derive(Debug, Default)]
struct QueryPlan {
    terms: Vec<String>,
    excluded_terms: Vec<String>,
    site: Option<String>,
    file_type: Option<String>,
    language: Option<String>,
}

pub fn search_index(
    index: &SearchIndex,
    query: &str,
    options: SearchOptions,
) -> Result<Vec<SearchResult>> {
    if options.limit == 0 {
        return Ok(Vec::new());
    }
    let plan = parse_query(query);
    if plan.terms.is_empty() {
        return Ok(Vec::new());
    }

    let mut scores: FxHashMap<u32, f32> = FxHashMap::default();
    let mut matched_terms: FxHashMap<u32, FxHashSet<String>> = FxHashMap::default();
    let doc_count = index.manifest().doc_count.max(1) as f32;

    for term in &plan.terms {
        let Some(postings) = index.postings(term)? else {
            continue;
        };
        let idf = index
            .term_entry(term)
            .map(|entry| {
                ((doc_count + 1.0) / (entry.doc_freq as f32 + 0.5))
                    .ln()
                    .max(0.1)
            })
            .unwrap_or(0.1);
        for posting in postings.iter() {
            let score = posting_score(posting, idf);
            *scores.entry(posting.doc_id).or_insert(0.0) += score;
            matched_terms
                .entry(posting.doc_id)
                .or_default()
                .insert(term.clone());
        }
    }

    if scores.is_empty() {
        return Ok(Vec::new());
    }

    for excluded in &plan.excluded_terms {
        if let Some(postings) = index.postings(excluded)? {
            for posting in postings.iter() {
                scores.remove(&posting.doc_id);
            }
        }
    }

    let required = plan.terms.iter().collect::<FxHashSet<_>>();
    let mut results = Vec::new();
    let mut seen_representatives = FxHashSet::default();
    for (doc_id, score) in scores {
        let Some(doc) = index.doc(doc_id) else {
            continue;
        };
        if !passes_filters(doc, &plan) {
            continue;
        }
        if !matched_terms
            .get(&doc_id)
            .map(|terms| required.iter().all(|term| terms.contains(*term)))
            .unwrap_or(false)
        {
            continue;
        }
        if !seen_representatives.insert(doc.duplicate_of) {
            continue;
        }
        let text = index.text(doc_id).unwrap_or("");
        results.push(SearchResult {
            doc_id,
            url: doc.url.clone(),
            canonical_url: doc.canonical_url.clone(),
            title: doc.title.clone(),
            language: doc.language.clone(),
            fetched_at_unix: doc.fetched_at_unix,
            score: score + doc.authority_score,
            authority_score: doc.authority_score,
            snippet: make_snippet(text, &plan.terms),
            duplicate_of: doc.duplicate_of,
            duplicate_count: doc.duplicate_count,
        });
    }

    results.sort_by(|left, right| {
        right
            .score
            .partial_cmp(&left.score)
            .unwrap_or(Ordering::Equal)
            .then_with(|| left.doc_id.cmp(&right.doc_id))
    });
    results.truncate(options.limit);
    Ok(results)
}

fn posting_score(posting: &Posting, idf: f32) -> f32 {
    let tf = posting.tf as f32;
    let field_boost = posting.field_tfs.weighted_tf().max(tf) / tf.max(1.0);
    (1.0 + tf.ln()) * idf * field_boost
}

fn parse_query(query: &str) -> QueryPlan {
    let mut plan = QueryPlan::default();
    for raw in query.split_whitespace() {
        if raw.eq_ignore_ascii_case("or") {
            continue;
        }
        let token = raw.trim_matches('"');
        if let Some(value) = token.strip_prefix("site:") {
            plan.site = Some(value.to_ascii_lowercase());
            continue;
        }
        if let Some(value) = token
            .strip_prefix("type:")
            .or_else(|| token.strip_prefix("filetype:"))
        {
            plan.file_type = Some(value.trim_start_matches('.').to_ascii_lowercase());
            continue;
        }
        if let Some(value) = token.strip_prefix("lang:") {
            plan.language = Some(value.to_ascii_lowercase());
            continue;
        }
        let excluded = token.starts_with('-');
        let token = token
            .trim_start_matches('+')
            .trim_start_matches('-')
            .trim_matches('"');
        let mut terms = Vec::new();
        for_each_term(token, |term, _| terms.push(term.to_owned()));
        if excluded {
            plan.excluded_terms.extend(terms);
        } else {
            plan.terms.extend(terms);
        }
    }
    plan.terms.sort();
    plan.terms.dedup();
    plan.excluded_terms.sort();
    plan.excluded_terms.dedup();
    plan
}

fn passes_filters(doc: &crate::index::DocMeta, plan: &QueryPlan) -> bool {
    if let Some(site) = &plan.site
        && !doc.url.to_ascii_lowercase().contains(site)
    {
        return false;
    }
    if let Some(language) = &plan.language
        && doc
            .language
            .as_deref()
            .map(str::to_ascii_lowercase)
            .as_deref()
            != Some(language)
    {
        return false;
    }
    if let Some(file_type) = &plan.file_type {
        let url = doc
            .url
            .split('?')
            .next()
            .unwrap_or(&doc.url)
            .to_ascii_lowercase();
        if !url.ends_with(&format!(".{file_type}")) {
            return false;
        }
    }
    true
}

fn make_snippet(text: &str, terms: &[String]) -> String {
    if text.is_empty() {
        return String::new();
    }
    let lower = text.to_ascii_lowercase();
    let pos = terms
        .iter()
        .filter_map(|term| lower.find(term))
        .min()
        .unwrap_or(0);
    let start = text[..pos.min(text.len())]
        .char_indices()
        .rev()
        .nth(80)
        .map(|(idx, _)| idx)
        .unwrap_or(0);
    let mut end = pos.saturating_add(220).min(text.len());
    while end > start && !text.is_char_boundary(end) {
        end -= 1;
    }
    let mut snippet = text[start..end].trim().to_owned();
    if start > 0 {
        snippet.insert_str(0, "...");
    }
    if end < text.len() {
        snippet.push_str("...");
    }
    snippet
}
