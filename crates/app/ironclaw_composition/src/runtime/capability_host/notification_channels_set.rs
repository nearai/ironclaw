//! `builtin.notification_channels_set` synthetic capability — the second
//! capability `outbound_delivery::outbound_delivery_capabilities` wires, split
//! out of the sibling `outbound_delivery` module to keep both files under the
//! repo's file-size budget (`.claude/rules/architecture.md` §5: a `.rs` file
//! over 1,500 lines needs either an owning tracking issue or a decomposition;
//! this is the decomposition).
//!
//! `outbound_delivery` and this module are SIBLING children of `capability_host`
//! (not ancestor/descendant), so — unlike `ironclaw_assistant`'s
//! `outbound_preferences::notification_channels` split, which reaches its
//! parent's private items for free through Rust's ancestor-visibility rule —
//! every helper and type this module shares with `outbound_delivery` had to be
//! widened to `pub(super)` on that side (visible to their common parent
//! `capability_host` and all its descendants, which includes this module). Those
//! shared items (`ApprovedDispatchLease`, `ApprovedResumeDecision`,
//! `OutboundDeliveryApprovalSettingsDecision`, `write_completed_result`) still
//! live in `outbound_delivery.rs` and carry their own doc-comments; only the
//! visibility changed, not the behavior.

use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_approvals::ApprovalStatus;
use ironclaw_approvals::ToolPermissionOverride;
use ironclaw_assistant::OutboundPreferencesProductService;
use ironclaw_authorization::CapabilityLeaseStatus;
use ironclaw_host_api::approval::ApprovalRequest;
use ironclaw_host_api::gate_record::GateRecord;
use ironclaw_host_api::{
    action::Action,
    ids::{ApprovalRequestId, CapabilityId, CorrelationId, GateRef, InvocationId, UserId},
    resolution::Resolution,
    resource::ResourceEstimate,
    result_meta::FailureKind,
    safe_summary::SafeSummary,
};
use ironclaw_loop_contracts::{
    AgentLoopHostError, AgentLoopHostErrorKind, CapabilityApprovalResume, ConcurrencyHint,
    resolution,
};

use super::outbound_delivery::{
    ApprovedDispatchLease, ApprovedResumeDecision, OutboundDeliveryApprovalSettingsDecision,
    OutboundPreferenceOperation, approval_denied, approval_fingerprint, approval_gate_ref,
    approval_lease_outcome, approval_request_matches_capability, approval_store_error,
    caller_for_run, input_error, invocation_id_from_resume_token, invocation_replay_input,
    outbound_delivery_outcome, outbound_delivery_synthetic_grantee, replay_payload_store_error,
    resource_scope_for_run, resume_token_from_invocation_id, settings_scope_for_run,
    write_completed_result,
};
use ironclaw_approvals::ApprovalSettingsProvider;
use ironclaw_assistant::{
    NotificationChannelsSetInput, OUTBOUND_NOTIFICATION_CHANNELS_SET_CAPABILITY_ID,
    OUTBOUND_NOTIFICATION_CHANNELS_SET_DESCRIPTION,
    OUTBOUND_NOTIFICATION_CHANNELS_SET_PROVIDER_TOOL_NAME, notification_channels_set_input_schema,
    parse_notification_channels_set_input, set_notification_channels_for_model,
};
use ironclaw_loop_host::{
    SyntheticCapability, SyntheticCapabilityDescriptor, SyntheticCapabilityHandler,
    SyntheticCapabilityInvocation,
};

/// Host-authored, model-independent summary for the notification-channels
/// approval gate. Host-authored and model-independent: the persisted
/// [`GateRecord`] the approver renders and the loop-facing outcome share this
/// one string so the two can never drift, and it interpolates nothing
/// caller-controlled (a target id may legally carry `/ < >`, which would trip
/// the safe-summary validator and kill the run).
const NOTIFICATION_CHANNELS_APPROVAL_GATE_SUMMARY: &str =
    "changing the notification channel list requires approval";

/// Carries the full approval-gate dance: gate raise (persisting a durable
/// `GateRecord` under the canonical `GateRef::for_approval_request`), replay
/// payload, approved-resume verification, and lease consume. `pub(super)`
/// (struct and every field): constructed from the sibling
/// `outbound_delivery::outbound_delivery_capabilities`, which also owns the
/// shared helpers (`ApprovedDispatchLease`, `ApprovedResumeDecision`,
/// `OutboundDeliveryApprovalSettingsDecision`, `write_completed_result`).
pub(super) struct NotificationChannelsSetHandler {
    pub(super) service: Arc<dyn OutboundPreferencesProductService>,
    pub(super) fallback_user_id: UserId,
    pub(super) approval_requests: Arc<dyn ironclaw_approvals::ApprovalRequestStorePort>,
    pub(super) capability_leases: Arc<dyn ironclaw_authorization::CapabilityLeaseStorePort>,
    pub(super) replay_payload_store: Arc<dyn ironclaw_capabilities::ReplayPayloadStorePort>,
    pub(super) gate_record_store: Arc<dyn ironclaw_approvals::GateRecordStorePort>,
    pub(super) requires_approval: bool,
    pub(super) approval_settings: Arc<dyn ApprovalSettingsProvider>,
}

#[async_trait]
impl SyntheticCapabilityHandler for NotificationChannelsSetHandler {
    fn validate_provider_arguments(
        &self,
        arguments: &serde_json::Value,
    ) -> Result<(), AgentLoopHostError> {
        parse_notification_channels_set_input(arguments)
            .map(|_| ())
            .map_err(input_error)
    }

    async fn invoke(
        &self,
        invocation: SyntheticCapabilityInvocation,
    ) -> Result<Resolution, AgentLoopHostError> {
        if invocation.request.auth_resume.is_some() {
            return Err(AgentLoopHostError::new(
                AgentLoopHostErrorKind::InvalidInvocation,
                "notification channels setter does not support auth resume",
            ));
        }

        let input = invocation_replay_input(&invocation).clone();
        let channels_input = parse_notification_channels_set_input(&input).map_err(input_error)?;
        let capability_id = notification_channels_set_capability_id()?;
        let approved_lease = if self.requires_approval {
            match invocation.request.approval_resume.clone() {
                Some(resume) => {
                    // A lost / expired / not-yet-granted approval lease is
                    // recoverable (the user can re-request approval), so route
                    // those arms to `Ok(Denied)` rather than a terminal
                    // `Err(Unauthorized)` that would end the run.
                    match self
                        .verify_approved_resume(&invocation, &resume, &input)
                        .await?
                    {
                        ApprovedResumeDecision::Approved(lease) => Some(lease),
                        ApprovedResumeDecision::Denied(denied) => {
                            return Ok(denied);
                        }
                    }
                }
                None => match self.settings_decision(&invocation, &capability_id).await? {
                    OutboundDeliveryApprovalSettingsDecision::Allow => None,
                    OutboundDeliveryApprovalSettingsDecision::Ask => {
                        return self
                            .request_approval(&invocation, &input, &channels_input)
                            .await;
                    }
                    OutboundDeliveryApprovalSettingsDecision::Deny => {
                        return Ok(super::diagnostic_failure(
                            FailureKind::PolicyDenied,
                            "notification channels setter is disabled by tool approval settings"
                                .to_string(),
                        ));
                    }
                },
            }
        } else {
            if invocation.request.approval_resume.is_some() {
                return Err(AgentLoopHostError::new(
                    AgentLoopHostErrorKind::InvalidInvocation,
                    "notification channels approval resume is not expected",
                ));
            }
            None
        };

        let caller = caller_for_run(&invocation, &self.fallback_user_id);
        let response = match set_notification_channels_for_model(
            self.service.as_ref(),
            caller,
            channels_input,
        )
        .await
        {
            Ok(response) => response,
            // See `outbound_delivery_outcome`: recoverable service errors are
            // model-visible failures, not terminal host errors.
            Err(error) => {
                return outbound_delivery_outcome(
                    OutboundPreferenceOperation::SetNotificationChannels,
                    error,
                );
            }
        };
        if let Some(approved_lease) = approved_lease {
            match self
                .capability_leases
                .consume(&approved_lease.scope, approved_lease.lease_id)
                .await
            {
                Ok(_) => {}
                Err(error) => match approval_lease_outcome(
                    OutboundPreferenceOperation::SetNotificationChannels,
                    "consume_approval_lease",
                    error,
                ) {
                    Ok(denied) => return Ok(denied),
                    Err(host_error) => return Err(host_error),
                },
            }
        }
        // The safe summary stays fixed and delimiter-free: only a `usize`
        // count is interpolated, never a raw target id (a target id may legally
        // carry `/ < >`, which would trip the loop's safe-summary validator and
        // kill the run). The model still gets every applied channel from the
        // result `output`.
        let channel_count = response.channels.len();
        let output = serde_json::to_value(response).map_err(|error| {
            AgentLoopHostError::new(
                AgentLoopHostErrorKind::Internal,
                format!("notification channels set output serialization failed: {error}"),
            )
        })?;
        write_completed_result(
            invocation,
            output,
            format!("set {channel_count} notification channel(s)"),
        )
        .await
    }
}

impl NotificationChannelsSetHandler {
    async fn settings_decision(
        &self,
        invocation: &SyntheticCapabilityInvocation,
        capability_id: &CapabilityId,
    ) -> Result<OutboundDeliveryApprovalSettingsDecision, AgentLoopHostError> {
        let scope = settings_scope_for_run(&invocation.run_context, &self.fallback_user_id);
        let grantee = outbound_delivery_synthetic_grantee()?;
        match self
            .approval_settings
            .tool_override(&scope, capability_id)
            .await
        {
            Some(ToolPermissionOverride::Disabled) => {
                return Ok(OutboundDeliveryApprovalSettingsDecision::Deny);
            }
            Some(ToolPermissionOverride::AskEachTime) => {
                return Ok(OutboundDeliveryApprovalSettingsDecision::Ask);
            }
            None => {}
        }
        if self
            .approval_settings
            .tool_always_allow(&scope, capability_id, &grantee)
            .await
            || self.approval_settings.global_auto_approve(&scope).await
        {
            return Ok(OutboundDeliveryApprovalSettingsDecision::Allow);
        }
        Ok(OutboundDeliveryApprovalSettingsDecision::Ask)
    }

    async fn request_approval(
        &self,
        invocation: &SyntheticCapabilityInvocation,
        input: &serde_json::Value,
        channels_input: &NotificationChannelsSetInput,
    ) -> Result<Resolution, AgentLoopHostError> {
        let capability_id = notification_channels_set_capability_id()?;
        let approval_request_id = ApprovalRequestId::new();
        let correlation_id = CorrelationId::new();
        let invocation_id = InvocationId::new();
        let estimate = ResourceEstimate::default();
        let scope = resource_scope_for_run(
            &invocation.run_context,
            &self.fallback_user_id,
            invocation_id,
        );
        let fingerprint = approval_fingerprint(&scope, &capability_id, &estimate, input)?;
        self.replay_payload_store
            .save(
                super::resource_scope_for_run(&invocation.run_context, &self.fallback_user_id),
                invocation_id,
                ironclaw_capabilities::ReplayPayload {
                    input: input.clone(),
                    estimate: estimate.clone(),
                    prior_approval: None,
                    input_ref: invocation.request.input_ref.clone(),
                    correlation_id,
                },
            )
            .await
            .map_err(replay_payload_store_error)?;
        self.approval_requests
            .save_pending(
                scope,
                ApprovalRequest {
                    id: approval_request_id,
                    correlation_id,
                    requested_by: outbound_delivery_synthetic_grantee()?,
                    action: Box::new(Action::Dispatch {
                        capability: capability_id,
                        estimated_resources: estimate.clone(),
                    }),
                    invocation_fingerprint: Some(fingerprint),
                    reason: format!(
                        "Set the notification channel list ({} target(s))",
                        channels_input.target_ids().len()
                    ),
                    reusable_scope: None,
                },
            )
            .await
            .map_err(|error| approval_store_error("save_pending_approval", error))?;

        let gate_summary =
            SafeSummary::new(NOTIFICATION_CHANNELS_APPROVAL_GATE_SUMMARY).map_err(|error| {
                ironclaw_loop_host::raw_agent_loop_host_error(
                    "notification_channels_set",
                    "gate_record_summary",
                    AgentLoopHostErrorKind::Internal,
                    "notification channels gate summary is not renderable",
                    error,
                )
            })?;
        self.gate_record_store
            .save(
                super::resource_scope_for_run(&invocation.run_context, &self.fallback_user_id),
                GateRef::for_approval_request(approval_request_id),
                GateRecord::Approval {
                    summary: gate_summary,
                },
            )
            .await
            .map_err(|error| approval_store_error("save_gate_record", error))?;

        Ok(resolution::approval_required(
            approval_gate_ref(approval_request_id)?,
            NOTIFICATION_CHANNELS_APPROVAL_GATE_SUMMARY.to_string(),
            Some(CapabilityApprovalResume {
                approval_request_id,
                resume_token: resume_token_from_invocation_id(invocation_id)?,
                correlation_id,
                input_ref: invocation.request.input_ref.clone(),
            }),
        )
        .resolution)
    }

    async fn verify_approved_resume(
        &self,
        invocation: &SyntheticCapabilityInvocation,
        resume: &CapabilityApprovalResume,
        input: &serde_json::Value,
    ) -> Result<ApprovedResumeDecision, AgentLoopHostError> {
        let capability_id = notification_channels_set_capability_id()?;
        let invocation_id = invocation_id_from_resume_token(&resume.resume_token)?;
        let replay_scope =
            super::resource_scope_for_run(&invocation.run_context, &self.fallback_user_id);
        let replay = self
            .replay_payload_store
            .load(&replay_scope, invocation_id)
            .await
            .map_err(replay_payload_store_error)?
            .ok_or_else(|| {
                AgentLoopHostError::new(
                    AgentLoopHostErrorKind::Unavailable,
                    "notification channels approval replay payload is unavailable",
                )
            })?;
        if replay.input != *input {
            return Err(AgentLoopHostError::new(
                AgentLoopHostErrorKind::InvalidInvocation,
                "notification channels approval resume input does not match",
            ));
        }
        let scope = resource_scope_for_run(
            &invocation.run_context,
            &self.fallback_user_id,
            invocation_id,
        );
        let fingerprint = approval_fingerprint(&scope, &capability_id, &replay.estimate, input)?;
        let approval_record = match self
            .approval_requests
            .get(&scope, resume.approval_request_id)
            .await
            .map_err(|error| approval_store_error("load_approval", error))?
        {
            Some(record) => record,
            None => {
                return Ok(ApprovedResumeDecision::Denied(approval_denied(
                    "notification channels approval is unavailable; re-request approval",
                )?));
            }
        };
        if approval_record.status != ApprovalStatus::Approved {
            return Ok(ApprovedResumeDecision::Denied(approval_denied(
                "notification channels approval has not been granted; re-request approval",
            )?));
        }
        if approval_record.request.correlation_id != replay.correlation_id {
            return Err(AgentLoopHostError::new(
                AgentLoopHostErrorKind::InvalidInvocation,
                "notification channels approval correlation does not match",
            ));
        }
        if approval_record.request.invocation_fingerprint.as_ref() != Some(&fingerprint) {
            return Err(AgentLoopHostError::new(
                AgentLoopHostErrorKind::InvalidInvocation,
                "notification channels approval fingerprint does not match",
            ));
        }
        if !approval_request_matches_capability(
            approval_record.request.action.as_ref(),
            &capability_id,
        ) {
            return Err(AgentLoopHostError::new(
                AgentLoopHostErrorKind::InvalidInvocation,
                "notification channels approval action does not match",
            ));
        }

        let lease = match self
            .capability_leases
            .leases_for_scope(&scope)
            .await
            .into_iter()
            .find(|lease| {
                lease.status == CapabilityLeaseStatus::Active
                    && lease.grant.capability == capability_id
                    && lease.grant.grantee == approval_record.request.requested_by
                    && lease.invocation_fingerprint.as_ref() == Some(&fingerprint)
            }) {
            Some(lease) => lease,
            None => {
                return Ok(ApprovedResumeDecision::Denied(approval_denied(
                    "notification channels approval lease is unavailable; re-request approval",
                )?));
            }
        };
        match self
            .capability_leases
            .claim(&scope, lease.grant.id, &fingerprint)
            .await
        {
            Ok(_) => {}
            Err(error) => match approval_lease_outcome(
                OutboundPreferenceOperation::SetNotificationChannels,
                "claim_approval_lease",
                error,
            ) {
                Ok(denied) => return Ok(ApprovedResumeDecision::Denied(denied)),
                Err(host_error) => return Err(host_error),
            },
        }
        Ok(ApprovedResumeDecision::Approved(ApprovedDispatchLease {
            scope,
            lease_id: lease.grant.id,
        }))
    }
}

fn notification_channels_set_capability_id() -> Result<CapabilityId, AgentLoopHostError> {
    CapabilityId::new(OUTBOUND_NOTIFICATION_CHANNELS_SET_CAPABILITY_ID).map_err(|error| {
        AgentLoopHostError::new(
            AgentLoopHostErrorKind::Internal,
            format!("notification channels set capability id is invalid: {error}"),
        )
    })
}

/// Build the `builtin.notification_channels_set` synthetic capability from an
/// already-constructed handler. Takes the handler struct as one parameter
/// (rather than its 8 constituent fields) so this factory needs no
/// `#[allow(clippy::too_many_arguments)]` of its own — see
/// `.claude/rules/architecture.md` §1: aggregating into a named struct is the
/// preferred fix, not another `#[allow]`.
pub(super) fn notification_channels_set_capability(
    handler: NotificationChannelsSetHandler,
) -> Result<SyntheticCapability, AgentLoopHostError> {
    Ok(SyntheticCapability::new(
        SyntheticCapabilityDescriptor::new(
            OUTBOUND_NOTIFICATION_CHANNELS_SET_CAPABILITY_ID,
            OUTBOUND_NOTIFICATION_CHANNELS_SET_PROVIDER_TOOL_NAME,
            OUTBOUND_NOTIFICATION_CHANNELS_SET_DESCRIPTION,
            ConcurrencyHint::Exclusive,
            notification_channels_set_input_schema(),
        )?,
        Arc::new(handler),
    ))
}

#[cfg(test)]
mod tests {
    use ironclaw_loop_contracts::LoopSafeSummary;

    use super::*;

    #[test]
    fn notification_channels_set_summary_is_fixed_and_count_only() {
        // Same invariant as `outbound_delivery`'s
        // `set_delivery_target_summary_is_fixed_and_validator_safe`: the
        // completion summary must never interpolate a raw, model-controlled
        // target id (which may legally contain a forbidden delimiter). The
        // notification-channels handler interpolates only a `usize` count,
        // which can never carry a delimiter, so every count must validate.
        for channel_count in [0usize, 1, 8] {
            LoopSafeSummary::new(format!("set {channel_count} notification channel(s)"))
                .unwrap_or_else(|reason| {
                    panic!("count-only summary must satisfy the loop validator: {reason}")
                });
        }
    }

    #[test]
    fn notification_channels_set_capability_id_resolves_to_public_constant() {
        let capability_id = notification_channels_set_capability_id()
            .expect("notification channels capability id must be valid");
        assert_eq!(
            capability_id.as_str(),
            OUTBOUND_NOTIFICATION_CHANNELS_SET_CAPABILITY_ID
        );
    }
}
