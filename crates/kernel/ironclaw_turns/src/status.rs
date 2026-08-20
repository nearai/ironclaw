use ironclaw_host_api::output::OutputContract;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use ironclaw_host_api::{
    decision::RuntimeCredentialAuthRequirement,
    turn::{
        AcceptedMessageRef, CapabilityActivityId, EventCursor, RunProfileId, RunProfileVersion,
        SanitizedFailure, TurnActor, TurnCheckpointId, TurnGateRef, TurnId, TurnRunId, TurnScope,
        TurnStatus,
    },
};

use crate::{ProductTurnContext, TurnAdmissionClass, request::TurnTimestamp};
use ironclaw_loop_contracts::{LoopModelRouteSnapshot, ResolvedRunProfile};

/// The recoverability-critical transition boundary (#6263 Step 3 / #6284 / Step 5b).
///
/// A run status is **recoverability-critical** when it is a gate-park
/// ([`TurnStatus::is_blocked`]), a terminal ([`TurnStatus::is_terminal`]), or a
/// [`TurnStatus::CancelRequested`]:
///
/// * losing a gate-park strands a run away from the human who must act on it;
/// * losing a terminal re-runs an already-performed side effect, or loses the
///   sanitized, model-visible failure cause the model must see;
/// * losing a `CancelRequested` re-runs work the caller was told was cancelled:
///   `request_cancel` reports success once the transition is committed, so a
///   write-behind crash that reverts it to `Running`/`Queued` would execute a
///   run the caller successfully cancelled (and drop its idempotency record).
///   The caller is waiting on this transition exactly as on a gate-park.
///
/// These transitions MUST stay synchronously durable even under async
/// write-behind: the async path may move only NON-critical transitions off the
/// synchronous ack. The row store's `delta_is_recoverability_critical` also
/// treats a brand-new run (one `baseline` has never seen — `submit_turn`,
/// `submit_child_turn`, and the runs `resume_turn`/`retry_turn` spawn) as
/// critical: it has no durable fallback to recover from if lost. The
/// crash-consistency suite references THIS function (not a copy) as the
/// single boundary write-behind flips.
pub fn is_recoverability_critical(status: TurnStatus) -> bool {
    status.is_blocked() || status.is_terminal() || matches!(status, TurnStatus::CancelRequested)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TurnActiveRunRefState {
    Missing,
    Nonterminal,
    Terminal,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TurnRunProfile {
    pub id: RunProfileId,
    pub version: RunProfileVersion,
    #[serde(default = "TurnAdmissionClass::interactive")]
    pub admission_class: TurnAdmissionClass,
    pub allow_steering: bool,
    pub auto_queue_followups: bool,
    pub resolved: ResolvedRunProfile,
}

impl TurnRunProfile {
    pub fn from_resolved(resolved: ResolvedRunProfile) -> Self {
        let id = compatibility_profile_id(&resolved);
        Self {
            id,
            version: resolved.profile_version,
            admission_class: TurnAdmissionClass::interactive(),
            allow_steering: resolved.steering_policy.allow_steering,
            auto_queue_followups: false,
            resolved,
        }
    }
}

impl<'de> Deserialize<'de> for TurnRunProfile {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct WireProfile {
            id: RunProfileId,
            version: RunProfileVersion,
            #[serde(default = "TurnAdmissionClass::interactive")]
            admission_class: TurnAdmissionClass,
            allow_steering: bool,
            auto_queue_followups: bool,
            resolved: Option<ResolvedRunProfile>,
        }

        let wire = WireProfile::deserialize(deserializer)?;
        let resolved = wire.resolved.unwrap_or_else(|| {
            ResolvedRunProfile::legacy_compatibility(
                wire.id.clone(),
                wire.version,
                wire.allow_steering,
            )
        });
        Ok(Self {
            id: wire.id,
            version: wire.version,
            admission_class: wire.admission_class,
            allow_steering: wire.allow_steering,
            auto_queue_followups: wire.auto_queue_followups,
            resolved,
        })
    }
}

fn compatibility_profile_id(resolved: &ResolvedRunProfile) -> RunProfileId {
    if resolved.profile_id.is_interactive_default() {
        RunProfileId::default_profile()
    } else {
        resolved.profile_id.clone()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AdmissionRejectionReason {
    TenantLimit,
    ProfileRejected,
    Policy,
    Unauthorized,
    Unavailable,
    /// The thread's consecutive autonomous-wake budget is exhausted. Distinct
    /// from the other reasons because nothing is wrong with the request or the
    /// caller — the thread is simply parked pending human attention, and a
    /// caller may retry after a human activates it.
    SystemWakeStreak,
}

impl AdmissionRejectionReason {
    pub fn category(self) -> &'static str {
        match self {
            Self::TenantLimit => "tenant_limit",
            Self::ProfileRejected => "profile_rejected",
            Self::Policy => "policy",
            Self::Unauthorized => "unauthorized",
            Self::Unavailable => "unavailable",
            Self::SystemWakeStreak => "system_wake_streak",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdmissionRejection {
    pub reason: AdmissionRejectionReason,
    pub retry_after_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capacity_denial: Option<crate::TurnAdmissionCapacityDenial>,
}

impl AdmissionRejection {
    pub fn new(reason: AdmissionRejectionReason) -> Self {
        Self {
            reason,
            retry_after_ms: None,
            capacity_denial: None,
        }
    }

    pub fn with_retry_after_ms(mut self, retry_after_ms: u64) -> Self {
        self.retry_after_ms = Some(retry_after_ms);
        self
    }

    pub fn with_capacity_denial(mut self, denial: crate::TurnAdmissionCapacityDenial) -> Self {
        self.retry_after_ms = denial.retry_after_ms;
        self.capacity_denial = Some(denial);
        self
    }
}

fn steering_allowed_default() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TurnRunState {
    pub scope: TurnScope,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor: Option<TurnActor>,
    pub turn_id: TurnId,
    pub run_id: TurnRunId,
    pub status: TurnStatus,
    pub accepted_message_ref: AcceptedMessageRef,
    pub resolved_run_profile_id: RunProfileId,
    pub resolved_run_profile_version: RunProfileVersion,
    /// Immutable terminal output contract admitted for this run. Defaults to
    /// the historical assistant-message result for legacy state snapshots.
    #[serde(default, skip_serializing_if = "OutputContract::is_assistant_message")]
    pub output_contract: OutputContract,
    /// Whether the resolved run profile admits mid-run steering input
    /// (`SteeringPolicy::allow_steering`, snapshotted at submit resolution).
    /// The busy-submit enqueue gateway consults this before queueing a message
    /// for the active run; `false` falls back to the `RejectedBusy` outcome.
    /// Legacy persisted rows predate the field and default to allowed.
    #[serde(default = "steering_allowed_default")]
    pub allow_steering: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved_model_route: Option<LoopModelRouteSnapshot>,
    /// Cumulative provider-reported token usage for this run's model calls,
    /// captured at loop exit. `None` for runs that reported no usage (replay
    /// stubs) or that pre-date usage capture. Read by the OpenAI-compatible
    /// Responses/Chat surfaces to report `usage` and cost.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_usage: Option<ironclaw_loop_contracts::LoopModelUsage>,
    /// Semantic result of trusted completion settlement. Separate from
    /// notification/transport delivery state; absent on nonterminal and
    /// legacy persisted rows.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution_outcome: Option<crate::TurnExecutionOutcome>,
    pub received_at: TurnTimestamp,
    pub checkpoint_id: Option<TurnCheckpointId>,
    pub gate_ref: Option<TurnGateRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blocked_activity_id: Option<CapabilityActivityId>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub credential_requirements: Vec<RuntimeCredentialAuthRequirement>,
    pub failure: Option<SanitizedFailure>,
    pub event_cursor: EventCursor,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub product_context: Option<ProductTurnContext>,
    #[serde(
        rename = "auth_resume_disposition",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub resume_disposition: Option<crate::GateResumeDisposition>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TurnErrorCategory {
    ThreadBusy,
    AdmissionRejected,
    ScopeNotFound,
    Unauthorized,
    InvalidRequest,
    Unavailable,
    Conflict,
    CapacityExceeded,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TurnCapacityResource {
    SpawnTreeDescendants,
    SubmitTurn,
    #[serde(other)]
    Replayed,
}

impl TurnCapacityResource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::SpawnTreeDescendants => "spawn_tree_descendants",
            Self::SubmitTurn => "submit_turn",
            Self::Replayed => "replayed",
        }
    }
}

impl std::fmt::Display for TurnCapacityResource {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum TurnError {
    #[error("thread already has an active run")]
    ThreadBusy(crate::response::ThreadBusy),
    #[error("turn admission rejected: {0:?}")]
    AdmissionRejected(AdmissionRejection),
    #[error("turn run not found")]
    ScopeNotFound,
    #[error("turn request is unauthorized")]
    Unauthorized,
    #[error("invalid turn request: {reason}")]
    InvalidRequest { reason: String },
    #[error("turn service unavailable: {reason}")]
    Unavailable { reason: String },
    #[error("turn conflict: {reason}")]
    Conflict { reason: String },
    #[error("turn capacity exceeded for {resource}: cap {cap}")]
    CapacityExceeded {
        resource: TurnCapacityResource,
        cap: u64,
    },
    #[error("turn run {run_id} is not retryable")]
    RunNotRetryable { run_id: TurnRunId },
    #[error("invalid turn transition from {from:?} to {to:?}")]
    InvalidTransition { from: TurnStatus, to: TurnStatus },
    #[error("turn run lease mismatch")]
    LeaseMismatch,
    // Coordinator-level rejection of an unusable run-origin adapter. Callers
    // that build the value themselves surface
    // `ironclaw_host_api::turn::RunOriginAdapter::INVALID_MESSAGE` directly;
    // this variant keeps the same wording so both paths read identically.
    #[error("invalid run-origin adapter: must be 1..=512 bytes")]
    InvalidRunOriginAdapter,
}

impl TurnError {
    pub fn category(&self) -> TurnErrorCategory {
        match self {
            Self::ThreadBusy(_) => TurnErrorCategory::ThreadBusy,
            Self::AdmissionRejected(rejection) => match rejection.reason {
                AdmissionRejectionReason::TenantLimit => TurnErrorCategory::AdmissionRejected,
                AdmissionRejectionReason::ProfileRejected => TurnErrorCategory::InvalidRequest,
                AdmissionRejectionReason::Policy | AdmissionRejectionReason::Unauthorized => {
                    TurnErrorCategory::Unauthorized
                }
                AdmissionRejectionReason::Unavailable => TurnErrorCategory::Unavailable,
                // Capacity-shaped, not caller error: the thread's autonomous
                // budget is spent and a human activation restores it, which is
                // the same "retry later, nothing is malformed" shape the
                // tenant-limit rejection carries.
                AdmissionRejectionReason::SystemWakeStreak => TurnErrorCategory::AdmissionRejected,
            },
            Self::ScopeNotFound => TurnErrorCategory::ScopeNotFound,
            Self::Unauthorized => TurnErrorCategory::Unauthorized,
            Self::InvalidRequest { .. } => TurnErrorCategory::InvalidRequest,
            Self::Unavailable { .. } => TurnErrorCategory::Unavailable,
            Self::Conflict { .. }
            | Self::RunNotRetryable { .. }
            | Self::InvalidTransition { .. }
            | Self::LeaseMismatch => TurnErrorCategory::Conflict,
            Self::CapacityExceeded { .. } => TurnErrorCategory::CapacityExceeded,
            Self::InvalidRunOriginAdapter => TurnErrorCategory::InvalidRequest,
        }
    }

    pub fn capacity_exceeded(resource: TurnCapacityResource, cap: u64) -> Self {
        Self::CapacityExceeded { resource, cap }
    }

    pub fn is_expected_admission_outcome(&self) -> bool {
        matches!(self, Self::ThreadBusy(_) | Self::AdmissionRejected(_))
    }

    pub fn adapter_status_code(&self) -> u16 {
        match self.category() {
            TurnErrorCategory::ThreadBusy | TurnErrorCategory::Conflict => 409,
            TurnErrorCategory::AdmissionRejected => 429,
            TurnErrorCategory::CapacityExceeded => 429,
            TurnErrorCategory::ScopeNotFound => 404,
            TurnErrorCategory::Unauthorized => 403,
            TurnErrorCategory::InvalidRequest => 400,
            TurnErrorCategory::Unavailable => 503,
        }
    }
}
