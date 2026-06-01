use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use anyhow::{Context, Result, bail};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};

use crate::index::{PreloadMode, SearchIndex};
use crate::protocol::{DaemonRequest, DaemonResponse};
use crate::query::SearchOptions;
use crate::render::render_target;

pub fn default_socket_path(index_dir: &Path) -> PathBuf {
    index_dir.join("brutal-searchd.sock")
}

pub async fn run_daemon(index_dir: PathBuf, socket: PathBuf, preload: PreloadMode) -> Result<()> {
    if socket.exists() {
        std::fs::remove_file(&socket)
            .with_context(|| format!("remove stale socket {}", socket.display()))?;
    }

    let index = Arc::new(SearchIndex::open(&index_dir, preload)?);
    let listener = UnixListener::bind(&socket)
        .with_context(|| format!("bind daemon socket {}", socket.display()))?;

    loop {
        let (stream, _) = listener.accept().await?;
        let index = Arc::clone(&index);
        tokio::spawn(async move {
            if let Err(error) = handle_client(stream, index).await {
                eprintln!("daemon client error: {error:#}");
            }
        });
    }
}

pub async fn send_request(socket: &Path, request: &DaemonRequest) -> Result<DaemonResponse> {
    let mut stream = UnixStream::connect(socket)
        .await
        .with_context(|| format!("connect daemon socket {}", socket.display()))?;
    let mut payload = serde_json::to_vec(request)?;
    payload.push(b'\n');
    stream.write_all(&payload).await?;
    stream.flush().await?;

    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    let bytes = reader.read_line(&mut line).await?;
    if bytes == 0 {
        bail!("daemon closed connection without a response");
    }

    Ok(serde_json::from_str(&line)?)
}

async fn handle_client(stream: UnixStream, index: Arc<SearchIndex>) -> Result<()> {
    let (reader, mut writer) = stream.into_split();
    let mut reader = BufReader::new(reader);
    let mut line = String::new();

    while reader.read_line(&mut line).await? != 0 {
        let response = match serde_json::from_str::<DaemonRequest>(&line) {
            Ok(request) => execute(&index, request),
            Err(error) => DaemonResponse::Error {
                message: error.to_string(),
            },
        };
        let mut payload = serde_json::to_vec(&response)?;
        payload.push(b'\n');
        writer.write_all(&payload).await?;
        writer.flush().await?;
        line.clear();
    }

    Ok(())
}

fn execute(index: &SearchIndex, request: DaemonRequest) -> DaemonResponse {
    match request {
        DaemonRequest::Search { query, limit } => {
            match index.search(&query, SearchOptions { limit }) {
                Ok(results) => DaemonResponse::Search { results },
                Err(error) => DaemonResponse::Error {
                    message: error.to_string(),
                },
            }
        }
        DaemonRequest::Suggest { prefix, limit } => DaemonResponse::Suggest {
            suggestions: index.suggest(&prefix, limit),
        },
        DaemonRequest::Spell { term, limit } => DaemonResponse::Spell {
            corrections: index.spellcheck(&term, limit),
        },
        DaemonRequest::Render { target } => match render_target(index, &target) {
            Ok(text) => DaemonResponse::Render { text },
            Err(error) => DaemonResponse::Error {
                message: error.to_string(),
            },
        },
        DaemonRequest::BenchSearch {
            queries,
            limit,
            warmup,
        } => bench_search(index, &queries, limit, warmup),
        DaemonRequest::Stats => {
            let manifest = index.manifest();
            DaemonResponse::Stats {
                doc_count: manifest.doc_count,
                term_count: manifest.term_count,
                total_terms: manifest.total_terms,
                avg_doc_len: manifest.avg_doc_len,
                duplicate_cluster_count: manifest.duplicate_cluster_count,
                duplicate_doc_count: manifest.duplicate_doc_count,
                skipped_noindex_count: manifest.skipped_noindex_count,
                skipped_thin_count: manifest.skipped_thin_count,
                max_authority_score: manifest.max_authority_score,
                corpus_hash: manifest.corpus_hash.clone(),
            }
        }
    }
}

fn bench_search(
    index: &SearchIndex,
    queries: &[String],
    limit: usize,
    warmup: usize,
) -> DaemonResponse {
    for query in queries.iter().take(warmup) {
        if let Err(error) = index.search(query, SearchOptions { limit }) {
            return DaemonResponse::Error {
                message: error.to_string(),
            };
        }
    }

    let started = Instant::now();
    let mut timings_us = Vec::with_capacity(queries.len());
    for query in queries {
        let t0 = Instant::now();
        match index.search(query, SearchOptions { limit }) {
            Ok(results) => {
                let _rendered_len: usize = results
                    .iter()
                    .map(|result| result.url.len() + result.title.len() + result.snippet.len())
                    .sum();
            }
            Err(error) => {
                return DaemonResponse::Error {
                    message: error.to_string(),
                };
            }
        }
        timings_us.push(t0.elapsed().as_micros().min(u64::MAX as u128) as u64);
    }

    DaemonResponse::BenchSearch {
        timings_us,
        total_us: started.elapsed().as_micros().min(u64::MAX as u128) as u64,
    }
}
