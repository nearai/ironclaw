//! Durable agent-turn metadata carried by neutral process snapshots.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{
    AcceptedMessageRef, GateResumeDisposition, ProductTurnContext, RunProfileId, RunProfileVersion,
    TurnActor, TurnRunRecord, TurnRunState, runner::ClaimedTurnRun,
};
use ironclaw_host_api::turn::TurnExecutionOutcome;
use ironclaw_loop_contracts::{LoopModelRouteSnapshot, LoopModelUsage, ResolvedRunProfile};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentTurnProcessMetadata {
    pub turn_id: crate::TurnId,
    pub accepted_message_ref: AcceptedMessageRef,
    pub resolved_run_profile_id: RunProfileId,
    pub resolved_run_profile_version: RunProfileVersion,
    /// Snapshot of the resolved profile's `SteeringPolicy::allow_steering`,
    /// persisted so busy-submit admission can consult it without re-resolving
    /// the profile. Legacy rows predate the field and default to allowed.
    #[serde(default = "steering_allowed_metadata_default")]
    pub allow_steering: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved_model_route: Option<LoopModelRouteSnapshot>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_usage: Option<LoopModelUsage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution_outcome: Option<TurnExecutionOutcome>,
    #[serde(default)]
    pub subagent_depth: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub product_context: Option<ProductTurnContext>,
    #[serde(
        rename = "auth_resume_disposition",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub resume_disposition: Option<GateResumeDisposition>,
    /// True when the run's thread owner is `Ownerless` (unbound runs). The
    /// `__system__` owner slot alone cannot distinguish ownerless runs from
    /// actor-fallback runs without an explicit owner, so the disposition is
    /// journaled; absent (legacy rows) means actor-fallback.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub ownerless_thread: bool,
}

impl AgentTurnProcessMetadata {
    pub(super) fn from_record(record: &TurnRunRecord) -> Self {
        Self {
            turn_id: record.turn_id,
            accepted_message_ref: record.accepted_message_ref.clone(),
            resolved_run_profile_id: record.profile.id.clone(),
            resolved_run_profile_version: record.profile.version,
            allow_steering: record.profile.allow_steering,
            resolved_model_route: record.resolved_model_route.clone(),
            model_usage: record.model_usage,
            execution_outcome: record.execution_outcome,
            subagent_depth: record.subagent_depth,
            product_context: record.product_context.clone(),
            resume_disposition: record.resume_disposition.clone(),
            ownerless_thread: record.scope.thread_owner
                == ironclaw_host_api::turn::TurnThreadOwner::Ownerless,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentTurnProcessStateMetadata {
    pub turn_id: crate::TurnId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor: Option<TurnActor>,
    pub accepted_message_ref: AcceptedMessageRef,
    pub resolved_run_profile_id: RunProfileId,
    pub resolved_run_profile_version: RunProfileVersion,
    /// Snapshot of the resolved profile's `SteeringPolicy::allow_steering`.
    /// Persisted explicitly (not derived from `resolved_run_profile`, which
    /// state-derived rewrites drop) so busy-submit admission can consult it
    /// without re-resolving. Legacy rows default to allowed.
    #[serde(default = "steering_allowed_metadata_default")]
    pub allow_steering: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved_run_profile: Option<ResolvedRunProfile>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved_model_route: Option<LoopModelRouteSnapshot>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_usage: Option<LoopModelUsage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution_outcome: Option<TurnExecutionOutcome>,
    #[serde(default)]
    pub subagent_depth: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spawn_tree_descendant_cap: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub product_context: Option<ProductTurnContext>,
    #[serde(
        rename = "auth_resume_disposition",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub resume_disposition: Option<GateResumeDisposition>,
    /// True when the run's thread owner is `Ownerless` (unbound runs). The
    /// `__system__` owner slot alone cannot distinguish ownerless runs from
    /// actor-fallback runs without an explicit owner, so the disposition is
    /// journaled; absent (legacy rows) means actor-fallback.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub ownerless_thread: bool,
}

impl AgentTurnProcessStateMetadata {
    pub(super) fn from_state(state: &TurnRunState) -> Self {
        Self {
            turn_id: state.turn_id,
            actor: state.actor.clone(),
            accepted_message_ref: state.accepted_message_ref.clone(),
            resolved_run_profile_id: state.resolved_run_profile_id.clone(),
            resolved_run_profile_version: state.resolved_run_profile_version,
            allow_steering: state.allow_steering,
            resolved_run_profile: None,
            resolved_model_route: state.resolved_model_route.clone(),
            model_usage: state.model_usage,
            execution_outcome: state.execution_outcome,
            subagent_depth: 0,
            spawn_tree_descendant_cap: None,
            product_context: state.product_context.clone(),
            resume_disposition: state.resume_disposition.clone(),
            ownerless_thread: state.scope.thread_owner
                == ironclaw_host_api::turn::TurnThreadOwner::Ownerless,
        }
    }

    pub(super) fn from_claimed(claimed: &ClaimedTurnRun) -> Self {
        Self {
            resolved_run_profile: Some(claimed.resolved_run_profile.clone()),
            subagent_depth: claimed.subagent_depth,
            spawn_tree_descendant_cap: claimed.spawn_tree_descendant_cap,
            ..Self::from_state(&claimed.state)
        }
    }
}

pub(crate) fn agent_turn_metadata_from_claimed(
    claimed: &ClaimedTurnRun,
    model_usage: Option<LoopModelUsage>,
    execution_outcome: Option<TurnExecutionOutcome>,
) -> Value {
    let mut metadata = AgentTurnProcessStateMetadata::from_claimed(claimed);
    if model_usage.is_some() {
        metadata.model_usage = model_usage;
    }
    if execution_outcome.is_some() {
        metadata.execution_outcome = execution_outcome;
    }
    json!({ "agent_turn": metadata })
}

fn steering_allowed_metadata_default() -> bool {
    true
}
