//! Generic product command adapter into the canonical host-runtime pipeline.

use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_assistant::{
    EXTENSION_ACTIVATE_CAPABILITY_ID, EXTENSION_INSTALL_CAPABILITY_ID,
    EXTENSION_REMOVE_CAPABILITY_ID, ProductCapabilityInvoker,
    SKILL_AUTO_ACTIVATE_SET_CAPABILITY_ID, SKILL_INSTALL_CAPABILITY_ID, SKILL_REMOVE_CAPABILITY_ID,
    SKILL_UPDATE_CAPABILITY_ID,
};
use ironclaw_extension_registry::ExtensionRegistry;
use ironclaw_filesystem::{
    CasApply, CompositeRootFilesystem, ContentType, Entry, FilesystemError, RootFilesystem,
    ScopedFilesystem, cas_update,
};
use ironclaw_host_api::{
    action::NetworkPolicy,
    artifact::{ArtifactNamespaceId, ArtifactRef},
    capability::{
        CapabilityDescriptor, CapabilityGrant, CapabilitySet, EffectKind, GrantConstraints,
    },
    decision::DenyReason,
    ids::{
        ActivityId, CapabilityGrantId, CapabilityId, CorrelationId, DenyRef, ExtensionId, GateRef,
        InvocationId, ProcessRef, ProductKind, ResultRef, RunId,
    },
    invocation::InvocationOrigin,
    mount::MountView,
    path::ScopedPath,
    resolution::{
        Blocked, Denial, GateWaypoint, Outcome, OutcomeRefs, ProcessWaypoint, Resolution,
        ResultPreviewMeta, Suspension, ToolVerdict,
    },
    resource::{ResourceEstimate, ResourceScope},
    result_meta::{
        FailureKind, ModelDiagnostic, ModelFailureDiagnostic, OutputDigest, ResultProgress,
        ResumeToken, TerminateHint,
    },
    runtime::{RuntimeKind, TrustClass},
    safe_summary::SafeSummary,
    scope::{ExecutionContext, Principal},
};
use ironclaw_host_runtime::{HostRuntime, RuntimeCapabilityOutcome};
use ironclaw_product_contracts::surface::{ProductSurfaceCaller, ProductSurfaceError};

use crate::RebornRuntime;
use ironclaw_skills::ScopedSkillManagementMountResolver;
use serde::{Deserialize, Serialize};

const PRODUCT_RESULT_MAX_BYTES: usize = 4 * 1024 * 1024;
const PRODUCT_RESULT_METADATA_MAX_BYTES: usize = 4 * 1024;
const PRODUCT_RESULT_ROOT: &str = "/product-results";
const PRODUCT_INGRESS_EXTENSION_ID: &str = "ironclaw_webui";

#[derive(Clone)]
pub(crate) struct RuntimeProductCapabilityInvoker {
    host_runtime: Arc<dyn HostRuntime>,
    registry: Arc<ExtensionRegistry>,
    results: ProductResultFilesystem,
    // The scope→mount-view resolver the runtime's skill-management port was
    // composed with. Reused here (rather than re-deriving a standalone vs
    // production branch) so product-surface skill gestures resolve exactly the
    // mounts the agent loop's skill tools do; the unified runtime graph exposes
    // a single composite filesystem, so which resolver is live is the only
    // deployment-shape distinction the invoker still needs.
    skill_mount_resolver: Arc<ScopedSkillManagementMountResolver>,
    system_extensions_lifecycle_mounts: MountView,
}

#[derive(serde::Deserialize, serde::Serialize)]
struct ProductResultMetadata {
    summary: String,
}

#[derive(Clone)]
pub(crate) enum ProductResultFilesystem {
    Composite(Arc<ScopedFilesystem<CompositeRootFilesystem>>),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct ProductResultArtifactMetadata {
    artifact_ref: ArtifactRef,
    byte_len: u64,
    output_digest: Option<OutputDigest>,
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
        let requested_capability = capability.clone();
        let outcome = host_runtime
            .invoke_capability((context, capability, ResourceEstimate::default(), input))
            .await
            .map_err(ProductSurfaceError::internal_from)?;
        ensure_matching_capability(&requested_capability, &outcome)?;
        product_resolution(results, &scope, invocation_id, outcome).await
    }

    async fn complete_product_result(
        &self,
        caller: ProductSurfaceCaller,
        output: serde_json::Value,
        activity_id: ActivityId,
        summary: &'static str,
    ) -> Result<Resolution, ProductSurfaceError> {
        let invocation_id = InvocationId::from_uuid(activity_id.as_uuid());
        let scope = product_resource_scope(&caller, invocation_id);
        if let Some(replayed) = self.results.replay(&scope, invocation_id).await? {
            return Ok(replayed);
        }
        persist_product_output(&self.results, &scope, invocation_id, output, summary, None).await
    }
}

fn product_execution_context(
    caller: &ProductSurfaceCaller,
    activity_id: ActivityId,
    descriptor: Option<&CapabilityDescriptor>,
    skill_mount_resolver: &ScopedSkillManagementMountResolver,
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
        artifact_namespace: Some(ArtifactNamespaceId::from_root_run(RunId::from_uuid(
            activity_id.as_uuid(),
        ))),
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
    skill_mount_resolver: &ScopedSkillManagementMountResolver,
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
    matches!(
        capability.as_str(),
        EXTENSION_INSTALL_CAPABILITY_ID
            | EXTENSION_ACTIVATE_CAPABILITY_ID
            | EXTENSION_REMOVE_CAPABILITY_ID
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
            let artifact = completed_artifact_metadata(&completed);
            persist_product_output(
                results,
                scope,
                invocation_id,
                completed.output,
                "capability completed",
                artifact,
            )
            .await
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
                FailureKind::Authorization | FailureKind::PolicyDenied
            ) =>
        {
            let reason = match failure.kind {
                FailureKind::Authorization => DenyReason::MissingGrant,
                FailureKind::PolicyDenied => DenyReason::PolicyDenied,
                _ => DenyReason::InternalInvariantViolation,
            };
            Ok(Resolution::Denied(
                Denial::new(DenyRef::from_uuid(invocation_id.as_uuid()))
                    .with_reason_kind(reason)
                    .with_summary(runtime_failure_summary(&failure)),
            ))
        }
        RuntimeCapabilityOutcome::Failed(failure) => {
            let summary = runtime_failure_summary(&failure);
            let diagnostic = model_diagnostic(
                failure
                    .model_visible_cause()
                    .unwrap_or_else(|| summary.as_str()),
            );
            Ok(recoverable_failure(
                invocation_id,
                FailureKind::from_tag(failure.kind.as_str()),
                summary,
                diagnostic,
            ))
        }
    }
}

async fn persist_product_output(
    results: &ProductResultFilesystem,
    scope: &ResourceScope,
    invocation_id: InvocationId,
    output: serde_json::Value,
    summary: &'static str,
    artifact: Option<ProductResultArtifactMetadata>,
) -> Result<Resolution, ProductSurfaceError> {
    let body = serde_json::to_vec(&output).map_err(ProductSurfaceError::internal_from)?;
    if body.len() > PRODUCT_RESULT_MAX_BYTES {
        return Err(ProductSurfaceError::internal_from(
            "product capability result exceeded the durable output bound",
        ));
    }
    let result_ref = ResultRef::from_uuid(invocation_id.as_uuid());
    results
        .persist(scope, result_ref, body.clone(), summary, artifact.clone())
        .await?;
    Ok(Resolution::Done(Outcome {
        refs: product_outcome_refs(result_ref, body.len(), artifact),
        verdict: ToolVerdict::Success,
        summary: fixed_summary(summary),
        progress: ResultProgress::MadeProgress,
        terminate_hint: TerminateHint::Continue,
    }))
}

fn completed_artifact_metadata(
    completed: &ironclaw_host_runtime::RuntimeCapabilityCompleted,
) -> Option<ProductResultArtifactMetadata> {
    completed
        .completed_artifact
        .as_ref()
        .map(|artifact| ProductResultArtifactMetadata {
            artifact_ref: artifact.artifact_ref,
            byte_len: artifact.byte_len,
            output_digest: completed.canonical_output_digest,
        })
}

/// The one place artifact metadata becomes outcome refs. A spilled result's
/// durable body is only a bounded preview, so the sidecar owns the real byte
/// length and canonical digest; a fresh completion and its later replay must
/// not disagree about them.
fn product_outcome_refs(
    result_ref: ResultRef,
    preview_bytes: usize,
    artifact: Option<ProductResultArtifactMetadata>,
) -> OutcomeRefs {
    let (byte_len, artifact_ref, output_digest) =
        artifact.map_or((preview_bytes as u64, None, None), |metadata| {
            (
                metadata.byte_len,
                Some(metadata.artifact_ref),
                metadata.output_digest,
            )
        });
    OutcomeRefs {
        result: result_ref,
        byte_len,
        preview: None,
        preview_meta: ResultPreviewMeta {
            artifact_ref,
            ..ResultPreviewMeta::default()
        },
        origin: None,
        output_digest,
    }
}

fn recoverable_failure(
    invocation_id: InvocationId,
    kind: FailureKind,
    summary: SafeSummary,
    diagnostic: ModelFailureDiagnostic,
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
        verdict: ToolVerdict::recoverable_failure_with_diagnostic(kind, diagnostic),
        summary,
        progress: ResultProgress::Unknown,
        terminate_hint: TerminateHint::Continue,
    })
}

fn model_diagnostic(cause: &str) -> ModelFailureDiagnostic {
    let scrubbed = ironclaw_loop_host::scrub_model_visible_detail(cause);
    let text = ModelDiagnostic::truncating(scrubbed).unwrap_or_else(|error| {
        // silent-ok: the model-visible diagnostic boundary fails closed to a
        // fixed sentence rather than failing the turn. Reaching this arm means
        // the scrub upstream did not fully sanitize, which an operator wants
        // to see; `debug!` avoids corrupting the REPL/TUI.
        tracing::debug!(
            %error,
            "model-visible diagnostic rejected after scrubbing; substituting the fixed fallback"
        );
        ModelDiagnostic::unavailable()
    });
    ModelFailureDiagnostic::Diagnostic { text }
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
        summary: &'static str,
        artifact: Option<ProductResultArtifactMetadata>,
    ) -> Result<(), ProductSurfaceError> {
        match self {
            Self::Composite(filesystem) => {
                persist_product_result_with_metadata(
                    filesystem, scope, result_ref, body, summary, artifact,
                )
                .await
            }
        }
    }
}

async fn persist_product_result<F>(
    filesystem: &ScopedFilesystem<F>,
    scope: &ResourceScope,
    result_ref: ResultRef,
    body: Vec<u8>,
    summary: &'static str,
) -> Result<(), ProductSurfaceError>
where
    F: RootFilesystem + ?Sized,
{
    persist_product_result_metadata(filesystem, scope, result_ref, summary).await?;
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

async fn persist_product_result_with_metadata<F>(
    filesystem: &ScopedFilesystem<F>,
    scope: &ResourceScope,
    result_ref: ResultRef,
    body: Vec<u8>,
    summary: &'static str,
    artifact: Option<ProductResultArtifactMetadata>,
) -> Result<(), ProductSurfaceError>
where
    F: RootFilesystem + ?Sized,
{
    persist_product_result(filesystem, scope, result_ref, body, summary).await?;
    let Some(artifact) = artifact else {
        return Ok(());
    };
    let path = ScopedPath::new(format!("{PRODUCT_RESULT_ROOT}/{result_ref}.artifact.json"))
        .map_err(ProductSurfaceError::internal_from)?;
    let encoded = serde_json::to_vec(&artifact).map_err(ProductSurfaceError::internal_from)?;
    persist_product_result_entry(filesystem, scope, path, encoded).await
}

async fn persist_product_result_metadata<F>(
    filesystem: &ScopedFilesystem<F>,
    scope: &ResourceScope,
    result_ref: ResultRef,
    summary: &'static str,
) -> Result<(), ProductSurfaceError>
where
    F: RootFilesystem + ?Sized,
{
    let path = ScopedPath::new(format!("{PRODUCT_RESULT_ROOT}/{result_ref}.meta.json"))
        .map_err(ProductSurfaceError::internal_from)?;
    let metadata = serde_json::to_vec(&ProductResultMetadata {
        summary: summary.to_string(),
    })
    .map_err(ProductSurfaceError::internal_from)?;
    persist_product_result_entry(filesystem, scope, path, metadata).await
}

async fn persist_product_result_entry<F>(
    filesystem: &ScopedFilesystem<F>,
    scope: &ResourceScope,
    path: ScopedPath,
    body: Vec<u8>,
) -> Result<(), ProductSurfaceError>
where
    F: RootFilesystem + ?Sized,
{
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
    let artifact_path =
        ScopedPath::new(format!("{PRODUCT_RESULT_ROOT}/{result_ref}.artifact.json"))
            .map_err(ProductSurfaceError::internal_from)?;
    let artifact = match filesystem
        .read_bytes_bounded(scope, &artifact_path, PRODUCT_RESULT_METADATA_MAX_BYTES)
        .await
    {
        Ok(Some(encoded)) => {
            let metadata = serde_json::from_slice::<ProductResultArtifactMetadata>(&encoded)
                .map_err(ProductSurfaceError::internal_from)?;
            if metadata.byte_len < body.len() as u64 {
                return Err(ProductSurfaceError::internal_from(
                    "product artifact metadata is shorter than its bounded preview",
                ));
            }
            Some(metadata)
        }
        Ok(None) | Err(FilesystemError::NotFound { .. }) => None,
        Err(error) => return Err(ProductSurfaceError::internal_from(error)),
    };
    let metadata_path = ScopedPath::new(format!("{PRODUCT_RESULT_ROOT}/{result_ref}.meta.json"))
        .map_err(ProductSurfaceError::internal_from)?;
    let summary = match filesystem
        .read_bytes_bounded(scope, &metadata_path, PRODUCT_RESULT_METADATA_MAX_BYTES)
        .await
    {
        Ok(Some(metadata)) => {
            serde_json::from_slice::<ProductResultMetadata>(&metadata)
                .map_err(ProductSurfaceError::internal_from)?
                .summary
        }
        Ok(None) | Err(FilesystemError::NotFound { .. }) => "capability completed".to_string(),
        Err(error) => return Err(ProductSurfaceError::internal_from(error)),
    };
    Ok(Some(Resolution::Done(Outcome {
        refs: product_outcome_refs(result_ref, body.len(), artifact),
        verdict: ToolVerdict::Success,
        summary: SafeSummary::new(summary).unwrap_or_else(|_| SafeSummary::placeholder()),
        progress: ResultProgress::MadeProgress,
        terminate_hint: TerminateHint::Continue,
    })))
}

#[cfg(test)]
#[path = "product_capability_tests.rs"]
mod tests;
