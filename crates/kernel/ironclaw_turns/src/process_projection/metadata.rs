//! Durable agent-turn metadata carried by neutral process snapshots.

use ironclaw_host_api::output::OutputContract;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{
    AcceptedMessageRef, ActivationProvenance, GateResumeDisposition, ProductTurnContext,
    RunProfileId, RunProfileVersion, TurnActor, TurnRunState, runner::ClaimedTurnRun,
};
use ironclaw_host_api::turn::TurnExecutionOutcome;
use ironclaw_loop_contracts::{LoopModelRouteSnapshot, LoopModelUsage, ResolvedRunProfile};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentTurnProcessStateMetadata {
    pub turn_id: crate::TurnId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor: Option<TurnActor>,
    pub accepted_message_ref: AcceptedMessageRef,
    pub resolved_run_profile_id: RunProfileId,
    pub resolved_run_profile_version: RunProfileVersion,
    /// Immutable terminal output contract. Omitted legacy metadata defaults
    /// to an ordinary assistant message.
    #[serde(default, skip_serializing_if = "OutputContract::is_assistant_message")]
    pub output_contract: OutputContract,
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
    /// Why this run was activated on its thread. Set once at run creation,
    /// never mutated. Absent on rows written before the field existed, and on
    /// every ordinary human-initiated submission.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subagent_activation_provenance: Option<ActivationProvenance>,
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
            output_contract: state.output_contract.clone(),
            allow_steering: state.allow_steering,
            resolved_run_profile: None,
            resolved_model_route: state.resolved_model_route.clone(),
            model_usage: state.model_usage,
            execution_outcome: state.execution_outcome,
            subagent_depth: 0,
            // State-derived rewrites do not carry lineage (see subagent_depth
            // above); the durable provenance stays on the originally journaled
            // metadata.
            subagent_activation_provenance: None,
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
            subagent_activation_provenance: claimed.subagent_activation_provenance,
            ..Self::from_state(&claimed.state)
        }
    }
}

/// Build the complete agent-turn metadata envelope for a process transition.
///
/// Runner-owned failure paths use this same typed projection as normal loop
/// exits so supplemental model usage survives terminalization without being
/// encoded in diagnostic text or replacing unrelated run metadata.
pub fn agent_turn_metadata_from_claimed(
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
