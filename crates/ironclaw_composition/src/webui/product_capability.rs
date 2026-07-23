//! Generic product command adapter into the canonical host-runtime pipeline.

use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_extensions::ExtensionRegistry;
use ironclaw_filesystem::{
    CasApply, CompositeRootFilesystem, ContentType, Entry, LibSqlRootFilesystem,
    PostgresRootFilesystem, RootFilesystem, ScopedFilesystem, cas_update,
};
use ironclaw_host_api::{
    ActivityId, Blocked, CapabilityDescriptor, CapabilityGrant, CapabilityGrantId, CapabilityId,
    CapabilitySet, CorrelationId, Denial, DenyReason, DenyRef, EffectKind, ExecutionContext,
    ExtensionId, FailureKind, GateRef, GateWaypoint, GrantConstraints, InvocationId,
    InvocationOrigin, MountView, NetworkPolicy, Outcome, OutcomeRefs, Principal, ProcessRef,
    ProcessWaypoint, ProductKind, Resolution, ResourceEstimate, ResourceScope, ResultPreviewMeta,
    ResultProgress, ResultRef, ResumeToken, RuntimeKind, SafeSummary, ScopedPath, Suspension,
    TerminateHint, ToolVerdict, TrustClass,
};
use ironclaw_host_runtime::{HostRuntime, RuntimeCapabilityOutcome, RuntimeFailureKind};
use ironclaw_product_workflow::{
    IronClawServicesError, IronClawServicesErrorCode, IronClawServicesErrorKind,
    ProductCapabilityInvoker, SKILL_AUTO_ACTIVATE_SET_CAPABILITY_ID, SKILL_INSTALL_CAPABILITY_ID,
    SKILL_REMOVE_CAPABILITY_ID, SKILL_UPDATE_CAPABILITY_ID, WebUiAuthenticatedCaller,
};

use crate::factory::{
    IronClawProductionRuntimeServices, IronClawServices, production_skill_management_mount_view,
};

const PRODUCT_RESULT_MAX_BYTES: usize = 4 * 1024 * 1024;
const PRODUCT_RESULT_ROOT: &str = "/product-results";
const PRODUCT_INGRESS_EXTENSION_ID: &str = "ironclaw_webui";

#[derive(Clone)]
pub(crate) enum RuntimeProductCapabilityInvoker {
    Available {
        host_runtime: Arc<dyn HostRuntime>,
        registry: Arc<ExtensionRegistry>,
        results: ProductResultFilesystem,
        mounts: ProductCapabilityMounts,
    },
    Unavailable,
}

#[derive(Clone)]
pub(crate) enum ProductResultFilesystem {
    Composite(Arc<ScopedFilesystem<CompositeRootFilesystem>>),
    LibSql(Arc<ScopedFilesystem<LibSqlRootFilesystem>>),
    Postgres(Arc<ScopedFilesystem<PostgresRootFilesystem>>),
}

#[derive(Clone, Copy)]
pub(crate) enum ProductCapabilityMounts {
    LocalDev,
    Production,
}

impl RuntimeProductCapabilityInvoker {
    pub(crate) fn from_services(services: &IronClawServices) -> Self {
        let Some(host_runtime) = services.host_runtime.as_ref().map(Arc::clone) else {
            return Self::Unavailable;
        };
        let (results, registry, mounts) = if let Some(local) = &services.local_runtime {
            (
                ProductResultFilesystem::Composite(crate::wrap_scoped(Arc::clone(
                    &local.extension_filesystem,
                ))),
                Arc::clone(&local.extension_registry),
                ProductCapabilityMounts::LocalDev,
            )
        } else if let Some(production) = &services.production_runtime {
            match production {
                IronClawProductionRuntimeServices::LibSql(graph) => (
                    ProductResultFilesystem::LibSql(Arc::clone(&graph.scoped_filesystem)),
                    Arc::clone(&graph.extension_registry),
                    ProductCapabilityMounts::Production,
                ),
                IronClawProductionRuntimeServices::Postgres(graph) => (
                    ProductResultFilesystem::Postgres(Arc::clone(&graph.scoped_filesystem)),
                    Arc::clone(&graph.extension_registry),
                    ProductCapabilityMounts::Production,
                ),
            }
        } else {
            return Self::Unavailable;
        };
        Self::Available {
            host_runtime,
            registry,
            results,
            mounts,
        }
    }
}

#[async_trait]
impl ProductCapabilityInvoker for RuntimeProductCapabilityInvoker {
    async fn invoke(
        &self,
        caller: WebUiAuthenticatedCaller,
        capability: CapabilityId,
        input: serde_json::Value,
        activity_id: ActivityId,
    ) -> Result<Resolution, IronClawServicesError> {
        let Self::Available {
            host_runtime,
            registry,
            results,
            mounts,
        } = self
        else {
            return Err(product_runtime_unavailable());
        };
        // The origin-to-gate matrix is still provisional in today's kernel.
        // Encode the direct user gesture as one exact, host-issued grant. The
        // runtime independently re-resolves the descriptor and authorizes it,
        // so a concurrent stronger replacement no longer fits this attenuated
        // grant and fails closed.
        let descriptor = registry.get_capability(&capability);
        let context = product_execution_context(&caller, activity_id, descriptor, *mounts)?;
        let scope = context.resource_scope.clone();
        let invocation_id = context.invocation_id;
        let requested_capability = capability.clone();
        let outcome = host_runtime
            .invoke_capability((context, capability, ResourceEstimate::default(), input))
            .await
            .map_err(IronClawServicesError::internal_from)?;
        ensure_matching_capability(&requested_capability, &outcome)?;
        product_resolution(results, &scope, invocation_id, outcome).await
    }
}

fn product_execution_context(
    caller: &WebUiAuthenticatedCaller,
    activity_id: ActivityId,
    descriptor: Option<&CapabilityDescriptor>,
    mounts: ProductCapabilityMounts,
) -> Result<ExecutionContext, IronClawServicesError> {
    let invocation_id = InvocationId::from_uuid(activity_id.as_uuid());
    let scope = ResourceScope {
        tenant_id: caller.tenant_id.clone(),
        user_id: caller.user_id.clone(),
        agent_id: caller.agent_id.clone(),
        project_id: caller.project_id.clone(),
        mission_id: None,
        thread_id: None,
        invocation_id,
    };
    let extension_id = ExtensionId::new(PRODUCT_INGRESS_EXTENSION_ID)
        .map_err(IronClawServicesError::internal_from)?;
    let invocation_mounts = product_invocation_mounts(&scope, descriptor, mounts)?;
    let grants = descriptor
        .map(|descriptor| CapabilitySet {
            grants: vec![product_gesture_grant(
                descriptor,
                &extension_id,
                invocation_mounts.clone(),
            )],
        })
        .unwrap_or_default();
    let context = ExecutionContext {
        invocation_id,
        correlation_id: CorrelationId::new(),
        process_id: None,
        parent_process_id: None,
        tenant_id: caller.tenant_id.clone(),
        user_id: caller.user_id.clone(),
        authenticated_actor_user_id: Some(caller.user_id.clone()),
        agent_id: caller.agent_id.clone(),
        project_id: caller.project_id.clone(),
        mission_id: None,
        thread_id: None,
        run_id: None,
        origin: Some(InvocationOrigin::Product(
            ProductKind::new("webui").map_err(IronClawServicesError::internal_from)?,
        )),
        extension_id,
        // Both are provisional input to the kernel. Resolve/authorize derives
        // the real lane and effective trust from the capability descriptor.
        runtime: RuntimeKind::FirstParty,
        trust: TrustClass::Sandbox,
        grants,
        mounts: invocation_mounts,
        resource_scope: scope,
    };
    context
        .validate()
        .map_err(IronClawServicesError::internal_from)?;
    Ok(context)
}

fn product_gesture_grant(
    descriptor: &CapabilityDescriptor,
    product_ingress: &ExtensionId,
    mounts: MountView,
) -> CapabilityGrant {
    let mut secrets = Vec::new();
    let mut network_targets = descriptor.network_targets.clone();
    for credential in &descriptor.runtime_credentials {
        if !secrets.contains(&credential.handle) {
            secrets.push(credential.handle.clone());
        }
        if !network_targets.contains(&credential.audience) {
            network_targets.push(credential.audience.clone());
        }
    }
    let network = if descriptor.effects.contains(&EffectKind::Network) && network_targets.is_empty()
    {
        crate::builtin_capability_policy::dev_wildcard_network_policy()
    } else {
        let has_network_targets = !network_targets.is_empty();
        NetworkPolicy {
            allowed_targets: network_targets,
            // An empty policy must remain unconstrained. Marking it as
            // private-range constrained would synthesize an `ApplyNetworkPolicy`
            // obligation for a capability that has no network surface, and fail
            // before dispatch when no network-policy store is composed.
            // Networked capabilities retain the private-IP guard on their
            // manifest allowlist.
            deny_private_ip_ranges: has_network_targets,
            max_egress_bytes: None,
        }
    };
    CapabilityGrant {
        id: CapabilityGrantId::new(),
        capability: descriptor.id.clone(),
        grantee: Principal::Extension(product_ingress.clone()),
        issued_by: Principal::HostRuntime,
        constraints: GrantConstraints {
            allowed_effects: descriptor.effects.clone(),
            mounts,
            network,
            secrets,
            resource_ceiling: None,
            expires_at: None,
            max_invocations: Some(1),
        },
    }
}

fn product_invocation_mounts(
    scope: &ResourceScope,
    descriptor: Option<&CapabilityDescriptor>,
    mounts: ProductCapabilityMounts,
) -> Result<MountView, IronClawServicesError> {
    let Some(descriptor) = descriptor else {
        return Ok(MountView::default());
    };
    if !is_skill_management_capability(&descriptor.id) {
        return Ok(MountView::default());
    }
    match mounts {
        ProductCapabilityMounts::LocalDev => {
            crate::local_dev_mounts::scoped_skill_management_mount_view(scope)
                .map_err(IronClawServicesError::internal_from)
        }
        ProductCapabilityMounts::Production => production_skill_management_mount_view(scope)
            .map_err(IronClawServicesError::internal_from),
    }
}

fn is_skill_management_capability(capability: &CapabilityId) -> bool {
    matches!(
        capability.as_str(),
        SKILL_INSTALL_CAPABILITY_ID
            | SKILL_UPDATE_CAPABILITY_ID
            | SKILL_REMOVE_CAPABILITY_ID
            | SKILL_AUTO_ACTIVATE_SET_CAPABILITY_ID
    )
}

async fn product_resolution(
    results: &ProductResultFilesystem,
    scope: &ResourceScope,
    invocation_id: InvocationId,
    outcome: RuntimeCapabilityOutcome,
) -> Result<Resolution, IronClawServicesError> {
    match outcome {
        RuntimeCapabilityOutcome::Completed(completed) => {
            let body = serde_json::to_vec(&completed.output)
                .map_err(IronClawServicesError::internal_from)?;
            if body.len() > PRODUCT_RESULT_MAX_BYTES {
                return Err(IronClawServicesError::internal_from(
                    "product capability result exceeded the durable output bound",
                ));
            }
            let result_ref = ResultRef::from_uuid(invocation_id.as_uuid());
            results.persist(scope, result_ref, body.clone()).await?;
            Ok(Resolution::Done(Outcome {
                refs: OutcomeRefs {
                    result: result_ref,
                    byte_len: body.len() as u64,
                    preview: None,
                    preview_meta: ResultPreviewMeta::default(),
                    origin: None,
                    output_digest: None,
                },
                verdict: ToolVerdict::Success,
                summary: fixed_summary("capability completed"),
                progress: ResultProgress::MadeProgress,
                terminate_hint: TerminateHint::Continue,
            }))
        }
        RuntimeCapabilityOutcome::ApprovalRequired(gate) => {
            let resume = ResumeToken::new(invocation_id.to_string())
                .map_err(IronClawServicesError::internal_from)?;
            Ok(Resolution::Blocked(Blocked::Approval(
                GateWaypoint::new(GateRef::for_approval_request(gate.approval_request_id))
                    .with_resume(resume),
            )))
        }
        RuntimeCapabilityOutcome::AuthRequired(gate) => {
            let resume = ResumeToken::new(invocation_id.to_string())
                .map_err(IronClawServicesError::internal_from)?;
            Ok(Resolution::Blocked(Blocked::Auth(
                GateWaypoint::new(GateRef::for_auth_gate(gate.gate_id.as_str()))
                    .with_resume(resume),
            )))
        }
        RuntimeCapabilityOutcome::ResourceBlocked(_gate) => {
            Ok(Resolution::Blocked(Blocked::Resource(GateWaypoint::new(
                GateRef::from_uuid(invocation_id.as_uuid()),
            ))))
        }
        RuntimeCapabilityOutcome::SpawnedProcess(process) => {
            Ok(Resolution::Suspended(Suspension::Process(
                ProcessWaypoint::new(ProcessRef::from_uuid(process.process_id.as_uuid())),
            )))
        }
        RuntimeCapabilityOutcome::Failed(failure)
            if matches!(
                failure.kind,
                RuntimeFailureKind::Authorization | RuntimeFailureKind::PolicyDenied
            ) =>
        {
            let reason = match failure.kind {
                RuntimeFailureKind::Authorization => DenyReason::MissingGrant,
                RuntimeFailureKind::PolicyDenied => DenyReason::PolicyDenied,
                _ => DenyReason::InternalInvariantViolation,
            };
            Ok(Resolution::Denied(
                Denial::new(DenyRef::from_uuid(invocation_id.as_uuid()))
                    .with_reason_kind(reason)
                    .with_summary(runtime_failure_summary(&failure)),
            ))
        }
        RuntimeCapabilityOutcome::Failed(failure) => Ok(recoverable_failure(
            invocation_id,
            FailureKind::from_tag(failure.kind.as_str()),
            runtime_failure_summary(&failure),
        )),
        RuntimeCapabilityOutcome::Unknown(unknown) => Ok(recoverable_failure(
            invocation_id,
            FailureKind::from_tag(&unknown.kind),
            unknown
                .message
                .and_then(|value| SafeSummary::new(value).ok())
                .unwrap_or_else(SafeSummary::placeholder),
        )),
    }
}

fn recoverable_failure(
    invocation_id: InvocationId,
    kind: FailureKind,
    summary: SafeSummary,
) -> Resolution {
    Resolution::Done(Outcome {
        refs: OutcomeRefs {
            result: ResultRef::from_uuid(invocation_id.as_uuid()),
            byte_len: 0,
            preview: None,
            preview_meta: ResultPreviewMeta::default(),
            origin: None,
            output_digest: None,
        },
        verdict: ToolVerdict::recoverable_failure(kind),
        summary,
        progress: ResultProgress::Unknown,
        terminate_hint: TerminateHint::Continue,
    })
}

fn runtime_failure_summary(
    failure: &ironclaw_host_runtime::RuntimeCapabilityFailure,
) -> SafeSummary {
    failure
        .safe_summary()
        .and_then(|summary| SafeSummary::new(summary).ok())
        .unwrap_or_else(SafeSummary::placeholder)
}

fn fixed_summary(summary: &'static str) -> SafeSummary {
    SafeSummary::new(summary).unwrap_or_else(|_| SafeSummary::placeholder())
}

fn ensure_matching_capability(
    requested: &CapabilityId,
    outcome: &RuntimeCapabilityOutcome,
) -> Result<(), IronClawServicesError> {
    let actual = match outcome {
        RuntimeCapabilityOutcome::Completed(completed) => &completed.capability_id,
        RuntimeCapabilityOutcome::ApprovalRequired(gate) => &gate.capability_id,
        RuntimeCapabilityOutcome::AuthRequired(gate) => &gate.capability_id,
        RuntimeCapabilityOutcome::ResourceBlocked(gate) => &gate.capability_id,
        RuntimeCapabilityOutcome::SpawnedProcess(process) => &process.capability_id,
        RuntimeCapabilityOutcome::Failed(failure) => &failure.capability_id,
        RuntimeCapabilityOutcome::Unknown(unknown) => &unknown.capability_id,
    };
    if actual != requested {
        return Err(IronClawServicesError::internal_from(
            "host runtime returned an outcome for a different capability",
        ));
    }
    Ok(())
}

impl ProductResultFilesystem {
    async fn persist(
        &self,
        scope: &ResourceScope,
        result_ref: ResultRef,
        body: Vec<u8>,
    ) -> Result<(), IronClawServicesError> {
        match self {
            Self::Composite(filesystem) => {
                persist_product_result(filesystem, scope, result_ref, body).await
            }
            Self::LibSql(filesystem) => {
                persist_product_result(filesystem, scope, result_ref, body).await
            }
            Self::Postgres(filesystem) => {
                persist_product_result(filesystem, scope, result_ref, body).await
            }
        }
    }
}

async fn persist_product_result<F>(
    filesystem: &ScopedFilesystem<F>,
    scope: &ResourceScope,
    result_ref: ResultRef,
    body: Vec<u8>,
) -> Result<(), IronClawServicesError>
where
    F: RootFilesystem + ?Sized,
{
    let path = ScopedPath::new(format!("{PRODUCT_RESULT_ROOT}/{result_ref}.json"))
        .map_err(IronClawServicesError::internal_from)?;
    let write_body = body.clone();
    cas_update(
        filesystem,
        scope,
        &path,
        |stored| Ok::<_, String>(stored.to_vec()),
        |stored| {
            Ok::<_, String>(Entry::bytes(stored.clone()).with_content_type(ContentType::json()))
        },
        move |existing| {
            let write_body = write_body.clone();
            async move {
                match existing {
                    None => Ok(CasApply::new(write_body, ())),
                    Some(existing) if existing == write_body => Ok(CasApply::no_op(existing, ())),
                    Some(_) => Err(
                        "product result replay produced different bytes for one activity"
                            .to_string(),
                    ),
                }
            }
        },
    )
    .await
    .map_err(IronClawServicesError::internal_from)
}

fn product_runtime_unavailable() -> IronClawServicesError {
    IronClawServicesError {
        code: IronClawServicesErrorCode::Unavailable,
        kind: IronClawServicesErrorKind::ServiceUnavailable,
        status_code: 503,
        retryable: false,
        field: None,
        validation_code: None,
    }
}

#[cfg(test)]
mod tests {
    use ironclaw_host_api::{
        EffectKind, NetworkScheme, NetworkTargetPattern, PermissionMode,
        RuntimeCredentialRequirement, RuntimeCredentialRequirementSource, RuntimeCredentialTarget,
        RuntimeKind, SecretHandle, TrustClass,
    };

    use super::*;

    #[test]
    fn product_gesture_grant_keeps_no_egress_policy_unconstrained() {
        let descriptor = descriptor_with_network(Vec::new(), Vec::new());

        let grant = product_gesture_grant(
            &descriptor,
            &ExtensionId::new(PRODUCT_INGRESS_EXTENSION_ID).unwrap(),
            MountView::default(),
        );

        assert_eq!(grant.constraints.network, NetworkPolicy::default());
    }

    #[test]
    fn product_gesture_grant_uses_dev_wildcard_for_networked_gesture_without_targets() {
        let mut descriptor = descriptor_with_network(Vec::new(), Vec::new());
        descriptor.effects.push(EffectKind::Network);

        let grant = product_gesture_grant(
            &descriptor,
            &ExtensionId::new(PRODUCT_INGRESS_EXTENSION_ID).unwrap(),
            MountView::default(),
        );

        assert_eq!(
            grant.constraints.network,
            crate::builtin_capability_policy::dev_wildcard_network_policy()
        );
    }

    #[test]
    fn product_gesture_grant_constrains_manifest_declared_egress() {
        let target = NetworkTargetPattern {
            scheme: Some(NetworkScheme::Https),
            host_pattern: "api.example.com".to_string(),
            port: None,
        };
        let descriptor = descriptor_with_network(vec![target.clone()], Vec::new());

        let grant = product_gesture_grant(
            &descriptor,
            &ExtensionId::new(PRODUCT_INGRESS_EXTENSION_ID).unwrap(),
            MountView::default(),
        );

        assert_eq!(grant.constraints.network.allowed_targets, vec![target]);
        assert!(grant.constraints.network.deny_private_ip_ranges);
        assert_eq!(grant.constraints.network.max_egress_bytes, None);
    }

    #[test]
    fn product_gesture_grant_folds_credential_audience_into_egress_policy() {
        let target = NetworkTargetPattern {
            scheme: Some(NetworkScheme::Https),
            host_pattern: "oauth.example.com".to_string(),
            port: None,
        };
        let credential = RuntimeCredentialRequirement {
            handle: SecretHandle::new("oauth_token").unwrap(),
            source: RuntimeCredentialRequirementSource::SecretHandle,
            provider_scopes: Vec::new(),
            audience: target.clone(),
            target: RuntimeCredentialTarget::Header {
                name: "authorization".to_string(),
                prefix: Some("Bearer ".to_string()),
            },
            required: true,
        };
        let descriptor = descriptor_with_network(Vec::new(), vec![credential]);

        let grant = product_gesture_grant(
            &descriptor,
            &ExtensionId::new(PRODUCT_INGRESS_EXTENSION_ID).unwrap(),
            MountView::default(),
        );

        assert_eq!(grant.constraints.network.allowed_targets, vec![target]);
        assert!(grant.constraints.network.deny_private_ip_ranges);
        assert_eq!(
            grant.constraints.secrets,
            vec![SecretHandle::new("oauth_token").unwrap()]
        );
    }

    fn descriptor_with_network(
        network_targets: Vec<NetworkTargetPattern>,
        runtime_credentials: Vec<RuntimeCredentialRequirement>,
    ) -> CapabilityDescriptor {
        CapabilityDescriptor {
            id: CapabilityId::new("builtin.product-gesture-test").unwrap(),
            provider: ExtensionId::new("builtin").unwrap(),
            runtime: RuntimeKind::FirstParty,
            trust_ceiling: TrustClass::UserTrusted,
            description: "product gesture test".to_string(),
            parameters_schema: serde_json::json!({}),
            effects: vec![EffectKind::DispatchCapability],
            default_permission: PermissionMode::Allow,
            runtime_credentials,
            network_targets,
            resource_profile: None,
            origin_gate_matrix: None,
        }
    }
}
