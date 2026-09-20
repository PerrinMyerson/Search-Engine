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
    required_terms: Vec<String>,
    or_groups: Vec<Vec<String>>,
    required_phrases: Vec<String>,
    excluded_terms: Vec<String>,
    excluded_phrases: Vec<String>,
    site: Option<String>,
    excluded_sites: Vec<String>,
    file_type: Option<String>,
    excluded_file_types: Vec<String>,
    language: Option<String>,
    excluded_languages: Vec<String>,
    after_unix: Option<u64>,
    before_unix: Option<u64>,
}

impl QueryPlan {
    fn retrieval_terms(&self) -> Vec<String> {
        let mut terms = self.required_terms.clone();
        for group in &self.or_groups {
            terms.extend(group.iter().cloned());
        }
        for phrase in &self.required_phrases {
            collect_unique_terms(phrase, &mut terms);
        }
        terms.sort();
        terms.dedup();
        terms
    }

    fn snippet_terms(&self) -> Vec<String> {
        let mut terms = self.retrieval_terms();
        for phrase in &self.required_phrases {
            if !terms.iter().any(|term| term == phrase) {
                terms.push(phrase.clone());
            }
        }
        terms
    }
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
    let retrieval_terms = plan.retrieval_terms();
    if retrieval_terms.is_empty() {
        return Ok(Vec::new());
    }

    let mut scores: FxHashMap<u32, f32> = FxHashMap::default();
    let mut matched_terms: FxHashMap<u32, FxHashSet<String>> = FxHashMap::default();
    let doc_count = index.manifest().doc_count.max(1) as f32;

    for term in &retrieval_terms {
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

    let required = plan.required_terms.iter().collect::<FxHashSet<_>>();
    let snippet_terms = plan.snippet_terms();
    let mut results = Vec::new();
    let mut seen_representatives = FxHashSet::default();
    for (doc_id, score) in scores {
        let Some(doc) = index.doc(doc_id) else {
            continue;
        };
        if !passes_filters(doc, &plan) {
            continue;
        }
        let Some(matched) = matched_terms.get(&doc_id) else {
            continue;
        };
        if !required.iter().all(|term| matched.contains(*term)) {
            continue;
        }
        if !plan
            .or_groups
            .iter()
            .all(|group| group.iter().any(|term| matched.contains(term)))
        {
            continue;
        }
        if !passes_phrase_filters(index, doc_id, doc, &plan) {
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
            snippet: make_snippet(text, &snippet_terms),
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
    let tokens = lex_query(query);
    let mut query_tokens = Vec::new();
    for token in tokens {
        if is_filter_token(&token, &mut plan) {
            continue;
        }
        query_tokens.push(token);
    }

    let mut index = 0usize;
    while index < query_tokens.len() {
        let token = &query_tokens[index];
        if is_or_operator(token) {
            index += 1;
            continue;
        }
        if !token.excluded
            && index + 1 < query_tokens.len()
            && is_or_operator(&query_tokens[index + 1])
        {
            let mut group = Vec::new();
            collect_unique_terms(&token.text, &mut group);
            index += 2;
            while index < query_tokens.len() {
                let next = &query_tokens[index];
                if next.excluded || is_or_operator(next) {
                    break;
                }
                collect_unique_terms(&next.text, &mut group);
                if index + 1 < query_tokens.len() && is_or_operator(&query_tokens[index + 1]) {
                    index += 2;
                } else {
                    index += 1;
                    break;
                }
            }
            group.sort();
            group.dedup();
            if !group.is_empty() {
                plan.or_groups.push(group);
            }
            continue;
        }

        if token.quoted {
            let phrase = normalize_phrase_text(&token.text);
            if !phrase.is_empty() {
                if token.excluded {
                    plan.excluded_phrases.push(phrase);
                } else {
                    plan.required_phrases.push(phrase);
                }
            }
        } else if token.excluded {
            collect_unique_terms(&token.text, &mut plan.excluded_terms);
        } else {
            collect_unique_terms(&token.text, &mut plan.required_terms);
        }
        index += 1;
    }
    plan.required_terms.sort();
    plan.required_terms.dedup();
    plan.required_phrases.sort();
    plan.required_phrases.dedup();
    plan.excluded_terms.sort();
    plan.excluded_terms.dedup();
    plan.excluded_phrases.sort();
    plan.excluded_phrases.dedup();
    plan.excluded_sites.sort();
    plan.excluded_sites.dedup();
    plan.excluded_file_types.sort();
    plan.excluded_file_types.dedup();
    plan.excluded_languages.sort();
    plan.excluded_languages.dedup();
    plan
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct QueryToken {
    text: String,
    excluded: bool,
    quoted: bool,
}

fn lex_query(query: &str) -> Vec<QueryToken> {
    let chars = query.chars().collect::<Vec<_>>();
    let mut tokens = Vec::new();
    let mut index = 0usize;
    while index < chars.len() {
        while index < chars.len() && chars[index].is_whitespace() {
            index += 1;
        }
        if index >= chars.len() {
            break;
        }

        let mut excluded = false;
        if matches!(chars[index], '-' | '+') {
            excluded = chars[index] == '-';
            index += 1;
        }

        let mut quoted = false;
        let mut text = String::new();
        if index < chars.len() && chars[index] == '"' {
            quoted = true;
            index += 1;
            while index < chars.len() && chars[index] != '"' {
                text.push(chars[index]);
                index += 1;
            }
            if index < chars.len() && chars[index] == '"' {
                index += 1;
            }
        } else {
            while index < chars.len() && !chars[index].is_whitespace() {
                text.push(chars[index]);
                index += 1;
            }
            text = text.trim_matches('"').to_owned();
        }

        if !text.is_empty() {
            tokens.push(QueryToken {
                text,
                excluded,
                quoted,
            });
        }
    }
    tokens
}

fn is_or_operator(token: &QueryToken) -> bool {
    !token.excluded && !token.quoted && token.text.eq_ignore_ascii_case("or")
}

fn is_filter_token(token: &QueryToken, plan: &mut QueryPlan) -> bool {
    let Some((key, value)) = split_query_filter(&token.text) else {
        return false;
    };
    let value = value.trim();
    if value.is_empty() {
        return false;
    }
    match key.as_str() {
        "site" => {
            let value = value.to_ascii_lowercase();
            if token.excluded {
                plan.excluded_sites.push(value);
            } else {
                plan.site = Some(value);
            }
            true
        }
        "type" | "filetype" => {
            let value = value.trim_start_matches('.').to_ascii_lowercase();
            if token.excluded {
                plan.excluded_file_types.push(value);
            } else {
                plan.file_type = Some(value);
            }
            true
        }
        "lang" | "language" => {
            let value = value.to_ascii_lowercase();
            if token.excluded {
                plan.excluded_languages.push(value);
            } else {
                plan.language = Some(value);
            }
            true
        }
        "after" => {
            if !token.excluded {
                plan.after_unix = parse_ymd_unix_bound(value, false);
            }
            true
        }
        "before" => {
            if !token.excluded {
                plan.before_unix = parse_ymd_unix_bound(value, true);
            }
            true
        }
        _ => false,
    }
}

fn split_query_filter(token: &str) -> Option<(String, &str)> {
    let (key, value) = token.split_once(':')?;
    Some((key.to_ascii_lowercase(), value))
}

fn collect_unique_terms(text: &str, terms: &mut Vec<String>) {
    for_each_term(text, |term, _| {
        if !terms.iter().any(|seen| seen == term) {
            terms.push(term.to_owned());
        }
    });
}

fn passes_filters(doc: &crate::index::DocMeta, plan: &QueryPlan) -> bool {
    if let Some(site) = &plan.site
        && !doc.url.to_ascii_lowercase().contains(site)
    {
        return false;
    }
    let lower_url = doc.url.to_ascii_lowercase();
    if plan
        .excluded_sites
        .iter()
        .any(|site| lower_url.contains(site))
    {
        return false;
    }
    if let Some(language) = &plan.language
        && !doc_language_matches(doc.language.as_deref(), language)
    {
        return false;
    }
    if plan
        .excluded_languages
        .iter()
        .any(|language| doc_language_matches(doc.language.as_deref(), language))
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
    if !plan.excluded_file_types.is_empty() {
        let url = doc
            .url
            .split('?')
            .next()
            .unwrap_or(&doc.url)
            .to_ascii_lowercase();
        if plan
            .excluded_file_types
            .iter()
            .any(|file_type| url.ends_with(&format!(".{file_type}")))
        {
            return false;
        }
    }
    if let Some(after) = plan.after_unix
        && !doc
            .fetched_at_unix
            .is_some_and(|fetched_at| fetched_at >= after)
    {
        return false;
    }
    if let Some(before) = plan.before_unix
        && !doc
            .fetched_at_unix
            .is_some_and(|fetched_at| fetched_at <= before)
    {
        return false;
    }
    true
}

fn doc_language_matches(language: Option<&str>, filter: &str) -> bool {
    let Some(language) = language else {
        return false;
    };
    let language = language.to_ascii_lowercase();
    language == filter
        || language
            .strip_prefix(filter)
            .is_some_and(|suffix| suffix.starts_with('-') || suffix.starts_with('_'))
}

fn passes_phrase_filters(
    index: &SearchIndex,
    doc_id: u32,
    doc: &crate::index::DocMeta,
    plan: &QueryPlan,
) -> bool {
    if plan.required_phrases.is_empty() && plan.excluded_phrases.is_empty() {
        return true;
    }
    let text = normalized_document_search_text(index, doc_id, doc);
    plan.required_phrases
        .iter()
        .all(|phrase| text.contains(phrase))
        && plan
            .excluded_phrases
            .iter()
            .all(|phrase| !text.contains(phrase))
}

fn normalized_document_search_text(
    index: &SearchIndex,
    doc_id: u32,
    doc: &crate::index::DocMeta,
) -> String {
    let mut text = String::new();
    push_search_text(&mut text, &doc.title);
    push_search_text(&mut text, &doc.url);
    if let Some(canonical_url) = &doc.canonical_url {
        push_search_text(&mut text, canonical_url);
    }
    if let Some(field_doc) = index.field_doc(doc_id) {
        if let Some(meta_description) = &field_doc.meta_description {
            push_search_text(&mut text, meta_description);
        }
        for heading in &field_doc.headings {
            push_search_text(&mut text, heading);
        }
        push_search_text(&mut text, &field_doc.body);
        for anchor in &field_doc.anchor_text {
            push_search_text(&mut text, anchor);
        }
    } else if let Some(body) = index.text(doc_id) {
        push_search_text(&mut text, body);
    }
    normalize_phrase_text(&text)
}

fn push_search_text(target: &mut String, text: &str) {
    if text.trim().is_empty() {
        return;
    }
    if !target.is_empty() {
        target.push(' ');
    }
    target.push_str(text);
}

fn normalize_phrase_text(text: &str) -> String {
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase()
}

fn parse_ymd_unix_bound(value: &str, end_of_day: bool) -> Option<u64> {
    let mut parts = value.split('-');
    let year = parts.next()?.parse::<i32>().ok()?;
    let month = parts.next()?.parse::<u32>().ok()?;
    let day = parts.next()?.parse::<u32>().ok()?;
    if parts.next().is_some() || !valid_ymd(year, month, day) {
        return None;
    }
    let days = days_from_civil(year, month, day)?;
    let seconds = days.checked_mul(86_400)?;
    let seconds = if end_of_day {
        seconds.checked_add(86_399)?
    } else {
        seconds
    };
    u64::try_from(seconds).ok()
}

fn valid_ymd(year: i32, month: u32, day: u32) -> bool {
    if !(1..=12).contains(&month) {
        return false;
    }
    let max_day = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap_year(year) => 29,
        2 => 28,
        _ => return false,
    };
    (1..=max_day).contains(&day)
}

fn is_leap_year(year: i32) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

fn days_from_civil(year: i32, month: u32, day: u32) -> Option<i64> {
    let year = year - i32::from(month <= 2);
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let year_of_era = year - era * 400;
    let month = month as i32;
    let day_of_year = (153 * (month + if month > 2 { -3 } else { 9 }) + 2) / 5 + day as i32 - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    Some((era * 146_097 + day_of_era - 719_468) as i64)
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
