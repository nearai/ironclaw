//! Adapter-generic channel delivery machinery for immediate-ACK Reborn
//! webhooks: the final-reply delivery observer and the triggered-run delivery
//! driver.
//!
//! Push-channel webhooks must 2xx quickly, so the observer runs after the
//! workflow accepts an inbound message, waits for the submitted run to
//! finish, reads the finalized assistant reply, and sends it through the
//! host-mediated product outbound delivery seam. Everything channel-specific
//! (adapter, egress, sink, and the [`ChannelDeliveryProtocol`] details —
//! stored-ref decoding, DM classification, status messages) is injected by
//! the owning channel host; nothing here keys on a concrete channel.
use std::num::NonZeroUsize;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use ironclaw_outbound::{
    CommunicationPreferenceRepository, DeliveredGateRouteStore, OutboundStateStore,
    RunNotificationEventKind,
};
use ironclaw_product_adapters::{
    ExternalEventId, OutboundDeliverySink, ProductAdapter, ProductOutboundPayload,
    ProtocolHttpEgress,
};
use ironclaw_product_workflow::{
    AuthChallengeProvider, BlockedAuthFlowCanceller, ConversationBindingService,
};
use ironclaw_run_state::ApprovalRequestStore;
use ironclaw_threads::SessionThreadService;
use ironclaw_turns::{TurnCoordinator, TurnStatus};
use std::collections::{HashSet, VecDeque};

// The per-channel protocol seam this machinery is generic over lives in
// `ironclaw_channel_host` so channel host crates can implement it without a
// composition dependency; re-exported here so composition-internal consumers
// keep one canonical path to the delivery module's vocabulary.
pub use ironclaw_channel_host::delivery_protocol::{
    ChannelDeliveryProtocol, FinalReplyDeliveryError, PostedChannelMessage,
};
pub(crate) const MAX_RUN_POLL_INTERVAL: Duration = Duration::from_secs(5);
pub(crate) const DEFAULT_TRIGGERED_RUN_DELIVERY_MAX_WAIT: Duration = Duration::from_secs(30 * 60);
pub(crate) const RUN_POLL_JITTER_BUCKETS: u32 = 5;
pub(crate) const CHANNEL_WORKING_MESSAGE: &str = "Ironclaw is thinking...";
pub(crate) const CHANNEL_AUTH_CANCELED_MESSAGE: &str = "Authentication canceled.";
/// Posted when a run blocks on a credential-entry (non-OAuth) auth challenge:
/// entering a secret in chat is a security risk, so it must be done in the web app.
pub(crate) const CHANNEL_AUTH_UNAVAILABLE_MESSAGE: &str = "Setting this up needs a credential (an API key or token). Sharing one here is a security risk — anything entered in chat is stored in the conversation — so credential-based connections can only be set up in the Ironclaw web app. Connect it there, then ask me again here.";
pub(crate) const CHANNEL_DELIVERY_TIMEOUT_MESSAGE: &str =
    "This is taking longer than expected — check the WebUI for the result.";
pub(crate) const CHANNEL_DELIVERY_ERROR_MESSAGE: &str =
    "Something went wrong delivering the result here. Check the WebUI.";
/// Posted when the blocking run is `BlockedApproval` and no gate_ref is available.
pub(crate) const CHANNEL_BUSY_APPROVAL_MESSAGE: &str = "Ironclaw is waiting on a pending approval before taking new messages — reply `approve` or `deny` (or `approve gate:<ref>`) to resume.";
/// Posted for any other non-terminal blocking state, or when the state lookup fails.
pub(crate) const CHANNEL_BUSY_GENERIC_MESSAGE: &str = "Ironclaw is still working on a previous message and can't take this one yet — please resend it once the current task finishes.";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BlockedActionableMarker {
    pub(crate) status: TurnStatus,
    pub(crate) gate_ref: Option<String>,
}

pub(crate) struct ChannelActionableNotification {
    pub(crate) event_kind: RunNotificationEventKind,
    pub(crate) payload: ProductOutboundPayload,
    /// Gate ref for approval prompts on triggered runs; consumed by the
    /// delivered-gate route record so a DM reply can resolve the gate on the
    /// triggered run's thread. `None` for live-run notifications (same-thread
    /// replies need no routing) and non-approval and non-auth kinds.
    pub(crate) gate_ref_for_routing: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FinalReplyDeliverySettings {
    pub poll_interval: Duration,
    pub max_wait: Duration,
    pub max_concurrent_deliveries: NonZeroUsize,
    /// Bounds the total number of spawned delivery tasks (active + waiting for a
    /// delivery permit). When this limit is reached, new trigger fires are
    /// recorded as `Skipped` rather than spawning an unbounded waiting task.
    pub max_pending_deliveries: NonZeroUsize,
}

impl Default for FinalReplyDeliverySettings {
    fn default() -> Self {
        Self {
            poll_interval: Duration::from_millis(250),
            max_wait: Duration::from_secs(120),
            max_concurrent_deliveries: NonZeroUsize::new(64).unwrap_or(NonZeroUsize::MIN),
            max_pending_deliveries: NonZeroUsize::new(256).unwrap_or(NonZeroUsize::MIN),
        }
    }
}

pub struct FinalReplyDeliveryServices {
    /// Channel-specific protocol details (ref decoding, DM classification,
    /// status messages) — supplied by the owning channel host.
    pub channel_protocol: Arc<dyn ChannelDeliveryProtocol>,
    pub binding_service: Arc<dyn ConversationBindingService>,
    pub thread_service: Arc<dyn SessionThreadService>,
    pub turn_coordinator: Arc<dyn TurnCoordinator>,
    pub outbound_store: Arc<dyn OutboundStateStore>,
    pub route_store: Arc<dyn DeliveredGateRouteStore>,
    pub communication_preferences: Arc<dyn CommunicationPreferenceRepository>,
    pub adapter: Arc<dyn ProductAdapter>,
    pub egress: Arc<dyn ProtocolHttpEgress>,
    pub delivery_sink: Arc<dyn OutboundDeliverySink>,
    /// Resolves auth challenges for `BlockedAuth` runs. Only link-based OAuth
    /// challenges are surfaced in Slack; other challenge kinds are denied (see the
    /// `BlockedAuth` arm of `notification_for_actionable_state`).
    pub auth_challenges: Option<Arc<dyn AuthChallengeProvider>>,
    /// Cancels the durable `AuthFlow` record whenever a `BlockedAuth` run is
    /// auto-cancelled by the Slack delivery path. Threaded through the shared
    /// `cancel_auth_blocked_run` helper, so it covers every caller that cancels a
    /// blocked-auth run: the live observer non-OAuth deny arm, the triggered
    /// non-OAuth deny arm, and the OAuth send-time DM backstop. The Slack path
    /// cancels the run directly via `TurnCoordinator` (it does not go through the
    /// canonical `AuthInteractionService` deny path), which would otherwise leave
    /// the flow record non-terminal (#4952); this cancels the flow alongside the
    /// run, after the run cancel succeeds. `None` (e.g. no `flow_record_source`
    /// wired in) skips the flow cancel and still cancels the run — backward-compatible.
    pub auth_flow_canceller: Option<Arc<dyn BlockedAuthFlowCanceller>>,
    /// Store used to resolve an approval gate's request details (tool/action/reason)
    /// so the Slack approval prompt can say WHAT is being approved — the same
    /// source the WebUI projection reads. `None` disables the enrichment.
    pub approval_requests: Option<Arc<dyn ApprovalRequestStore>>,
}

/// Maximum number of (conversation, external_event_id) pairs remembered for hint dedup.
/// FIFO eviction beyond this cap keeps memory O(1); a false-negative after
/// eviction just means one extra hint, which is harmless.
pub(crate) const HINT_SEEN_CAP: usize = 256;

/// Throttle key for the busy-thread hint: one hint per (conversation fingerprint, external event id).
///
/// Using `ExternalEventId` instead of `TurnRunId` means:
/// - Transport retries of the **same** Slack event share the same `external_event_id`, so
///   they are deduplicated here — no duplicate hints on retries.
/// - Each **new** human message has a distinct `external_event_id`, so each new message
///   gets a fresh hint even if the same blocking run is still active.
pub(crate) type HintSeenKey = (String, ExternalEventId);
pub(crate) type HintSeenSet = Mutex<(VecDeque<HintSeenKey>, HashSet<HintSeenKey>)>;
