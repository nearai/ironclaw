//! Hybrid search over the filesystem-backed workspace store.
//!
//! The unified filesystem surface supports `Filter::Fts` and
//! `Filter::VectorNearest` natively on backends that declare those index
//! kinds. However, `query` returns `Vec<VersionedEntry>` without per-entry
//! paths, so we can't always recover the document path from a hit. We
//! therefore do a scan-and-rank pass over the user's chunk tree:
//!
//! 1. Read all chunks scoped by `user_id`/`agent_id`.
//! 2. For the FTS branch: rank chunks whose `content` matches the query
//!    tokens (case-insensitive whole-word containment).
//! 3. For the vector branch: rank chunks by cosine similarity against
//!    the supplied embedding.
//! 4. Fuse the two ranked lists with the workspace's
//!    [`fuse_results`](crate::workspace::fuse_results) function (RRF or
//!    weighted-score per the [`SearchConfig`]).
//!
//! This matches the legacy SQL semantics (filter by user_id/agent_id,
//! rank, fuse) without depending on backend-native FTS support. Deployments
//! that mount the libSQL/Postgres filesystem backends can later switch to
//! the native `Filter::Fts` / `Filter::VectorNearest` path; the helper is
//! kept simple so the migration shape stays portable.

use std::collections::HashMap;

use ironclaw_filesystem::RootFilesystem;
use uuid::Uuid;

use crate::error::WorkspaceError;
use crate::workspace::{RankedResult, SearchConfig, SearchResult, fuse_results};

use super::chunks::StoredChunk;
use super::{FilesystemWorkspaceStore, chunks, documents, paths};

pub(super) async fn hybrid_search<F>(
    store: &FilesystemWorkspaceStore<F>,
    user_id: &str,
    agent_id: Option<Uuid>,
    query: &str,
    embedding: Option<&[f32]>,
    config: &SearchConfig,
) -> Result<Vec<SearchResult>, WorkspaceError>
where
    F: RootFilesystem,
{
    // Collect chunks scoped to (user_id, agent_id) by walking
    // `/workspace/documents/<user_id>` and reading the chunks for each
    // owned document.
    let docs = documents::list_documents(store, user_id, agent_id).await?;
    let mut path_by_doc: HashMap<Uuid, String> = HashMap::new();
    let mut chunks: Vec<StoredChunk> = Vec::new();
    for doc in docs {
        let doc_chunks =
            read_chunks_for_doc_if_scope_matches(store, doc.id, user_id, agent_id).await?;
        if doc_chunks.is_empty() {
            continue;
        }
        path_by_doc.insert(doc.id, doc.path.clone());
        chunks.extend(doc_chunks);
    }

    let mut fts_results = Vec::new();
    if config.use_fts && !query.trim().is_empty() {
        fts_results = rank_by_fts(&chunks, query, config.pre_fusion_limit, &path_by_doc);
    }
    let mut vector_results = Vec::new();
    if config.use_vector
        && let Some(emb) = embedding
        && !emb.is_empty()
    {
        vector_results = rank_by_vector(&chunks, emb, config.pre_fusion_limit, &path_by_doc);
    }

    let fused = fuse_results(fts_results, vector_results, config);
    let filtered: Vec<SearchResult> = fused
        .into_iter()
        .filter(|r| r.score >= config.min_score)
        .take(config.limit)
        .collect();
    Ok(filtered)
}

async fn read_chunks_for_doc_if_scope_matches<F>(
    store: &FilesystemWorkspaceStore<F>,
    document_id: Uuid,
    user_id: &str,
    agent_id: Option<Uuid>,
) -> Result<Vec<StoredChunk>, WorkspaceError>
where
    F: RootFilesystem,
{
    let chunks = chunks::read_chunks_for_doc(store, document_id).await?;
    Ok(chunks
        .into_iter()
        .filter(|c| c.user_id == user_id && c.agent_id == agent_id)
        .collect())
}

fn rank_by_fts(
    chunks: &[StoredChunk],
    query: &str,
    pre_fusion_limit: usize,
    path_by_doc: &HashMap<Uuid, String>,
) -> Vec<RankedResult> {
    let needles: Vec<String> = query
        .split_whitespace()
        .map(|t| t.to_lowercase())
        .filter(|t| !t.is_empty())
        .collect();
    if needles.is_empty() {
        return Vec::new();
    }
    let mut scored: Vec<(usize, &StoredChunk)> = chunks
        .iter()
        .filter_map(|chunk| {
            let lower = chunk.content.to_lowercase();
            let mut hits = 0usize;
            for needle in &needles {
                if lower.contains(needle) {
                    hits += 1;
                }
            }
            if hits == 0 { None } else { Some((hits, chunk)) }
        })
        .collect();
    // Higher hit count = higher rank.
    scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.chunk_index.cmp(&b.1.chunk_index)));
    scored
        .into_iter()
        .take(pre_fusion_limit.max(1))
        .enumerate()
        .map(|(idx, (_hits, chunk))| RankedResult {
            chunk_id: chunk.id,
            document_id: chunk.document_id,
            document_path: path_by_doc
                .get(&chunk.document_id)
                .cloned()
                .unwrap_or_default(),
            content: chunk.content.clone(),
            rank: (idx + 1) as u32,
        })
        .collect()
}

fn rank_by_vector(
    chunks: &[StoredChunk],
    query_embedding: &[f32],
    pre_fusion_limit: usize,
    path_by_doc: &HashMap<Uuid, String>,
) -> Vec<RankedResult> {
    let query_norm = norm(query_embedding);
    if query_norm == 0.0 {
        return Vec::new();
    }
    let mut scored: Vec<(f32, &StoredChunk)> = chunks
        .iter()
        .filter_map(|chunk| {
            let emb = chunk.embedding.as_ref()?;
            if emb.len() != query_embedding.len() {
                return None;
            }
            let chunk_norm = norm(emb);
            if chunk_norm == 0.0 {
                return None;
            }
            let dot: f32 = emb
                .iter()
                .zip(query_embedding.iter())
                .map(|(a, b)| a * b)
                .sum();
            Some((dot / (chunk_norm * query_norm), chunk))
        })
        .collect();
    scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    scored
        .into_iter()
        .take(pre_fusion_limit.max(1))
        .enumerate()
        .map(|(idx, (_sim, chunk))| RankedResult {
            chunk_id: chunk.id,
            document_id: chunk.document_id,
            document_path: path_by_doc
                .get(&chunk.document_id)
                .cloned()
                .unwrap_or_default(),
            content: chunk.content.clone(),
            rank: (idx + 1) as u32,
        })
        .collect()
}

fn norm(values: &[f32]) -> f32 {
    values.iter().map(|v| v * v).sum::<f32>().sqrt()
}

// Keep the path helpers reachable from the search submodule's compilation
// unit so they don't trigger dead-code warnings when only chunks/versions
// reference them.
#[allow(dead_code)]
fn _retain_paths_used() {
    let _ = paths::agent_id_segment;
}
