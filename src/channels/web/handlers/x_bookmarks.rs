//! HTTP handlers for the X bookmarks skill.
//!
//! Routes:
//! - `POST /api/x-bookmarks/ingest`  — bulk-ingest scraped bookmarks.
//! - `POST /api/x-bookmarks/triage`  — run the configured triage LLM over
//!   the user's untriaged queue.
//! - `GET  /api/x-bookmarks/queue`   — return the current queue, filtered.
//! - `GET  /api/x-bookmarks/stats`   — aggregate counts per status.
//!
//! All routes require gateway bearer auth via [`AuthenticatedUser`]. Each
//! request is user-scoped: the `user_id` is taken from the authenticated
//! session, never from the request body, so cross-user contamination is
//! impossible at the gateway boundary.

use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex};

use axum::{
    Json,
    extract::{Query, State},
    http::StatusCode,
};
use serde::{Deserialize, Serialize};

use crate::channels::web::auth::AuthenticatedUser;
use crate::channels::web::platform::state::GatewayState;
use crate::x_bookmarks::{
    BookmarkIngestItem, BookmarkStatus, IngestError, MAX_INGEST_BATCH, validate_ingest_item,
};

/// Per-user serialization for the triage handler.
///
/// Codex adversarial review (high): without a lock, two concurrent POST
/// `/api/x-bookmarks/triage` requests for the same user can both pull the
/// same untriaged batch, both pay for the LLM call, and race to write
/// results back. The DB-level `status='untriaged'` guard catches the second
/// writer and prevents data corruption, but the second LLM call is still
/// wasteful. This mutex serializes the whole list-call-apply cycle per
/// user. The cost is one `tokio::sync::Mutex` per active user; the lock is
/// released as soon as the response is built.
static USER_TRIAGE_LOCKS: LazyLock<Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

fn user_triage_lock(user_id: &str) -> Arc<tokio::sync::Mutex<()>> {
    let mut map = match USER_TRIAGE_LOCKS.lock() {
        Ok(g) => g,
        Err(poisoned) => poisoned.into_inner(),
    };
    map.entry(user_id.to_string())
        .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
        .clone()
}

#[derive(Debug, Deserialize)]
pub struct IngestRequest {
    pub bookmarks: Vec<BookmarkIngestItem>,
    /// Free-form scraper identifier — recorded for ops debugging only.
    #[serde(default)]
    #[allow(dead_code)]
    pub source: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct IngestResponse {
    pub seen: u64,
    pub inserted: u64,
    pub duplicate: u64,
    pub rejected: u64,
    pub errors: Vec<IngestItemError>,
}

#[derive(Debug, Serialize)]
pub struct IngestItemError {
    pub index: usize,
    pub message: String,
}

/// `POST /api/x-bookmarks/ingest`
pub async fn x_bookmarks_ingest_handler(
    State(state): State<Arc<GatewayState>>,
    AuthenticatedUser(user): AuthenticatedUser,
    Json(body): Json<IngestRequest>,
) -> Result<Json<IngestResponse>, (StatusCode, String)> {
    let raw = body.bookmarks;
    if raw.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            "bookmarks must not be empty".into(),
        ));
    }
    if raw.len() > MAX_INGEST_BATCH {
        return Err((
            StatusCode::PAYLOAD_TOO_LARGE,
            format!(
                "bookmarks batch must contain <= {MAX_INGEST_BATCH} items (got {})",
                raw.len()
            ),
        ));
    }

    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".into(),
    ))?;

    let mut normalized = Vec::with_capacity(raw.len());
    let mut errors = Vec::new();
    let seen = raw.len() as u64;
    for (idx, item) in raw.iter().enumerate() {
        match validate_ingest_item(item) {
            Ok(n) => normalized.push(n),
            Err(IngestError::InvalidField(field, reason)) => errors.push(IngestItemError {
                index: idx,
                message: format!("{field}: {reason}"),
            }),
            Err(other) => errors.push(IngestItemError {
                index: idx,
                message: other.to_string(),
            }),
        }
    }

    let (inserted, duplicate) = if normalized.is_empty() {
        (0, 0)
    } else {
        store
            .insert_x_bookmarks(&user.user_id, &normalized)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
    };

    Ok(Json(IngestResponse {
        seen,
        inserted,
        duplicate,
        rejected: errors.len() as u64,
        errors,
    }))
}

#[derive(Debug, Deserialize)]
pub struct TriageRequest {
    /// How many untriaged bookmarks to process. Capped at the triage batch
    /// limit (`crate::x_bookmarks::triage::MAX_TRIAGE_BATCH`).
    #[serde(default)]
    pub limit: Option<u32>,
}

#[derive(Debug, Serialize)]
pub struct TriageResponse {
    pub triaged: u64,
    /// Echoes the model the triage call actually used. Either the
    /// per-skill override or the LLM provider's effective default.
    pub model: String,
    pub batch_size: u64,
}

/// `POST /api/x-bookmarks/triage`
pub async fn x_bookmarks_triage_handler(
    State(state): State<Arc<GatewayState>>,
    AuthenticatedUser(user): AuthenticatedUser,
    Json(body): Json<TriageRequest>,
) -> Result<Json<TriageResponse>, (StatusCode, String)> {
    // Per-user serialization. See `USER_TRIAGE_LOCKS` for rationale. We
    // hold the lock for the entire list-call-apply window so two concurrent
    // /triage requests for the same user run back-to-back, not concurrently.
    let lock = user_triage_lock(&user.user_id);
    let _guard = lock.lock().await;

    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".into(),
    ))?;
    let llm = state.llm_provider.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "LLM provider not configured".into(),
    ))?;

    // Triage-model override resolution order:
    //   1. settings table (`skills.x_bookmarks.triage_model`) — per-user
    //      tunable at runtime via the standard /api/settings/{key} endpoint.
    //   2. `X_BOOKMARKS_TRIAGE_MODEL` env var — operator-wide fallback.
    //   3. Unset → CompletionRequest::model = None → the LLM provider uses
    //      its global active model. This is the documented default and what
    //      a fresh install gets out of the box.
    let model_override = resolve_triage_model_override(&state, &user.user_id).await;

    let max = crate::x_bookmarks::triage::MAX_TRIAGE_BATCH as u32;
    let limit = body.limit.unwrap_or(max).min(max).max(1);

    let bookmarks = store
        .list_untriaged_x_bookmarks(&user.user_id, limit)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    if bookmarks.is_empty() {
        let model_used = model_override
            .clone()
            .unwrap_or_else(|| llm.active_model_name());
        return Ok(Json(TriageResponse {
            triaged: 0,
            model: model_used,
            batch_size: 0,
        }));
    }

    let batch_size = bookmarks.len() as u64;
    let decisions = crate::x_bookmarks::triage::triage_batch(
        llm.clone(),
        model_override.as_deref(),
        &bookmarks,
    )
    .await
    .map_err(|e| match e {
        crate::x_bookmarks::triage::TriageError::Llm(_) => (StatusCode::BAD_GATEWAY, e.to_string()),
        crate::x_bookmarks::triage::TriageError::EmptyResponse
        | crate::x_bookmarks::triage::TriageError::InvalidJson(_)
        | crate::x_bookmarks::triage::TriageError::LengthMismatch { .. } => {
            (StatusCode::BAD_GATEWAY, e.to_string())
        }
        crate::x_bookmarks::triage::TriageError::BatchTooLarge(_) => {
            (StatusCode::PAYLOAD_TOO_LARGE, e.to_string())
        }
    })?;

    // Validate the LLM response as a whole before any DB write.
    //
    // Codex adversarial review (high): partial application is unsafe. A
    // hostile LLM (or one steered by prompt injection in the tweet text)
    // can return N decisions all with `id = 0`, pass the length check, and
    // overwrite a single bookmark N times while leaving the rest untriaged.
    // Rule: ids MUST form the exact set 0..bookmarks.len(), each appearing
    // exactly once, and every status MUST be a valid triage output.
    // Anything less rejects the entire batch.
    let paired = match build_paired_decisions(&bookmarks, decisions) {
        Ok(paired) => paired,
        Err(err) => {
            tracing::warn!(error = %err, "triage: rejecting malformed LLM response");
            return Err((StatusCode::BAD_GATEWAY, err));
        }
    };

    let model_used = model_override
        .clone()
        .unwrap_or_else(|| llm.active_model_name());
    let triaged = store
        .apply_x_bookmark_triage(&user.user_id, &paired, &model_used)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(TriageResponse {
        triaged,
        model: model_used,
        batch_size,
    }))
}

#[derive(Debug, Deserialize)]
pub struct QueueQuery {
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub limit: Option<u32>,
}

#[derive(Debug, Serialize)]
pub struct QueueResponse {
    pub bookmarks: Vec<crate::x_bookmarks::Bookmark>,
}

/// `GET /api/x-bookmarks/queue`
pub async fn x_bookmarks_queue_handler(
    State(state): State<Arc<GatewayState>>,
    AuthenticatedUser(user): AuthenticatedUser,
    Query(q): Query<QueueQuery>,
) -> Result<Json<QueueResponse>, (StatusCode, String)> {
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".into(),
    ))?;

    let limit = q.limit.unwrap_or(50).clamp(1, 500);
    let bookmarks = store
        .list_x_bookmarks_by_status(&user.user_id, q.status.as_deref(), limit)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(QueueResponse { bookmarks }))
}

#[derive(Debug, Serialize)]
pub struct StatsResponse {
    pub by_status: std::collections::HashMap<String, u64>,
    pub total: u64,
}

/// `GET /api/x-bookmarks/stats`
pub async fn x_bookmarks_stats_handler(
    State(state): State<Arc<GatewayState>>,
    AuthenticatedUser(user): AuthenticatedUser,
) -> Result<Json<StatsResponse>, (StatusCode, String)> {
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".into(),
    ))?;
    let counts = store
        .x_bookmark_counts_by_status(&user.user_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let total: u64 = counts.values().copied().sum();
    Ok(Json(StatsResponse {
        by_status: counts,
        total,
    }))
}

/// Validate the LLM's decision array against the bookmark batch and pair
/// each decision with the corresponding bookmark UUID.
///
/// On any of:
/// - id outside `0..bookmarks.len()`,
/// - duplicate id,
/// - missing id (the produced set must cover every input bookmark),
/// - unknown status string,
/// - status `untriaged` (LLM must commit to a triage outcome),
///
/// the entire batch is rejected. This is the primary defence against
/// prompt-injection inside tweet text steering the LLM into a single-id
/// repeat that would partially overwrite the queue.
fn build_paired_decisions(
    bookmarks: &[crate::x_bookmarks::Bookmark],
    decisions: Vec<crate::x_bookmarks::triage::TriageDecision>,
) -> Result<Vec<(uuid::Uuid, crate::db::ResolvedTriageDecision)>, String> {
    if decisions.len() != bookmarks.len() {
        return Err(format!(
            "LLM returned {} decisions for a batch of {}",
            decisions.len(),
            bookmarks.len()
        ));
    }
    let mut paired: Vec<Option<(uuid::Uuid, crate::db::ResolvedTriageDecision)>> =
        (0..bookmarks.len()).map(|_| None).collect();

    for d in decisions {
        let idx = d.id;
        if idx < 0 || (idx as usize) >= bookmarks.len() {
            return Err(format!(
                "id {idx} is outside the batch range 0..{}",
                bookmarks.len()
            ));
        }
        let slot = &mut paired[idx as usize];
        if slot.is_some() {
            return Err(format!("duplicate id {idx} in LLM response"));
        }
        let Some(status) = BookmarkStatus::parse(&d.status) else {
            return Err(format!("unknown status {:?}", d.status));
        };
        if matches!(status, BookmarkStatus::Untriaged) {
            return Err("LLM returned `untriaged` for a triage decision".to_string());
        }
        let project_slug = if matches!(status, BookmarkStatus::Build) {
            d.project_slug.filter(|s| !s.is_empty())
        } else {
            None
        };
        *slot = Some((
            bookmarks[idx as usize].id,
            crate::db::ResolvedTriageDecision {
                status: status.as_str().to_string(),
                rationale: d.rationale.filter(|s| !s.is_empty()),
                project_slug,
                tags: d.tags,
            },
        ));
    }

    paired
        .into_iter()
        .enumerate()
        .map(|(idx, slot)| slot.ok_or_else(|| format!("missing decision for id {idx}")))
        .collect()
}

/// Resolve the configurable triage-model override. Returns `None` if no
/// override is configured (so the LLM provider falls back to its global
/// default model).
async fn resolve_triage_model_override(state: &GatewayState, user_id: &str) -> Option<String> {
    // 1. Per-user setting via the cached SettingsStore (preferred — runtime
    //    tunable without a process restart).
    if let Some(cache) = state.settings_cache.as_ref()
        && let Ok(Some(value)) = crate::db::SettingsStore::get_setting(
            cache.as_ref(),
            user_id,
            "skills.x_bookmarks.triage_model",
        )
        .await
        && let Some(s) = value.as_str()
    {
        let trimmed = s.trim();
        if !trimmed.is_empty() {
            return Some(trimmed.to_string());
        }
    }
    // 2. Operator-wide env var fallback.
    std::env::var("X_BOOKMARKS_TRIAGE_MODEL")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::x_bookmarks::Bookmark;
    use crate::x_bookmarks::triage::TriageDecision;
    use chrono::Utc;

    fn make_bookmarks(n: usize) -> Vec<Bookmark> {
        (0..n)
            .map(|i| Bookmark {
                id: uuid::Uuid::new_v4(),
                user_id: "u".to_string(),
                tweet_id: format!("{i:020}"),
                author_handle: None,
                author_name: None,
                text: String::new(),
                url: None,
                media_urls: vec![],
                quoted_tweet: None,
                thread_id: None,
                posted_at: None,
                scraped_at: Utc::now(),
                status: BookmarkStatus::Untriaged,
                rationale: None,
                project_slug: None,
                tags: vec![],
                triaged_at: None,
                triage_model: None,
            })
            .collect()
    }

    fn good_decision(id: i64, status: &str) -> TriageDecision {
        TriageDecision {
            id,
            status: status.to_string(),
            rationale: None,
            project_slug: None,
            tags: vec![],
        }
    }

    /// Codex finding (high): a hostile LLM with N decisions all `id = 0`
    /// must NOT partially apply — the entire batch must be rejected.
    #[test]
    fn build_paired_rejects_duplicate_ids() {
        let bookmarks = make_bookmarks(3);
        let decisions = vec![
            good_decision(0, "build"),
            good_decision(0, "read"),
            good_decision(0, "dead"),
        ];
        let err = build_paired_decisions(&bookmarks, decisions).unwrap_err();
        assert!(err.contains("duplicate"), "got: {err}");
    }

    #[test]
    fn build_paired_rejects_out_of_range_id() {
        let bookmarks = make_bookmarks(2);
        let decisions = vec![good_decision(0, "build"), good_decision(99, "read")];
        let err = build_paired_decisions(&bookmarks, decisions).unwrap_err();
        assert!(err.contains("99") && err.contains("range"), "got: {err}");
    }

    #[test]
    fn build_paired_rejects_unknown_status() {
        let bookmarks = make_bookmarks(1);
        let decisions = vec![good_decision(0, "shenanigans")];
        let err = build_paired_decisions(&bookmarks, decisions).unwrap_err();
        assert!(err.contains("unknown status"), "got: {err}");
    }

    #[test]
    fn build_paired_rejects_untriaged_status() {
        let bookmarks = make_bookmarks(1);
        let decisions = vec![good_decision(0, "untriaged")];
        let err = build_paired_decisions(&bookmarks, decisions).unwrap_err();
        assert!(err.contains("untriaged"), "got: {err}");
    }

    #[test]
    fn build_paired_rejects_length_mismatch() {
        let bookmarks = make_bookmarks(3);
        let decisions = vec![good_decision(0, "build")];
        let err = build_paired_decisions(&bookmarks, decisions).unwrap_err();
        assert!(err.contains("1") && err.contains("3"), "got: {err}");
    }

    #[test]
    fn build_paired_accepts_full_unique_range() {
        let bookmarks = make_bookmarks(3);
        let decisions = vec![
            good_decision(2, "dead"),
            good_decision(0, "build"),
            good_decision(1, "read"),
        ];
        let paired = build_paired_decisions(&bookmarks, decisions).unwrap();
        assert_eq!(paired.len(), 3);
        // Order is by bookmark index.
        assert_eq!(paired[0].0, bookmarks[0].id);
        assert_eq!(paired[0].1.status, "build");
        assert_eq!(paired[1].0, bookmarks[1].id);
        assert_eq!(paired[1].1.status, "read");
        assert_eq!(paired[2].0, bookmarks[2].id);
        assert_eq!(paired[2].1.status, "dead");
    }

    #[test]
    fn build_paired_drops_project_slug_for_non_build() {
        let bookmarks = make_bookmarks(1);
        let mut d = good_decision(0, "read");
        d.project_slug = Some("not-build".to_string());
        let paired = build_paired_decisions(&bookmarks, vec![d]).unwrap();
        assert!(paired[0].1.project_slug.is_none());
    }
}
