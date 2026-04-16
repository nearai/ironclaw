//! Pending change store — holds proposed self-improvement changes awaiting user approval.
//!
//! Unlike [`PendingGateStore`] which pauses a running thread, this store holds
//! proposals from *completed* mission threads. The changes are only persisted
//! when the user explicitly accepts them.
//!
//! [`PendingGateStore`]: super::store::PendingGateStore

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use ironclaw_engine::{MissionId, SelfImprovementProposal, ThreadId};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;
use uuid::Uuid;

/// Default expiry for pending changes (24 hours).
const DEFAULT_EXPIRY_SECS: i64 = 86_400;

/// A proposed change awaiting user review.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingChange {
    /// Unique ID for this pending change request.
    pub request_id: Uuid,
    /// User who triggered the `/expected` command.
    pub user_id: String,
    /// Mission that produced this proposal.
    pub mission_id: MissionId,
    /// Human-readable mission name.
    pub mission_name: String,
    /// Thread that investigated the issue.
    pub thread_id: ThreadId,
    /// The extracted proposal (rules to add, current/proposed overlay content).
    #[serde(skip)]
    pub proposal: Option<SelfImprovementProposal>,
    /// Individual rules being proposed (for serialization to frontend).
    pub proposed_rules: Vec<String>,
    /// Current overlay content (for diff display).
    pub current_content: String,
    /// Proposed overlay content after changes.
    pub proposed_content: String,
    /// When this pending change was created.
    pub created_at: DateTime<Utc>,
    /// When this pending change expires (fail-closed: discard without persisting).
    pub expires_at: DateTime<Utc>,
}

impl PendingChange {
    /// Check whether this pending change has expired.
    pub fn is_expired(&self) -> bool {
        Utc::now() > self.expires_at
    }
}

/// Read-only view for API responses (history rehydration).
#[derive(Debug, Clone, Serialize)]
pub struct PendingChangeView {
    pub request_id: String,
    pub mission_name: String,
    pub mission_thread_id: String,
    pub proposed_rules: Vec<String>,
    pub current_content: String,
    pub proposed_content: String,
}

impl From<&PendingChange> for PendingChangeView {
    fn from(pc: &PendingChange) -> Self {
        Self {
            request_id: pc.request_id.to_string(),
            mission_name: pc.mission_name.clone(),
            mission_thread_id: pc.thread_id.to_string(),
            proposed_rules: pc.proposed_rules.clone(),
            current_content: pc.current_content.clone(),
            proposed_content: pc.proposed_content.clone(),
        }
    }
}

/// Thread-safe, in-memory store for pending changes.
///
/// Simpler than [`PendingGateStore`] — no persistence backend, no channel
/// verification (only the web gateway resolves these).
pub struct PendingChangeStore {
    inner: Mutex<HashMap<Uuid, PendingChange>>,
}

impl PendingChangeStore {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(HashMap::new()),
        }
    }

    /// Insert a new pending change from a self-improvement proposal.
    pub async fn insert(&self, change: PendingChange) {
        let mut inner = self.inner.lock().await;
        inner.insert(change.request_id, change);
    }

    /// Create a `PendingChange` from a mission notification + proposal.
    pub fn create_pending_change(
        user_id: &str,
        mission_id: MissionId,
        mission_name: &str,
        thread_id: ThreadId,
        proposal: SelfImprovementProposal,
    ) -> PendingChange {
        let now = Utc::now();
        PendingChange {
            request_id: Uuid::new_v4(),
            user_id: user_id.to_string(),
            mission_id,
            mission_name: mission_name.to_string(),
            thread_id,
            proposed_rules: proposal.proposed_rules.clone(),
            current_content: proposal.current_overlay_content.clone(),
            proposed_content: proposal.proposed_overlay_content.clone(),
            proposal: Some(proposal),
            created_at: now,
            expires_at: now + chrono::Duration::seconds(DEFAULT_EXPIRY_SECS),
        }
    }

    /// Atomically take a pending change by request ID and user ID.
    ///
    /// Returns `None` if not found, wrong user, or expired.
    pub async fn take(&self, request_id: Uuid, user_id: &str) -> Option<PendingChange> {
        let mut inner = self.inner.lock().await;
        if let Some(change) = inner.get(&request_id) {
            if change.user_id != user_id {
                return None;
            }
            if change.is_expired() {
                inner.remove(&request_id);
                return None;
            }
        } else {
            return None;
        }
        inner.remove(&request_id)
    }

    /// Peek at all non-expired pending changes for a user (for history rehydration).
    pub async fn list_for_user(&self, user_id: &str) -> Vec<PendingChangeView> {
        let inner = self.inner.lock().await;
        inner
            .values()
            .filter(|c| c.user_id == user_id && !c.is_expired())
            .map(PendingChangeView::from)
            .collect()
    }

    /// Remove expired entries.
    pub async fn expire_stale(&self) {
        let mut inner = self.inner.lock().await;
        inner.retain(|_, c| !c.is_expired());
    }
}

impl Default for PendingChangeStore {
    fn default() -> Self {
        Self::new()
    }
}
