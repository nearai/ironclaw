//! Thread list/create/delete + timeline reads.
//!
//! Wire source: `RebornListThreadsResponse { threads: Vec<SessionThreadRecord>,
//! next_cursor: Option<String> }` (`ironclaw_product_workflow::reborn_services::types`);
//! `SessionThreadRecord` fields at `ironclaw_threads::contract`.
//! Timeline wire source: `RebornTimelineResponse { thread: SessionThreadRecord,
//! messages: Vec<ThreadMessageRecord>, summary_artifacts: Vec<SummaryArtifact>,
//! next_cursor: Option<String> }`; `ThreadMessageRecord` fields at
//! `ironclaw_threads::contract`.
//!
//! `ThreadSummary`/`ThreadMessageSummary`/`TimelinePage` are subtractive
//! mirrors (`.claude/rules/type-placement.md` "Subtractive" case): narrow
//! TUI-display projections of `SessionThreadRecord`/`ThreadMessageRecord`,
//! which carry internal fields (`scope`, `metadata_json`, `goal`,
//! `source_binding_id`, provider replay metadata, …) the TUI never renders.
//! Timestamps are kept as raw RFC3339 `String` rather than adding a `chrono`
//! dependency the crate's pinned dep list deliberately omits. `next_cursor`
//! on the list-threads response is a dead wire field for this crate today
//! (no pagination consumer yet) and is intentionally dropped — serde ignores
//! unknown fields, so the wire response still deserializes.

use serde::Deserialize;
use uuid::Uuid;

use super::{ApiClient, ClientError};

#[derive(Debug, Clone, Deserialize)]
pub struct ThreadSummary {
    pub thread_id: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub created_at: Option<String>,
    #[serde(default)]
    pub updated_at: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ThreadMessageSummary {
    pub message_id: String,
    pub sequence: u64,
    /// Raw `MessageKind` wire string: user/assistant/system/summary/
    /// checkpoint_reference/tool_result_reference/capability_display_preview.
    pub kind: String,
    /// Raw `MessageStatus` wire string: accepted/submitted/rejected_busy/
    /// deferred_busy/draft/finalized/interrupted/superseded/redacted/deleted.
    pub status: String,
    #[serde(default)]
    pub content: Option<String>,
    #[serde(default)]
    pub actor_id: Option<String>,
    #[serde(default)]
    pub created_at: Option<String>,
    /// Present on an `assistant` row: the run that produced it. The wire
    /// `ThreadMessageRecord` always carries this field (server-side, it's
    /// how a retried draft is deduped against its run — see
    /// `ThreadMessageRecord::turn_run_id` at `ironclaw_threads::contract`);
    /// this crate previously dropped it as an unused subtraction (serde
    /// silently ignores unknown fields), but it is the stable id `lib.rs`'s
    /// `apply_timeline_page` and `app/transcript.rs`'s replay filtering now
    /// share to dedupe an SSE replay against an already-loaded timeline
    /// snapshot — see `app/mod.rs`'s `AppState::settled_run_ids` doc.
    #[serde(default)]
    pub turn_run_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TimelinePage {
    pub thread: ThreadSummary,
    pub messages: Vec<ThreadMessageSummary>,
    #[serde(default)]
    pub next_cursor: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RebornListThreadsResponseWire {
    threads: Vec<ThreadSummary>,
}

#[derive(Deserialize)]
struct CreateThreadWire {
    thread: ThreadSummary,
}

impl ApiClient {
    pub async fn list_threads(&self) -> Result<Vec<ThreadSummary>, ClientError> {
        let wire: RebornListThreadsResponseWire = self
            .send_json(self.http.get(self.url("/api/webchat/v2/threads")))
            .await?;
        Ok(wire.threads)
    }

    pub async fn create_thread(&self) -> Result<ThreadSummary, ClientError> {
        let wire: CreateThreadWire = self
            .send_json(self.http.post(self.url("/api/webchat/v2/threads")).json(
                &serde_json::json!({
                    "client_action_id": Uuid::new_v4().to_string(),
                }),
            ))
            .await?;
        Ok(wire.thread)
    }

    pub async fn delete_thread(&self, thread_id: &str) -> Result<(), ClientError> {
        self.send_unit(
            self.http
                .delete(self.url(&format!("/api/webchat/v2/threads/{thread_id}"))),
        )
        .await
    }

    pub async fn timeline(
        &self,
        thread_id: &str,
        limit: u32,
        cursor: Option<String>,
    ) -> Result<TimelinePage, ClientError> {
        let mut request = self
            .http
            .get(self.url(&format!("/api/webchat/v2/threads/{thread_id}/timeline")))
            .query(&[("limit", limit.to_string())]);
        if let Some(cursor) = cursor {
            request = request.query(&[("cursor", cursor)]);
        }
        self.send_json(request).await
    }
}
