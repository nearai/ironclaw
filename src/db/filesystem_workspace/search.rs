//! Hybrid search over the filesystem-backed workspace store.
//!
//! Prefers backend-native `Filter::Fts` and `Filter::VectorNearest`
//! query paths when the mounted [`RootFilesystem`] advertises
//! `Capability::IndexFts` / `Capability::IndexVector` (libSQL FTS5,
//! Postgres tsvector + brute-force cosine, the in-memory reference
//! backend). The chunk store's [`ensure_chunk_indexes`] declares the
//! FTS and Vector indexes once per process at `/workspace/chunks` so
//! the backend can serve them; subsequent calls are a cached no-op.
//!
//! When a backend cannot serve those filters and returns
//! [`FilesystemError::Unsupported`], the search falls back to a
//! scan-and-rank pass over the user's chunks: read every chunk under
//! `/workspace/chunks/<doc_id>/` for the user's owned documents,
//! score by case-insensitive whole-word containment (FTS branch) and
//! cosine similarity (vector branch), and feed those ranked lists into
//! the same fusion stage as the native path.
//!
//! Either way the fusion logic (RRF or weighted score, configured by
//! [`SearchConfig`]) is identical — only the candidate-set source
//! changes. The trait contract (`Vec<SearchResult>`) is unchanged.
//!
//! Scope filtering on `(user_id, agent_id)` runs in Rust after the
//! query for both paths. The native `Filter::VectorNearest` is a
//! top-level ranking operation on libsql/postgres, so it can't be
//! composed inside an `And` with scope predicates — the post-filter
//! is the source of truth.

use std::collections::HashMap;

use ironclaw_filesystem::{
    Filter, IndexKey, IndexValue, Page, RootFilesystem, {FilesystemError, VersionedEntry},
};
use uuid::Uuid;

use crate::error::WorkspaceError;
use crate::workspace::{RankedResult, SearchConfig, SearchResult, fuse_results};

use super::chunks::{self, StoredChunk};
use super::{FilesystemWorkspaceStore, documents, fs_to_workspace_error, paths};

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
    // Declare the chunk FTS + Vector indexes once per process. If the
    // backend declines (capability-light backend), every native branch
    // below short-circuits to the scan-and-rank fallback.
    let native_supported = *store
        .chunk_indexes_ready
        .get_or_try_init(|| async { chunks::ensure_chunk_indexes(store).await })
        .await?;

    let mut fts_results: Vec<RankedResult> = Vec::new();
    let mut vector_results: Vec<RankedResult> = Vec::new();

    // Build a doc_id -> path map lazily. The native query path returns
    // chunks without their document path attached, so we resolve any
    // ids we see through the document store. The fallback path
    // populates the same map up front while it's walking the user's
    // docs.
    let mut path_by_doc: HashMap<Uuid, String> = HashMap::new();

    let need_fts = config.use_fts && !query.trim().is_empty();
    let need_vector = config.use_vector && embedding.is_some_and(|emb| !emb.is_empty());

    let mut fallback_chunks: Option<Vec<StoredChunk>> = None;

    if need_fts {
        if native_supported {
            match run_native_fts(
                store,
                user_id,
                agent_id,
                query,
                config.pre_fusion_limit,
                &mut path_by_doc,
            )
            .await
            {
                Ok(results) => fts_results = results,
                Err(NativeSearchError::Unsupported) => {
                    let chunks = ensure_fallback_chunks(
                        store,
                        user_id,
                        agent_id,
                        &mut path_by_doc,
                        &mut fallback_chunks,
                    )
                    .await?;
                    fts_results = rank_by_fts(chunks, query, config.pre_fusion_limit, &path_by_doc);
                }
                Err(NativeSearchError::Workspace(error)) => return Err(error),
            }
        } else {
            let chunks = ensure_fallback_chunks(
                store,
                user_id,
                agent_id,
                &mut path_by_doc,
                &mut fallback_chunks,
            )
            .await?;
            fts_results = rank_by_fts(chunks, query, config.pre_fusion_limit, &path_by_doc);
        }
    }

    if need_vector {
        let emb = embedding.unwrap_or(&[]);
        if native_supported {
            match run_native_vector(
                store,
                user_id,
                agent_id,
                emb,
                config.pre_fusion_limit,
                &mut path_by_doc,
            )
            .await
            {
                Ok(results) => vector_results = results,
                Err(NativeSearchError::Unsupported) => {
                    let chunks = ensure_fallback_chunks(
                        store,
                        user_id,
                        agent_id,
                        &mut path_by_doc,
                        &mut fallback_chunks,
                    )
                    .await?;
                    vector_results =
                        rank_by_vector(chunks, emb, config.pre_fusion_limit, &path_by_doc);
                }
                Err(NativeSearchError::Workspace(error)) => return Err(error),
            }
        } else {
            let chunks = ensure_fallback_chunks(
                store,
                user_id,
                agent_id,
                &mut path_by_doc,
                &mut fallback_chunks,
            )
            .await?;
            vector_results = rank_by_vector(chunks, emb, config.pre_fusion_limit, &path_by_doc);
        }
    }

    let fused = fuse_results(fts_results, vector_results, config);
    let filtered: Vec<SearchResult> = fused
        .into_iter()
        .filter(|r| r.score >= config.min_score)
        .take(config.limit)
        .collect();
    Ok(filtered)
}

enum NativeSearchError {
    /// Backend reported the filter is not supported. Caller falls back
    /// to scan-and-rank.
    Unsupported,
    /// A non-capability backend or workspace error; propagate.
    Workspace(WorkspaceError),
}

impl From<WorkspaceError> for NativeSearchError {
    fn from(error: WorkspaceError) -> Self {
        NativeSearchError::Workspace(error)
    }
}

/// Run `filesystem.query(/workspace/chunks, Filter::Fts { content, query })`
/// and project the results into `RankedResult`s. Filters the candidate
/// set by `(user_id, agent_id)` in Rust because the libsql FTS-table
/// path predicate doesn't carry scope.
async fn run_native_fts<F>(
    store: &FilesystemWorkspaceStore<F>,
    user_id: &str,
    agent_id: Option<Uuid>,
    query: &str,
    pre_fusion_limit: usize,
    path_by_doc: &mut HashMap<Uuid, String>,
) -> Result<Vec<RankedResult>, NativeSearchError>
where
    F: RootFilesystem,
{
    let root = paths::chunks_root().map_err(NativeSearchError::Workspace)?;
    let content_key = IndexKey::new(chunks::fs_keys::CONTENT).map_err(|e| {
        NativeSearchError::Workspace(WorkspaceError::SearchFailed {
            reason: format!("content index key: {e}"),
        })
    })?;
    let scoped = scoped_filter(
        user_id,
        agent_id,
        Filter::Fts {
            key: content_key,
            query: query.to_string(),
        },
    )?;
    // FTS5 / tsvector deliver pre-ranked results in match order; we
    // ask the backend for `pre_fusion_limit` rows and treat the order
    // it returns as the FTS ranking. Scope filtering may drop some
    // rows; the surviving order is still the FTS rank.
    let page = Page::new(0, pre_fusion_limit_to_u32(pre_fusion_limit));
    let entries = match store.filesystem.query(&root, &scoped, page).await {
        Ok(entries) => entries,
        Err(FilesystemError::Unsupported { .. }) => return Err(NativeSearchError::Unsupported),
        Err(error) => return Err(NativeSearchError::Workspace(fs_to_workspace_error(error))),
    };
    Ok(project_to_ranked(store, entries, user_id, agent_id, path_by_doc).await?)
}

/// Run `filesystem.query(/workspace/chunks, Filter::VectorNearest)` and
/// project results. VectorNearest is a top-level operation on the SQL
/// backends — they ignore compound filters — so we don't try to nest
/// the scope predicate inside `And`. Scope filtering happens after.
async fn run_native_vector<F>(
    store: &FilesystemWorkspaceStore<F>,
    user_id: &str,
    agent_id: Option<Uuid>,
    embedding: &[f32],
    pre_fusion_limit: usize,
    path_by_doc: &mut HashMap<Uuid, String>,
) -> Result<Vec<RankedResult>, NativeSearchError>
where
    F: RootFilesystem,
{
    let root = paths::chunks_root().map_err(NativeSearchError::Workspace)?;
    let embedding_key = IndexKey::new(chunks::fs_keys::EMBEDDING).map_err(|e| {
        NativeSearchError::Workspace(WorkspaceError::SearchFailed {
            reason: format!("embedding index key: {e}"),
        })
    })?;
    let filter = Filter::VectorNearest {
        key: embedding_key,
        embedding: embedding.to_vec(),
        limit: pre_fusion_limit_to_u32(pre_fusion_limit),
    };
    // VectorNearest overrides the surrounding page limit — pass the
    // canonical default.
    let entries = match store
        .filesystem
        .query(&root, &filter, Page::default())
        .await
    {
        Ok(entries) => entries,
        Err(FilesystemError::Unsupported { .. }) => return Err(NativeSearchError::Unsupported),
        Err(error) => return Err(NativeSearchError::Workspace(fs_to_workspace_error(error))),
    };
    Ok(project_to_ranked(store, entries, user_id, agent_id, path_by_doc).await?)
}

/// Project backend-returned chunks into `RankedResult`s, dropping any
/// rows whose stored `(user_id, agent_id)` don't match the caller's
/// scope. The result order is preserved (1-based rank).
async fn project_to_ranked<F>(
    store: &FilesystemWorkspaceStore<F>,
    entries: Vec<VersionedEntry>,
    user_id: &str,
    agent_id: Option<Uuid>,
    path_by_doc: &mut HashMap<Uuid, String>,
) -> Result<Vec<RankedResult>, WorkspaceError>
where
    F: RootFilesystem,
{
    let mut out: Vec<RankedResult> = Vec::with_capacity(entries.len());
    for versioned in entries {
        let Ok(chunk) = serde_json::from_slice::<StoredChunk>(&versioned.entry.body) else {
            continue;
        };
        if chunk.user_id != user_id || chunk.agent_id != agent_id {
            continue;
        }
        let document_path = resolve_doc_path(store, chunk.document_id, path_by_doc).await?;
        let rank = (out.len() + 1) as u32;
        out.push(RankedResult {
            chunk_id: chunk.id,
            document_id: chunk.document_id,
            document_path,
            content: chunk.content,
            rank,
        });
    }
    Ok(out)
}

async fn resolve_doc_path<F>(
    store: &FilesystemWorkspaceStore<F>,
    document_id: Uuid,
    path_by_doc: &mut HashMap<Uuid, String>,
) -> Result<String, WorkspaceError>
where
    F: RootFilesystem,
{
    if let Some(path) = path_by_doc.get(&document_id) {
        return Ok(path.clone());
    }
    match documents::get_by_id(store, document_id).await {
        Ok(doc) => {
            path_by_doc.insert(document_id, doc.path.clone());
            Ok(doc.path)
        }
        Err(WorkspaceError::DocumentNotFound { .. }) => {
            // Chunk's document was deleted under us. The legacy SQL
            // schema joined memory_chunks -> memory_documents and
            // would have dropped the row at fetch time. Mirror that
            // by reporting an empty path.
            path_by_doc.insert(document_id, String::new());
            Ok(String::new())
        }
        Err(error) => Err(error),
    }
}

/// Build the `Filter` tree for the scope predicate combined with the
/// caller-provided text filter. Used by the FTS branch; the vector
/// branch can't nest predicates so it post-filters instead.
fn scoped_filter(
    user_id: &str,
    agent_id: Option<Uuid>,
    leaf: Filter,
) -> Result<Filter, NativeSearchError> {
    let user_key = IndexKey::new(chunks::fs_keys::USER_ID).map_err(|e| {
        NativeSearchError::Workspace(WorkspaceError::SearchFailed {
            reason: format!("user_id index key: {e}"),
        })
    })?;
    let agent_key = IndexKey::new(chunks::fs_keys::AGENT_ID).map_err(|e| {
        NativeSearchError::Workspace(WorkspaceError::SearchFailed {
            reason: format!("agent_id index key: {e}"),
        })
    })?;
    let children = vec![
        Filter::Eq {
            key: user_key,
            value: IndexValue::Text(user_id.to_string()),
        },
        Filter::Eq {
            key: agent_key,
            value: IndexValue::Text(paths::agent_id_segment(agent_id)),
        },
        leaf,
    ];
    Ok(Filter::And(children))
}

fn pre_fusion_limit_to_u32(limit: usize) -> u32 {
    let bounded = limit.max(1).min(u32::MAX as usize);
    bounded as u32
}

/// Load the user-scoped chunk set used by the scan-and-rank fallback,
/// caching it inside the search call so the two branches don't pay
/// for two walks of the document tree.
async fn ensure_fallback_chunks<'a, F>(
    store: &FilesystemWorkspaceStore<F>,
    user_id: &str,
    agent_id: Option<Uuid>,
    path_by_doc: &mut HashMap<Uuid, String>,
    cache: &'a mut Option<Vec<StoredChunk>>,
) -> Result<&'a [StoredChunk], WorkspaceError>
where
    F: RootFilesystem,
{
    if cache.is_none() {
        let docs = documents::list_documents(store, user_id, agent_id).await?;
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
        *cache = Some(chunks);
    }
    Ok(cache.as_deref().unwrap_or(&[]))
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
