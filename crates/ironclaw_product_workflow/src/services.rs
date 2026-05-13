//! Port traits and outcome types for the non-`UserMessage` dispatch arms.
//!
//! Each non-`UserMessage` variant of [`ironclaw_product_adapters::ProductInboundPayload`]
//! routes through one of the traits in this module. The trait shapes are
//! Reborn-side facades: production wiring in `src/app.rs` (or a composition
//! crate) is expected to implement them by adapting existing host-layer
//! services (e.g. [`ironclaw_approvals::ApprovalResolver`],
//! [`ironclaw_authorization::CapabilityDispatchAuthorizer`],
//! [`ironclaw_outbound::OutboundPolicyService`]).
//!
//! The trait shapes align with the contract sketches from:
//! - #3094 (`ApprovalInteractionService`, `AuthInteractionService`)
//! - #3278 (`MissionService`, `MissionFireRequest`, `MissionFireOutcome`)
//! - #3280 (`ProductCommandRouter` seam; full matrix in #3286)
//! - #3266 (`ProjectionSubscriptionAuthority` wraps `OutboundPolicyService`)

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use ironclaw_product_adapters::{
    AdapterInstallationId, ApprovalDecision, AuthResolutionResult, ExternalActorRef,
    ExternalConversationRef, ExternalEventId, ProductAdapterId, ProductInboundEnvelope,
    ProductTriggerReason, ProjectionCursor, ProjectionSubscriptionRequest, RedactedString,
};
use ironclaw_turns::{LoopGateRef, TurnRunId};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::action::{AuthRequestRef, LinkedThreadActionId, ProductCommandName};
use crate::error::ProductWorkflowError;

// ---------------------------------------------------------------------------
// BeforeInbound hook port
// ---------------------------------------------------------------------------

/// Request passed to a [`BeforeInboundPolicy`] before an inbound user message
/// is staged into the session thread service.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BeforeInboundRequest {
    pub adapter_id: ProductAdapterId,
    pub installation_id: AdapterInstallationId,
    pub external_event_id: ExternalEventId,
    pub external_actor_ref: ExternalActorRef,
    pub external_conversation_ref: ExternalConversationRef,
    pub received_at: DateTime<Utc>,
    pub original_text: String,
    pub trigger: ProductTriggerReason,
}

/// Outcome of a [`BeforeInboundPolicy::evaluate_inbound`] call.
///
/// Mirrors the v1 `HookOutcome` shape (`src/hooks/hook.rs`): the policy can
/// either let the message through (optionally rewriting the text) or reject it
/// outright with a redacted reason. Rejection prevents staging into the
/// transcript and prevents any `TurnCoordinator::submit_turn` call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BeforeInboundOutcome {
    /// Pass the message through. If `rewritten_text` is `Some`, that text
    /// replaces the original before staging.
    Continue { rewritten_text: Option<String> },
    /// Reject the message. No transcript content, no turn submission. The
    /// idempotency ledger row settles as a redacted rejection.
    Reject { reason: RedactedString },
}

#[async_trait]
pub trait BeforeInboundPolicy: Send + Sync {
    async fn evaluate_inbound(
        &self,
        request: BeforeInboundRequest,
    ) -> Result<BeforeInboundOutcome, ProductWorkflowError>;
}

// ---------------------------------------------------------------------------
// ProductCommandRouter port
// ---------------------------------------------------------------------------

/// Outcome of a [`ProductCommandRouter::route_command`] call.
///
/// The first slice only proves that a command reached the seam. The full
/// command compatibility matrix (return-to-thread semantics, dedicated
/// commands like `/interrupt`, `/cancel`, `/model`) is owned by #3286.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProductCommandOutcome {
    /// Command was accepted and routed to its handler.
    Routed { command: ProductCommandName },
    /// Command is unknown to the router.
    UnknownCommand { command: ProductCommandName },
}

#[async_trait]
pub trait ProductCommandRouter: Send + Sync {
    async fn route_command(
        &self,
        envelope: &ProductInboundEnvelope,
        command: ProductCommandName,
        arguments: String,
    ) -> Result<ProductCommandOutcome, ProductWorkflowError>;
}

// ---------------------------------------------------------------------------
// Approval interaction service (port for ironclaw_approvals::ApprovalResolver)
// ---------------------------------------------------------------------------

/// Outcome of an [`ApprovalInteractionService::resolve_approval`] call.
///
/// Production implementations wrap [`ironclaw_approvals::ApprovalResolver`]
/// (`approve_dispatch` / `approve_spawn` / `deny`). The workflow does **not**
/// call `TurnCoordinator::resume_turn` directly — gate resume is the resolver's
/// job once the lease is minted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApprovalResolutionOutcome {
    /// Gate was found and the decision was applied.
    Handled { gate_ref: LoopGateRef },
    /// Gate ref is unknown, expired, or already settled. Surface as a redacted
    /// rejection so adapters cannot probe gate-ref existence.
    StaleOrUnknown,
}

#[async_trait]
pub trait ApprovalInteractionService: Send + Sync {
    async fn resolve_approval(
        &self,
        envelope: &ProductInboundEnvelope,
        gate_ref: LoopGateRef,
        decision: ApprovalDecision,
    ) -> Result<ApprovalResolutionOutcome, ProductWorkflowError>;
}

// ---------------------------------------------------------------------------
// Auth interaction service (port for ironclaw_authorization)
// ---------------------------------------------------------------------------

/// Outcome of an [`AuthInteractionService::resolve_auth`] call.
///
/// Production implementations wrap host-side auth flow management — for OAuth
/// callbacks, credential injection, or cancellation — and ultimately resume
/// the parked loop via the trusted approval/lease path, never directly through
/// `TurnCoordinator::resume_turn`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthResolutionOutcome {
    /// Auth request was found and the result was applied.
    Handled { auth_request_ref: AuthRequestRef },
    /// Auth request ref is unknown, expired, or already settled.
    StaleOrUnknown,
}

#[async_trait]
pub trait AuthInteractionService: Send + Sync {
    async fn resolve_auth(
        &self,
        envelope: &ProductInboundEnvelope,
        auth_request_ref: AuthRequestRef,
        result: AuthResolutionResult,
    ) -> Result<AuthResolutionOutcome, ProductWorkflowError>;
}

// ---------------------------------------------------------------------------
// Linked thread action service
// ---------------------------------------------------------------------------

/// Outcome of a [`LinkedThreadActionService::handle_action`] call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkedThreadActionOutcome {
    /// Action was routed to its handler.
    Routed { action_id: LinkedThreadActionId },
}

#[async_trait]
pub trait LinkedThreadActionService: Send + Sync {
    async fn handle_action(
        &self,
        envelope: &ProductInboundEnvelope,
        action_id: LinkedThreadActionId,
        data: Option<String>,
        reply_target_message_id: Option<String>,
    ) -> Result<LinkedThreadActionOutcome, ProductWorkflowError>;
}

// ---------------------------------------------------------------------------
// Mission service (per issue #3278)
// ---------------------------------------------------------------------------

/// Stable handle to a durable mission-fire record. Mints from the workflow
/// before delegating to a real `MissionService` so retries replay the same
/// reference even if the service implementation is unreachable.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct MissionFireRef(Uuid);

impl MissionFireRef {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    pub fn as_uuid(&self) -> Uuid {
        self.0
    }
}

impl Default for MissionFireRef {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for MissionFireRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Reason a mission fire was suppressed pre-submit (cadence, cooldown, dedup,
/// busy-thread policy). Names mirror #3278 `MissionFireOutcome` variants.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MissionFireSuppressionReason {
    Cadence,
    Cooldown,
    Deduplicated,
    BusyThread,
}

/// Reason a mission fire was rejected before any suppression evaluation
/// (mission unknown, scope mismatch, mission disabled).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MissionFireRejectionReason {
    UnknownMission,
    ScopeMismatch,
    MissionDisabled,
}

/// Request to fire a mission via [`MissionService::fire_mission`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MissionFireRequest {
    pub adapter_id: ProductAdapterId,
    pub installation_id: AdapterInstallationId,
    pub external_event_id: ExternalEventId,
    pub external_actor_ref: ExternalActorRef,
    pub external_conversation_ref: ExternalConversationRef,
    pub received_at: DateTime<Utc>,
    pub mission_intent: String,
    pub mission_id_hint: Option<String>,
    pub data: Option<String>,
}

/// Outcome of a [`MissionService::fire_mission`] call. Maps 1:1 to the
/// `MissionSubmitted` / `MissionSuppressed` / `Rejected` adapter-facing acks
/// in #3280 AC #13.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MissionFireOutcome {
    Submitted {
        mission_fire_ref: MissionFireRef,
        run_id: TurnRunId,
    },
    DeferredBusy {
        mission_fire_ref: MissionFireRef,
        active_run_id: TurnRunId,
    },
    Suppressed {
        mission_fire_ref: MissionFireRef,
        reason: MissionFireSuppressionReason,
    },
    Rejected {
        reason: MissionFireRejectionReason,
    },
}

#[async_trait]
pub trait MissionService: Send + Sync {
    async fn fire_mission(
        &self,
        request: MissionFireRequest,
    ) -> Result<MissionFireOutcome, ProductWorkflowError>;
}

// ---------------------------------------------------------------------------
// System action service
// ---------------------------------------------------------------------------

/// Outcome of a [`SystemActionService::handle_action`] call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SystemActionOutcome {
    /// System action was routed to its handler.
    Routed,
}

#[async_trait]
pub trait SystemActionService: Send + Sync {
    /// Handle a typed system action. Implementations must require an
    /// accountable `system_actor_ref` and a typed `kind` — there is no
    /// generic `is_internal` bypass.
    async fn handle_action(
        &self,
        envelope: &ProductInboundEnvelope,
        system_actor_ref: String,
        kind: String,
        scope_thread_id: Option<String>,
        data: Option<String>,
    ) -> Result<SystemActionOutcome, ProductWorkflowError>;
}

// ---------------------------------------------------------------------------
// Projection subscription authority (wraps OutboundPolicyService)
// ---------------------------------------------------------------------------

/// Request to authorize a projection subscription. Production implementations
/// wrap [`ironclaw_outbound::OutboundPolicyService::authorize_subscription`]
/// from #3542, translating the workflow-level request to the outbound
/// service's own request type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectionSubscriptionAuthorityRequest {
    pub adapter_id: ProductAdapterId,
    pub installation_id: AdapterInstallationId,
    pub external_event_id: ExternalEventId,
    pub external_actor_ref: ExternalActorRef,
    pub external_conversation_ref: ExternalConversationRef,
    pub thread_id_hint: Option<String>,
    pub after_cursor: Option<ProjectionCursor>,
}

#[async_trait]
pub trait ProjectionSubscriptionAuthority: Send + Sync {
    /// Authorize a projection subscription request and return the canonical
    /// [`ProjectionSubscriptionRequest`] the adapter should use for subsequent
    /// projection-stream calls. Implementations are responsible for resolving
    /// the binding, checking the projection access policy, and (if applicable)
    /// minting a cursor checkpoint via the underlying outbound policy service.
    async fn authorize_subscription(
        &self,
        request: ProjectionSubscriptionAuthorityRequest,
    ) -> Result<ProjectionSubscriptionRequest, ProductWorkflowError>;
}
