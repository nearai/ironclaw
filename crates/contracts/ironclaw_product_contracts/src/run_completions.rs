//! Run-completion notification wire vocabulary (2026-08-13 design §7.6–§7.8).
//!
//! One immutable completion fact per eligible run, projected to the browser
//! through the `RunCompletions` logical stream and mutated only over
//! authenticated HTTP operations. Everything here is metadata: notice IDs,
//! typed thread IDs, opaque purpose-separated tags, sequence members,
//! timestamps, and bounded counts. No reply text, prompt, thread title,
//! actor/project name, tool name, failure detail, or arbitrary URL ever
//! enters this vocabulary — the browser joins thread titles from data it
//! already fetched through ordinary access checks.
//!
//! Declared here (not in the product crate) per the transport/product
//! boundary, the same placement as [`crate::notification_setup`]; the notice
//! store, arbitration coordinator, and push facade stay in the product
//! crate.

use serde::{Deserialize, Serialize};

use crate::descriptors::{ProductSurfaceCommandDescriptor, ProductView};

/// Byte bound on every opaque identifier in this vocabulary (notice IDs,
/// grant IDs, browser instance IDs, tab IDs, collapse tags, sequence
/// members). Purpose-separated digests and UUIDs fit comfortably; anything
/// larger is rejected at the admission seam.
pub const RUN_COMPLETION_OPAQUE_ID_MAX_BYTES: usize = 128;

/// Bound on one bounded unread/rebase snapshot (§5.4).
pub const RUN_COMPLETION_UNREAD_SNAPSHOT_LIMIT: usize = 250;

/// Bound on concurrently retained browser-profile intents per notice (§5.4).
pub const RUN_COMPLETION_MAX_INTENTS_PER_NOTICE: usize = 32;

/// Schema tags stamped on the stream payloads.
pub const RUN_COMPLETION_NOTICE_SCHEMA: &str = "webui.run_completion.v1";
pub const RUN_COMPLETION_GRANT_SCHEMA: &str = "webui.run_completion_grant.v1";
pub const RUN_COMPLETION_CLEAR_SCHEMA: &str = "webui.run_completion_clear.v1";

/// The closed run-completion logical stream vocabulary. `Notice` opens or
/// replays arbitration state, `Grant` names the one selected browser
/// profile and surface, `Clear` tells every connected page and worker
/// ledger to dismiss local surfaces after a durable read transition. None
/// of these variants is a product mutation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RunCompletionStreamEvent {
    Notice(RunCompletionNoticeEvent),
    Grant(RunCompletionGrantEvent),
    Clear(RunCompletionClearEvent),
}

/// One completion notice (or its replay) on the owner-scoped stream.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunCompletionNoticeEvent {
    /// Always [`RUN_COMPLETION_NOTICE_SCHEMA`].
    pub schema: String,
    /// Opaque member of the owner's completion sequence; also the stream's
    /// resume cursor domain.
    pub sequence: String,
    pub notice_id: String,
    pub run_id: String,
    pub thread_id: String,
    /// Opaque purpose-separated collapse digest of owner scope and thread —
    /// the OS notification tag; never a display string or authorization
    /// token.
    pub thread_tag: String,
    /// RFC3339 completion timestamp.
    pub completed_at: String,
    pub read: bool,
    /// Bounded count of unread completions for this thread, for grouped
    /// badges and generic plural copy.
    pub unread_count_for_thread: u16,
}

/// Surfaces a grant may name (§5.6). `NoSurfaceWatchingThread` leases the
/// focused tab time to confirm the exact reply rendered; `InApp` posts one
/// toast in one deterministic tab; `LocalOs` presents through
/// `ServiceWorkerRegistration.showNotification`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunCompletionGrantSurface {
    NoSurfaceWatchingThread,
    InApp,
    LocalOs,
}

/// One presentation grant naming exactly one browser profile and surface.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunCompletionGrantEvent {
    /// Always [`RUN_COMPLETION_GRANT_SCHEMA`].
    pub schema: String,
    /// Sequence member for stream resume; grants ride the same owner-scoped
    /// cursor domain as notices.
    pub sequence: String,
    pub notice_id: String,
    pub grant_id: String,
    /// The one browser profile allowed to apply this grant. Every connected
    /// page may receive the event; only the named worker acts on it.
    pub browser_instance_id: String,
    /// The browser state revision the grant was issued against; the worker
    /// rejects the grant as stale when its current revision is newer and
    /// incompatible.
    pub state_revision: u64,
    pub surface: RunCompletionGrantSurface,
    /// RFC3339 grant expiry; expiry triggers one re-arbitration.
    pub expires_at: String,
}

/// A durable read transition: every page and worker ledger dismisses local
/// surfaces for the notice (and closes OS notifications by thread tag).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunCompletionClearEvent {
    /// Always [`RUN_COMPLETION_CLEAR_SCHEMA`].
    pub schema: String,
    /// Sequence member for stream resume.
    pub sequence: String,
    pub notice_id: String,
    pub thread_id: String,
    /// The thread's collapse tag, so sleeping workers can close OS
    /// notifications without a second lookup.
    pub thread_tag: String,
    /// RFC3339 read timestamp.
    pub read_at: String,
}

/// Presentation intents a browser profile may offer for one notice, in
/// priority order (§5.6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunCompletionIntentKind {
    /// A focused, visible tab on the thread confirms the exact reply
    /// rendered.
    ReplyObserved,
    /// A focused, visible tab on the thread expects the reply to render
    /// before the window closes.
    WatchingThread,
    /// A focused, visible tab elsewhere can show one in-app toast.
    InApp,
    /// Tabs exist but none is focused; the profile can present an OS
    /// notification if permission, enrollment, and target selection allow.
    LocalOs,
    /// This profile cannot present now.
    Unavailable,
}

/// `webui.run_completion.intent.v1` input (§7.8). The server derives
/// user/tenant authority from the bound caller and rejects a foreign notice
/// as `NotFound`; client claims cannot mint permission or a target.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunCompletionIntentRequest {
    pub notice_id: String,
    pub browser_instance_id: String,
    pub tab_id: String,
    /// Monotonically increasing browser state revision (§4).
    pub state_revision: u64,
    /// Focus epoch for deterministic equal-candidate tie-breaks.
    pub focus_epoch: u64,
    pub intent: RunCompletionIntentKind,
}

/// Terminal outcomes a browser reports for one grant (§7.8).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunCompletionAcknowledgeOutcome {
    /// The exact finalized reply rendered in a focused, visible tab: read
    /// evidence, settles the notice.
    ReplyRendered,
    /// The granted surface was presented (still unread).
    Presented,
    /// The worker's current state is newer and incompatible with the grant;
    /// one re-arbitration follows.
    StaleState,
    /// The browser effect failed; fallback proceeds without a false
    /// suppressed-surface state.
    EffectFailed,
}

/// `webui.run_completion.acknowledge.v1` input (§7.8).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunCompletionAcknowledgeRequest {
    pub notice_id: String,
    pub grant_id: String,
    pub state_revision: u64,
    pub outcome: RunCompletionAcknowledgeOutcome,
}

/// `webui.run_completion.thread_read.v1` input (§7.8): the greatest
/// completion sequence whose finalized reply the focused view has rendered.
/// The server advances only through notices that exist, belong to the
/// caller, target this thread, and have finalized replies at or below the
/// supplied sequence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunCompletionThreadReadRequest {
    pub thread_id: String,
    pub through_sequence: String,
    pub browser_instance_id: String,
}

/// Sanitized acknowledgement/settlement echo for the three write
/// operations. Carries only the affected notice IDs so callers can settle
/// local ledgers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunCompletionMutationResponse {
    pub settled_notice_ids: Vec<String>,
}

/// `webui.run-completions.unread.v1` params: the bounded unread/unsettled
/// snapshot for boot recovery and stream rebase.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunCompletionUnreadRequest {}

/// The bounded unread snapshot (§5.4: at most
/// [`RUN_COMPLETION_UNREAD_SNAPSHOT_LIMIT`] notices, grouped by thread in
/// the UI).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunCompletionUnreadResponse {
    pub notices: Vec<RunCompletionNoticeEvent>,
    /// The cursor a stream subscription resumes from after consuming this
    /// snapshot.
    pub resume_sequence: String,
}

/// Frozen operation IDs (§7.8). The concrete descriptor constants live here
/// with the other generic notification descriptors; behavior stays behind
/// the product surface.
pub const RUN_COMPLETION_INTENT_COMMAND_ID: &str = "webui.run_completion.intent.v1";
pub const RUN_COMPLETION_INTENT_COMMAND: ProductSurfaceCommandDescriptor<
    RunCompletionIntentRequest,
    RunCompletionMutationResponse,
> = ProductSurfaceCommandDescriptor::new(RUN_COMPLETION_INTENT_COMMAND_ID);

pub const RUN_COMPLETION_ACKNOWLEDGE_COMMAND_ID: &str = "webui.run_completion.acknowledge.v1";
pub const RUN_COMPLETION_ACKNOWLEDGE_COMMAND: ProductSurfaceCommandDescriptor<
    RunCompletionAcknowledgeRequest,
    RunCompletionMutationResponse,
> = ProductSurfaceCommandDescriptor::new(RUN_COMPLETION_ACKNOWLEDGE_COMMAND_ID);

pub const RUN_COMPLETION_THREAD_READ_COMMAND_ID: &str = "webui.run_completion.thread_read.v1";
pub const RUN_COMPLETION_THREAD_READ_COMMAND: ProductSurfaceCommandDescriptor<
    RunCompletionThreadReadRequest,
    RunCompletionMutationResponse,
> = ProductSurfaceCommandDescriptor::new(RUN_COMPLETION_THREAD_READ_COMMAND_ID);

pub const RUN_COMPLETION_UNREAD_VIEW_ID: &str = "webui.run-completions.unread.v1";
pub const RUN_COMPLETION_UNREAD_VIEW: ProductView<
    RunCompletionUnreadRequest,
    RunCompletionUnreadResponse,
> = ProductView::unpaginated(RUN_COMPLETION_UNREAD_VIEW_ID);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stream_event_wire_shapes_are_tagged_and_schema_stamped() {
        let notice = RunCompletionStreamEvent::Notice(RunCompletionNoticeEvent {
            schema: RUN_COMPLETION_NOTICE_SCHEMA.to_string(),
            sequence: "42".to_string(),
            notice_id: "notice-1".to_string(),
            run_id: "run-1".to_string(),
            thread_id: "thread-1".to_string(),
            thread_tag: "tag-1".to_string(),
            completed_at: "2026-08-13T00:00:00Z".to_string(),
            read: false,
            unread_count_for_thread: 2,
        });
        let encoded = serde_json::to_value(&notice).expect("notice serializes");
        assert_eq!(encoded["type"], "notice");
        assert_eq!(encoded["schema"], "webui.run_completion.v1");
        assert_eq!(encoded["unread_count_for_thread"], 2);
        let decoded: RunCompletionStreamEvent =
            serde_json::from_value(encoded).expect("notice deserializes");
        assert_eq!(decoded, notice);

        let grant = RunCompletionStreamEvent::Grant(RunCompletionGrantEvent {
            schema: RUN_COMPLETION_GRANT_SCHEMA.to_string(),
            sequence: "43".to_string(),
            notice_id: "notice-1".to_string(),
            grant_id: "grant-1".to_string(),
            browser_instance_id: "browser-1".to_string(),
            state_revision: 41,
            surface: RunCompletionGrantSurface::InApp,
            expires_at: "2026-08-13T00:00:02Z".to_string(),
        });
        let encoded = serde_json::to_value(&grant).expect("grant serializes");
        assert_eq!(encoded["type"], "grant");
        assert_eq!(encoded["surface"], "in_app");
        let decoded: RunCompletionStreamEvent =
            serde_json::from_value(encoded).expect("grant deserializes");
        assert_eq!(decoded, grant);
    }

    #[test]
    fn write_operation_inputs_round_trip_with_closed_vocabularies() {
        let intent: RunCompletionIntentRequest = serde_json::from_value(serde_json::json!({
            "notice_id": "notice-1",
            "browser_instance_id": "browser-1",
            "tab_id": "tab-1",
            "state_revision": 41,
            "focus_epoch": 9,
            "intent": "reply_observed",
        }))
        .expect("intent parses");
        assert_eq!(intent.intent, RunCompletionIntentKind::ReplyObserved);

        let acknowledge: RunCompletionAcknowledgeRequest =
            serde_json::from_value(serde_json::json!({
                "notice_id": "notice-1",
                "grant_id": "grant-1",
                "state_revision": 41,
                "outcome": "stale_state",
            }))
            .expect("acknowledgement parses");
        assert_eq!(
            acknowledge.outcome,
            RunCompletionAcknowledgeOutcome::StaleState
        );

        assert!(
            serde_json::from_value::<RunCompletionIntentRequest>(serde_json::json!({
                "notice_id": "notice-1",
                "browser_instance_id": "browser-1",
                "tab_id": "tab-1",
                "state_revision": 41,
                "focus_epoch": 9,
                "intent": "grab_the_push_target",
            }))
            .is_err(),
            "unknown intent kinds are rejected, not defaulted",
        );
    }
}
