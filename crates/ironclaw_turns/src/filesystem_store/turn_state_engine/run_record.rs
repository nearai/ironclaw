//! `RunRecord` inherent constructors/projections and run-slot derivation.
use super::*;

impl RunRecord {
    /// A freshly-submitted run: `Queued`, with no resolved model route, model
    /// usage, checkpoint, gate, failure, runner lease, lineage, or product
    /// context yet. Callers set the few fields that differ by submit path
    /// (`resolved_model_route`, `checkpoint_id`, `parent_run_id`,
    /// `subagent_depth`, `spawn_tree_root_run_id`, `product_context`) on the
    /// returned record.
    pub(super) fn queued(fields: QueuedRunFields) -> Self {
        Self {
            scope: fields.scope,
            actor: fields.actor,
            turn_id: fields.turn_id,
            run_id: fields.run_id,
            status: RunStatusCell::new(TurnStatus::Queued),
            profile: fields.profile,
            resolved_model_route: None,
            model_usage: None,
            accepted_message_ref: fields.accepted_message_ref,
            source_binding_ref: fields.source_binding_ref,
            reply_target_binding_ref: fields.reply_target_binding_ref,
            checkpoint_id: None,
            gate_ref: None,
            expected_tx_hash: None,
            blocked_activity_id: None,
            credential_requirements: Vec::new(),
            failure: None,
            event_cursor: fields.event_cursor,
            runner_id: None,
            lease_token: None,
            lease_expires_at: None,
            last_heartbeat_at: None,
            claim_count: 0,
            received_at: fields.received_at,
            parent_run_id: None,
            subagent_depth: 0,
            spawn_tree_root_run_id: None,
            product_context: None,
            resume_disposition: None,
        }
    }

    pub(super) fn persistence_record(&self) -> TurnRunRecord {
        TurnRunRecord {
            run_id: self.run_id,
            turn_id: self.turn_id,
            scope: self.scope.clone(),
            accepted_message_ref: self.accepted_message_ref.clone(),
            source_binding_ref: self.source_binding_ref.clone(),
            reply_target_binding_ref: self.reply_target_binding_ref.clone(),
            status: self.status.get(),
            profile: self.profile.clone(),
            resolved_model_route: self.resolved_model_route.clone(),
            model_usage: self.model_usage,
            checkpoint_id: self.checkpoint_id,
            gate_ref: self.gate_ref.clone(),
            expected_tx_hash: self.expected_tx_hash.clone(),
            blocked_activity_id: self.blocked_activity_id,
            credential_requirements: self.credential_requirements.clone(),
            failure: self.failure.clone(),
            event_cursor: self.event_cursor,
            runner_id: self.runner_id,
            lease_token: self.lease_token,
            lease_expires_at: self.lease_expires_at,
            last_heartbeat_at: self.last_heartbeat_at,
            claim_count: self.claim_count,
            received_at: self.received_at,
            parent_run_id: self.parent_run_id,
            subagent_depth: self.subagent_depth,
            spawn_tree_root_run_id: self.spawn_tree_root_run_id,
            product_context: self.product_context.clone(),
            resume_disposition: self.resume_disposition.clone(),
        }
    }

    pub(super) fn state(&self) -> TurnRunState {
        TurnRunState {
            scope: self.scope.clone(),
            actor: Some(self.actor.clone()),
            turn_id: self.turn_id,
            run_id: self.run_id,
            status: self.status.get(),
            accepted_message_ref: self.accepted_message_ref.clone(),
            source_binding_ref: self.source_binding_ref.clone(),
            reply_target_binding_ref: self.reply_target_binding_ref.clone(),
            resolved_run_profile_id: self.profile.id.clone(),
            resolved_run_profile_version: self.profile.version,
            resolved_model_route: self.resolved_model_route.clone(),
            model_usage: self.model_usage,
            received_at: self.received_at,
            checkpoint_id: self.checkpoint_id,
            gate_ref: self.gate_ref.clone(),
            blocked_activity_id: self.blocked_activity_id,
            credential_requirements: self.credential_requirements.clone(),
            failure: self.failure.clone(),
            event_cursor: self.event_cursor,
            product_context: self.product_context.clone(),
            resume_disposition: self.resume_disposition.clone(),
        }
    }
}

pub(super) fn slot_info_for(record: &RunRecord) -> RunSlotInfo<'_> {
    RunSlotInfo {
        tenant_id: &record.scope.tenant_id,
        thread_owner: &record.scope.thread_owner,
        actor_user_id: &record.actor.user_id,
        product_context: match record.product_context.as_ref().map(|pc| pc.origin) {
            Some(crate::TurnOriginKind::ScheduledTrigger) => Some(OriginClass::Trigger),
            Some(crate::TurnOriginKind::Inbound) | Some(crate::TurnOriginKind::WebUi) => {
                Some(OriginClass::Conversation)
            }
            None => None,
        },
    }
}
