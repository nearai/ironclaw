//! Generic product command adapter into the canonical host-runtime pipeline.

use std::{collections::HashMap, sync::Arc};

use async_trait::async_trait;
use ironclaw_extensions::ExtensionRegistry;
use ironclaw_filesystem::{
    CasApply, CompositeRootFilesystem, ContentType, Entry, FilesystemError, RootFilesystem,
    ScopedFilesystem, cas_update,
};
use ironclaw_host_api::{
    ActivityId, Blocked, CapabilityDescriptor, CapabilityGrant, CapabilityGrantId, CapabilityId,
    CapabilitySet, CorrelationId, Denial, DenyReason, DenyRef, EffectKind, ExecutionContext,
    ExtensionId, FailureKind, GateRef, GateWaypoint, GrantConstraints, InvocationId,
    InvocationOrigin, MountView, NetworkPolicy, Outcome, OutcomeRefs, Principal, ProcessRef,
    ProcessWaypoint, ProductKind, ProductSurfaceCaller, ProductSurfaceError, Resolution,
    ResourceEstimate, ResourceScope, ResultPreviewMeta, ResultProgress, ResultRef, ResumeToken,
    RuntimeKind, SafeSummary, ScopedPath, Suspension, TerminateHint, ToolVerdict, TrustClass,
};
use ironclaw_host_runtime::{HostRuntime, RuntimeCapabilityOutcome, RuntimeFailureKind};
use ironclaw_product::{
    EXTENSION_INSTALL_CAPABILITY_ID, EXTENSION_REMOVE_CAPABILITY_ID, ProductCapabilityInvoker,
    SKILL_AUTO_ACTIVATE_SET_CAPABILITY_ID, SKILL_INSTALL_CAPABILITY_ID, SKILL_REMOVE_CAPABILITY_ID,
    SKILL_UPDATE_CAPABILITY_ID,
};

use crate::RebornRuntime;
use crate::extension_host::lifecycle::SkillManagementMountResolver;
use tokio::sync::Mutex as AsyncMutex;

const PRODUCT_RESULT_MAX_BYTES: usize = 4 * 1024 * 1024;
const PRODUCT_RESULT_ROOT: &str = "/product-results";
const PRODUCT_INGRESS_EXTENSION_ID: &str = "ironclaw_webui";

#[derive(Clone)]
pub(crate) struct RuntimeProductCapabilityInvoker {
    host_runtime: Arc<dyn HostRuntime>,
    registry: Arc<ExtensionRegistry>,
    results: ProductResultFilesystem,
    // The scope→mount-view resolver the runtime's skill-management port was
    // composed with. Reused here (rather than re-deriving a local-dev vs
    // production branch) so product-surface skill gestures resolve exactly the
    // mounts the agent loop's skill tools do; the unified runtime graph exposes
    // a single composite filesystem, so which resolver is live is the only
    // deployment-shape distinction the invoker still needs.
    skill_mount_resolver: Arc<SkillManagementMountResolver>,
    system_extensions_lifecycle_mounts: MountView,
    activity_locks: Arc<AsyncMutex<HashMap<ActivityId, Arc<AsyncMutex<()>>>>>,
}

#[derive(Clone)]
pub(crate) enum ProductResultFilesystem {
    Composite(Arc<ScopedFilesystem<CompositeRootFilesystem>>),
}

impl RuntimeProductCapabilityInvoker {
    pub(crate) fn from_runtime(runtime: &RebornRuntime) -> Self {
        Self {
            host_runtime: Arc::clone(&runtime.host_runtime),
            registry: Arc::clone(&runtime.extension_registry),
            results: ProductResultFilesystem::Composite(crate::wrap_scoped(Arc::clone(
                &runtime.extension_filesystem,
            ))),
            skill_mount_resolver: runtime.skill_management.mount_resolver(),
            system_extensions_lifecycle_mounts: runtime.system_extensions_lifecycle_mounts.clone(),
            activity_locks: Arc::new(AsyncMutex::new(HashMap::new())),
        }
    }

    async fn lock_for_activity(&self, activity_id: ActivityId) -> Arc<AsyncMutex<()>> {
        let mut locks = self.activity_locks.lock().await;
        Arc::clone(
            locks
                .entry(activity_id)
                .or_insert_with(|| Arc::new(AsyncMutex::new(()))),
        )
    }

    async fn release_activity_lock(&self, activity_id: ActivityId, lock: &Arc<AsyncMutex<()>>) {
        let mut locks = self.activity_locks.lock().await;
        if locks
            .get(&activity_id)
            .is_some_and(|current| Arc::ptr_eq(current, lock))
            && Arc::strong_count(lock) <= 2
        {
            locks.remove(&activity_id);
        }
    }
}

#[async_trait]
impl ProductCapabilityInvoker for RuntimeProductCapabilityInvoker {
    async fn invoke(
        &self,
        caller: ProductSurfaceCaller,
        capability: CapabilityId,
        input: serde_json::Value,
        activity_id: ActivityId,
    ) -> Result<Resolution, ProductSurfaceError> {
        let Self {
            host_runtime,
            registry,
            results,
            skill_mount_resolver,
            system_extensions_lifecycle_mounts,
            activity_locks: _,
        } = self;
        // The origin-to-gate matrix is still provisional in today's kernel.
        // Encode the direct user gesture as one exact, host-issued grant. The
        // runtime independently re-resolves the descriptor and authorizes it,
        // so a concurrent stronger replacement no longer fits this attenuated
        // grant and fails closed.
        let descriptor = registry.get_capability(&capability);
        let context = product_execution_context(
            &caller,
            activity_id,
            descriptor,
            &**skill_mount_resolver,
            system_extensions_lifecycle_mounts,
        )?;
        let scope = context.resource_scope.clone();
        let invocation_id = context.invocation_id;
        if let Some(replayed) = results.replay(&scope, invocation_id).await? {
            return Ok(replayed);
        }
        let activity_lock = self.lock_for_activity(activity_id).await;
        let _activity_guard = activity_lock.lock().await;
        if let Some(replayed) = results.replay(&scope, invocation_id).await? {
            drop(_activity_guard);
            self.release_activity_lock(activity_id, &activity_lock)
                .await;
            return Ok(replayed);
        }
        let requested_capability = capability.clone();
        let result = async {
            let outcome = host_runtime
                .invoke_capability((context, capability, ResourceEstimate::default(), input))
                .await
                .map_err(ProductSurfaceError::internal_from)?;
            ensure_matching_capability(&requested_capability, &outcome)?;
            product_resolution(results, &scope, invocation_id, outcome).await
        }
        .await;
        drop(_activity_guard);
        self.release_activity_lock(activity_id, &activity_lock)
            .await;
        result
    }
}

fn product_execution_context(
    caller: &ProductSurfaceCaller,
    activity_id: ActivityId,
    descriptor: Option<&CapabilityDescriptor>,
    skill_mount_resolver: &SkillManagementMountResolver,
    system_extensions_lifecycle_mounts: &MountView,
) -> Result<ExecutionContext, ProductSurfaceError> {
    let invocation_id = InvocationId::from_uuid(activity_id.as_uuid());
    let scope = product_resource_scope(caller, invocation_id);
    let extension_id = ExtensionId::new(PRODUCT_INGRESS_EXTENSION_ID)
        .map_err(ProductSurfaceError::internal_from)?;
    let invocation_mounts = product_invocation_mounts(
        &scope,
        descriptor,
        skill_mount_resolver,
        system_extensions_lifecycle_mounts,
    )?;
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
            ProductKind::new("webui").map_err(ProductSurfaceError::internal_from)?,
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
        .map_err(ProductSurfaceError::internal_from)?;
    Ok(context)
}

fn product_resource_scope(
    caller: &ProductSurfaceCaller,
    invocation_id: InvocationId,
) -> ResourceScope {
    ResourceScope {
        tenant_id: caller.tenant_id.clone(),
        user_id: caller.user_id.clone(),
        agent_id: caller.agent_id.clone(),
        project_id: caller.project_id.clone(),
        mission_id: None,
        thread_id: None,
        invocation_id,
    }
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
    skill_mount_resolver: &SkillManagementMountResolver,
    system_extensions_lifecycle_mounts: &MountView,
) -> Result<MountView, ProductSurfaceError> {
    let Some(descriptor) = descriptor else {
        return Ok(MountView::default());
    };
    if is_extension_lifecycle_capability(&descriptor.id) {
        return Ok(system_extensions_lifecycle_mounts.clone());
    }
    if !is_skill_management_capability(&descriptor.id) {
        return Ok(MountView::default());
    }
    skill_mount_resolver(scope).map_err(ProductSurfaceError::internal_from)
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

fn is_extension_lifecycle_capability(capability: &CapabilityId) -> bool {
    // #6520 removed the separate activate capability; install drives readiness.
    matches!(
        capability.as_str(),
        EXTENSION_INSTALL_CAPABILITY_ID | EXTENSION_REMOVE_CAPABILITY_ID
    )
}

async fn product_resolution(
    results: &ProductResultFilesystem,
    scope: &ResourceScope,
    invocation_id: InvocationId,
    outcome: RuntimeCapabilityOutcome,
) -> Result<Resolution, ProductSurfaceError> {
    match outcome {
        RuntimeCapabilityOutcome::Completed(completed) => {
            let body = serde_json::to_vec(&completed.output)
                .map_err(ProductSurfaceError::internal_from)?;
            if body.len() > PRODUCT_RESULT_MAX_BYTES {
                return Err(ProductSurfaceError::internal_from(
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
                .map_err(ProductSurfaceError::internal_from)?;
            Ok(Resolution::Blocked(Blocked::Approval(
                GateWaypoint::new(GateRef::for_approval_request(gate.approval_request_id))
                    .with_resume(resume),
            )))
        }
        RuntimeCapabilityOutcome::AuthRequired(gate) => {
            let resume = ResumeToken::new(invocation_id.to_string())
                .map_err(ProductSurfaceError::internal_from)?;
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
) -> Result<(), ProductSurfaceError> {
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
        return Err(ProductSurfaceError::internal_from(
            "host runtime returned an outcome for a different capability",
        ));
    }
    Ok(())
}

impl ProductResultFilesystem {
    async fn replay(
        &self,
        scope: &ResourceScope,
        invocation_id: InvocationId,
    ) -> Result<Option<Resolution>, ProductSurfaceError> {
        match self {
            Self::Composite(filesystem) => {
                replay_product_result(filesystem, scope, invocation_id).await
            }
        }
    }

    async fn persist(
        &self,
        scope: &ResourceScope,
        result_ref: ResultRef,
        body: Vec<u8>,
    ) -> Result<(), ProductSurfaceError> {
        match self {
            Self::Composite(filesystem) => {
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
) -> Result<(), ProductSurfaceError>
where
    F: RootFilesystem + ?Sized,
{
    let path = ScopedPath::new(format!("{PRODUCT_RESULT_ROOT}/{result_ref}.json"))
        .map_err(ProductSurfaceError::internal_from)?;
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
    .map_err(ProductSurfaceError::internal_from)
}

async fn replay_product_result<F>(
    filesystem: &ScopedFilesystem<F>,
    scope: &ResourceScope,
    invocation_id: InvocationId,
) -> Result<Option<Resolution>, ProductSurfaceError>
where
    F: RootFilesystem + ?Sized,
{
    let result_ref = ResultRef::from_uuid(invocation_id.as_uuid());
    let path = ScopedPath::new(format!("{PRODUCT_RESULT_ROOT}/{result_ref}.json"))
        .map_err(ProductSurfaceError::internal_from)?;
    let body = match filesystem
        .read_bytes_bounded(scope, &path, PRODUCT_RESULT_MAX_BYTES)
        .await
    {
        Ok(Some(body)) => body,
        Ok(None) | Err(FilesystemError::NotFound { .. }) => return Ok(None),
        Err(error) => return Err(ProductSurfaceError::internal_from(error)),
    };
    Ok(Some(Resolution::Done(Outcome {
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
    })))
}

#[cfg(test)]
mod tests {
    use ironclaw_filesystem::InMemoryBackend;
    use ironclaw_host_api::{
        EffectKind, MountAlias, MountGrant, MountPermissions, NetworkScheme, NetworkTargetPattern,
        PermissionMode, RuntimeCredentialRequirement, RuntimeCredentialRequirementSource,
        RuntimeCredentialTarget, RuntimeKind, SecretHandle, TrustClass, VirtualPath,
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

    #[test]
    fn product_invocation_mounts_grants_extension_lifecycle_mounts() {
        let skill_mount_resolver = |_scope: &ResourceScope| Ok(MountView::default());
        for capability in [
            EXTENSION_INSTALL_CAPABILITY_ID,
            EXTENSION_REMOVE_CAPABILITY_ID,
        ] {
            let descriptor = descriptor_with_id(capability);
            let lifecycle_mounts =
                crate::local_dev_mounts::system_extensions_lifecycle_mount_view()
                    .expect("expected extension lifecycle mounts");
            let mounts = product_invocation_mounts(
                &resource_scope(),
                Some(&descriptor),
                &skill_mount_resolver,
                &lifecycle_mounts,
            )
            .expect("extension lifecycle product mounts");

            assert_eq!(mounts, lifecycle_mounts);

            let production_lifecycle_mounts =
                crate::factory::production_system_extensions_lifecycle_mount_view()
                    .expect("expected production extension lifecycle mounts");
            let production_mounts = product_invocation_mounts(
                &resource_scope(),
                Some(&descriptor),
                &skill_mount_resolver,
                &production_lifecycle_mounts,
            )
            .expect("production extension lifecycle product mounts");
            assert_eq!(production_mounts, production_lifecycle_mounts);
        }
    }

    #[test]
    fn product_invocation_mounts_keeps_skill_mounts_scoped() {
        let scope = resource_scope();
        let descriptor = descriptor_with_id(SKILL_REMOVE_CAPABILITY_ID);
        let skill_mount_resolver = |scope: &ResourceScope| {
            crate::local_dev_mounts::scoped_skill_management_mount_view(scope)
        };
        let lifecycle_mounts = MountView::default();
        let mounts = product_invocation_mounts(
            &scope,
            Some(&descriptor),
            &skill_mount_resolver,
            &lifecycle_mounts,
        )
        .expect("skill product mounts");

        assert_eq!(
            mounts,
            crate::local_dev_mounts::scoped_skill_management_mount_view(&scope)
                .expect("expected skill mounts")
        );
    }

    #[test]
    fn product_invocation_mounts_leaves_unclassified_capabilities_empty() {
        let descriptor = descriptor_with_id("builtin.product-gesture-test");
        let skill_mount_resolver = |_scope: &ResourceScope| Ok(MountView::default());
        let lifecycle_mounts = MountView::default();
        let mounts = product_invocation_mounts(
            &resource_scope(),
            Some(&descriptor),
            &skill_mount_resolver,
            &lifecycle_mounts,
        )
        .expect("product mounts");

        assert_eq!(mounts, MountView::default());
    }

    #[tokio::test]
    async fn product_result_replay_returns_persisted_resolution() {
        let filesystem = scoped_product_results_filesystem();
        let scope = resource_scope();
        let invocation_id = InvocationId::new();
        let result_ref = ResultRef::from_uuid(invocation_id.as_uuid());
        let body = br#"{"status":"installed"}"#.to_vec();

        persist_product_result(&filesystem, &scope, result_ref, body.clone())
            .await
            .expect("product result persists");
        let replayed = replay_product_result(&filesystem, &scope, invocation_id)
            .await
            .expect("product result replays")
            .expect("persisted result should replay");

        let Resolution::Done(outcome) = replayed else {
            panic!("persisted product result should replay as a completed outcome");
        };
        assert_eq!(outcome.refs.result, result_ref);
        assert_eq!(outcome.refs.byte_len, body.len() as u64);
        assert_eq!(outcome.verdict, ToolVerdict::Success);
    }

    fn descriptor_with_id(id: &str) -> CapabilityDescriptor {
        let mut descriptor = descriptor_with_network(Vec::new(), Vec::new());
        descriptor.id = CapabilityId::new(id).unwrap();
        descriptor
    }

    fn resource_scope() -> ResourceScope {
        ResourceScope {
            tenant_id: ironclaw_host_api::TenantId::new("tenant-test").unwrap(),
            user_id: ironclaw_host_api::UserId::new("user-test").unwrap(),
            agent_id: Some(ironclaw_host_api::AgentId::new("agent-test").unwrap()),
            project_id: Some(ironclaw_host_api::ProjectId::new("project-test").unwrap()),
            mission_id: None,
            thread_id: None,
            invocation_id: InvocationId::new(),
        }
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
            max_egress_bytes: None,
            resource_profile: None,
            origin_gate_matrix: None,
        }
    }

    fn scoped_product_results_filesystem() -> ScopedFilesystem<InMemoryBackend> {
        ScopedFilesystem::with_fixed_view(
            Arc::new(InMemoryBackend::new()),
            MountView::new(vec![MountGrant::new(
                MountAlias::new(PRODUCT_RESULT_ROOT).unwrap(),
                VirtualPath::new(PRODUCT_RESULT_ROOT).unwrap(),
                MountPermissions::read_write_list_delete(),
            )])
            .expect("product results mount view"),
        )
    }
}
