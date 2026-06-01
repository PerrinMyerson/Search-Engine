use serde::{Deserialize, Serialize};

use crate::index::{TermCorrection, TermSuggestion};
use crate::query::SearchResult;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
pub enum DaemonRequest {
    Search {
        query: String,
        limit: usize,
    },
    Suggest {
        prefix: String,
        limit: usize,
    },
    Spell {
        term: String,
        limit: usize,
    },
    Render {
        target: String,
    },
    BenchSearch {
        queries: Vec<String>,
        limit: usize,
        warmup: usize,
    },
    Stats,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum DaemonResponse {
    Search {
        results: Vec<SearchResult>,
    },
    Suggest {
        suggestions: Vec<TermSuggestion>,
    },
    Spell {
        corrections: Vec<TermCorrection>,
    },
    Render {
        text: String,
    },
    BenchSearch {
        timings_us: Vec<u64>,
        total_us: u64,
    },
    Stats {
        doc_count: u32,
        term_count: u32,
        total_terms: u64,
        avg_doc_len: f32,
        duplicate_cluster_count: u32,
        duplicate_doc_count: u32,
        skipped_noindex_count: u32,
        skipped_thin_count: u32,
        max_authority_score: f32,
        corpus_hash: String,
    },
    Error {
        message: String,
    },
}
