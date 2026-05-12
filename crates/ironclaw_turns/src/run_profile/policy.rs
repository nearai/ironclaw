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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResourceBudgetPolicy {
    pub tier: ResourceBudgetTier,
    pub max_model_calls: u32,
    pub max_capability_invocations: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeProfileConstraints {
    pub allow_raw_runtime_backend_selection: bool,
    pub allow_broad_capability_surface: bool,
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
