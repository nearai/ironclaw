use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_authorization::{CapabilityLeaseError, CapabilityLeaseStatus, CapabilityLeaseStore};
use ironclaw_host_api::{
    Action, ApprovalRequest, ApprovalRequestId, CapabilityGrantId, CapabilityId, CorrelationId,
    InvocationFingerprint, InvocationId, Principal, ResourceEstimate, ResourceScope, UserId,
};
use ironclaw_loop_support::{CapabilityResultWrite, loop_driver_execution_extension_id};
use ironclaw_product_workflow::{
    OutboundPreferencesProductFacade, RebornOutboundDeliveryTargetId, RebornServicesError,
    RebornServicesErrorCode, WebUiAuthenticatedCaller,
};
use ironclaw_run_state::{ApprovalRequestStore, ApprovalStatus, RunStateError};
use ironclaw_turns::{
    LoopGateRef,
    run_profile::{
        AgentLoopHostError, AgentLoopHostErrorKind, CapabilityApprovalResume, CapabilityInputRef,
        CapabilityOutcome, CapabilityProgress, CapabilityResultMessage, CapabilityResumeToken,
        ConcurrencyHint, LoopRunContext,
    },
};

use crate::outbound_delivery_capability_surface::{
    OUTBOUND_DELIVERY_TARGET_SET_CAPABILITY_ID, OUTBOUND_DELIVERY_TARGET_SET_DESCRIPTION,
    OUTBOUND_DELIVERY_TARGET_SET_PROVIDER_TOOL_NAME, OUTBOUND_DELIVERY_TARGETS_LIST_CAPABILITY_ID,
    OUTBOUND_DELIVERY_TARGETS_LIST_DESCRIPTION, OUTBOUND_DELIVERY_TARGETS_LIST_PROVIDER_TOOL_NAME,
    OutboundDeliveryCapabilityInputError, list_outbound_delivery_targets_for_model,
    outbound_delivery_target_set_input_schema, outbound_delivery_targets_list_input_schema,
    parse_outbound_delivery_target_set_input, parse_outbound_delivery_targets_list_input,
    set_outbound_delivery_target_for_model,
};
use crate::runtime::local_dev::synthetic_capability::{
    LocalDevSyntheticCapability, LocalDevSyntheticCapabilityDescriptor,
    LocalDevSyntheticCapabilityHandler, LocalDevSyntheticCapabilityInvocation,
};

pub(super) fn outbound_delivery_capabilities(
    facade: Arc<dyn OutboundPreferencesProductFacade>,
    fallback_user_id: UserId,
    approval_requests: Arc<dyn ApprovalRequestStore>,
    capability_leases: Arc<dyn CapabilityLeaseStore>,
    target_set_requires_approval: bool,
) -> Result<Vec<LocalDevSyntheticCapability>, AgentLoopHostError> {
    Ok(vec![
        LocalDevSyntheticCapability::new(
            LocalDevSyntheticCapabilityDescriptor::new(
                OUTBOUND_DELIVERY_TARGETS_LIST_CAPABILITY_ID,
                OUTBOUND_DELIVERY_TARGETS_LIST_PROVIDER_TOOL_NAME,
                OUTBOUND_DELIVERY_TARGETS_LIST_DESCRIPTION,
                ConcurrencyHint::SafeForParallel,
                outbound_delivery_targets_list_input_schema(),
            )?,
            Arc::new(OutboundDeliveryTargetsListHandler {
                facade: Arc::clone(&facade),
                fallback_user_id: fallback_user_id.clone(),
            }),
        ),
        LocalDevSyntheticCapability::new(
            LocalDevSyntheticCapabilityDescriptor::new(
                OUTBOUND_DELIVERY_TARGET_SET_CAPABILITY_ID,
                OUTBOUND_DELIVERY_TARGET_SET_PROVIDER_TOOL_NAME,
                OUTBOUND_DELIVERY_TARGET_SET_DESCRIPTION,
                ConcurrencyHint::Exclusive,
                outbound_delivery_target_set_input_schema(),
            )?,
            Arc::new(OutboundDeliveryTargetSetHandler {
                facade,
                fallback_user_id,
                approval_requests,
                capability_leases,
                requires_approval: target_set_requires_approval,
            }),
        ),
    ])
}

struct OutboundDeliveryTargetsListHandler {
    facade: Arc<dyn OutboundPreferencesProductFacade>,
    fallback_user_id: UserId,
}

#[async_trait]
impl LocalDevSyntheticCapabilityHandler for OutboundDeliveryTargetsListHandler {
    fn validate_provider_arguments(
        &self,
        arguments: &serde_json::Value,
    ) -> Result<(), AgentLoopHostError> {
        parse_outbound_delivery_targets_list_input(arguments)
            .map(|_| ())
            .map_err(input_error)
    }

    async fn invoke(
        &self,
        invocation: LocalDevSyntheticCapabilityInvocation,
    ) -> Result<CapabilityOutcome, AgentLoopHostError> {
        let input =
            parse_outbound_delivery_targets_list_input(&invocation.input).map_err(input_error)?;
        let caller = caller_for_run(&invocation, &self.fallback_user_id);
        let response =
            list_outbound_delivery_targets_for_model(self.facade.as_ref(), caller, input)
                .await
                .map_err(|error| outbound_delivery_host_error("list_targets", error))?;
        let count = response.targets.len();
        let output = serde_json::to_value(response).map_err(|error| {
            AgentLoopHostError::new(
                AgentLoopHostErrorKind::Internal,
                format!("outbound delivery target list output serialization failed: {error}"),
            )
        })?;
        write_completed_result(
            invocation,
            output,
            format!("found {count} delivery target(s)"),
        )
        .await
    }
}

struct OutboundDeliveryTargetSetHandler {
    facade: Arc<dyn OutboundPreferencesProductFacade>,
    fallback_user_id: UserId,
    approval_requests: Arc<dyn ApprovalRequestStore>,
    capability_leases: Arc<dyn CapabilityLeaseStore>,
    requires_approval: bool,
}

struct ApprovedDispatchLease {
    scope: ResourceScope,
    lease_id: CapabilityGrantId,
}

#[async_trait]
impl LocalDevSyntheticCapabilityHandler for OutboundDeliveryTargetSetHandler {
    fn validate_provider_arguments(
        &self,
        arguments: &serde_json::Value,
    ) -> Result<(), AgentLoopHostError> {
        parse_outbound_delivery_target_set_input(arguments)
            .map(|_| ())
            .map_err(input_error)
    }

    async fn invoke(
        &self,
        invocation: LocalDevSyntheticCapabilityInvocation,
    ) -> Result<CapabilityOutcome, AgentLoopHostError> {
        if invocation.request.auth_resume.is_some() {
            return Err(AgentLoopHostError::new(
                AgentLoopHostErrorKind::InvalidInvocation,
                "outbound delivery target setter does not support auth resume",
            ));
        }

        let input = invocation_replay_input(&invocation).clone();
        let target_input = parse_outbound_delivery_target_set_input(&input).map_err(input_error)?;
        let approved_lease = if self.requires_approval {
            match invocation.request.approval_resume.clone() {
                Some(resume) => Some(
                    self.verify_approved_resume(&invocation, &resume, &input)
                        .await?,
                ),
                None => {
                    return self
                        .request_approval(&invocation, &input, target_input.target_id())
                        .await;
                }
            }
        } else {
            if invocation.request.approval_resume.is_some() {
                return Err(AgentLoopHostError::new(
                    AgentLoopHostErrorKind::InvalidInvocation,
                    "outbound delivery target approval resume is not expected",
                ));
            }
            None
        };

        let target_summary = target_input.target_id().as_str().to_string();
        let caller = caller_for_run(&invocation, &self.fallback_user_id);
        if let Some(approved_lease) = approved_lease {
            self.capability_leases
                .consume(&approved_lease.scope, approved_lease.lease_id)
                .await
                .map_err(|error| approval_lease_error("consume_approval_lease", error))?;
        }
        let response =
            set_outbound_delivery_target_for_model(self.facade.as_ref(), caller, target_input)
                .await
                .map_err(|error| outbound_delivery_host_error("set_target", error))?;
        let output = serde_json::to_value(response).map_err(|error| {
            AgentLoopHostError::new(
                AgentLoopHostErrorKind::Internal,
                format!("outbound delivery target set output serialization failed: {error}"),
            )
        })?;
        write_completed_result(
            invocation,
            output,
            format!("set delivery target to {target_summary}"),
        )
        .await
    }
}

impl OutboundDeliveryTargetSetHandler {
    async fn request_approval(
        &self,
        invocation: &LocalDevSyntheticCapabilityInvocation,
        input: &serde_json::Value,
        target_id: &RebornOutboundDeliveryTargetId,
    ) -> Result<CapabilityOutcome, AgentLoopHostError> {
        let capability_id = outbound_delivery_target_set_capability_id()?;
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
        self.approval_requests
            .save_pending(
                scope,
                ApprovalRequest {
                    id: approval_request_id,
                    correlation_id,
                    requested_by: Principal::Extension(loop_driver_execution_extension_id(
                        &invocation.run_context,
                    )?),
                    action: Box::new(Action::Dispatch {
                        capability: capability_id,
                        estimated_resources: estimate.clone(),
                    }),
                    invocation_fingerprint: Some(fingerprint),
                    reason: format!(
                        "Change final reply delivery target to `{}`",
                        target_id.as_str()
                    ),
                    reusable_scope: None,
                },
            )
            .await
            .map_err(|error| approval_store_error("save_pending_approval", error))?;

        Ok(CapabilityOutcome::ApprovalRequired {
            gate_ref: approval_gate_ref(approval_request_id)?,
            safe_summary: "changing the outbound delivery target requires approval".to_string(),
            approval_resume: Some(CapabilityApprovalResume {
                approval_request_id,
                resume_token: resume_token_from_invocation_id(invocation_id)?,
                correlation_id,
                input_ref: invocation.request.input_ref.clone(),
                input: input.clone(),
                estimate,
            }),
        })
    }

    async fn verify_approved_resume(
        &self,
        invocation: &LocalDevSyntheticCapabilityInvocation,
        resume: &CapabilityApprovalResume,
        input: &serde_json::Value,
    ) -> Result<ApprovedDispatchLease, AgentLoopHostError> {
        if resume.input != *input {
            return Err(AgentLoopHostError::new(
                AgentLoopHostErrorKind::InvalidInvocation,
                "outbound delivery target approval resume input does not match",
            ));
        }

        let capability_id = outbound_delivery_target_set_capability_id()?;
        let invocation_id = invocation_id_from_resume_token(&resume.resume_token)?;
        let scope = resource_scope_for_run(
            &invocation.run_context,
            &self.fallback_user_id,
            invocation_id,
        );
        let fingerprint = approval_fingerprint(&scope, &capability_id, &resume.estimate, input)?;
        let approval_record = self
            .approval_requests
            .get(&scope, resume.approval_request_id)
            .await
            .map_err(|error| approval_store_error("load_approval", error))?
            .ok_or_else(|| {
                AgentLoopHostError::new(
                    AgentLoopHostErrorKind::Unauthorized,
                    "outbound delivery target approval is unavailable",
                )
            })?;
        if approval_record.status != ApprovalStatus::Approved {
            return Err(AgentLoopHostError::new(
                AgentLoopHostErrorKind::Unauthorized,
                "outbound delivery target approval has not been granted",
            ));
        }
        if approval_record.request.correlation_id != resume.correlation_id {
            return Err(AgentLoopHostError::new(
                AgentLoopHostErrorKind::InvalidInvocation,
                "outbound delivery target approval correlation does not match",
            ));
        }
        if approval_record.request.invocation_fingerprint.as_ref() != Some(&fingerprint) {
            return Err(AgentLoopHostError::new(
                AgentLoopHostErrorKind::InvalidInvocation,
                "outbound delivery target approval fingerprint does not match",
            ));
        }
        if !approval_request_matches_capability(
            approval_record.request.action.as_ref(),
            &capability_id,
        ) {
            return Err(AgentLoopHostError::new(
                AgentLoopHostErrorKind::InvalidInvocation,
                "outbound delivery target approval action does not match",
            ));
        }

        let lease = self
            .capability_leases
            .leases_for_scope(&scope)
            .await
            .into_iter()
            .find(|lease| {
                lease.status == CapabilityLeaseStatus::Active
                    && lease.grant.capability == capability_id
                    && lease.grant.grantee == approval_record.request.requested_by
                    && lease.invocation_fingerprint.as_ref() == Some(&fingerprint)
            })
            .ok_or_else(|| {
                AgentLoopHostError::new(
                    AgentLoopHostErrorKind::Unauthorized,
                    "outbound delivery target approval lease is unavailable",
                )
            })?;
        self.capability_leases
            .claim(&scope, lease.grant.id, &fingerprint)
            .await
            .map_err(|error| approval_lease_error("claim_approval_lease", error))?;
        Ok(ApprovedDispatchLease {
            scope,
            lease_id: lease.grant.id,
        })
    }
}

async fn write_completed_result(
    invocation: LocalDevSyntheticCapabilityInvocation,
    output: serde_json::Value,
    safe_summary: String,
) -> Result<CapabilityOutcome, AgentLoopHostError> {
    let write_result = invocation
        .result_writer
        .write_capability_result(CapabilityResultWrite {
            run_context: &invocation.run_context,
            input_ref: invocation_effective_input_ref(&invocation),
            invocation_id: InvocationId::new(),
            capability_id: &invocation.request.capability_id,
            output,
            display_preview: None,
        })
        .await?;
    Ok(CapabilityOutcome::Completed(CapabilityResultMessage {
        result_ref: write_result.result_ref,
        safe_summary,
        progress: CapabilityProgress::MadeProgress,
        terminate_hint: false,
        byte_len: write_result.byte_len,
        output_digest: write_result.output_digest,
    }))
}

fn invocation_replay_input(
    invocation: &LocalDevSyntheticCapabilityInvocation,
) -> &serde_json::Value {
    invocation
        .request
        .approval_resume
        .as_ref()
        .map(|resume| &resume.input)
        .unwrap_or(&invocation.input)
}

fn invocation_effective_input_ref(
    invocation: &LocalDevSyntheticCapabilityInvocation,
) -> &CapabilityInputRef {
    invocation
        .request
        .approval_resume
        .as_ref()
        .map(|resume| &resume.input_ref)
        .unwrap_or(&invocation.request.input_ref)
}

fn caller_for_run(
    invocation: &LocalDevSyntheticCapabilityInvocation,
    fallback_user_id: &UserId,
) -> WebUiAuthenticatedCaller {
    WebUiAuthenticatedCaller::new(
        invocation.run_context.scope.tenant_id.clone(),
        effective_user_id(&invocation.run_context, fallback_user_id),
        invocation.run_context.scope.agent_id.clone(),
        invocation.run_context.scope.project_id.clone(),
    )
}

fn resource_scope_for_run(
    run_context: &LoopRunContext,
    fallback_user_id: &UserId,
    invocation_id: InvocationId,
) -> ResourceScope {
    let mut scope = run_context.scope.to_resource_scope();
    scope.user_id = effective_user_id(run_context, fallback_user_id);
    scope.invocation_id = invocation_id;
    scope
}

fn effective_user_id(run_context: &LoopRunContext, fallback_user_id: &UserId) -> UserId {
    run_context
        .scope
        .explicit_owner_user_id()
        .cloned()
        .or_else(|| {
            run_context
                .actor
                .as_ref()
                .map(|actor| actor.user_id.clone())
        })
        .unwrap_or_else(|| fallback_user_id.clone())
}

fn outbound_delivery_target_set_capability_id() -> Result<CapabilityId, AgentLoopHostError> {
    CapabilityId::new(OUTBOUND_DELIVERY_TARGET_SET_CAPABILITY_ID).map_err(|error| {
        AgentLoopHostError::new(
            AgentLoopHostErrorKind::Internal,
            format!("outbound delivery target set capability id is invalid: {error}"),
        )
    })
}

fn approval_fingerprint(
    scope: &ResourceScope,
    capability_id: &CapabilityId,
    estimate: &ResourceEstimate,
    input: &serde_json::Value,
) -> Result<InvocationFingerprint, AgentLoopHostError> {
    InvocationFingerprint::for_dispatch(scope, capability_id, estimate, input).map_err(|error| {
        AgentLoopHostError::new(
            AgentLoopHostErrorKind::Internal,
            format!("outbound delivery target approval fingerprint could not be computed: {error}"),
        )
    })
}

fn approval_request_matches_capability(action: &Action, capability_id: &CapabilityId) -> bool {
    matches!(action, Action::Dispatch { capability, .. } if capability == capability_id)
}

fn approval_gate_ref(request_id: ApprovalRequestId) -> Result<LoopGateRef, AgentLoopHostError> {
    LoopGateRef::new(format!("gate:approval-{request_id}")).map_err(|error| {
        AgentLoopHostError::new(
            AgentLoopHostErrorKind::Internal,
            format!("outbound delivery target approval gate ref is invalid: {error}"),
        )
    })
}

fn resume_token_from_invocation_id(
    invocation_id: InvocationId,
) -> Result<CapabilityResumeToken, AgentLoopHostError> {
    CapabilityResumeToken::new(invocation_id.to_string()).map_err(|reason| {
        AgentLoopHostError::new(
            AgentLoopHostErrorKind::Internal,
            format!("outbound delivery target resume token is invalid: {reason}"),
        )
    })
}

fn invocation_id_from_resume_token(
    resume_token: &CapabilityResumeToken,
) -> Result<InvocationId, AgentLoopHostError> {
    InvocationId::parse(resume_token.as_str()).map_err(|error| {
        AgentLoopHostError::new(
            AgentLoopHostErrorKind::InvalidInvocation,
            format!("outbound delivery target approval resume token is invalid: {error}"),
        )
    })
}

fn input_error(error: OutboundDeliveryCapabilityInputError) -> AgentLoopHostError {
    AgentLoopHostError::new(AgentLoopHostErrorKind::InvalidInvocation, error.to_string())
}

fn outbound_delivery_host_error(
    operation: &'static str,
    error: RebornServicesError,
) -> AgentLoopHostError {
    let kind = match error.code {
        RebornServicesErrorCode::InvalidRequest | RebornServicesErrorCode::NotFound => {
            AgentLoopHostErrorKind::InvalidInvocation
        }
        RebornServicesErrorCode::Unauthenticated | RebornServicesErrorCode::Forbidden => {
            AgentLoopHostErrorKind::Unauthorized
        }
        RebornServicesErrorCode::Conflict | RebornServicesErrorCode::RateLimited => {
            AgentLoopHostErrorKind::Unavailable
        }
        RebornServicesErrorCode::Unavailable => AgentLoopHostErrorKind::Unavailable,
        RebornServicesErrorCode::Internal => AgentLoopHostErrorKind::Internal,
    };
    ironclaw_loop_support::raw_agent_loop_host_error(
        "local_dev_outbound_delivery",
        operation,
        kind,
        "outbound delivery target operation failed",
        error,
    )
}

fn approval_store_error(operation: &'static str, error: RunStateError) -> AgentLoopHostError {
    ironclaw_loop_support::raw_agent_loop_host_error(
        "local_dev_outbound_delivery",
        operation,
        AgentLoopHostErrorKind::Unavailable,
        "outbound delivery approval state operation failed",
        error,
    )
}

fn approval_lease_error(
    operation: &'static str,
    error: CapabilityLeaseError,
) -> AgentLoopHostError {
    let kind = match error {
        CapabilityLeaseError::UnknownLease { .. }
        | CapabilityLeaseError::ExpiredLease { .. }
        | CapabilityLeaseError::ExhaustedLease { .. }
        | CapabilityLeaseError::UnclaimedFingerprintLease { .. }
        | CapabilityLeaseError::FingerprintMismatch { .. }
        | CapabilityLeaseError::InactiveLease { .. } => AgentLoopHostErrorKind::Unauthorized,
        CapabilityLeaseError::Persistence { .. }
        | CapabilityLeaseError::VersionMismatch
        | CapabilityLeaseError::CasExhausted => AgentLoopHostErrorKind::Unavailable,
    };
    ironclaw_loop_support::raw_agent_loop_host_error(
        "local_dev_outbound_delivery",
        operation,
        kind,
        "outbound delivery approval lease operation failed",
        error,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_outbound_delivery_targets_list_input_rejects_empty_channel() {
        let error =
            parse_outbound_delivery_targets_list_input(&serde_json::json!({"channel": "  "}))
                .expect_err("empty channel should fail");

        assert!(error.to_string().contains("must be a non-empty string"));
    }

    #[test]
    fn parse_outbound_delivery_targets_list_input_rejects_non_object_input() {
        let error = parse_outbound_delivery_targets_list_input(&serde_json::Value::Null)
            .expect_err("non-object input should fail");

        assert!(error.to_string().contains("input must be an object"));
    }

    #[test]
    fn parse_outbound_delivery_targets_list_input_rejects_unknown_fields() {
        let error =
            parse_outbound_delivery_targets_list_input(&serde_json::json!({"unexpected": "value"}))
                .expect_err("unknown fields should fail");

        assert!(error.to_string().contains("unsupported field `unexpected`"));
    }

    #[test]
    fn parse_outbound_delivery_target_set_input_requires_target_id() {
        let error = parse_outbound_delivery_target_set_input(&serde_json::json!({}))
            .expect_err("missing target id should fail");

        assert!(error.to_string().contains("target_id must be a string"));
    }

    #[test]
    fn parse_outbound_delivery_target_set_input_rejects_malformed_target_id() {
        let error = parse_outbound_delivery_target_set_input(&serde_json::json!({
            "target_id": "bad\nid"
        }))
        .expect_err("malformed target id should fail");

        assert!(error.to_string().contains("target_id is invalid"));
    }

    #[test]
    fn parse_outbound_delivery_target_set_input_rejects_unknown_fields() {
        let error = parse_outbound_delivery_target_set_input(&serde_json::json!({
            "target_id": "slack:test",
            "unexpected": "value"
        }))
        .expect_err("unknown fields should fail");

        assert!(error.to_string().contains("unsupported field `unexpected`"));
    }
}
