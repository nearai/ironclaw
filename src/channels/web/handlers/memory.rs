//! Memory/workspace API handlers.

use std::sync::Arc;

use axum::{
    Json,
    extract::{Query, State},
    http::StatusCode,
};
use serde::Deserialize;

use crate::channels::web::auth::{AuthenticatedUser, UserIdentity};
use crate::channels::web::server::GatewayState;
use crate::channels::web::types::*;
use crate::workspace::Workspace;
use crate::workspace::card_metadata::{
    self, CardMetadata, extract_card_metadata, generate_fallback_metadata, is_hidden_from_cards,
    merge_card_metadata,
};

/// Resolve the workspace for the authenticated user.
///
/// Prefers `workspace_pool` (multi-user mode) when available, falling back
/// to the single-user `state.workspace`.
pub(crate) async fn resolve_workspace(
    state: &GatewayState,
    user: &UserIdentity,
) -> Result<Arc<Workspace>, (StatusCode, String)> {
    if let Some(pool) = state.workspace_pool() {
        return Ok(pool.get_or_create(user).await);
    }
    state.workspace().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Workspace not available".to_string(),
    ))
}

#[derive(Deserialize)]
pub struct TreeQuery {
    #[allow(dead_code)]
    pub depth: Option<usize>,
}

pub async fn memory_tree_handler(
    State(state): State<Arc<GatewayState>>,
    AuthenticatedUser(user): AuthenticatedUser,
    Query(_query): Query<TreeQuery>,
) -> Result<Json<MemoryTreeResponse>, (StatusCode, String)> {
    let workspace = resolve_workspace(&state, &user).await?;

    // Build tree from list_all (flat list of all paths)
    let all_paths = workspace
        .list_all()
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    // Collect unique directories and files
    let mut entries: Vec<TreeEntry> = Vec::new();
    let mut seen_dirs: std::collections::HashSet<String> = std::collections::HashSet::new();

    for path in &all_paths {
        // Add parent directories
        let parts: Vec<&str> = path.split('/').collect();
        for i in 0..parts.len().saturating_sub(1) {
            let dir_path = parts[..=i].join("/");
            if seen_dirs.insert(dir_path.clone()) {
                entries.push(TreeEntry {
                    path: dir_path,
                    is_dir: true,
                });
            }
        }
        // Add the file itself
        entries.push(TreeEntry {
            path: path.clone(),
            is_dir: false,
        });
    }

    entries.sort_by(|a, b| a.path.cmp(&b.path));

    Ok(Json(MemoryTreeResponse { entries }))
}

#[derive(Deserialize)]
pub struct ListQuery {
    pub path: Option<String>,
}

pub async fn memory_list_handler(
    State(state): State<Arc<GatewayState>>,
    AuthenticatedUser(user): AuthenticatedUser,
    Query(query): Query<ListQuery>,
) -> Result<Json<MemoryListResponse>, (StatusCode, String)> {
    let workspace = resolve_workspace(&state, &user).await?;

    let path = query.path.as_deref().unwrap_or("");
    let entries = workspace
        .list(path)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let list_entries: Vec<ListEntry> = entries
        .iter()
        .map(|e| ListEntry {
            name: e.path.rsplit('/').next().unwrap_or(&e.path).to_string(),
            path: e.path.clone(),
            is_dir: e.is_directory,
            updated_at: e.updated_at.map(|dt| dt.to_rfc3339()),
        })
        .collect();

    Ok(Json(MemoryListResponse {
        path: path.to_string(),
        entries: list_entries,
    }))
}

#[derive(Deserialize)]
pub struct ReadQuery {
    pub path: String,
}

pub async fn memory_read_handler(
    State(state): State<Arc<GatewayState>>,
    AuthenticatedUser(user): AuthenticatedUser,
    Query(query): Query<ReadQuery>,
) -> Result<Json<MemoryReadResponse>, (StatusCode, String)> {
    let workspace = resolve_workspace(&state, &user).await?;

    let doc = workspace
        .read(&query.path)
        .await
        .map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))?;

    Ok(Json(MemoryReadResponse {
        path: query.path,
        content: doc.content,
        updated_at: Some(doc.updated_at.to_rfc3339()),
    }))
}

pub async fn memory_write_handler(
    State(state): State<Arc<GatewayState>>,
    AuthenticatedUser(user): AuthenticatedUser,
    Json(req): Json<MemoryWriteRequest>,
) -> Result<Json<MemoryWriteResponse>, (StatusCode, String)> {
    let workspace = resolve_workspace(&state, &user).await?;

    // Route through layer-aware methods when a layer is specified.
    //
    // Note: unlike MemoryWriteTool, this endpoint does NOT block writes to
    // identity files (IDENTITY.md, SOUL.md, etc.). The HTTP API is an
    // authenticated admin interface; the supervisor uses it to seed identity
    // files at startup. Identity-file protection is enforced at the tool
    // layer (LLM-facing) where the write originates from an untrusted agent.
    if let Some(ref layer_name) = req.layer {
        let result = if req.append {
            workspace
                .append_to_layer(layer_name, &req.path, &req.content, req.force)
                .await
        } else {
            workspace
                .write_to_layer(layer_name, &req.path, &req.content, req.force)
                .await
        }
        .map_err(|e| {
            use crate::error::WorkspaceError;
            let status = match &e {
                WorkspaceError::LayerNotFound { .. } => StatusCode::BAD_REQUEST,
                WorkspaceError::LayerReadOnly { .. } => StatusCode::FORBIDDEN,
                WorkspaceError::PrivacyRedirectFailed => StatusCode::UNPROCESSABLE_ENTITY,
                _ => StatusCode::INTERNAL_SERVER_ERROR,
            };
            (status, e.to_string())
        })?;

        // Generate card metadata for layer writes too
        spawn_card_metadata_generation(
            state.clone(),
            workspace.clone(),
            req.path.clone(),
            req.content.clone(),
        );

        return Ok(Json(MemoryWriteResponse {
            path: req.path,
            status: "written",
            redirected: Some(result.redirected),
            actual_layer: Some(result.actual_layer),
        }));
    }

    // Non-layer path: honor the append field
    if req.append {
        workspace
            .append(&req.path, &req.content)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    } else {
        workspace
            .write(&req.path, &req.content)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    }

    // Generate card metadata asynchronously after write
    spawn_card_metadata_generation(
        state.clone(),
        workspace.clone(),
        req.path.clone(),
        req.content.clone(),
    );

    Ok(Json(MemoryWriteResponse {
        path: req.path,
        status: "written",
        redirected: None,
        actual_layer: None,
    }))
}

/// Spawn async card metadata generation (fallback first, then LLM).
fn spawn_card_metadata_generation(
    state: Arc<GatewayState>,
    workspace: Arc<Workspace>,
    path: String,
    content: String,
) {
    // Skip metadata for system/identity files
    if is_hidden_from_cards(&path) {
        return;
    }

    // Skip metadata for empty content
    if content.trim().is_empty() {
        return;
    }

    // Generate and store fallback metadata synchronously-ish in a spawn
    let fallback = generate_fallback_metadata(&content, &path);
    let llm_provider = state.llm_provider.clone();

    tokio::spawn(async move {
        // Write fallback metadata first
        if let Err(e) = write_card_metadata(&workspace, &path, &fallback).await {
            tracing::debug!("Failed to write fallback card metadata for {}: {}", path, e);
            return;
        }

        // Try LLM generation if provider is available
        if let Some(provider) = llm_provider {
            match generate_llm_card_metadata(&provider, &content).await {
                Ok(llm_meta) => {
                    if let Err(e) = write_card_metadata(&workspace, &path, &llm_meta).await {
                        tracing::debug!("Failed to write LLM card metadata for {}: {}", path, e);
                    }
                }
                Err(e) => {
                    tracing::debug!("LLM card metadata generation failed for {}: {}", path, e);
                    // Fallback is already written, so this is fine
                }
            }
        }
    });
}

/// Write card metadata into a document's metadata field.
async fn write_card_metadata(
    workspace: &Workspace,
    path: &str,
    card: &CardMetadata,
) -> Result<(), String> {
    let doc = workspace.read(path).await.map_err(|e| e.to_string())?;

    let mut metadata = doc.metadata.clone();
    merge_card_metadata(&mut metadata, card);

    workspace
        .update_metadata(doc.id, &metadata)
        .await
        .map_err(|e| e.to_string())
}

/// Generate card metadata using the LLM provider.
async fn generate_llm_card_metadata(
    provider: &Arc<dyn crate::llm::LlmProvider>,
    content: &str,
) -> Result<CardMetadata, String> {
    use crate::llm::{ChatMessage, CompletionRequest};

    let (system_prompt, user_prompt) = card_metadata::build_metadata_prompt(content);

    let request = CompletionRequest::new(vec![
        ChatMessage::system(system_prompt),
        ChatMessage::user(user_prompt),
    ])
    .with_max_tokens(300);

    let response = provider
        .complete(request)
        .await
        .map_err(|e| format!("LLM error: {e}"))?;

    card_metadata::parse_llm_response(&response.content)
        .ok_or_else(|| "Failed to parse LLM metadata response".to_string())
}

pub async fn memory_search_handler(
    State(state): State<Arc<GatewayState>>,
    AuthenticatedUser(user): AuthenticatedUser,
    Json(req): Json<MemorySearchRequest>,
) -> Result<Json<MemorySearchResponse>, (StatusCode, String)> {
    let workspace = resolve_workspace(&state, &user).await?;

    let limit = req.limit.unwrap_or(10);
    let results = workspace
        .search(&req.query, limit)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let mut hits: Vec<SearchHit> = Vec::with_capacity(results.len());
    for r in &results {
        // Try to read document for card metadata
        let (card_title, card_summary, card_tags) =
            if let Ok(doc) = workspace.read(&r.document_path).await {
                match extract_card_metadata(&doc.metadata) {
                    Some(card) => (
                        Some(card.card_title),
                        Some(card.card_summary),
                        Some(card.card_tags),
                    ),
                    None => (None, None, None),
                }
            } else {
                (None, None, None)
            };

        // Build match excerpt if the search query appears in content but not in summary
        let match_excerpt = if let Some(ref summary) = card_summary {
            let query_lower = req.query.to_ascii_lowercase();
            let summary_lower = summary.to_ascii_lowercase();
            let content_lower = r.content.to_ascii_lowercase();
            if !summary_lower.contains(&query_lower) && content_lower.contains(&query_lower) {
                // Extract a snippet around the match
                Some(build_excerpt(&r.content, &req.query, 120))
            } else {
                None
            }
        } else {
            None
        };

        hits.push(SearchHit {
            path: r.document_path.clone(),
            content: r.content.clone(),
            score: r.score as f64,
            card_title,
            card_summary,
            card_tags,
            match_excerpt,
        });
    }

    Ok(Json(MemorySearchResponse { results: hits }))
}

/// Build a content excerpt around a search match.
fn build_excerpt(content: &str, query: &str, max_len: usize) -> String {
    let content_lower = content.to_ascii_lowercase();
    let query_lower = query.to_ascii_lowercase();

    if let Some(pos) = content_lower.find(&query_lower) {
        let context_before = 40;
        let start = pos.saturating_sub(context_before);
        // Find char boundary
        let mut start = start;
        while start > 0 && !content.is_char_boundary(start) {
            start -= 1;
        }
        let mut end = (pos + query.len() + max_len - context_before).min(content.len());
        while end < content.len() && !content.is_char_boundary(end) {
            end += 1;
        }
        let end = end.min(content.len());

        let mut excerpt = String::new();
        if start > 0 {
            excerpt.push_str("...");
        }
        excerpt.push_str(&content[start..end]);
        if end < content.len() {
            excerpt.push_str("...");
        }
        excerpt
    } else {
        // Fallback: just take the beginning
        let mut end = max_len.min(content.len());
        while end < content.len() && !content.is_char_boundary(end) {
            end += 1;
        }
        let mut excerpt = content[..end].to_string();
        if end < content.len() {
            excerpt.push_str("...");
        }
        excerpt
    }
}

/// Handler for the knowledge cards endpoint.
/// Returns all non-system memory documents with card metadata.
pub async fn memory_cards_handler(
    State(state): State<Arc<GatewayState>>,
    AuthenticatedUser(user): AuthenticatedUser,
) -> Result<Json<MemoryCardsResponse>, (StatusCode, String)> {
    let workspace = resolve_workspace(&state, &user).await?;

    let all_paths = workspace
        .list_all()
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let mut cards: Vec<CardEntry> = Vec::new();

    for path in &all_paths {
        // Skip system/identity files
        if is_hidden_from_cards(path) {
            continue;
        }

        // Read the document
        let doc = match workspace.read(path).await {
            Ok(doc) => doc,
            Err(_) => continue,
        };

        // Skip empty documents
        if doc.content.trim().is_empty() {
            continue;
        }

        // Extract card metadata or generate fallback
        let card_meta = extract_card_metadata(&doc.metadata).unwrap_or_else(|| {
            let fallback = generate_fallback_metadata(&doc.content, path);

            // Lazy backfill: spawn async metadata generation
            if let Some(ref llm_provider) = state.llm_provider {
                let provider = llm_provider.clone();
                let ws = workspace.clone();
                let p = path.clone();
                let content = doc.content.clone();
                tokio::spawn(async move {
                    match generate_llm_card_metadata(&provider, &content).await {
                        Ok(llm_meta) => {
                            if let Err(e) = write_card_metadata(&ws, &p, &llm_meta).await {
                                tracing::debug!(
                                    "Lazy backfill: failed to write metadata for {}: {}",
                                    p,
                                    e
                                );
                            }
                        }
                        Err(e) => {
                            tracing::debug!("Lazy backfill: LLM failed for {}: {}", p, e);
                        }
                    }
                });
            }

            fallback
        });

        cards.push(CardEntry {
            path: path.clone(),
            title: card_meta.card_title,
            summary: card_meta.card_summary,
            tags: card_meta.card_tags,
            updated_at: doc.updated_at.to_rfc3339(),
        });
    }

    // Sort by updated_at descending (newest first)
    cards.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));

    Ok(Json(MemoryCardsResponse { cards }))
}
