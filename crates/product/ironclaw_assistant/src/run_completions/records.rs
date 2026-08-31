//! Durable run-completion notice records (2026-08-13 design §5.3).
//!
//! One immutable completion fact per eligible run plus its orthogonal
//! delivery and read state machines. Records carry typed identities,
//! timestamps, the terminal projection reference, and state-machine fields —
//! never reply text, prompts, titles, or any other generated content.
//! Records are retained with timestamps; caches may evict, durable rows are
//! never deleted as "cleanup".

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use ironclaw_common::hashing::sha256_hex;
use ironclaw_product_contracts::run_completions::RunCompletionIntentKind;

/// Purpose separator for the stable notice identity (§5.2).
const NOTICE_ID_PURPOSE: &str = "web-app-run-completion/v1";
/// Purpose separator for the OS-notification collapse tag (§7.10).
const THREAD_TAG_PURPOSE: &str = "web-app-run-completion-collapse/v1";

/// Derive the stable, purpose-separated notice id for one owner + run.
/// Deterministic so duplicate journal delivery rewrites nothing.
pub fn notice_id_for(tenant_id: &str, user_id: &str, run_id: &str) -> String {
    let input = format!("{NOTICE_ID_PURPOSE}\u{1f}{tenant_id}\u{1f}{user_id}\u{1f}{run_id}");
    let digest = sha256_hex(input.as_bytes());
    format!("rcn-{}", &digest[..40])
}

/// Derive the bounded, purpose-separated OS collapse tag for one owner +
/// thread. Not a display string and not an authorization token.
pub fn thread_tag_for(tenant_id: &str, user_id: &str, thread_id: &str) -> String {
    let input = format!("{THREAD_TAG_PURPOSE}\u{1f}{tenant_id}\u{1f}{user_id}\u{1f}{thread_id}");
    let digest = sha256_hex(input.as_bytes());
    format!("rct-{}", &digest[..40])
}

/// Presentation surfaces a notice can settle through (§5.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompletionSurface {
    NoSurfaceWatchingThread,
    InApp,
    LocalOs,
    WebAppPush,
}

/// The delivery half of the notice state machine (§5.3). Every transition
/// is a bounded CAS update; only a `PendingArbitration` record can become
/// `PushOwned`, and only one replica can win that CAS.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum CompletionDeliveryState {
    PendingArbitration {
        closes_at: DateTime<Utc>,
        /// Grants issued so far for this notice, carried through regress so
        /// the "exactly one re-arbitration before fallback" bound (§5.4)
        /// survives the Granted -> PendingArbitration transition. Serde
        /// default keeps pre-existing records readable.
        #[serde(default)]
        grants_issued: u32,
    },
    Granted {
        grant_id: String,
        browser_instance_id: String,
        surface: CompletionSurface,
        state_revision: u64,
        expires_at: DateTime<Utc>,
        /// Grant expiry triggers exactly one re-arbitration before push
        /// fallback (§5.4); this counts issued grants for that bound.
        grants_issued: u32,
    },
    Presented {
        surface: CompletionSurface,
        presented_at: DateTime<Utc>,
    },
    PushOwned {
        delivery_id: String,
        claimed_at: DateTime<Utc>,
    },
    NoExternalTarget {
        settled_at: DateTime<Utc>,
    },
}

/// Read evidence vocabulary (§5.3). Display is not read: only exact
/// reply-render evidence or a subsequent focused visit marks a notice read.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "evidence", rename_all = "snake_case")]
pub enum CompletionReadEvidence {
    ReplyRendered { browser_instance_id: String },
    FocusedThreadVisit { browser_instance_id: String },
}

/// The read half of the state machine, orthogonal to delivery.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "read", rename_all = "snake_case")]
pub enum CompletionReadState {
    Unread,
    Read {
        read_at: DateTime<Utc>,
        #[serde(flatten)]
        evidence: CompletionReadEvidence,
    },
}

/// One browser profile's short-lived presentation intent for one notice.
/// Retained only inside the notice record's arbitration history — never a
/// reusable presence API (§5.6).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompletionIntentRecord {
    pub browser_instance_id: String,
    pub tab_id: String,
    pub state_revision: u64,
    pub focus_epoch: u64,
    pub intent: RunCompletionIntentKind,
    pub offered_at: DateTime<Utc>,
}

/// The durable notice record: one immutable completion fact plus its
/// delivery/read state machines and bounded arbitration history.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunCompletionNotice {
    /// Record schema version for tolerant wire evolution.
    pub version: u32,
    pub notice_id: String,
    /// Member of the owner's monotonic completion sequence; the stream's
    /// cursor domain.
    pub sequence: u64,
    pub tenant_id: String,
    pub owner_user_id: String,
    pub run_id: String,
    pub thread_id: String,
    /// Agent half of the completed run's scope, kept so the push fallback
    /// can rebuild the exact `TurnScope` for outbound authorization without
    /// a transcript lookup. Absent on agentless scopes (never eligible).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    /// Project half of the completed run's scope (see `agent_id`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    /// Opaque purpose-separated OS collapse tag for this owner + thread.
    pub thread_tag: String,
    /// Reference to the completed-turn projection update that proves the
    /// finalized reply (opaque; used for push-candidate planning).
    pub terminal_projection_ref: String,
    pub completed_at: DateTime<Utc>,
    pub delivery: CompletionDeliveryState,
    pub read: CompletionReadState,
    /// Bounded per-notice intent history (§5.4: at most 32, newest revision
    /// per browser profile wins).
    #[serde(default)]
    pub intents: Vec<CompletionIntentRecord>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

pub const RUN_COMPLETION_NOTICE_VERSION: u32 = 1;

impl RunCompletionNotice {
    pub fn is_read(&self) -> bool {
        matches!(self.read, CompletionReadState::Read { .. })
    }

    /// Whether the notice still needs arbitration/presentation work
    /// (pending or granted, and unread).
    pub fn is_unsettled(&self) -> bool {
        !self.is_read()
            && matches!(
                self.delivery,
                CompletionDeliveryState::PendingArbitration { .. }
                    | CompletionDeliveryState::Granted { .. }
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notice_ids_are_stable_and_purpose_separated_from_thread_tags() {
        let a1 = notice_id_for("tenant-a", "user-a", "run-1");
        let a2 = notice_id_for("tenant-a", "user-a", "run-1");
        assert_eq!(a1, a2, "identical inputs derive identical notice ids");
        assert_ne!(
            a1,
            notice_id_for("tenant-a", "user-b", "run-1"),
            "the owner scope is part of the identity",
        );
        let tag = thread_tag_for("tenant-a", "user-a", "run-1");
        assert_ne!(
            a1.trim_start_matches("rcn-"),
            tag.trim_start_matches("rct-"),
            "identical raw coordinates never collide across purposes",
        );
        assert!(a1.len() <= 64 && tag.len() <= 64);
    }

    #[test]
    fn record_round_trips_with_tagged_state_machines() {
        let notice = RunCompletionNotice {
            version: RUN_COMPLETION_NOTICE_VERSION,
            notice_id: "rcn-abc".to_string(),
            sequence: 7,
            tenant_id: "tenant-a".to_string(),
            owner_user_id: "user-a".to_string(),
            run_id: "run-1".to_string(),
            thread_id: "thread-1".to_string(),
            agent_id: Some("agent-a".to_string()),
            project_id: None,
            thread_tag: "rct-def".to_string(),
            terminal_projection_ref: "run-completion/rcn-abc".to_string(),
            completed_at: Utc::now(),
            delivery: CompletionDeliveryState::PendingArbitration {
                closes_at: Utc::now(),
                grants_issued: 0,
            },
            read: CompletionReadState::Unread,
            intents: Vec::new(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        let encoded = serde_json::to_value(&notice).expect("notice serializes");
        assert_eq!(encoded["delivery"]["state"], "pending_arbitration");
        assert_eq!(encoded["read"]["read"], "unread");
        let decoded: RunCompletionNotice =
            serde_json::from_value(encoded).expect("notice deserializes");
        assert_eq!(decoded, notice);
    }
}
