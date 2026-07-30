//! Durable agent-turn metadata carried by neutral process snapshots.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{
    AcceptedMessageRef, GateResumeDisposition, ProductTurnContext, ReplyTargetBindingRef,
    ResolvedRunProfile, RunProfileId, RunProfileVersion, SourceBindingRef, TurnActor,
    TurnRunRecord, TurnRunState,
    run_profile::{LoopModelRouteSnapshot, LoopModelUsage},
    runner::ClaimedTurnRun,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentTurnProcessMetadata {
    pub turn_id: crate::TurnId,
    pub accepted_message_ref: AcceptedMessageRef,
    pub source_binding_ref: SourceBindingRef,
    pub reply_target_binding_ref: ReplyTargetBindingRef,
    pub resolved_run_profile_id: RunProfileId,
    pub resolved_run_profile_version: RunProfileVersion,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved_model_route: Option<LoopModelRouteSnapshot>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_usage: Option<LoopModelUsage>,
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
}

impl AgentTurnProcessMetadata {
    pub(super) fn from_record(record: &TurnRunRecord) -> Self {
        Self {
            turn_id: record.turn_id,
            accepted_message_ref: record.accepted_message_ref.clone(),
            source_binding_ref: record.source_binding_ref.clone(),
            reply_target_binding_ref: record.reply_target_binding_ref.clone(),
            resolved_run_profile_id: record.profile.id.clone(),
            resolved_run_profile_version: record.profile.version,
            resolved_model_route: record.resolved_model_route.clone(),
            model_usage: record.model_usage,
            subagent_depth: record.subagent_depth,
            product_context: record.product_context.clone(),
            resume_disposition: record.resume_disposition.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentTurnProcessStateMetadata {
    pub turn_id: crate::TurnId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor: Option<TurnActor>,
    pub accepted_message_ref: AcceptedMessageRef,
    pub source_binding_ref: SourceBindingRef,
    pub reply_target_binding_ref: ReplyTargetBindingRef,
    pub resolved_run_profile_id: RunProfileId,
    pub resolved_run_profile_version: RunProfileVersion,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved_run_profile: Option<ResolvedRunProfile>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved_model_route: Option<LoopModelRouteSnapshot>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_usage: Option<LoopModelUsage>,
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
}

impl AgentTurnProcessStateMetadata {
    pub(super) fn from_state(state: &TurnRunState) -> Self {
        Self {
            turn_id: state.turn_id,
            actor: state.actor.clone(),
            accepted_message_ref: state.accepted_message_ref.clone(),
            source_binding_ref: state.source_binding_ref.clone(),
            reply_target_binding_ref: state.reply_target_binding_ref.clone(),
            resolved_run_profile_id: state.resolved_run_profile_id.clone(),
            resolved_run_profile_version: state.resolved_run_profile_version,
            resolved_run_profile: None,
            resolved_model_route: state.resolved_model_route.clone(),
            model_usage: state.model_usage,
            subagent_depth: 0,
            spawn_tree_descendant_cap: None,
            product_context: state.product_context.clone(),
            resume_disposition: state.resume_disposition.clone(),
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
) -> Value {
    let mut metadata = AgentTurnProcessStateMetadata::from_claimed(claimed);
    if model_usage.is_some() {
        metadata.model_usage = model_usage;
    }
    json!({ "agent_turn": metadata })
}
