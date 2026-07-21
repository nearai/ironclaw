use serde::{Deserialize, Serialize};
use thiserror::Error;

use super::refs::{ResourceBudgetTier, RunProfileSourceLayer, RunProfileSourceRef};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SteeringPolicy {
    pub allow_steering: bool,
    pub allow_interrupt: bool,
    pub allow_driver_specific_nudges: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CancellationPolicy {
    pub allow_cancel: bool,
    pub require_checkpoint_before_cancel: bool,
}

impl CancellationPolicy {
    /// Cancellable without requiring a pre-cancel checkpoint — the shared
    /// interactive / legacy-interactive cancellation policy.
    pub(crate) fn interactive() -> Self {
        Self {
            allow_cancel: true,
            require_checkpoint_before_cancel: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckpointPolicy {
    pub require_before_model: bool,
    pub require_before_side_effect: bool,
    pub require_before_block: bool,
    pub max_checkpoint_bytes: u64,
    /// When true, terminal exits (Completed, Cancelled, Failed) require a
    /// final_checkpoint_id. Missing wire fields default to required; local/test
    /// profiles must explicitly relax the gate.
    #[serde(default = "default_require_final_checkpoint")]
    pub require_final_checkpoint: bool,
    /// `LoopCompletionKind::NoReply` is trusted only for profiles that
    /// explicitly permit no-reply completion.
    #[serde(default)]
    pub allow_no_reply_completion: bool,
}

fn default_require_final_checkpoint() -> bool {
    true
}

impl CheckpointPolicy {
    /// The interactive-coding tier checkpoint policy, shared by the interactive
    /// profile and its legacy-persisted reconstruction: gate before side effects
    /// and blocks (not before every model call), a 64 KiB cap, no final
    /// checkpoint, no no-reply completion.
    pub(crate) fn interactive() -> Self {
        Self {
            require_before_model: false,
            require_before_side_effect: true,
            require_before_block: true,
            max_checkpoint_bytes: 64 * 1024,
            require_final_checkpoint: false,
            allow_no_reply_completion: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResourceBudgetPolicy {
    pub tier: ResourceBudgetTier,
    pub max_model_calls: u32,
    pub max_capability_invocations: u32,
}

impl ResourceBudgetPolicy {
    /// Ceilings the privileged `mission_high` tier is clamped to when the caller
    /// lacks the `HighBudget` authority (the mission-standard budget bound).
    pub(crate) const MISSION_STANDARD_MAX_MODEL_CALLS: u32 = 128;
    pub(crate) const MISSION_STANDARD_MAX_CAPABILITY_INVOCATIONS: u32 = 512;

    /// The interactive-coding tier resource budget, shared by the interactive
    /// profile and its legacy-persisted reconstruction.
    pub(crate) fn interactive() -> Self {
        Self {
            tier: ResourceBudgetTier::from_trusted_static("interactive_standard"),
            max_model_calls: 32,
            max_capability_invocations: 64,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeProfileConstraints {
    pub allow_raw_runtime_backend_selection: bool,
    pub allow_broad_capability_surface: bool,
}

impl RuntimeProfileConstraints {
    /// No raw runtime-backend selection and no broad capability surface — the
    /// locked default every built-in profile uses.
    pub(crate) fn locked() -> Self {
        Self {
            allow_raw_runtime_backend_selection: false,
            allow_broad_capability_surface: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RedactedRunProfileProvenance {
    pub sources: Vec<RedactedRunProfileSource>,
    pub effective_privileges: Vec<PrivilegedRunProfileDimension>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RedactedRunProfileSource {
    pub layer: RunProfileSourceLayer,
    pub source_ref: RunProfileSourceRef,
    pub summary: String,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum RunProfileRequestAuthority {
    #[default]
    User,
    ProductSurface,
    Admin,
    System,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum PersonalContextAuthority {
    Direct,
    #[default]
    Shared,
}

impl RunProfileRequestAuthority {
    pub(super) fn allows(self, dimension: PrivilegedRunProfileDimension) -> bool {
        match dimension {
            PrivilegedRunProfileDimension::LongRunningMission
            | PrivilegedRunProfileDimension::SpecialDriver
            | PrivilegedRunProfileDimension::RunnerPool => {
                matches!(self, Self::ProductSurface | Self::Admin | Self::System)
            }
            PrivilegedRunProfileDimension::BroadCapabilitySurface
            | PrivilegedRunProfileDimension::HighBudget => {
                matches!(self, Self::Admin | Self::System)
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PrivilegedRunProfileDimension {
    LongRunningMission,
    BroadCapabilitySurface,
    HighBudget,
    SpecialDriver,
    RunnerPool,
}

impl PrivilegedRunProfileDimension {
    pub(super) fn category(self) -> &'static str {
        match self {
            Self::LongRunningMission => "long_running_mission",
            Self::BroadCapabilitySurface => "broad_capability_surface",
            Self::HighBudget => "high_budget",
            Self::SpecialDriver => "special_driver",
            Self::RunnerPool => "runner_pool",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum RunProfileResolutionError {
    #[error("run profile request is unauthorized for {dimension:?}")]
    Unauthorized {
        dimension: PrivilegedRunProfileDimension,
    },
    #[error("run profile is unavailable: {profile_id}")]
    ProfileUnavailable { profile_id: String },
    #[error("invalid run profile request: {reason}")]
    InvalidRequest { reason: String },
}
