use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{
    AcceptedMessageRef, GateRef, ReplyTargetBindingRef, ResolvedRunProfile, RunProfileId,
    RunProfileVersion, SourceBindingRef, TurnActor, TurnAdmissionClass, TurnCheckpointId, TurnId,
    TurnRunId, TurnScope, events::EventCursor, request::TurnTimestamp,
    run_profile::LoopModelRouteSnapshot,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TurnStatus {
    Queued,
    Running,
    BlockedApproval,
    BlockedAuth,
    BlockedResource,
    /// Blocked awaiting an attested-signing ceremony. Resume validates the
    /// untrusted attestation claim through the injected `AttestedResumePort`
    /// and, on success, transitions to [`Self::AttestedResolved`] — never back
    /// onto the normal agent-loop queue.
    BlockedAttested,
    /// An attested-signing gate has been validated and the run is handed off to
    /// a straight-line signer continuation in the reborn/runner layer. This is
    /// the deterministic terminal-of-turns transition for a resolved attested
    /// gate: the crypto-free store sets this status only, and never invokes a
    /// signer or performs chain I/O itself.
    AttestedResolved,
    CancelRequested,
    Cancelled,
    Completed,
    Failed,
    RecoveryRequired,
}

impl TurnStatus {
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Cancelled | Self::Completed | Self::Failed)
    }

    pub fn keeps_active_lock(self) -> bool {
        !self.is_terminal()
    }
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum BlockedReason {
    Approval {
        gate_ref: GateRef,
    },
    Auth {
        gate_ref: GateRef,
    },
    Resource {
        gate_ref: GateRef,
    },
    /// Blocked awaiting an attested-signing ceremony. Carries the opaque
    /// expected-transaction-hash binding alongside the gate so that, on resume,
    /// the store can hand the binding to the injected `AttestedResumePort`. The
    /// binding is metadata only — turns never verifies it.
    Attested {
        gate_ref: GateRef,
        expected_tx_hash: crate::ApprovedTxHashRef,
    },
}

impl BlockedReason {
    pub fn status(&self) -> TurnStatus {
        match self {
            Self::Approval { .. } => TurnStatus::BlockedApproval,
            Self::Auth { .. } => TurnStatus::BlockedAuth,
            Self::Resource { .. } => TurnStatus::BlockedResource,
            Self::Attested { .. } => TurnStatus::BlockedAttested,
        }
    }

    pub fn gate_ref(&self) -> &GateRef {
        match self {
            Self::Approval { gate_ref }
            | Self::Auth { gate_ref }
            | Self::Resource { gate_ref }
            | Self::Attested { gate_ref, .. } => gate_ref,
        }
    }

    /// The opaque expected-transaction-hash binding, present only for an
    /// attested-signing gate.
    pub fn expected_tx_hash(&self) -> Option<&crate::ApprovedTxHashRef> {
        match self {
            Self::Attested {
                expected_tx_hash, ..
            } => Some(expected_tx_hash),
            Self::Approval { .. } | Self::Auth { .. } | Self::Resource { .. } => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SanitizedFailure {
    category: String,
}

impl SanitizedFailure {
    pub fn new(category: impl Into<String>) -> Result<Self, String> {
        let category = category.into();
        validate_sanitized_category("failure_category", &category)?;
        Ok(Self { category })
    }

    pub(crate) fn from_trusted_static(category: &'static str) -> Self {
        debug_assert!(validate_sanitized_category("failure_category", category).is_ok());
        Self {
            category: category.to_string(),
        }
    }

    pub fn category(&self) -> &str {
        &self.category
    }

    pub fn into_category(self) -> String {
        self.category
    }
}

impl<'de> Deserialize<'de> for SanitizedFailure {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct WireFailure {
            category: String,
        }

        let wire = WireFailure::deserialize(deserializer)?;
        Self::new(wire.category).map_err(serde::de::Error::custom)
    }
}

fn validate_sanitized_category(kind: &'static str, value: &str) -> Result<(), String> {
    if value.is_empty() {
        return Err(format!("{kind} must not be empty"));
    }
    if value.len() > 256 {
        return Err(format!("{kind} must be at most 256 bytes"));
    }
    if value.chars().any(|c| c == '\0' || c.is_control()) {
        return Err(format!("{kind} must not contain control characters"));
    }
    if !value
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
    {
        return Err(format!(
            "{kind} must contain only lowercase ASCII letters, digits, or underscores"
        ));
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SanitizedCancelReason {
    UserRequested,
    Superseded,
    Timeout,
    OperatorRequested,
    Policy,
}

impl SanitizedCancelReason {
    pub fn category(self) -> &'static str {
        match self {
            Self::UserRequested => "user_requested",
            Self::Superseded => "superseded",
            Self::Timeout => "timeout",
            Self::OperatorRequested => "operator_requested",
            Self::Policy => "policy",
        }
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
}

impl AdmissionRejectionReason {
    pub fn category(self) -> &'static str {
        match self {
            Self::TenantLimit => "tenant_limit",
            Self::ProfileRejected => "profile_rejected",
            Self::Policy => "policy",
            Self::Unauthorized => "unauthorized",
            Self::Unavailable => "unavailable",
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TurnRunState {
    pub scope: TurnScope,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor: Option<TurnActor>,
    pub turn_id: TurnId,
    pub run_id: TurnRunId,
    pub status: TurnStatus,
    pub accepted_message_ref: AcceptedMessageRef,
    pub source_binding_ref: SourceBindingRef,
    pub reply_target_binding_ref: ReplyTargetBindingRef,
    pub resolved_run_profile_id: RunProfileId,
    pub resolved_run_profile_version: RunProfileVersion,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved_model_route: Option<LoopModelRouteSnapshot>,
    pub received_at: TurnTimestamp,
    pub checkpoint_id: Option<TurnCheckpointId>,
    pub gate_ref: Option<GateRef>,
    /// Opaque expected-transaction-hash binding for a `BlockedAttested` gate.
    /// Persisted alongside `gate_ref` when an attested gate is raised so resume
    /// can pass it to the injected `AttestedResumePort`. `None` for every other
    /// blocked reason.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_tx_hash: Option<crate::ApprovedTxHashRef>,
    pub failure: Option<SanitizedFailure>,
    pub event_cursor: EventCursor,
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
    #[error("invalid turn transition from {from:?} to {to:?}")]
    InvalidTransition { from: TurnStatus, to: TurnStatus },
    #[error("turn run lease mismatch")]
    LeaseMismatch,
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
            },
            Self::ScopeNotFound => TurnErrorCategory::ScopeNotFound,
            Self::Unauthorized => TurnErrorCategory::Unauthorized,
            Self::InvalidRequest { .. } => TurnErrorCategory::InvalidRequest,
            Self::Unavailable { .. } => TurnErrorCategory::Unavailable,
            Self::Conflict { .. } | Self::InvalidTransition { .. } | Self::LeaseMismatch => {
                TurnErrorCategory::Conflict
            }
        }
    }

    pub fn is_expected_admission_outcome(&self) -> bool {
        matches!(self, Self::ThreadBusy(_) | Self::AdmissionRejected(_))
    }

    pub fn adapter_status_code(&self) -> u16 {
        match self.category() {
            TurnErrorCategory::ThreadBusy | TurnErrorCategory::Conflict => 409,
            TurnErrorCategory::AdmissionRejected => 429,
            TurnErrorCategory::ScopeNotFound => 404,
            TurnErrorCategory::Unauthorized => 403,
            TurnErrorCategory::InvalidRequest => 400,
            TurnErrorCategory::Unavailable => 503,
        }
    }
}
