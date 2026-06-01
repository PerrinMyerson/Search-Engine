use std::cmp::Ordering;
use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use memmap2::{Mmap, MmapOptions};
use rustc_hash::{FxHashMap, FxHashSet};
use serde::{Deserialize, Serialize};
use url::Url;

use crate::document::{DocumentQuality, FieldedDocument, SearchDocument};
use crate::extract::extract_html;
use crate::query::{SearchOptions, SearchResult, search_index};
use crate::tokenizer::for_each_term;
use crate::varint::{put_u32, read_u32};

const FORMAT_VERSION: u32 = 7;
const MANIFEST: &str = "manifest.json";
const DOCS: &str = "docs.bin";
const FIELD_DOCS: &str = "field_docs.bin";
const LEXICON: &str = "lexicon.bin";
const POSTINGS: &str = "postings.bin";
const TEXTS: &str = "texts.bin";
const MAX_POSITIONS_PER_DOC_TERM: usize = 32;
const NEAR_DUPLICATE_MIN_TERMS: usize = 8;
const SIMHASH_SHINGLE_TERMS: usize = 4;
const SIMHASH_HAMMING_THRESHOLD: u32 = 12;

pub use crate::document::SearchDocument as RawDocument;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreloadMode {
    Lazy,
    Aggressive,
}

impl std::str::FromStr for PreloadMode {
    type Err = anyhow::Error;

    fn from_str(value: &str) -> Result<Self> {
        match value {
            "lazy" => Ok(Self::Lazy),
            "aggressive" => Ok(Self::Aggressive),
            other => bail!("unknown preload mode: {other}"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct IndexBuildOptions {
    pub overwrite: bool,
    pub respect_noindex: bool,
    pub min_body_terms: u32,
}

impl Default for IndexBuildOptions {
    fn default() -> Self {
        Self {
            overwrite: true,
            respect_noindex: true,
            min_body_terms: 0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuildStats {
    pub doc_count: u32,
    pub term_count: u32,
    pub total_terms: u64,
    pub avg_doc_len: f32,
    pub duplicate_cluster_count: u32,
    pub duplicate_doc_count: u32,
    pub skipped_noindex_count: u32,
    pub skipped_thin_count: u32,
    pub max_authority_score: f32,
    pub corpus_hash: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexManifest {
    pub format_version: u32,
    pub created_unix_seconds: u64,
    pub doc_count: u32,
    pub term_count: u32,
    pub total_terms: u64,
    pub avg_doc_len: f32,
    pub duplicate_cluster_count: u32,
    pub duplicate_doc_count: u32,
    pub skipped_noindex_count: u32,
    pub skipped_thin_count: u32,
    pub max_authority_score: f32,
    pub corpus_hash: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocMeta {
    pub id: u32,
    pub url: String,
    pub canonical_url: Option<String>,
    pub title: String,
    pub language: Option<String>,
    pub fetched_at_unix: Option<u64>,
    pub content_hash: Option<String>,
    pub quality: DocumentQuality,
    pub duplicate_of: u32,
    pub duplicate_count: u32,
    pub authority_score: f32,
    pub text_start: u64,
    pub text_len: u32,
    pub term_count: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TermEntry {
    pub term: String,
    pub postings_start: u64,
    pub postings_len: u64,
    pub doc_freq: u32,
    pub collection_freq: u32,
    pub max_tf: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TermSuggestion {
    pub term: String,
    pub doc_freq: u32,
    pub collection_freq: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TermCorrection {
    pub term: String,
    pub doc_freq: u32,
    pub collection_freq: u32,
    pub distance: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Posting {
    pub doc_id: u32,
    pub tf: u32,
    pub field_tfs: FieldTfs,
    pub positions: Vec<u32>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FieldTfs {
    pub title: u32,
    pub meta: u32,
    pub heading: u32,
    pub body: u32,
    pub anchor: u32,
    pub url: u32,
}

impl FieldTfs {
    fn add(&mut self, field: IndexedField) {
        match field {
            IndexedField::Title => self.title += 1,
            IndexedField::Meta => self.meta += 1,
            IndexedField::Heading => self.heading += 1,
            IndexedField::Body => self.body += 1,
            IndexedField::Anchor => self.anchor += 1,
            IndexedField::Url => self.url += 1,
        }
    }

    pub fn weighted_tf(&self) -> f32 {
        self.title as f32 * 5.0
            + self.heading as f32 * 3.0
            + self.anchor as f32 * 2.5
            + self.meta as f32 * 2.0
            + self.url as f32 * 1.5
            + self.body as f32
    }
}

#[derive(Debug, Clone, Copy)]
enum IndexedField {
    Title,
    Meta,
    Heading,
    Body,
    Anchor,
    Url,
}

#[derive(Debug)]
struct IndexFieldText {
    field: IndexedField,
    text: String,
}

#[derive(Debug)]
struct IndexDocumentInput {
    url: String,
    canonical_url: Option<String>,
    title: String,
    language: Option<String>,
    fetched_at_unix: Option<u64>,
    content_hash: Option<String>,
    quality: DocumentQuality,
    stored_text: String,
    fields: Vec<IndexFieldText>,
    outbound_links: Vec<String>,
}

#[derive(Debug)]
pub struct SearchIndex {
    root: PathBuf,
    manifest: IndexManifest,
    docs: Vec<DocMeta>,
    field_docs: Option<Vec<FieldedDocument>>,
    url_to_doc: FxHashMap<String, u32>,
    lexicon: FxHashMap<String, TermEntry>,
    postings_mmap: Mmap,
    texts_mmap: Mmap,
    preloaded: FxHashMap<String, Arc<Vec<Posting>>>,
}

#[derive(Debug)]
struct DocTerm {
    tf: u32,
    field_tfs: FieldTfs,
    positions: Vec<u32>,
}

impl SearchIndex {
    pub fn open(path: impl AsRef<Path>, preload: PreloadMode) -> Result<Self> {
        let root = path.as_ref().to_path_buf();
        let manifest_path = root.join(MANIFEST);
        let manifest_bytes = fs::read(&manifest_path).with_context(|| {
            format!(
                "index manifest not found at {}. Build the index first, for example: brutal-search crawl https://example.com --index {}",
                manifest_path.display(),
                root.display()
            )
        })?;
        let manifest: IndexManifest = serde_json::from_slice(&manifest_bytes)?;
        anyhow::ensure!(
            manifest.format_version == FORMAT_VERSION,
            "unsupported index format {}; expected {}. Rebuild the index with the current binary.",
            manifest.format_version,
            FORMAT_VERSION
        );

        let docs: Vec<DocMeta> = bincode::deserialize(
            &fs::read(root.join(DOCS)).with_context(|| format!("read {}", DOCS))?,
        )?;
        let field_docs = match fs::read(root.join(FIELD_DOCS)) {
            Ok(bytes) => Some(bincode::deserialize(&bytes)?),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => return Err(error).with_context(|| format!("read {}", FIELD_DOCS)),
        };
        let terms: Vec<TermEntry> = bincode::deserialize(
            &fs::read(root.join(LEXICON)).with_context(|| format!("read {}", LEXICON))?,
        )?;

        let postings_file = File::open(root.join(POSTINGS))?;
        let texts_file = File::open(root.join(TEXTS))?;
        let postings_mmap = unsafe { MmapOptions::new().map(&postings_file)? };
        let texts_mmap = unsafe { MmapOptions::new().map(&texts_file)? };

        let mut url_to_doc = FxHashMap::default();
        for doc in &docs {
            url_to_doc.insert(doc.url.clone(), doc.id);
        }

        let mut lexicon = FxHashMap::default();
        for term in terms {
            lexicon.insert(term.term.clone(), term);
        }

        let mut index = Self {
            root,
            manifest,
            docs,
            field_docs,
            url_to_doc,
            lexicon,
            postings_mmap,
            texts_mmap,
            preloaded: FxHashMap::default(),
        };

        if preload == PreloadMode::Aggressive {
            index.preload_all_postings()?;
        }

        Ok(index)
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn manifest(&self) -> &IndexManifest {
        &self.manifest
    }

    pub fn docs(&self) -> &[DocMeta] {
        &self.docs
    }

    pub fn doc(&self, doc_id: u32) -> Option<&DocMeta> {
        self.docs
            .get(doc_id as usize)
            .filter(|doc| doc.id == doc_id)
    }

    pub fn field_doc(&self, doc_id: u32) -> Option<&FieldedDocument> {
        self.field_docs
            .as_ref()?
            .get(doc_id as usize)
            .filter(|doc| self.doc(doc_id).is_some_and(|meta| meta.url == doc.url))
    }

    pub fn field_docs(&self) -> Option<&[FieldedDocument]> {
        self.field_docs.as_deref()
    }

    pub fn doc_id_for_url(&self, url: &str) -> Option<u32> {
        self.url_to_doc.get(url).copied()
    }

    pub fn text(&self, doc_id: u32) -> Option<&str> {
        let doc = self.doc(doc_id)?;
        let start = doc.text_start as usize;
        let end = start.checked_add(doc.text_len as usize)?;
        std::str::from_utf8(self.texts_mmap.get(start..end)?).ok()
    }

    pub fn search(&self, query: &str, options: SearchOptions) -> Result<Vec<SearchResult>> {
        search_index(self, query, options)
    }

    pub fn suggest(&self, prefix: &str, limit: usize) -> Vec<TermSuggestion> {
        let Some(prefix) = normalized_trailing_term(prefix) else {
            return Vec::new();
        };
        if limit == 0 {
            return Vec::new();
        }

        let mut suggestions = self
            .lexicon
            .iter()
            .filter(|(term, _)| term.starts_with(&prefix))
            .map(|(term, entry)| TermSuggestion {
                term: term.clone(),
                doc_freq: entry.doc_freq,
                collection_freq: entry.collection_freq,
            })
            .collect::<Vec<_>>();
        suggestions.sort_unstable_by(|left, right| {
            right
                .collection_freq
                .cmp(&left.collection_freq)
                .then_with(|| right.doc_freq.cmp(&left.doc_freq))
                .then_with(|| left.term.cmp(&right.term))
        });
        suggestions.truncate(limit);
        suggestions
    }

    pub fn spellcheck(&self, input: &str, limit: usize) -> Vec<TermCorrection> {
        let Some(term) = normalized_trailing_term(input) else {
            return Vec::new();
        };
        if limit == 0 || term.len() < 3 || self.lexicon.contains_key(&term) {
            return Vec::new();
        }

        let max_distance = max_spell_distance(term.len());
        let mut corrections = self
            .lexicon
            .iter()
            .filter_map(|(candidate, entry)| {
                let distance = bounded_edit_distance(&term, candidate, max_distance)?;
                Some(TermCorrection {
                    term: candidate.clone(),
                    doc_freq: entry.doc_freq,
                    collection_freq: entry.collection_freq,
                    distance,
                })
            })
            .collect::<Vec<_>>();
        corrections.sort_unstable_by(|left, right| {
            left.distance
                .cmp(&right.distance)
                .then_with(|| right.collection_freq.cmp(&left.collection_freq))
                .then_with(|| right.doc_freq.cmp(&left.doc_freq))
                .then_with(|| left.term.cmp(&right.term))
        });
        corrections.truncate(limit);
        corrections
    }

    pub fn postings(&self, term: &str) -> Result<Option<Arc<Vec<Posting>>>> {
        if let Some(postings) = self.preloaded.get(term) {
            return Ok(Some(Arc::clone(postings)));
        }

        let Some(entry) = self.lexicon.get(term) else {
            return Ok(None);
        };

        Ok(Some(Arc::new(self.decode_postings(entry)?)))
    }

    pub fn term_entry(&self, term: &str) -> Option<&TermEntry> {
        self.lexicon.get(term)
    }

    fn preload_all_postings(&mut self) -> Result<()> {
        let mut entries: Vec<(String, TermEntry)> = self
            .lexicon
            .iter()
            .map(|(term, entry)| (term.clone(), entry.clone()))
            .collect();
        entries.sort_by(|a, b| a.0.cmp(&b.0));

        for (term, entry) in entries {
            let postings = self.decode_postings(&entry)?;
            self.preloaded.insert(term, Arc::new(postings));
        }
        Ok(())
    }

    fn decode_postings(&self, entry: &TermEntry) -> Result<Vec<Posting>> {
        let start = entry.postings_start as usize;
        let end = start + entry.postings_len as usize;
        decode_postings_bytes(
            self.postings_mmap
                .get(start..end)
                .context("postings slice out of bounds")?,
        )
    }
}

fn normalized_trailing_term(input: &str) -> Option<String> {
    let bytes = input.as_bytes();
    let mut end = bytes.len();
    while end > 0 && !bytes[end - 1].is_ascii_alphanumeric() {
        end -= 1;
    }
    let mut start = end;
    while start > 0 && bytes[start - 1].is_ascii_alphanumeric() {
        start -= 1;
    }
    if start == end {
        return None;
    }

    let raw = &input[start..end];
    let mut prefix = String::with_capacity(raw.len());
    for byte in raw.bytes() {
        prefix.push((byte as char).to_ascii_lowercase());
    }
    Some(prefix)
}

fn max_spell_distance(term_len: usize) -> usize {
    match term_len {
        0..=4 => 1,
        5..=8 => 2,
        _ => 3,
    }
}

fn bounded_edit_distance(left: &str, right: &str, max_distance: usize) -> Option<usize> {
    let left = left.as_bytes();
    let right = right.as_bytes();
    if left.len().abs_diff(right.len()) > max_distance {
        return None;
    }

    let mut previous = (0..=right.len()).collect::<Vec<_>>();
    let mut current = vec![0usize; right.len() + 1];

    for (left_index, left_byte) in left.iter().enumerate() {
        current[0] = left_index + 1;
        let mut row_min = current[0];

        for (right_index, right_byte) in right.iter().enumerate() {
            let substitution_cost = usize::from(left_byte != right_byte);
            let deletion = previous[right_index + 1] + 1;
            let insertion = current[right_index] + 1;
            let substitution = previous[right_index] + substitution_cost;
            let value = deletion.min(insertion).min(substitution);
            current[right_index + 1] = value;
            row_min = row_min.min(value);
        }

        if row_min > max_distance {
            return None;
        }

        std::mem::swap(&mut previous, &mut current);
    }

    (previous[right.len()] <= max_distance).then_some(previous[right.len()])
}

pub fn build_from_corpus(
    corpus_dir: impl AsRef<Path>,
    index_dir: impl AsRef<Path>,
    options: IndexBuildOptions,
) -> Result<BuildStats> {
    let corpus_dir = corpus_dir.as_ref();
    let mut paths = Vec::new();

    for entry in walkdir::WalkDir::new(corpus_dir)
        .follow_links(false)
        .into_iter()
        .filter_map(Result::ok)
    {
        if !entry.file_type().is_file() {
            continue;
        }

        let path = entry.into_path();
        if is_htmlish(&path) {
            paths.push(path);
        }
    }

    paths.sort();
    anyhow::ensure!(
        !paths.is_empty(),
        "no .html, .htm, .xhtml, or .txt files found in {}. If you want to crawl the web instead, run: brutal-search crawl https://example.com --index .brutal-index",
        corpus_dir.display()
    );

    let mut docs = Vec::with_capacity(paths.len());
    for path in paths {
        let bytes = fs::read(&path).with_context(|| format!("read {}", path.display()))?;
        let url = file_url(&path);
        let content_hash = Some(blake3::hash(&bytes).to_hex().to_string());
        let doc = if looks_like_html(&path) {
            let base = url::Url::parse(&url)?;
            let extracted = extract_html(&base, &bytes);
            FieldedDocument::from_extracted(&base, extracted, content_hash, None)
        } else {
            FieldedDocument::from_plain_text(
                url,
                path.file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("text")
                    .to_owned(),
                String::from_utf8_lossy(&bytes).into_owned(),
                content_hash,
            )
        };

        docs.push(doc);
    }

    build_from_fielded_documents(docs, index_dir, options)
}

pub fn build_from_fielded_documents(
    docs: Vec<FieldedDocument>,
    index_dir: impl AsRef<Path>,
    options: IndexBuildOptions,
) -> Result<BuildStats> {
    let (docs, quality_stats) = filter_fielded_documents(docs, &options);
    let inputs = docs
        .iter()
        .map(index_input_from_fielded)
        .collect::<Vec<_>>();
    let stats = build_index_documents(inputs, index_dir.as_ref(), options, quality_stats)?;
    fs::write(
        index_dir.as_ref().join(FIELD_DOCS),
        bincode::serialize(&docs)?,
    )?;
    Ok(stats)
}

pub fn build_from_documents(
    docs: Vec<SearchDocument>,
    index_dir: impl AsRef<Path>,
    options: IndexBuildOptions,
) -> Result<BuildStats> {
    let inputs = docs.into_iter().map(index_input_from_search).collect();
    let (inputs, quality_stats) = filter_index_documents(inputs, &options);
    build_index_documents(inputs, index_dir, options, quality_stats)
}

fn build_index_documents(
    docs: Vec<IndexDocumentInput>,
    index_dir: impl AsRef<Path>,
    options: IndexBuildOptions,
    quality_stats: QualityFilterStats,
) -> Result<BuildStats> {
    anyhow::ensure!(
        !docs.is_empty(),
        "no documents to index. Try a crawl command such as: brutal-search crawl https://example.com --index .brutal-index --max-pages 1000"
    );

    let index_dir = index_dir.as_ref();
    if index_dir.exists() {
        if options.overwrite {
            remove_index_artifacts(index_dir)?;
        } else if index_artifacts_exist(index_dir) {
            bail!("index directory already exists: {}", index_dir.display());
        }
    } else {
        fs::create_dir_all(index_dir)?;
    }
    fs::create_dir_all(index_dir)?;

    let mut texts = BufWriter::new(File::create(index_dir.join(TEXTS))?);
    let mut doc_metas = Vec::with_capacity(docs.len());
    let mut postings: FxHashMap<String, Vec<Posting>> = FxHashMap::default();
    let mut text_offset = 0u64;
    let mut total_terms = 0u64;
    let mut hasher = blake3::Hasher::new();

    let duplicate_of = duplicate_representatives(&docs);
    let duplicate_counts = duplicate_counts(&duplicate_of);
    let duplicate_cluster_count = duplicate_cluster_count(&duplicate_of, &duplicate_counts);
    let duplicate_doc_count = duplicate_doc_count(&duplicate_of);
    let authority_scores = link_authority_scores(&docs);
    let max_authority_score = authority_scores.iter().copied().fold(0.0f32, f32::max);

    for (doc_id, doc) in docs.into_iter().enumerate() {
        let doc_id = doc_id as u32;
        let text_bytes = doc.stored_text.as_bytes();
        let mut terms: FxHashMap<String, DocTerm> = FxHashMap::default();
        let mut term_count = 0u32;

        for field_text in &doc.fields {
            collect_field_terms(field_text, &mut terms, &mut term_count);
        }

        for (term, doc_term) in terms {
            postings.entry(term).or_default().push(Posting {
                doc_id,
                tf: doc_term.tf,
                field_tfs: doc_term.field_tfs,
                positions: doc_term.positions,
            });
        }

        texts.write_all(text_bytes)?;
        hasher.update(doc.url.as_bytes());
        hasher.update(&[0]);
        if let Some(language) = &doc.language {
            hasher.update(language.as_bytes());
        }
        hasher.update(&[0]);
        if let Some(fetched_at_unix) = doc.fetched_at_unix {
            hasher.update(&fetched_at_unix.to_le_bytes());
        }
        hasher.update(&[0]);
        for outbound_link in &doc.outbound_links {
            hasher.update(outbound_link.as_bytes());
            hasher.update(&[0]);
        }
        hasher.update(text_bytes);
        hasher.update(&[0]);

        doc_metas.push(DocMeta {
            id: doc_id,
            url: doc.url,
            canonical_url: doc.canonical_url,
            title: doc.title,
            language: doc.language,
            fetched_at_unix: doc.fetched_at_unix,
            content_hash: doc.content_hash,
            quality: doc.quality,
            duplicate_of: duplicate_of[doc_id as usize],
            duplicate_count: duplicate_counts[duplicate_of[doc_id as usize] as usize],
            authority_score: authority_scores[doc_id as usize],
            text_start: text_offset,
            text_len: text_bytes.len() as u32,
            term_count,
        });

        text_offset += text_bytes.len() as u64;
        total_terms += term_count as u64;
    }
    texts.flush()?;

    let mut term_entries = Vec::with_capacity(postings.len());
    let mut postings_file = BufWriter::new(File::create(index_dir.join(POSTINGS))?);
    let mut postings_offset = 0u64;
    let mut terms: Vec<_> = postings.into_iter().collect();
    terms.sort_by(|a, b| a.0.cmp(&b.0));

    for (term, mut term_postings) in terms {
        term_postings.sort_by_key(|posting| posting.doc_id);
        let encoded = encode_postings(&term_postings);
        let collection_freq = term_postings.iter().map(|posting| posting.tf).sum();
        let max_tf = term_postings
            .iter()
            .map(|posting| posting.tf)
            .max()
            .unwrap_or(0);
        let postings_len = encoded.len() as u64;
        postings_file.write_all(&encoded)?;

        term_entries.push(TermEntry {
            term,
            postings_start: postings_offset,
            postings_len,
            doc_freq: term_postings.len() as u32,
            collection_freq,
            max_tf,
        });
        postings_offset += postings_len;
    }
    postings_file.flush()?;

    fs::write(index_dir.join(DOCS), bincode::serialize(&doc_metas)?)?;
    fs::write(index_dir.join(LEXICON), bincode::serialize(&term_entries)?)?;

    let avg_doc_len = if doc_metas.is_empty() {
        0.0
    } else {
        total_terms as f32 / doc_metas.len() as f32
    };
    let corpus_hash = hasher.finalize().to_hex().to_string();
    let manifest = IndexManifest {
        format_version: FORMAT_VERSION,
        created_unix_seconds: SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs(),
        doc_count: doc_metas.len() as u32,
        term_count: term_entries.len() as u32,
        total_terms,
        avg_doc_len,
        duplicate_cluster_count,
        duplicate_doc_count,
        skipped_noindex_count: quality_stats.skipped_noindex,
        skipped_thin_count: quality_stats.skipped_thin,
        max_authority_score,
        corpus_hash: corpus_hash.clone(),
    };
    fs::write(
        index_dir.join(MANIFEST),
        serde_json::to_vec_pretty(&manifest)?,
    )?;

    Ok(BuildStats {
        doc_count: manifest.doc_count,
        term_count: manifest.term_count,
        total_terms: manifest.total_terms,
        avg_doc_len: manifest.avg_doc_len,
        duplicate_cluster_count,
        duplicate_doc_count,
        skipped_noindex_count: quality_stats.skipped_noindex,
        skipped_thin_count: quality_stats.skipped_thin,
        max_authority_score,
        corpus_hash,
    })
}

fn remove_index_artifacts(index_dir: &Path) -> Result<()> {
    for file_name in [MANIFEST, DOCS, FIELD_DOCS, LEXICON, POSTINGS, TEXTS] {
        let path = index_dir.join(file_name);
        match fs::remove_file(&path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error).with_context(|| format!("remove {}", path.display())),
        }
    }
    Ok(())
}

fn index_artifacts_exist(index_dir: &Path) -> bool {
    [MANIFEST, DOCS, FIELD_DOCS, LEXICON, POSTINGS, TEXTS]
        .iter()
        .any(|file_name| index_dir.join(file_name).exists())
}

fn index_input_from_search(doc: SearchDocument) -> IndexDocumentInput {
    let fields = vec![
        IndexFieldText {
            field: IndexedField::Url,
            text: doc.url.clone(),
        },
        IndexFieldText {
            field: IndexedField::Title,
            text: doc.title.clone(),
        },
        IndexFieldText {
            field: IndexedField::Body,
            text: doc.text.clone(),
        },
    ];

    IndexDocumentInput {
        url: doc.url,
        canonical_url: None,
        title: doc.title,
        language: None,
        fetched_at_unix: None,
        content_hash: None,
        quality: DocumentQuality::for_body(&doc.text, false),
        stored_text: doc.text,
        fields,
        outbound_links: Vec::new(),
    }
}

fn index_input_from_fielded(doc: &FieldedDocument) -> IndexDocumentInput {
    let mut fields = vec![
        IndexFieldText {
            field: IndexedField::Url,
            text: doc.url.clone(),
        },
        IndexFieldText {
            field: IndexedField::Title,
            text: doc.title.clone(),
        },
        IndexFieldText {
            field: IndexedField::Body,
            text: doc.body.clone(),
        },
    ];

    if let Some(canonical_url) = &doc.canonical_url {
        fields.push(IndexFieldText {
            field: IndexedField::Url,
            text: canonical_url.clone(),
        });
    }
    if let Some(meta_description) = &doc.meta_description {
        fields.push(IndexFieldText {
            field: IndexedField::Meta,
            text: meta_description.clone(),
        });
    }
    for heading in &doc.headings {
        fields.push(IndexFieldText {
            field: IndexedField::Heading,
            text: heading.clone(),
        });
    }
    for anchor in &doc.anchor_text {
        fields.push(IndexFieldText {
            field: IndexedField::Anchor,
            text: anchor.clone(),
        });
    }

    IndexDocumentInput {
        url: doc.url.clone(),
        canonical_url: doc.canonical_url.clone(),
        title: doc.title.clone(),
        language: doc.language.clone(),
        fetched_at_unix: doc.fetched_at_unix,
        content_hash: doc.content_hash.clone(),
        quality: doc.quality.clone(),
        stored_text: doc.body.clone(),
        fields,
        outbound_links: doc.outbound_links.clone(),
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct QualityFilterStats {
    skipped_noindex: u32,
    skipped_thin: u32,
}

fn filter_fielded_documents(
    docs: Vec<FieldedDocument>,
    options: &IndexBuildOptions,
) -> (Vec<FieldedDocument>, QualityFilterStats) {
    let mut stats = QualityFilterStats::default();
    let docs = docs
        .into_iter()
        .filter(|doc| document_is_indexable(&doc.quality, options, &mut stats))
        .collect();
    (docs, stats)
}

fn filter_index_documents(
    docs: Vec<IndexDocumentInput>,
    options: &IndexBuildOptions,
) -> (Vec<IndexDocumentInput>, QualityFilterStats) {
    let mut stats = QualityFilterStats::default();
    let docs = docs
        .into_iter()
        .filter(|doc| document_is_indexable(&doc.quality, options, &mut stats))
        .collect();
    (docs, stats)
}

fn document_is_indexable(
    quality: &DocumentQuality,
    options: &IndexBuildOptions,
    stats: &mut QualityFilterStats,
) -> bool {
    if options.respect_noindex && quality.robots_noindex {
        stats.skipped_noindex += 1;
        return false;
    }
    if options.min_body_terms > 0 && quality.body_term_count < options.min_body_terms {
        stats.skipped_thin += 1;
        return false;
    }
    true
}

fn link_authority_scores(docs: &[IndexDocumentInput]) -> Vec<f32> {
    let doc_count = docs.len();
    if doc_count == 0 {
        return Vec::new();
    }
    if doc_count == 1 {
        return vec![1.0];
    }

    let mut url_to_doc = FxHashMap::default();
    for (doc_id, doc) in docs.iter().enumerate() {
        if let Some(key) = link_key(&doc.url) {
            url_to_doc.entry(key).or_insert(doc_id);
        }
        if let Some(canonical_url) = &doc.canonical_url
            && let Some(key) = link_key(canonical_url)
        {
            url_to_doc.entry(key).or_insert(doc_id);
        }
    }

    let mut edges = vec![Vec::<usize>::new(); doc_count];
    for (doc_id, doc) in docs.iter().enumerate() {
        let mut seen_targets = FxHashSet::default();
        for outbound_link in &doc.outbound_links {
            let Some(key) = link_key(outbound_link) else {
                continue;
            };
            let Some(&target_doc_id) = url_to_doc.get(&key) else {
                continue;
            };
            if target_doc_id == doc_id || !seen_targets.insert(target_doc_id) {
                continue;
            }
            edges[doc_id].push(target_doc_id);
        }
    }

    let damping = 0.85f32;
    let doc_count_f = doc_count as f32;
    let mut rank = vec![1.0 / doc_count_f; doc_count];
    for _ in 0..20 {
        let dangling_rank = rank
            .iter()
            .zip(&edges)
            .filter(|(_, targets)| targets.is_empty())
            .map(|(score, _)| *score)
            .sum::<f32>();
        let mut next =
            vec![(1.0 - damping) / doc_count_f + damping * dangling_rank / doc_count_f; doc_count];

        for (source_doc_id, targets) in edges.iter().enumerate() {
            if targets.is_empty() {
                continue;
            }
            let contribution = damping * rank[source_doc_id] / targets.len() as f32;
            for &target_doc_id in targets {
                next[target_doc_id] += contribution;
            }
        }
        rank = next;
    }

    let max_rank = rank.iter().copied().fold(0.0f32, f32::max);
    if max_rank > 0.0 {
        for score in &mut rank {
            *score /= max_rank;
        }
    }
    rank
}

fn link_key(url: &str) -> Option<String> {
    let url = url.trim();
    if url.is_empty() {
        return None;
    }
    if let Ok(mut parsed) = Url::parse(url) {
        parsed.set_fragment(None);
        return Some(parsed.to_string());
    }
    Some(url.split('#').next().unwrap_or(url).to_owned())
}

fn duplicate_representatives(docs: &[IndexDocumentInput]) -> Vec<u32> {
    let mut exact_representatives = FxHashMap::default();
    let mut near_buckets = FxHashMap::<u64, Vec<NearDuplicateCandidate>>::default();
    let mut duplicate_of = Vec::with_capacity(docs.len());

    for (doc_id, doc) in docs.iter().enumerate() {
        let exact_keys = duplicate_keys(doc);
        let signature = near_duplicate_signature(doc);
        let mut representative = exact_keys
            .iter()
            .find_map(|key| exact_representatives.get(key).copied());

        if representative.is_none()
            && let Some(signature) = signature
        {
            representative = find_near_duplicate_representative(signature, &near_buckets);
        }

        let representative = representative.unwrap_or(doc_id as u32);
        duplicate_of.push(representative);

        for key in exact_keys {
            exact_representatives.entry(key).or_insert(representative);
        }

        if representative == doc_id as u32
            && let Some(signature) = signature
        {
            let candidate = NearDuplicateCandidate {
                doc_id: doc_id as u32,
                signature,
            };
            for bucket in simhash_buckets(signature.simhash) {
                near_buckets.entry(bucket).or_default().push(candidate);
            }
        }
    }

    duplicate_of
}

fn duplicate_counts(duplicate_of: &[u32]) -> Vec<u32> {
    let mut counts = vec![0u32; duplicate_of.len()];
    for &representative in duplicate_of {
        if let Some(count) = counts.get_mut(representative as usize) {
            *count += 1;
        }
    }

    duplicate_of
        .iter()
        .map(|&representative| counts[representative as usize])
        .collect()
}

fn duplicate_cluster_count(duplicate_of: &[u32], duplicate_counts: &[u32]) -> u32 {
    duplicate_counts
        .iter()
        .enumerate()
        .filter(|(doc_id, count)| duplicate_of[*doc_id] == *doc_id as u32 && **count > 1)
        .count() as u32
}

fn duplicate_doc_count(duplicate_of: &[u32]) -> u32 {
    duplicate_of
        .iter()
        .enumerate()
        .filter(|(doc_id, representative)| **representative != *doc_id as u32)
        .count() as u32
}

fn duplicate_keys(doc: &IndexDocumentInput) -> Vec<String> {
    let mut keys = Vec::new();

    if let Some(canonical_url) = doc.canonical_url.as_deref().map(str::trim)
        && !canonical_url.is_empty()
    {
        keys.push(format!("canonical:{canonical_url}"));
    }

    let normalized = normalized_duplicate_text(&doc.stored_text);
    if normalized.len() >= 32 {
        let hash = blake3::hash(normalized.as_bytes()).to_hex().to_string();
        keys.push(format!("text:{hash}"));
    }

    keys
}

fn normalized_duplicate_text(text: &str) -> String {
    let mut normalized = String::with_capacity(text.len().min(4096));
    let mut last_was_space = true;

    for ch in text.chars().flat_map(char::to_lowercase) {
        if ch.is_whitespace() {
            if !last_was_space {
                normalized.push(' ');
                last_was_space = true;
            }
        } else {
            normalized.push(ch);
            last_was_space = false;
        }
    }

    if normalized.ends_with(' ') {
        normalized.pop();
    }

    normalized
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct NearDuplicateSignature {
    simhash: u64,
    term_count: usize,
}

#[derive(Debug, Clone, Copy)]
struct NearDuplicateCandidate {
    doc_id: u32,
    signature: NearDuplicateSignature,
}

fn near_duplicate_signature(doc: &IndexDocumentInput) -> Option<NearDuplicateSignature> {
    let terms = duplicate_terms(&doc.stored_text);
    if terms.len() < NEAR_DUPLICATE_MIN_TERMS {
        return None;
    }

    Some(NearDuplicateSignature {
        simhash: shingled_simhash(&terms, SIMHASH_SHINGLE_TERMS),
        term_count: terms.len(),
    })
}

fn duplicate_terms(text: &str) -> Vec<String> {
    let mut terms = Vec::new();
    for_each_term(text, |term, _| terms.push(term.to_owned()));
    terms
}

fn shingled_simhash(terms: &[String], shingle_terms: usize) -> u64 {
    let shingle_terms = shingle_terms.max(1).min(terms.len());
    let mut weights = [0i32; 64];

    for shingle in terms.windows(shingle_terms) {
        let mut hasher = blake3::Hasher::new();
        for term in shingle {
            hasher.update(term.as_bytes());
            hasher.update(&[0]);
        }
        let digest = hasher.finalize();
        let hash = u64::from_le_bytes(digest.as_bytes()[..8].try_into().unwrap_or([0; 8]));
        for (bit, weight) in weights.iter_mut().enumerate() {
            if hash & (1u64 << bit) == 0 {
                *weight -= 1;
            } else {
                *weight += 1;
            }
        }
    }

    weights
        .iter()
        .enumerate()
        .fold(0u64, |mut simhash, (bit, weight)| {
            if *weight >= 0 {
                simhash |= 1u64 << bit;
            }
            simhash
        })
}

fn find_near_duplicate_representative(
    signature: NearDuplicateSignature,
    buckets: &FxHashMap<u64, Vec<NearDuplicateCandidate>>,
) -> Option<u32> {
    let mut best = None::<NearDuplicateCandidate>;

    for bucket in simhash_buckets(signature.simhash) {
        let Some(candidates) = buckets.get(&bucket) else {
            continue;
        };
        for &candidate in candidates {
            if !near_duplicate_matches(signature, candidate.signature) {
                continue;
            }
            if best.is_none_or(|best| candidate.doc_id < best.doc_id) {
                best = Some(candidate);
            }
        }
    }

    best.map(|candidate| candidate.doc_id)
}

fn near_duplicate_matches(left: NearDuplicateSignature, right: NearDuplicateSignature) -> bool {
    let min_terms = left.term_count.min(right.term_count);
    let max_terms = left.term_count.max(right.term_count).max(1);
    min_terms * 100 >= max_terms * 70
        && (left.simhash ^ right.simhash).count_ones() <= SIMHASH_HAMMING_THRESHOLD
}

fn simhash_buckets(simhash: u64) -> [u64; 16] {
    std::array::from_fn(|bucket| {
        let nibble = (simhash >> (bucket * 4)) & 0x0f;
        ((bucket as u64) << 4) | nibble
    })
}

fn collect_field_terms(
    field_text: &IndexFieldText,
    terms: &mut FxHashMap<String, DocTerm>,
    term_count: &mut u32,
) {
    for_each_term(&field_text.text, |term, pos| {
        *term_count += 1;
        let entry = terms.entry(term.to_owned()).or_insert_with(|| DocTerm {
            tf: 0,
            field_tfs: FieldTfs::default(),
            positions: Vec::new(),
        });
        entry.tf += 1;
        entry.field_tfs.add(field_text.field);
        if matches!(field_text.field, IndexedField::Body)
            && entry.positions.len() < MAX_POSITIONS_PER_DOC_TERM
        {
            entry.positions.push(pos);
        }
    });
}

fn encode_postings(postings: &[Posting]) -> Vec<u8> {
    let mut bytes = Vec::new();
    let mut prev_doc = 0u32;

    for posting in postings {
        put_u32(posting.doc_id - prev_doc, &mut bytes);
        put_u32(posting.tf, &mut bytes);
        put_u32(posting.field_tfs.title, &mut bytes);
        put_u32(posting.field_tfs.meta, &mut bytes);
        put_u32(posting.field_tfs.heading, &mut bytes);
        put_u32(posting.field_tfs.body, &mut bytes);
        put_u32(posting.field_tfs.anchor, &mut bytes);
        put_u32(posting.field_tfs.url, &mut bytes);
        put_u32(posting.positions.len() as u32, &mut bytes);

        let mut prev_pos = 0u32;
        for &pos in &posting.positions {
            put_u32(pos - prev_pos, &mut bytes);
            prev_pos = pos;
        }

        prev_doc = posting.doc_id;
    }

    bytes
}

fn decode_postings_bytes(bytes: &[u8]) -> Result<Vec<Posting>> {
    let mut postings = Vec::new();
    let mut cursor = 0usize;
    let mut prev_doc = 0u32;

    while cursor < bytes.len() {
        let doc_delta = read_u32(bytes, &mut cursor)?;
        let doc_id = prev_doc + doc_delta;
        let tf = read_u32(bytes, &mut cursor)?;
        let field_tfs = FieldTfs {
            title: read_u32(bytes, &mut cursor)?,
            meta: read_u32(bytes, &mut cursor)?,
            heading: read_u32(bytes, &mut cursor)?,
            body: read_u32(bytes, &mut cursor)?,
            anchor: read_u32(bytes, &mut cursor)?,
            url: read_u32(bytes, &mut cursor)?,
        };
        let position_len = read_u32(bytes, &mut cursor)? as usize;
        let mut positions = Vec::with_capacity(position_len);
        let mut prev_pos = 0u32;

        for _ in 0..position_len {
            let pos_delta = read_u32(bytes, &mut cursor)?;
            let pos = prev_pos + pos_delta;
            positions.push(pos);
            prev_pos = pos;
        }

        postings.push(Posting {
            doc_id,
            tf,
            field_tfs,
            positions,
        });
        prev_doc = doc_id;
    }

    Ok(postings)
}

fn is_htmlish(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| {
            matches!(
                ext.to_ascii_lowercase().as_str(),
                "html" | "htm" | "xhtml" | "txt"
            )
        })
        .unwrap_or(false)
}

fn looks_like_html(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| matches!(ext.to_ascii_lowercase().as_str(), "html" | "htm" | "xhtml"))
        .unwrap_or(false)
}

fn file_url(path: &Path) -> String {
    match path.canonicalize() {
        Ok(path) => url::Url::from_file_path(&path)
            .map(|url| url.to_string())
            .unwrap_or_else(|_| format!("file://{}", path.display())),
        Err(_) => format!("file://{}", path.display()),
    }
}

impl PartialEq for SearchIndex {
    fn eq(&self, other: &Self) -> bool {
        self.root == other.root && self.manifest.corpus_hash == other.manifest.corpus_hash
    }
}

impl Eq for SearchIndex {}

impl PartialOrd for SearchIndex {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.root.cmp(&other.root))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn postings_round_trip() {
        let postings = vec![
            Posting {
                doc_id: 0,
                tf: 2,
                field_tfs: FieldTfs {
                    body: 2,
                    ..FieldTfs::default()
                },
                positions: vec![4, 20],
            },
            Posting {
                doc_id: 3,
                tf: 1,
                field_tfs: FieldTfs {
                    title: 1,
                    ..FieldTfs::default()
                },
                positions: vec![9],
            },
        ];
        let bytes = encode_postings(&postings);
        assert_eq!(decode_postings_bytes(&bytes).unwrap(), postings);
    }

    #[test]
    fn builds_and_searches_small_index() {
        let dir = tempfile::tempdir().unwrap();
        build_from_documents(
            vec![
                RawDocument {
                    url: "mem://1".to_owned(),
                    title: "One".to_owned(),
                    text: "fast rust search fast".to_owned(),
                },
                RawDocument {
                    url: "mem://2".to_owned(),
                    title: "Two".to_owned(),
                    text: "slow browser layout".to_owned(),
                },
            ],
            dir.path(),
            IndexBuildOptions::default(),
        )
        .unwrap();

        let index = SearchIndex::open(dir.path(), PreloadMode::Aggressive).unwrap();
        let results = index
            .search("fast search", SearchOptions { limit: 5 })
            .unwrap();
        assert_eq!(results[0].doc_id, 0);
        assert!(results[0].snippet.contains("fast"));
    }

    #[test]
    fn suggests_terms_by_prefix_and_frequency() {
        let dir = tempfile::tempdir().unwrap();
        build_from_documents(
            vec![
                RawDocument {
                    url: "mem://1".to_owned(),
                    title: "One".to_owned(),
                    text: "fast faster fast".to_owned(),
                },
                RawDocument {
                    url: "mem://2".to_owned(),
                    title: "Two".to_owned(),
                    text: "faster face".to_owned(),
                },
            ],
            dir.path(),
            IndexBuildOptions::default(),
        )
        .unwrap();

        let index = SearchIndex::open(dir.path(), PreloadMode::Lazy).unwrap();
        let suggestions = index.suggest("fa", 3);
        assert_eq!(
            suggestions
                .iter()
                .map(|suggestion| suggestion.term.as_str())
                .collect::<Vec<_>>(),
            vec!["faster", "fast", "face"]
        );

        let suggestions = index.suggest("query faster", 1);
        assert_eq!(suggestions[0].term, "faster");
    }

    #[test]
    fn spellcheck_returns_near_terms_by_distance_then_frequency() {
        let dir = tempfile::tempdir().unwrap();
        build_from_documents(
            vec![
                RawDocument {
                    url: "mem://1".to_owned(),
                    title: "One".to_owned(),
                    text: "example example search".to_owned(),
                },
                RawDocument {
                    url: "mem://2".to_owned(),
                    title: "Two".to_owned(),
                    text: "samples search".to_owned(),
                },
            ],
            dir.path(),
            IndexBuildOptions::default(),
        )
        .unwrap();

        let index = SearchIndex::open(dir.path(), PreloadMode::Lazy).unwrap();
        let corrections = index.spellcheck("exampel", 3);
        assert_eq!(corrections[0].term, "example");
        assert_eq!(corrections[0].distance, 2);
        assert!(index.spellcheck("example", 3).is_empty());
    }

    #[test]
    fn builds_index_with_fielded_document_metadata() {
        let dir = tempfile::tempdir().unwrap();
        build_from_fielded_documents(
            vec![FieldedDocument {
                url: "https://example.com".to_owned(),
                canonical_url: Some("https://example.com/".to_owned()),
                title: "Fielded".to_owned(),
                meta_description: Some("Search metadata".to_owned()),
                language: Some("en".to_owned()),
                quality: DocumentQuality::default(),
                headings: vec!["Fast Heading".to_owned()],
                body: "body text".to_owned(),
                anchor_text: vec!["anchor signal".to_owned()],
                outbound_links: vec!["https://example.com/next".to_owned()],
                content_hash: Some("hash".to_owned()),
                fetched_at_unix: Some(123),
                extraction_mode: crate::document::ExtractionMode::StaticHtml,
            }],
            dir.path(),
            IndexBuildOptions::default(),
        )
        .unwrap();

        let index = SearchIndex::open(dir.path(), PreloadMode::Lazy).unwrap();
        let field_doc = index.field_doc(0).unwrap();
        assert_eq!(field_doc.language.as_deref(), Some("en"));
        assert_eq!(field_doc.headings, vec!["Fast Heading"]);
        assert_eq!(field_doc.anchor_text, vec!["anchor signal"]);
        let doc_meta = index.doc(0).unwrap();
        assert_eq!(doc_meta.language.as_deref(), Some("en"));
        assert_eq!(doc_meta.fetched_at_unix, Some(123));

        let results = index
            .search("metadata heading anchor", SearchOptions { limit: 5 })
            .unwrap();
        assert_eq!(results[0].doc_id, 0);
    }

    #[test]
    fn build_skips_noindex_documents_by_default() {
        let dir = tempfile::tempdir().unwrap();
        build_from_fielded_documents(
            vec![
                FieldedDocument {
                    url: "https://example.com/noindex".to_owned(),
                    canonical_url: None,
                    title: "Noindex".to_owned(),
                    meta_description: None,
                    language: None,
                    quality: DocumentQuality::for_body("needle hidden body", true),
                    headings: Vec::new(),
                    body: "needle hidden body".to_owned(),
                    anchor_text: Vec::new(),
                    outbound_links: Vec::new(),
                    content_hash: None,
                    fetched_at_unix: None,
                    extraction_mode: crate::document::ExtractionMode::StaticHtml,
                },
                FieldedDocument {
                    url: "https://example.com/indexed".to_owned(),
                    canonical_url: None,
                    title: "Indexed".to_owned(),
                    meta_description: None,
                    language: None,
                    quality: DocumentQuality::for_body("needle visible body", false),
                    headings: Vec::new(),
                    body: "needle visible body".to_owned(),
                    anchor_text: Vec::new(),
                    outbound_links: Vec::new(),
                    content_hash: None,
                    fetched_at_unix: None,
                    extraction_mode: crate::document::ExtractionMode::StaticHtml,
                },
            ],
            dir.path(),
            IndexBuildOptions::default(),
        )
        .unwrap();

        let index = SearchIndex::open(dir.path(), PreloadMode::Lazy).unwrap();
        assert_eq!(index.manifest().doc_count, 1);
        assert_eq!(index.manifest().skipped_noindex_count, 1);
        assert_eq!(index.doc(0).unwrap().url, "https://example.com/indexed");
        let results = index.search("needle", SearchOptions { limit: 10 }).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].url, "https://example.com/indexed");
    }

    #[test]
    fn build_can_skip_thin_documents_when_configured() {
        let dir = tempfile::tempdir().unwrap();
        build_from_documents(
            vec![
                RawDocument {
                    url: "mem://thin".to_owned(),
                    title: "Thin".to_owned(),
                    text: "needle tiny".to_owned(),
                },
                RawDocument {
                    url: "mem://substantial".to_owned(),
                    title: "Substantial".to_owned(),
                    text: "needle alpha beta gamma delta epsilon zeta eta".to_owned(),
                },
            ],
            dir.path(),
            IndexBuildOptions {
                min_body_terms: 5,
                ..IndexBuildOptions::default()
            },
        )
        .unwrap();

        let index = SearchIndex::open(dir.path(), PreloadMode::Lazy).unwrap();
        assert_eq!(index.manifest().doc_count, 1);
        assert_eq!(index.manifest().skipped_thin_count, 1);
        assert_eq!(index.doc(0).unwrap().url, "mem://substantial");
    }

    #[test]
    fn fielded_ranking_boosts_title_over_body_repetition() {
        let dir = tempfile::tempdir().unwrap();
        build_from_fielded_documents(
            vec![
                FieldedDocument {
                    url: "https://example.com/title".to_owned(),
                    canonical_url: None,
                    title: "needle".to_owned(),
                    meta_description: None,
                    language: None,
                    quality: DocumentQuality::default(),
                    headings: Vec::new(),
                    body: "plain body".to_owned(),
                    anchor_text: Vec::new(),
                    outbound_links: Vec::new(),
                    content_hash: None,
                    fetched_at_unix: None,
                    extraction_mode: crate::document::ExtractionMode::StaticHtml,
                },
                FieldedDocument {
                    url: "https://example.com/body".to_owned(),
                    canonical_url: None,
                    title: "body only".to_owned(),
                    meta_description: None,
                    language: None,
                    quality: DocumentQuality::default(),
                    headings: Vec::new(),
                    body: "needle needle".to_owned(),
                    anchor_text: Vec::new(),
                    outbound_links: Vec::new(),
                    content_hash: None,
                    fetched_at_unix: None,
                    extraction_mode: crate::document::ExtractionMode::StaticHtml,
                },
            ],
            dir.path(),
            IndexBuildOptions::default(),
        )
        .unwrap();

        let index = SearchIndex::open(dir.path(), PreloadMode::Lazy).unwrap();
        let results = index.search("needle", SearchOptions { limit: 2 }).unwrap();
        assert_eq!(results[0].url, "https://example.com/title");
        assert_eq!(index.text(results[0].doc_id), Some("plain body"));
    }

    #[test]
    fn link_authority_boosts_otherwise_equal_results() {
        let dir = tempfile::tempdir().unwrap();
        build_from_fielded_documents(
            vec![
                FieldedDocument {
                    url: "https://example.com/source-a".to_owned(),
                    canonical_url: None,
                    title: "source a".to_owned(),
                    meta_description: None,
                    language: None,
                    quality: DocumentQuality::default(),
                    headings: Vec::new(),
                    body: "source page".to_owned(),
                    anchor_text: Vec::new(),
                    outbound_links: vec!["https://example.com/linked".to_owned()],
                    content_hash: None,
                    fetched_at_unix: None,
                    extraction_mode: crate::document::ExtractionMode::StaticHtml,
                },
                FieldedDocument {
                    url: "https://example.com/linked".to_owned(),
                    canonical_url: None,
                    title: "candidate".to_owned(),
                    meta_description: None,
                    language: None,
                    quality: DocumentQuality::default(),
                    headings: Vec::new(),
                    body: "needle common body".to_owned(),
                    anchor_text: Vec::new(),
                    outbound_links: Vec::new(),
                    content_hash: None,
                    fetched_at_unix: None,
                    extraction_mode: crate::document::ExtractionMode::StaticHtml,
                },
                FieldedDocument {
                    url: "https://example.com/source-b".to_owned(),
                    canonical_url: None,
                    title: "source b".to_owned(),
                    meta_description: None,
                    language: None,
                    quality: DocumentQuality::default(),
                    headings: Vec::new(),
                    body: "source page".to_owned(),
                    anchor_text: Vec::new(),
                    outbound_links: vec!["https://example.com/linked".to_owned()],
                    content_hash: None,
                    fetched_at_unix: None,
                    extraction_mode: crate::document::ExtractionMode::StaticHtml,
                },
                FieldedDocument {
                    url: "https://example.com/unlinked".to_owned(),
                    canonical_url: None,
                    title: "candidate".to_owned(),
                    meta_description: None,
                    language: None,
                    quality: DocumentQuality::default(),
                    headings: Vec::new(),
                    body: "needle common body".to_owned(),
                    anchor_text: Vec::new(),
                    outbound_links: Vec::new(),
                    content_hash: None,
                    fetched_at_unix: None,
                    extraction_mode: crate::document::ExtractionMode::StaticHtml,
                },
            ],
            dir.path(),
            IndexBuildOptions::default(),
        )
        .unwrap();

        let index = SearchIndex::open(dir.path(), PreloadMode::Lazy).unwrap();
        let linked = index.doc(1).unwrap();
        let unlinked = index.doc(3).unwrap();
        assert!(linked.authority_score > unlinked.authority_score);
        assert_eq!(index.manifest().max_authority_score, 1.0);

        let results = index.search("needle", SearchOptions { limit: 2 }).unwrap();
        assert_eq!(results[0].url, "https://example.com/linked");
        assert_eq!(results[0].authority_score, linked.authority_score);
    }

    #[test]
    fn search_supports_site_filetype_and_negative_filters() {
        let dir = tempfile::tempdir().unwrap();
        build_from_fielded_documents(
            vec![
                FieldedDocument {
                    url: "https://example.com/docs/rust.html".to_owned(),
                    canonical_url: None,
                    title: "Rust guide".to_owned(),
                    meta_description: None,
                    language: None,
                    quality: DocumentQuality::default(),
                    headings: Vec::new(),
                    body: "needle stable rust guide".to_owned(),
                    anchor_text: Vec::new(),
                    outbound_links: Vec::new(),
                    content_hash: None,
                    fetched_at_unix: None,
                    extraction_mode: crate::document::ExtractionMode::StaticHtml,
                },
                FieldedDocument {
                    url: "https://blog.example.com/docs/rust.txt".to_owned(),
                    canonical_url: None,
                    title: "Rust notes".to_owned(),
                    meta_description: None,
                    language: None,
                    quality: DocumentQuality::default(),
                    headings: Vec::new(),
                    body: "needle stable rust notes".to_owned(),
                    anchor_text: Vec::new(),
                    outbound_links: Vec::new(),
                    content_hash: None,
                    fetched_at_unix: None,
                    extraction_mode: crate::document::ExtractionMode::StaticHtml,
                },
                FieldedDocument {
                    url: "https://other.test/docs/rust.html".to_owned(),
                    canonical_url: None,
                    title: "Rust elsewhere".to_owned(),
                    meta_description: None,
                    language: None,
                    quality: DocumentQuality::default(),
                    headings: Vec::new(),
                    body: "needle stable rust elsewhere".to_owned(),
                    anchor_text: Vec::new(),
                    outbound_links: Vec::new(),
                    content_hash: None,
                    fetched_at_unix: None,
                    extraction_mode: crate::document::ExtractionMode::StaticHtml,
                },
                FieldedDocument {
                    url: "https://example.com/docs/old.html".to_owned(),
                    canonical_url: None,
                    title: "Old Rust guide".to_owned(),
                    meta_description: None,
                    language: None,
                    quality: DocumentQuality::default(),
                    headings: Vec::new(),
                    body: "needle stable rust deprecated".to_owned(),
                    anchor_text: Vec::new(),
                    outbound_links: Vec::new(),
                    content_hash: None,
                    fetched_at_unix: None,
                    extraction_mode: crate::document::ExtractionMode::StaticHtml,
                },
            ],
            dir.path(),
            IndexBuildOptions::default(),
        )
        .unwrap();

        let index = SearchIndex::open(dir.path(), PreloadMode::Lazy).unwrap();
        let results = index
            .search(
                "needle site:example.com/docs filetype:html -deprecated",
                SearchOptions { limit: 10 },
            )
            .unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].url, "https://example.com/docs/rust.html");
    }

    #[test]
    fn search_supports_required_and_negative_phrases() {
        let dir = tempfile::tempdir().unwrap();
        build_from_fielded_documents(
            vec![
                FieldedDocument {
                    url: "https://example.com/body".to_owned(),
                    canonical_url: None,
                    title: "Body hit".to_owned(),
                    meta_description: None,
                    language: None,
                    quality: DocumentQuality::default(),
                    headings: Vec::new(),
                    body: "alpha brutal search beta".to_owned(),
                    anchor_text: Vec::new(),
                    outbound_links: Vec::new(),
                    content_hash: None,
                    fetched_at_unix: None,
                    extraction_mode: crate::document::ExtractionMode::StaticHtml,
                },
                FieldedDocument {
                    url: "https://example.com/reversed".to_owned(),
                    canonical_url: None,
                    title: "Reversed".to_owned(),
                    meta_description: None,
                    language: None,
                    quality: DocumentQuality::default(),
                    headings: Vec::new(),
                    body: "alpha search brutal beta".to_owned(),
                    anchor_text: Vec::new(),
                    outbound_links: Vec::new(),
                    content_hash: None,
                    fetched_at_unix: None,
                    extraction_mode: crate::document::ExtractionMode::StaticHtml,
                },
                FieldedDocument {
                    url: "https://example.com/title".to_owned(),
                    canonical_url: None,
                    title: "Brutal Search".to_owned(),
                    meta_description: None,
                    language: None,
                    quality: DocumentQuality::default(),
                    headings: Vec::new(),
                    body: "alpha beta".to_owned(),
                    anchor_text: Vec::new(),
                    outbound_links: Vec::new(),
                    content_hash: None,
                    fetched_at_unix: None,
                    extraction_mode: crate::document::ExtractionMode::StaticHtml,
                },
                FieldedDocument {
                    url: "https://example.com/excluded".to_owned(),
                    canonical_url: None,
                    title: "Excluded".to_owned(),
                    meta_description: None,
                    language: None,
                    quality: DocumentQuality::default(),
                    headings: Vec::new(),
                    body: "alpha brutal search old page beta".to_owned(),
                    anchor_text: Vec::new(),
                    outbound_links: Vec::new(),
                    content_hash: None,
                    fetched_at_unix: None,
                    extraction_mode: crate::document::ExtractionMode::StaticHtml,
                },
            ],
            dir.path(),
            IndexBuildOptions::default(),
        )
        .unwrap();

        let index = SearchIndex::open(dir.path(), PreloadMode::Lazy).unwrap();
        let results = index
            .search(
                r#""brutal search" -"old page""#,
                SearchOptions { limit: 10 },
            )
            .unwrap();
        let urls = results
            .iter()
            .map(|result| result.url.as_str())
            .collect::<Vec<_>>();
        assert!(urls.contains(&"https://example.com/body"));
        assert!(urls.contains(&"https://example.com/title"));
        assert!(!urls.contains(&"https://example.com/reversed"));
        assert!(!urls.contains(&"https://example.com/excluded"));
    }

    #[test]
    fn search_supports_language_and_freshness_filters() {
        let dir = tempfile::tempdir().unwrap();
        build_from_fielded_documents(
            vec![
                FieldedDocument {
                    url: "https://example.com/en-fresh".to_owned(),
                    canonical_url: None,
                    title: "Fresh English".to_owned(),
                    meta_description: None,
                    language: Some("en-US".to_owned()),
                    quality: DocumentQuality::default(),
                    headings: Vec::new(),
                    body: "needle freshness language match".to_owned(),
                    anchor_text: Vec::new(),
                    outbound_links: Vec::new(),
                    content_hash: None,
                    fetched_at_unix: Some(1_736_899_200),
                    extraction_mode: crate::document::ExtractionMode::StaticHtml,
                },
                FieldedDocument {
                    url: "https://example.com/fr-fresh".to_owned(),
                    canonical_url: None,
                    title: "Fresh French".to_owned(),
                    meta_description: None,
                    language: Some("fr".to_owned()),
                    quality: DocumentQuality::default(),
                    headings: Vec::new(),
                    body: "needle freshness language mismatch".to_owned(),
                    anchor_text: Vec::new(),
                    outbound_links: Vec::new(),
                    content_hash: None,
                    fetched_at_unix: Some(1_736_899_200),
                    extraction_mode: crate::document::ExtractionMode::StaticHtml,
                },
                FieldedDocument {
                    url: "https://example.com/en-old".to_owned(),
                    canonical_url: None,
                    title: "Old English".to_owned(),
                    meta_description: None,
                    language: Some("en".to_owned()),
                    quality: DocumentQuality::default(),
                    headings: Vec::new(),
                    body: "needle freshness old".to_owned(),
                    anchor_text: Vec::new(),
                    outbound_links: Vec::new(),
                    content_hash: None,
                    fetched_at_unix: Some(1_685_577_600),
                    extraction_mode: crate::document::ExtractionMode::StaticHtml,
                },
                FieldedDocument {
                    url: "https://example.com/en-undated".to_owned(),
                    canonical_url: None,
                    title: "Undated English".to_owned(),
                    meta_description: None,
                    language: Some("en".to_owned()),
                    quality: DocumentQuality::default(),
                    headings: Vec::new(),
                    body: "needle freshness undated".to_owned(),
                    anchor_text: Vec::new(),
                    outbound_links: Vec::new(),
                    content_hash: None,
                    fetched_at_unix: None,
                    extraction_mode: crate::document::ExtractionMode::StaticHtml,
                },
            ],
            dir.path(),
            IndexBuildOptions::default(),
        )
        .unwrap();

        let index = SearchIndex::open(dir.path(), PreloadMode::Lazy).unwrap();
        let results = index
            .search(
                "needle lang:en after:2025-01-01 before:2025-12-31",
                SearchOptions { limit: 10 },
            )
            .unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].url, "https://example.com/en-fresh");
        assert_eq!(results[0].language.as_deref(), Some("en-US"));
        assert_eq!(results[0].fetched_at_unix, Some(1_736_899_200));

        let results = index
            .search("needle -lang:fr", SearchOptions { limit: 10 })
            .unwrap();
        assert!(
            results
                .iter()
                .all(|result| result.url != "https://example.com/fr-fresh")
        );
    }

    #[test]
    fn search_supports_required_terms_and_or_groups() {
        let dir = tempfile::tempdir().unwrap();
        build_from_fielded_documents(
            vec![
                FieldedDocument {
                    url: "https://example.com/alpha-fast".to_owned(),
                    canonical_url: None,
                    title: "Alpha Fast".to_owned(),
                    meta_description: None,
                    language: None,
                    quality: DocumentQuality::default(),
                    headings: Vec::new(),
                    body: "alpha fast query match".to_owned(),
                    anchor_text: Vec::new(),
                    outbound_links: Vec::new(),
                    content_hash: None,
                    fetched_at_unix: None,
                    extraction_mode: crate::document::ExtractionMode::StaticHtml,
                },
                FieldedDocument {
                    url: "https://example.com/beta-fast".to_owned(),
                    canonical_url: None,
                    title: "Beta Fast".to_owned(),
                    meta_description: None,
                    language: None,
                    quality: DocumentQuality::default(),
                    headings: Vec::new(),
                    body: "beta fast query match".to_owned(),
                    anchor_text: Vec::new(),
                    outbound_links: Vec::new(),
                    content_hash: None,
                    fetched_at_unix: None,
                    extraction_mode: crate::document::ExtractionMode::StaticHtml,
                },
                FieldedDocument {
                    url: "https://example.com/alpha-slow".to_owned(),
                    canonical_url: None,
                    title: "Alpha Slow".to_owned(),
                    meta_description: None,
                    language: None,
                    quality: DocumentQuality::default(),
                    headings: Vec::new(),
                    body: "alpha slow query match".to_owned(),
                    anchor_text: Vec::new(),
                    outbound_links: Vec::new(),
                    content_hash: None,
                    fetched_at_unix: None,
                    extraction_mode: crate::document::ExtractionMode::StaticHtml,
                },
                FieldedDocument {
                    url: "https://example.com/gamma-fast".to_owned(),
                    canonical_url: None,
                    title: "Gamma Fast".to_owned(),
                    meta_description: None,
                    language: None,
                    quality: DocumentQuality::default(),
                    headings: Vec::new(),
                    body: "gamma fast query match".to_owned(),
                    anchor_text: Vec::new(),
                    outbound_links: Vec::new(),
                    content_hash: None,
                    fetched_at_unix: None,
                    extraction_mode: crate::document::ExtractionMode::StaticHtml,
                },
            ],
            dir.path(),
            IndexBuildOptions::default(),
        )
        .unwrap();

        let index = SearchIndex::open(dir.path(), PreloadMode::Lazy).unwrap();
        let results = index
            .search("+fast alpha OR beta", SearchOptions { limit: 10 })
            .unwrap();
        let urls = results
            .iter()
            .map(|result| result.url.as_str())
            .collect::<Vec<_>>();
        assert_eq!(urls.len(), 2);
        assert!(urls.contains(&"https://example.com/alpha-fast"));
        assert!(urls.contains(&"https://example.com/beta-fast"));
        assert!(!urls.contains(&"https://example.com/alpha-slow"));
        assert!(!urls.contains(&"https://example.com/gamma-fast"));
    }

    #[test]
    fn search_collapses_canonical_duplicate_clusters() {
        let dir = tempfile::tempdir().unwrap();
        build_from_fielded_documents(
            vec![
                FieldedDocument {
                    url: "https://example.com/a".to_owned(),
                    canonical_url: Some("https://example.com/canonical".to_owned()),
                    title: "needle first".to_owned(),
                    meta_description: None,
                    language: None,
                    quality: DocumentQuality::default(),
                    headings: Vec::new(),
                    body: "shared duplicate body needle".to_owned(),
                    anchor_text: Vec::new(),
                    outbound_links: Vec::new(),
                    content_hash: Some("hash-a".to_owned()),
                    fetched_at_unix: None,
                    extraction_mode: crate::document::ExtractionMode::StaticHtml,
                },
                FieldedDocument {
                    url: "https://example.com/b".to_owned(),
                    canonical_url: Some("https://example.com/canonical".to_owned()),
                    title: "needle second".to_owned(),
                    meta_description: None,
                    language: None,
                    quality: DocumentQuality::default(),
                    headings: Vec::new(),
                    body: "changed duplicate body needle".to_owned(),
                    anchor_text: Vec::new(),
                    outbound_links: Vec::new(),
                    content_hash: Some("hash-b".to_owned()),
                    fetched_at_unix: None,
                    extraction_mode: crate::document::ExtractionMode::StaticHtml,
                },
                FieldedDocument {
                    url: "https://example.com/c".to_owned(),
                    canonical_url: None,
                    title: "needle unique".to_owned(),
                    meta_description: None,
                    language: None,
                    quality: DocumentQuality::default(),
                    headings: Vec::new(),
                    body: "standalone unique body needle".to_owned(),
                    anchor_text: Vec::new(),
                    outbound_links: Vec::new(),
                    content_hash: Some("hash-c".to_owned()),
                    fetched_at_unix: None,
                    extraction_mode: crate::document::ExtractionMode::StaticHtml,
                },
            ],
            dir.path(),
            IndexBuildOptions::default(),
        )
        .unwrap();

        let index = SearchIndex::open(dir.path(), PreloadMode::Lazy).unwrap();
        let results = index.search("needle", SearchOptions { limit: 10 }).unwrap();

        assert_eq!(index.manifest().duplicate_cluster_count, 1);
        assert_eq!(index.manifest().duplicate_doc_count, 1);
        assert_eq!(results.len(), 2);
        assert!(results.iter().any(|result| result.duplicate_count == 2));
        assert!(results.iter().any(|result| result.duplicate_count == 1));
        assert_eq!(index.doc(0).unwrap().duplicate_of, 0);
        assert_eq!(index.doc(1).unwrap().duplicate_of, 0);
    }

    #[test]
    fn search_collapses_exact_text_duplicate_clusters() {
        let dir = tempfile::tempdir().unwrap();
        build_from_documents(
            vec![
                RawDocument {
                    url: "mem://one".to_owned(),
                    title: "One".to_owned(),
                    text: "Same page body with enough words to form duplicate text".to_owned(),
                },
                RawDocument {
                    url: "mem://two".to_owned(),
                    title: "Two".to_owned(),
                    text: "same   page body with enough words to form duplicate text".to_owned(),
                },
            ],
            dir.path(),
            IndexBuildOptions::default(),
        )
        .unwrap();

        let index = SearchIndex::open(dir.path(), PreloadMode::Lazy).unwrap();
        let results = index
            .search("duplicate text", SearchOptions { limit: 10 })
            .unwrap();

        assert_eq!(index.manifest().duplicate_cluster_count, 1);
        assert_eq!(index.manifest().duplicate_doc_count, 1);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].duplicate_count, 2);
    }

    #[test]
    fn search_collapses_near_duplicate_simhash_clusters() {
        let dir = tempfile::tempdir().unwrap();
        build_from_documents(
            vec![
                RawDocument {
                    url: "mem://near-one".to_owned(),
                    title: "Near One".to_owned(),
                    text: "needle alpha beta gamma delta epsilon zeta eta theta iota kappa lambda mu"
                        .to_owned(),
                },
                RawDocument {
                    url: "mem://near-two".to_owned(),
                    title: "Near Two".to_owned(),
                    text: "needle alpha beta gamma delta epsilon zeta eta theta iota kappa lambda nu"
                        .to_owned(),
                },
                RawDocument {
                    url: "mem://unique".to_owned(),
                    title: "Unique".to_owned(),
                    text: "needle orange browser runtime paint compose sandbox profile storage network"
                        .to_owned(),
                },
            ],
            dir.path(),
            IndexBuildOptions::default(),
        )
        .unwrap();

        let index = SearchIndex::open(dir.path(), PreloadMode::Lazy).unwrap();
        let results = index.search("needle", SearchOptions { limit: 10 }).unwrap();

        assert_eq!(index.manifest().duplicate_cluster_count, 1);
        assert_eq!(index.manifest().duplicate_doc_count, 1);
        assert_eq!(results.len(), 2);
        assert_eq!(index.doc(0).unwrap().duplicate_of, 0);
        assert_eq!(index.doc(1).unwrap().duplicate_of, 0);
        assert_eq!(index.doc(2).unwrap().duplicate_of, 2);
        assert!(results.iter().any(|result| result.duplicate_count == 2));
    }
}
