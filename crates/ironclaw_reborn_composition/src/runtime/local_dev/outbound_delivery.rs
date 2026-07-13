use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_approvals::ToolPermissionOverride;
use ironclaw_authorization::{CapabilityLeaseError, CapabilityLeaseStatus, CapabilityLeaseStore};
use ironclaw_host_api::{
    Action, ApprovalRequest, ApprovalRequestId, CapabilityGrantId, CapabilityId, CorrelationId,
    InvocationFingerprint, InvocationId, Principal, ResourceEstimate, ResourceScope, UserId,
};
use ironclaw_loop_support::{CapabilityResultWrite, DurablePersistence};
use ironclaw_product_workflow::{
    OutboundPreferencesProductFacade, RebornOutboundDeliveryTargetId, RebornServicesError,
    RebornServicesErrorCode, WebUiAuthenticatedCaller,
};
use ironclaw_run_state::{ApprovalRequestStore, ApprovalStatus, RunStateError};
use ironclaw_turns::{
    LoopGateRef,
    run_profile::{
        AgentLoopHostError, AgentLoopHostErrorKind, CapabilityApprovalResume, CapabilityDenied,
        CapabilityDeniedReasonKind, CapabilityFailure, CapabilityFailureKind, CapabilityInputRef,
        CapabilityOutcome, CapabilityProgress, CapabilityResultMessage, CapabilityResumeToken,
        ConcurrencyHint, LoopRunContext,
    },
};

use crate::outbound::{
    OUTBOUND_DELIVERY_TARGET_SET_CAPABILITY_ID, OUTBOUND_DELIVERY_TARGET_SET_DESCRIPTION,
    OUTBOUND_DELIVERY_TARGET_SET_PROVIDER_TOOL_NAME, OUTBOUND_DELIVERY_TARGETS_LIST_CAPABILITY_ID,
    OUTBOUND_DELIVERY_TARGETS_LIST_DESCRIPTION, OUTBOUND_DELIVERY_TARGETS_LIST_PROVIDER_TOOL_NAME,
    OutboundDeliveryCapabilityInputError, list_outbound_delivery_targets_for_model,
    outbound_delivery_synthetic_provider, outbound_delivery_target_set_input_schema,
    outbound_delivery_targets_list_input_schema, parse_outbound_delivery_target_set_input,
    parse_outbound_delivery_targets_list_input, set_outbound_delivery_target_for_model,
};
use crate::profile_approval_authorization::ApprovalSettingsProvider;
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
    approval_settings: Arc<dyn ApprovalSettingsProvider>,
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
                approval_settings,
            }),
        ),
    ])
}

/// Test-only bridge (C-SYNTH outbound seam): wrap `inner` with just the two
/// `outbound_delivery_*` local-dev synthetic capabilities, so the Reborn
/// integration-test harness can inject them onto its host-runtime capability
/// port the same way production does
/// (`RefreshingLocalDevCapabilityPort::build_inner`). Reuses the real
/// `outbound_delivery_capabilities` + `wrap_local_dev_synthetic_capabilities`,
/// so the test path never hand-mirrors the production wrap; builds the same
/// `StoreApprovalSettingsProvider` production wires so the settings-decision
/// (`Allow`/`Ask`/`Deny`) route is exercised for real. Mirrors
/// `wrap_project_create_capability_for_test`. Tests only.
#[cfg(feature = "test-support")]
pub(crate) fn wrap_outbound_delivery_capabilities_for_test(
    inner: Arc<dyn ironclaw_turns::run_profile::LoopCapabilityPort>,
    parts: crate::test_support::OutboundDeliveryCapabilityTestParts,
) -> Result<Arc<dyn ironclaw_turns::run_profile::LoopCapabilityPort>, AgentLoopHostError> {
    let crate::test_support::OutboundDeliveryCapabilityTestParts {
        facade,
        fallback_user_id,
        approval_requests,
        capability_leases,
        tool_permission_overrides,
        auto_approve,
        persistent_policies,
        target_set_requires_approval,
        run_context,
        input_resolver,
        result_writer,
    } = parts;

    let approval_settings: Arc<dyn ApprovalSettingsProvider> = Arc::new(
        crate::local_dev_authorization::StoreApprovalSettingsProvider::new(
            tool_permission_overrides,
            auto_approve,
            persistent_policies,
        ),
    );
    let capabilities = outbound_delivery_capabilities(
        facade,
        fallback_user_id,
        approval_requests,
        capability_leases,
        target_set_requires_approval,
        approval_settings,
    )?;
    super::synthetic_capability::wrap_local_dev_synthetic_capabilities(
        inner,
        capabilities,
        run_context,
        input_resolver,
        result_writer,
        // trajectory_observer: None — not wired in the integration-test harness.
        None,
    )
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
            match list_outbound_delivery_targets_for_model(self.facade.as_ref(), caller, input)
                .await
            {
                Ok(response) => response,
                // A model-recoverable service failure (invalid request, not
                // found, denied, conflict, rate limit, transient unavailability)
                // must surface as a model-visible tool error so the run continues
                // and the model can adapt — NOT a terminal
                // `Err(AgentLoopHostError)`, which `ironclaw_agent_loop`'s executor
                // maps to a run-ending `HostUnavailable { stage: Capability }`.
                // Only a genuine internal bug stays terminal. See
                // `outbound_delivery_outcome`.
                Err(error) => return outbound_delivery_outcome(error),
            };
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
    approval_settings: Arc<dyn ApprovalSettingsProvider>,
}

struct ApprovedDispatchLease {
    scope: ResourceScope,
    lease_id: CapabilityGrantId,
}

/// Outcome of verifying an approval resume before dispatch.
///
/// `Approved` carries the claimed lease to consume; `Denied` carries a
/// model-visible denial so the run continues and the user can re-request
/// approval, instead of a terminal `Err(AgentLoopHostError)` that would end the
/// run (see .claude/rules/agent-loop-capabilities.md, Invariant 1).
enum ApprovedResumeDecision {
    Approved(ApprovedDispatchLease),
    Denied(CapabilityDenied),
}

enum OutboundDeliveryApprovalSettingsDecision {
    Allow,
    Ask,
    Deny,
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
        let capability_id = outbound_delivery_target_set_capability_id()?;
        let approved_lease = if self.requires_approval {
            match invocation.request.approval_resume.clone() {
                Some(resume) => {
                    // A lost / expired / not-yet-granted approval lease is
                    // recoverable: the user can re-request approval. Route those
                    // arms to `Ok(Denied)` so the run continues instead of a
                    // terminal `Err(Unauthorized)`. Only genuine infra faults
                    // (lease persistence / CAS) stay terminal.
                    match self
                        .verify_approved_resume(&invocation, &resume, &input)
                        .await?
                    {
                        ApprovedResumeDecision::Approved(lease) => Some(lease),
                        ApprovedResumeDecision::Denied(denied) => {
                            return Ok(CapabilityOutcome::Denied(denied));
                        }
                    }
                }
                None => match self.settings_decision(&invocation, &capability_id).await? {
                    OutboundDeliveryApprovalSettingsDecision::Allow => None,
                    OutboundDeliveryApprovalSettingsDecision::Ask => {
                        return self
                            .request_approval(&invocation, &input, target_input.target_id())
                            .await;
                    }
                    OutboundDeliveryApprovalSettingsDecision::Deny => {
                        return Ok(CapabilityOutcome::Failed(CapabilityFailure {
                            error_kind: CapabilityFailureKind::PolicyDenied,
                            safe_summary: "outbound delivery target setter is disabled by tool approval settings".to_string(),
                            detail: None,
                        }));
                    }
                },
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

        let caller = caller_for_run(&invocation, &self.fallback_user_id);
        let response = match set_outbound_delivery_target_for_model(
            self.facade.as_ref(),
            caller,
            target_input,
        )
        .await
        {
            Ok(response) => response,
            // See `outbound_delivery_outcome`: recoverable service errors are
            // model-visible failures, not terminal host errors.
            Err(error) => return outbound_delivery_outcome(error),
        };
        if let Some(approved_lease) = approved_lease {
            // Lease consumption races (expired / exhausted between claim and
            // consume) are recoverable — surface as `Denied` so the model can
            // re-request approval rather than ending the run. Infra faults stay
            // terminal. See `approval_lease_outcome`.
            match self
                .capability_leases
                .consume(&approved_lease.scope, approved_lease.lease_id)
                .await
            {
                Ok(_) => {}
                Err(error) => match approval_lease_outcome("consume_approval_lease", error) {
                    Ok(denied) => return Ok(CapabilityOutcome::Denied(denied)),
                    Err(host_error) => return Err(host_error),
                },
            }
        }
        let output = serde_json::to_value(response).map_err(|error| {
            AgentLoopHostError::new(
                AgentLoopHostErrorKind::Internal,
                format!("outbound delivery target set output serialization failed: {error}"),
            )
        })?;
        // The safe summary must not interpolate the raw, model-controlled
        // `target_id`: a delimiter (`/ < > [ ] { } ` + "`" + ` \`) trips
        // `ToolResultSafeSummary` validation in `append_capability_result_ref`,
        // which surfaces as a terminal `HostUnavailable` that kills the whole
        // turn (see .claude/rules/agent-loop-capabilities.md, Invariant 2). The
        // model still gets the target id from the result `output`; the summary
        // stays a fixed, delimiter-free string.
        write_completed_result(invocation, output, "set delivery target".to_string()).await
    }
}

impl OutboundDeliveryTargetSetHandler {
    async fn settings_decision(
        &self,
        invocation: &LocalDevSyntheticCapabilityInvocation,
        capability_id: &CapabilityId,
    ) -> Result<OutboundDeliveryApprovalSettingsDecision, AgentLoopHostError> {
        let scope = settings_scope_for_run(&invocation.run_context, &self.fallback_user_id);
        let grantee = outbound_delivery_target_set_grantee()?;
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
                    requested_by: outbound_delivery_target_set_grantee()?,
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
    ) -> Result<ApprovedResumeDecision, AgentLoopHostError> {
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
        // A missing or not-yet-granted approval record is recoverable: the user
        // can re-request approval. Surface `Denied` so the run continues rather
        // than ending it with a terminal `Err(Unauthorized)`.
        let approval_record = match self
            .approval_requests
            .get(&scope, resume.approval_request_id)
            .await
            .map_err(|error| approval_store_error("load_approval", error))?
        {
            Some(record) => record,
            None => {
                return Ok(ApprovedResumeDecision::Denied(approval_denied(
                    "outbound delivery target approval is unavailable; re-request approval",
                )?));
            }
        };
        if approval_record.status != ApprovalStatus::Approved {
            return Ok(ApprovedResumeDecision::Denied(approval_denied(
                "outbound delivery target approval has not been granted; re-request approval",
            )?));
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
            // The approval lease expired or was lost between approval and resume;
            // recoverable by re-requesting approval, so deny instead of killing
            // the run.
            None => {
                return Ok(ApprovedResumeDecision::Denied(approval_denied(
                    "outbound delivery target approval lease is unavailable; re-request approval",
                )?));
            }
        };
        // A lease-state failure on claim (expired / exhausted / fingerprint
        // mismatch) is recoverable: deny and let the model re-request approval.
        // Only infra faults (persistence / CAS) stay terminal.
        match self
            .capability_leases
            .claim(&scope, lease.grant.id, &fingerprint)
            .await
        {
            Ok(_) => {}
            Err(error) => match approval_lease_outcome("claim_approval_lease", error) {
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
            durable_persistence: DurablePersistence::Persist,
        })
        .await?;
    Ok(CapabilityOutcome::Completed(CapabilityResultMessage {
        result_ref: write_result.result_ref,
        safe_summary,
        progress: CapabilityProgress::MadeProgress,
        terminate_hint: false,
        byte_len: write_result.byte_len,
        output_digest: write_result.output_digest,
        model_observation: write_result.model_observation,
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

fn settings_scope_for_run(
    run_context: &LoopRunContext,
    fallback_user_id: &UserId,
) -> ResourceScope {
    ResourceScope {
        tenant_id: run_context.scope.tenant_id.clone(),
        user_id: effective_user_id(run_context, fallback_user_id),
        agent_id: None,
        project_id: None,
        mission_id: None,
        thread_id: None,
        invocation_id: InvocationId::new(),
    }
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

fn outbound_delivery_target_set_grantee() -> Result<Principal, AgentLoopHostError> {
    outbound_delivery_synthetic_provider()
        .map(Principal::Extension)
        .map_err(|error| {
            AgentLoopHostError::new(
                AgentLoopHostErrorKind::Internal,
                format!("outbound delivery synthetic provider id is invalid: {error}"),
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

/// Disposition an outbound-delivery service failure into either a model-visible,
/// recoverable capability outcome or a terminal host error.
///
/// As with `project_service_outcome` and `skill_activation_selection_outcome`,
/// the two arms map onto the executor's two failure paths
/// (`ironclaw_agent_loop::executor::mapping`): `CapabilityOutcome::Failed` /
/// `Denied` is handed back to the model and the run continues (so the model can
/// fix its request or tell the user), while `Err(AgentLoopHostError)` becomes a
/// run-ending `HostUnavailable { stage: Capability }`. Only a genuine internal
/// bug stays terminal — invalid input, not-found, denials, conflicts, rate
/// limits, and transient unavailability are all surfaced to the model instead of
/// killing the turn.
///
/// Safe summaries stay fixed and host-authored: `RebornServicesError` carries a
/// free-form `field` that could contain a forbidden delimiter/marker and remap a
/// recoverable arm into a terminal `HostUnavailable` (Invariant 2).
fn outbound_delivery_outcome(
    error: RebornServicesError,
) -> Result<CapabilityOutcome, AgentLoopHostError> {
    match error.code {
        RebornServicesErrorCode::InvalidRequest | RebornServicesErrorCode::NotFound => {
            Ok(CapabilityOutcome::Failed(CapabilityFailure {
                error_kind: CapabilityFailureKind::InvalidInput,
                safe_summary: "invalid outbound delivery request".to_string(),
                detail: None,
            }))
        }
        RebornServicesErrorCode::Unauthenticated | RebornServicesErrorCode::Forbidden => {
            Ok(CapabilityOutcome::Denied(approval_denied(
                "not permitted to change the outbound delivery target",
            )?))
        }
        RebornServicesErrorCode::Conflict => Ok(CapabilityOutcome::Failed(CapabilityFailure {
            error_kind: CapabilityFailureKind::OperationFailed,
            safe_summary: "outbound delivery target operation conflicted".to_string(),
            detail: None,
        })),
        RebornServicesErrorCode::RateLimited => Ok(CapabilityOutcome::Failed(CapabilityFailure {
            error_kind: CapabilityFailureKind::Resource,
            safe_summary: "outbound delivery target operation rate limited".to_string(),
            detail: None,
        })),
        RebornServicesErrorCode::Unavailable => Ok(CapabilityOutcome::Failed(CapabilityFailure {
            error_kind: CapabilityFailureKind::Unavailable,
            safe_summary: "outbound delivery service temporarily unavailable".to_string(),
            detail: None,
        })),
        RebornServicesErrorCode::Internal => Err(AgentLoopHostError::new(
            AgentLoopHostErrorKind::Internal,
            "outbound delivery target operation failed",
        )),
    }
}

/// Build a model-visible `CapabilityDenied` with a fixed, host-authored summary.
/// The reason kind is a charset-safe identifier, so it never trips
/// safe-summary/identifier validation.
fn approval_denied(safe_summary: &str) -> Result<CapabilityDenied, AgentLoopHostError> {
    Ok(CapabilityDenied {
        reason_kind: CapabilityDeniedReasonKind::unknown("outbound_delivery_approval_required")
            .map_err(|reason| {
                AgentLoopHostError::new(
                    AgentLoopHostErrorKind::Internal,
                    format!("outbound delivery denial reason kind is invalid: {reason}"),
                )
            })?,
        safe_summary: safe_summary.to_string(),
    })
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

/// Disposition a capability-lease failure into either a model-visible denial
/// (recoverable — the user can re-request approval) or a terminal host error.
///
/// Lease-state arms (unknown / expired / exhausted / unclaimed-fingerprint /
/// fingerprint-mismatch / inactive) describe a lost or stale approval lease,
/// which the model can recover from by re-requesting approval — so they return
/// `Ok(CapabilityDenied)`. Genuine infra faults (lease persistence, version
/// mismatch, CAS exhaustion) stay terminal `Err(AgentLoopHostError)`.
fn approval_lease_outcome(
    operation: &'static str,
    error: CapabilityLeaseError,
) -> Result<CapabilityDenied, AgentLoopHostError> {
    match error {
        CapabilityLeaseError::UnknownLease { .. }
        | CapabilityLeaseError::ExpiredLease { .. }
        | CapabilityLeaseError::ExhaustedLease { .. }
        | CapabilityLeaseError::UnclaimedFingerprintLease { .. }
        | CapabilityLeaseError::FingerprintMismatch { .. }
        | CapabilityLeaseError::InactiveLease { .. } => approval_denied(
            "outbound delivery target approval lease is no longer valid; re-request approval",
        ),
        CapabilityLeaseError::Persistence { .. }
        | CapabilityLeaseError::VersionMismatch
        | CapabilityLeaseError::CasExhausted => {
            Err(ironclaw_loop_support::raw_agent_loop_host_error(
                "local_dev_outbound_delivery",
                operation,
                AgentLoopHostErrorKind::Unavailable,
                "outbound delivery approval lease operation failed",
                error,
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironclaw_product_workflow::RebornServicesErrorKind;
    use ironclaw_turns::run_profile::LoopSafeSummary;

    fn service_error(code: RebornServicesErrorCode) -> RebornServicesError {
        RebornServicesError {
            code,
            kind: RebornServicesErrorKind::Internal,
            status_code: 500,
            retryable: false,
            // A free-form `field` carrying a forbidden delimiter is the exact
            // shape that, if interpolated into a safe summary, would remap a
            // recoverable arm into a terminal `HostUnavailable` (Invariant 2).
            field: Some("slack/<channel>".to_string()),
            validation_code: None,
        }
    }

    fn lease_error_unknown() -> CapabilityLeaseError {
        CapabilityLeaseError::ExpiredLease {
            lease_id: CapabilityGrantId::new(),
        }
    }

    #[test]
    fn invalid_request_is_a_recoverable_tool_failure_not_terminal() {
        let outcome =
            outbound_delivery_outcome(service_error(RebornServicesErrorCode::InvalidRequest))
                .expect("invalid request must be a model-visible failure, not terminal");

        match outcome {
            CapabilityOutcome::Failed(failure) => {
                assert_eq!(failure.error_kind, CapabilityFailureKind::InvalidInput);
                LoopSafeSummary::new(failure.safe_summary)
                    .expect("safe summary must satisfy the loop validator");
            }
            other => panic!("expected CapabilityOutcome::Failed, got {other:?}"),
        }
    }

    #[test]
    fn not_found_is_a_recoverable_tool_failure_not_terminal() {
        let outcome = outbound_delivery_outcome(service_error(RebornServicesErrorCode::NotFound))
            .expect("not found must be a model-visible failure, not terminal");

        assert!(matches!(
            outcome,
            CapabilityOutcome::Failed(failure)
                if failure.error_kind == CapabilityFailureKind::InvalidInput
        ));
    }

    #[test]
    fn unauthenticated_is_a_recoverable_denial_not_terminal() {
        let outcome =
            outbound_delivery_outcome(service_error(RebornServicesErrorCode::Unauthenticated))
                .expect("unauthenticated must be a model-visible denial, not terminal");

        match outcome {
            CapabilityOutcome::Denied(denied) => {
                LoopSafeSummary::new(denied.safe_summary)
                    .expect("safe summary must satisfy the loop validator");
            }
            other => panic!("expected CapabilityOutcome::Denied, got {other:?}"),
        }
    }

    #[test]
    fn forbidden_is_a_recoverable_denial_not_terminal() {
        let outcome = outbound_delivery_outcome(service_error(RebornServicesErrorCode::Forbidden))
            .expect("forbidden must be a model-visible denial, not terminal");

        assert!(matches!(outcome, CapabilityOutcome::Denied(_)));
    }

    #[test]
    fn conflict_is_a_recoverable_tool_failure_not_terminal() {
        let outcome = outbound_delivery_outcome(service_error(RebornServicesErrorCode::Conflict))
            .expect("conflict must be a model-visible failure, not terminal");

        assert!(matches!(
            outcome,
            CapabilityOutcome::Failed(failure)
                if failure.error_kind == CapabilityFailureKind::OperationFailed
        ));
    }

    #[test]
    fn rate_limited_is_a_recoverable_tool_failure_not_terminal() {
        let outcome =
            outbound_delivery_outcome(service_error(RebornServicesErrorCode::RateLimited))
                .expect("rate limited must be a model-visible failure, not terminal");

        assert!(matches!(
            outcome,
            CapabilityOutcome::Failed(failure)
                if failure.error_kind == CapabilityFailureKind::Resource
        ));
    }

    #[test]
    fn unavailable_is_a_recoverable_tool_failure_not_terminal() {
        let outcome =
            outbound_delivery_outcome(service_error(RebornServicesErrorCode::Unavailable))
                .expect("transient unavailability must not kill the run");

        assert!(matches!(
            outcome,
            CapabilityOutcome::Failed(failure)
                if failure.error_kind == CapabilityFailureKind::Unavailable
        ));
    }

    #[test]
    fn internal_service_error_stays_terminal() {
        let error = outbound_delivery_outcome(service_error(RebornServicesErrorCode::Internal))
            .expect_err("internal bugs must stay terminal");

        assert_eq!(error.kind, AgentLoopHostErrorKind::Internal);
    }

    #[test]
    fn service_error_field_with_delimiter_does_not_reach_safe_summary() {
        // A `field` carrying a `/ < >` delimiter must never poison the fixed,
        // host-authored safe summary. Each recoverable outcome's summary must
        // still pass the loop safe-summary validator that fires at
        // `append_capability_result_ref` (the terminal-failure boundary).
        for code in [
            RebornServicesErrorCode::InvalidRequest,
            RebornServicesErrorCode::NotFound,
            RebornServicesErrorCode::Unauthenticated,
            RebornServicesErrorCode::Forbidden,
            RebornServicesErrorCode::Conflict,
            RebornServicesErrorCode::RateLimited,
            RebornServicesErrorCode::Unavailable,
        ] {
            let outcome = outbound_delivery_outcome(service_error(code))
                .unwrap_or_else(|_| panic!("{code:?} must be recoverable"));
            let summary = match outcome {
                CapabilityOutcome::Failed(failure) => failure.safe_summary,
                CapabilityOutcome::Denied(denied) => denied.safe_summary,
                other => panic!("expected a recoverable outcome, got {other:?}"),
            };
            assert!(
                !summary.contains("slack/<channel>"),
                "summary must not interpolate the service error field: {summary}"
            );
            LoopSafeSummary::new(summary).unwrap_or_else(|reason| {
                panic!("{code:?} summary must satisfy the loop validator: {reason}")
            });
        }
    }

    #[test]
    fn set_delivery_target_summary_is_fixed_and_validator_safe() {
        // The set-target completion summary is a fixed host-authored string and
        // must not interpolate the model-controlled target id. A target id may
        // legally contain a `/ < >` delimiter (it is rejected only for control
        // chars), so interpolating it would trip the safe-summary validator and
        // kill the run. Confirm the delimiter-bearing id parses and that the
        // fixed summary validates.
        let target = RebornOutboundDeliveryTargetId::new("slack/<channel>")
            .expect("a delimiter-bearing target id is a valid target id");
        assert!(target.as_str().contains('/'));
        LoopSafeSummary::new("set delivery target")
            .expect("the fixed set-target summary must satisfy the loop validator");
        // The previous interpolated summary would have been rejected:
        LoopSafeSummary::new(format!("set delivery target to {}", target.as_str()))
            .expect_err("interpolating the delimiter-bearing target id must trip the validator");
    }

    #[test]
    fn expired_lease_is_a_recoverable_denial_not_terminal() {
        let denied = approval_lease_outcome("claim_approval_lease", lease_error_unknown())
            .expect("an expired approval lease must be a model-visible denial, not terminal");

        LoopSafeSummary::new(denied.safe_summary)
            .expect("denial safe summary must satisfy the loop validator");
    }

    #[test]
    fn lease_persistence_failure_stays_terminal() {
        let error = approval_lease_outcome(
            "claim_approval_lease",
            CapabilityLeaseError::Persistence {
                reason: "disk".to_string(),
            },
        )
        .expect_err("genuine lease infra faults must stay terminal");

        assert_eq!(error.kind, AgentLoopHostErrorKind::Unavailable);
    }

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
