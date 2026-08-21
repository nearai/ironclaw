//! Capability-host adapters and assembly shared by every runtime profile.

use std::{
    collections::{BTreeMap, HashMap, HashSet, VecDeque},
    sync::{Arc, Mutex as StdMutex},
};

use chrono::Utc;
use ironclaw_assistant::OutboundPreferencesProductService;
use ironclaw_host_api::{
    artifact::{
        ARTIFACT_INLINE_PREVIEW_MAX_BYTES, AccountedArtifactPersister, ArtifactOwnerScope,
        ArtifactWriteMetadata,
    },
    capability::EffectKind,
    capability_surface::CapabilitySurfacePolicy,
    ids::{CapabilityId, ExtensionId, InvocationId, ResourceReservationId, UserId},
    model_result_preview::ModelResultPreview,
    mount::MountView,
    resolution::Resolution,
    resource::{
        ReservationStatus, ResourceEstimate, ResourceReceipt, ResourceScope, ResourceUsage,
    },
    result_meta::FailureKind,
    runtime::{RuntimeKind, TrustClass},
    scope::ExecutionContext,
};
use ironclaw_host_runtime::{
    HostRuntime, SurfaceKind, VisibleCapabilityRequest as HostVisibleCapabilityRequest,
};
#[cfg(test)]
use ironclaw_loop_host::HostManagedToolResultDiagnosticCapture;
use ironclaw_loop_host::{
    CapabilityResultUpdate, CapabilityResultWrite, CapabilityTrajectoryObserver,
    CapabilityWriteResult, DurablePersistence, HostManagedModelGateway,
    HostManagedPromptDiagnosticSink, HostManagedToolDiagnosticEmitter, LoopCapabilityInputResolver,
    LoopCapabilityPortFactory, LoopCapabilityResultWriter, ThreadScopeResolver,
    loop_driver_execution_extension_id,
};
use ironclaw_product_contracts::{
    inspector::TOOL_RESULT_DIAGNOSTIC_CAPTURE_MAX_BYTES, project_service::ProjectService,
};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use ironclaw_loop_contracts::{
    AgentLoopHostError, AgentLoopHostErrorKind, CapabilityFailureDetail, CapabilityInputRef,
    LoopCapabilityPort, LoopHostMilestoneSink, LoopRunContext,
    MODEL_VISIBLE_TOOL_OBSERVATION_SCHEMA_VERSION, ModelVisibleArtifact,
    ModelVisibleToolObservation, ObservationTrust, ProviderToolCall, ToolObservationDetail,
    ToolObservationStatus, resolution,
};
use ironclaw_resources::{ResourceError, ResourceGovernor};
use ironclaw_threads::{
    AppendCapabilityDisplayPreviewRequest, CapabilityDisplayPreviewEnvelope,
    CapabilityDisplayPreviewEnvelopeInput, CapabilityDisplayPreviewStatus, SessionThreadService,
    ThreadMessageId, ThreadScope, ToolResultSafeSummary, UpdateToolResultReferenceRequest,
};
use ironclaw_trust::{AuthorityCeiling, EffectiveTrustClass, TrustDecision, TrustProvenance};
use ironclaw_turns::{ExternalToolCatalog, LoopResultRef};

use crate::builtin_capability_policy::BuiltinCapabilityPolicy;
use crate::capability_authorization::{StoreApprovalSettingsProvider, effects_require_approval};
use crate::factory::RebornRuntimeStores;
use crate::runtime::ComposedSelectableSkillContextSource;
use crate::runtime_mounts::{WorkspaceMountPolicy, db_backed_skill_management_mount_view};
use ironclaw_approvals::ApprovalSettingsProvider;
use ironclaw_assistant::projection::{
    CapabilityDisplayPreviewResult, CapabilityDisplayPreviewStore,
};

mod notification_channels_set;
mod outbound_delivery;
mod refreshing_capability_port;
#[cfg(test)]
mod shell_tests;
#[cfg(test)]
mod workspace_scoping_tests;

#[cfg(any(test, feature = "test-support"))]
pub(crate) use ironclaw_assistant::PROJECT_CREATE_CAPABILITY_ID;
#[cfg(test)]
pub(crate) use ironclaw_assistant::{
    OUTBOUND_DELIVERY_TARGETS_LIST_CAPABILITY_ID, OUTBOUND_NOTIFICATION_CHANNELS_SET_CAPABILITY_ID,
};
use ironclaw_extension_host::capability_surface::{
    ExtensionCapabilitySurface, ExtensionCapabilitySurfaceSource,
};
#[cfg(any(test, feature = "test-support"))]
pub(crate) use ironclaw_loop_host::SKILL_ACTIVATE_CAPABILITY_ID;
use refreshing_capability_port::{
    RefreshingCapabilityPortConfig, create_refreshing_capability_port,
};

#[cfg(feature = "test-support")]
pub(super) use refreshing_capability_port::create_refreshing_capability_port_for_test;

fn diagnostic_failure(error_kind: FailureKind, safe_summary: String) -> Resolution {
    resolution::failed(
        error_kind,
        safe_summary.clone(),
        CapabilityFailureDetail::Diagnostic { text: safe_summary },
    )
}

pub(super) struct CapabilityPortWiring {
    pub(super) capability_factory: Arc<dyn LoopCapabilityPortFactory>,
    pub(super) capability_input_resolver: Arc<dyn LoopCapabilityInputResolver>,
    pub(super) capability_result_writer: Arc<dyn LoopCapabilityResultWriter>,
    pub(super) model_gateway: Arc<dyn HostManagedModelGateway>,
    pub(super) display_previews: Arc<CapabilityDisplayPreviewStore>,
}

// arch-exempt: too_many_args, a missing CapabilityWiringContext would only aggregate independently owned runtime services for this composition seam, plan #7193
#[allow(clippy::too_many_arguments)]
pub(super) fn capability_wiring(
    services: &RebornRuntimeStores,
    thread_service: Arc<dyn SessionThreadService>,
    fallback_user_id: UserId,
    policy: Arc<BuiltinCapabilityPolicy>,
    model_gateway: Arc<dyn HostManagedModelGateway>,
    milestone_sink: Arc<dyn LoopHostMilestoneSink>,
    skill_activation_source: Option<Arc<ComposedSelectableSkillContextSource>>,
    outbound_preferences_service: Option<Arc<dyn OutboundPreferencesProductService>>,
    trajectory_observer: Option<Arc<dyn crate::RebornTrajectoryObserver>>,
    tool_diagnostic_sink: Option<Arc<dyn HostManagedPromptDiagnosticSink>>,
    trigger_poller_enabled: bool,
) -> Result<CapabilityPortWiring, crate::runtime::RebornRuntimeError> {
    let runtime = services.host_runtime.clone();
    let workspace_mounts = services.workspace_mounts.clone();
    let memory_mounts = services.memory_mounts.clone();
    let system_extensions_lifecycle_mounts = services.system_extensions_lifecycle_mounts.clone();
    let approval_requests: Arc<dyn ironclaw_approvals::ApprovalRequestStorePort> =
        services.approval_requests.clone();
    let capability_leases: Arc<dyn ironclaw_authorization::CapabilityLeaseStorePort> =
        services.capability_leases.clone();
    let tool_permission_overrides: Arc<
        dyn ironclaw_approvals::CapabilityPermissionOverrideStorePort,
    > = services.tool_permission_overrides.clone();
    let auto_approve_settings: Arc<dyn ironclaw_approvals::AutoApproveSettingStorePort> =
        services.auto_approve_settings.clone();
    let approval_settings: Arc<dyn ApprovalSettingsProvider> =
        Arc::new(StoreApprovalSettingsProvider::new(
            tool_permission_overrides,
            auto_approve_settings,
            services.persistent_approval_policies.clone(),
        ));
    let outbound_preference_write_requires_approval = effects_require_approval(
        services.runtime_policy.as_ref(),
        policy.as_ref(),
        &[EffectKind::ExternalWrite],
    );
    let extension_surface_source =
        ExtensionCapabilitySurfaceSource::new(Some(services.extension_management.clone()));
    // First-class project creation reuses the same access-controlled
    // `ProjectService` service the WebUI v2 surface wires (composition owns the
    // service, never the raw repository), so an agent-created project is a real
    // entity that appears in the Projects list.
    let project_service: Arc<dyn ProjectService> = Arc::clone(&services.project_service);
    let display_previews = Arc::new(CapabilityDisplayPreviewStore::default());
    let capability_observer = trajectory_observer
        .clone()
        .map(crate::observability::trajectory_observer::as_capability_observer);
    let capability_io = Arc::new(
        StagedCapabilityIo::new_with_durable_previews(
            Arc::clone(&display_previews),
            Arc::clone(&thread_service),
            fallback_user_id.clone(),
            tool_diagnostic_sink,
        )
        .with_artifact_services(
            Arc::clone(&services.artifact_persistence),
            Arc::clone(&services.artifact_governor),
        )
        .with_observer(capability_observer),
    );
    let capability_input_resolver: Arc<dyn LoopCapabilityInputResolver> = capability_io.clone();
    let capability_result_writer: Arc<dyn LoopCapabilityResultWriter> = capability_io.clone();
    // Shared per-runtime catalog (owned by the composed runtime) so the
    // OpenAI-compatible Responses surface and this loop host see the same
    // run-scoped external-tool state.
    let external_tool_catalog: Arc<dyn ExternalToolCatalog> =
        services.external_tool_catalog.clone();
    let unavailable_capability_ids = if trigger_poller_enabled {
        HashSet::new()
    } else {
        HashSet::from([
            CapabilityId::new(ironclaw_host_runtime::TRIGGER_RUN_CAPABILITY_ID).map_err(
                |error| crate::runtime::RebornRuntimeError::MalformedConfig {
                    reason: format!("invalid trigger-run capability id: {error}"),
                },
            )?,
        ])
    };
    // Wire the durable gate-record and host-private replay-payload stores over
    // the composition-owned scoped filesystem (same backend + per-user mount view
    // as every other durable store; `extension_filesystem` is the shared composite
    // root). Before this, the loop-host capability port defaulted both to
    // no-op/fail-closed, so production gate records never persisted (the #6245 gap)
    // and a gate/auth resume had no host-side replay payload to reconstitute
    // {input, estimate} from (arch-simplification §5.3 Stage 2a-i).
    let capability_store_filesystem =
        crate::wrap_scoped(Arc::clone(&services.extension_filesystem));
    let gate_record_store: Arc<dyn ironclaw_approvals::GateRecordStorePort> = Arc::new(
        ironclaw_approvals::GateRecordStore::new(Arc::clone(&capability_store_filesystem)),
    );
    let replay_payload_store: Arc<dyn ironclaw_capabilities::ReplayPayloadStorePort> = Arc::new(
        ironclaw_capabilities::ReplayPayloadStore::new(capability_store_filesystem),
    );
    let capability_factory: Arc<dyn LoopCapabilityPortFactory> =
        Arc::new(RefreshingLoopCapabilityPortFactory {
            runtime,
            fallback_user_id,
            policy,
            workspace_mounts,
            memory_mounts,
            system_extensions_lifecycle_mounts,
            extension_surface_source,
            input_resolver: Arc::clone(&capability_input_resolver),
            result_writer: Arc::clone(&capability_result_writer),
            milestone_sink,
            skill_activation_source,
            project_service,
            trajectory_observer,
            outbound_preferences_service,
            outbound_preference_write_requires_approval,
            approval_settings,
            approval_requests,
            capability_leases,
            gate_record_store,
            replay_payload_store,
            external_tool_catalog,
            unavailable_capability_ids,
        });
    Ok(CapabilityPortWiring {
        capability_factory,
        capability_input_resolver,
        capability_result_writer,
        model_gateway,
        display_previews,
    })
}

#[derive(Clone)]
struct RefreshingLoopCapabilityPortFactory {
    runtime: Arc<dyn HostRuntime>,
    fallback_user_id: UserId,
    policy: Arc<BuiltinCapabilityPolicy>,
    /// Resolved per run, not once per runtime: under a per-caller policy the
    /// `mounts = "workspace"` grants must point at the caller's own subtree.
    workspace_mounts: WorkspaceMountPolicy,
    memory_mounts: MountView,
    system_extensions_lifecycle_mounts: MountView,
    extension_surface_source: ExtensionCapabilitySurfaceSource,
    input_resolver: Arc<dyn LoopCapabilityInputResolver>,
    result_writer: Arc<dyn LoopCapabilityResultWriter>,
    milestone_sink: Arc<dyn LoopHostMilestoneSink>,
    skill_activation_source: Option<Arc<ComposedSelectableSkillContextSource>>,
    project_service: Arc<dyn ProjectService>,
    trajectory_observer: Option<Arc<dyn crate::RebornTrajectoryObserver>>,
    outbound_preferences_service: Option<Arc<dyn OutboundPreferencesProductService>>,
    outbound_preference_write_requires_approval: bool,
    approval_settings: Arc<dyn ApprovalSettingsProvider>,
    approval_requests: Arc<dyn ironclaw_approvals::ApprovalRequestStorePort>,
    capability_leases: Arc<dyn ironclaw_authorization::CapabilityLeaseStorePort>,
    /// Durable model-visible gate-record store; one instance per runtime, shared
    /// by reference into every port this factory builds.
    gate_record_store: Arc<dyn ironclaw_approvals::GateRecordStorePort>,
    /// Durable host-private replay-payload store (§5.3 Stage 2a-i); one instance
    /// per runtime, shared by reference into every port this factory builds.
    replay_payload_store: Arc<dyn ironclaw_capabilities::ReplayPayloadStorePort>,
    /// Per-runtime catalog of client-supplied ("external") tools. Shared across
    /// all runs in this runtime so a parked external-tool call and its later
    /// client-submitted output (across a pause/resume) hit the same store.
    external_tool_catalog: Arc<dyn ExternalToolCatalog>,
    unavailable_capability_ids: HashSet<CapabilityId>,
}

/// Make skill files readable by the ordinary filesystem tools, read-only.
///
/// Skill mounts went to the skill capabilities only, so a model reaching for
/// `skills/<name>/SKILL.md` was told the path resolves in no available root. That is a parity gap,
/// not just a bad message: in Claude Code a SKILL.md *is* a file, and skills reference siblings
/// (`references/*.md`, `scripts/*.py`) progressive disclosure expects the agent to open.
///
/// Read-only and additive: writes stay with `skill_install`/`skill_update`, and existing aliases are
/// left untouched, so this can never widen or downgrade a grant.
fn with_read_only_skill_paths(
    workspace_mounts: MountView,
    scope: &ResourceScope,
) -> Result<MountView, ironclaw_host_api::error::HostApiError> {
    let skill_reads = crate::runtime_mounts::db_backed_skill_context_mount_view(scope)?;
    let mut mounts = workspace_mounts.mounts;
    for grant in skill_reads.mounts {
        if !mounts
            .iter()
            .any(|existing| existing.alias.as_str() == grant.alias.as_str())
        {
            mounts.push(grant);
        }
    }
    MountView::new(mounts)
}

#[cfg(feature = "test-support")]
pub(crate) fn with_read_only_skill_paths_for_test(
    workspace_mounts: MountView,
    scope: &ResourceScope,
) -> Result<MountView, ironclaw_host_api::error::HostApiError> {
    with_read_only_skill_paths(workspace_mounts, scope)
}

#[async_trait::async_trait]
impl LoopCapabilityPortFactory for RefreshingLoopCapabilityPortFactory {
    async fn create_capability_port(
        &self,
        run_context: &LoopRunContext,
    ) -> Result<Arc<dyn LoopCapabilityPort>, AgentLoopHostError> {
        self.create_capability_port_with_surface_policy(
            run_context,
            Arc::new(CapabilitySurfacePolicy::allow_all()),
        )
        .await
    }

    async fn create_capability_port_with_surface_policy(
        &self,
        run_context: &LoopRunContext,
        surface_policy: Arc<CapabilitySurfacePolicy>,
    ) -> Result<Arc<dyn LoopCapabilityPort>, AgentLoopHostError> {
        let resource_scope = resource_scope_for_run(run_context, &self.fallback_user_id);
        // Database-backed, same tree the reader and Settings use. This port is where an agent's own
        // `skill_install` lands, and it used to write host disk instead (nearai/ironclaw#7168).
        let skill_mounts = db_backed_skill_management_mount_view(&resource_scope)
            .map_err(host_api_agent_loop_error)?;
        // Same scope the skill mounts key off, so a run's workspace grants and
        // its skill mounts can never resolve to different callers.
        let workspace_mounts = self
            .workspace_mounts
            .capability_grant_view(&resource_scope)
            .map_err(host_api_agent_loop_error)?;
        let workspace_mounts = with_read_only_skill_paths(workspace_mounts, &resource_scope)
            .map_err(host_api_agent_loop_error)?;
        create_refreshing_capability_port(RefreshingCapabilityPortConfig {
            runtime: Arc::clone(&self.runtime),
            run_context: run_context.clone(),
            surface_policy,
            fallback_user_id: self.fallback_user_id.clone(),
            policy: Arc::clone(&self.policy),
            workspace_mounts,
            skill_mounts,
            memory_mounts: self.memory_mounts.clone(),
            system_extensions_lifecycle_mounts: self.system_extensions_lifecycle_mounts.clone(),
            extension_surface_source: self.extension_surface_source.clone(),
            input_resolver: Arc::clone(&self.input_resolver),
            result_writer: Arc::clone(&self.result_writer),
            milestone_sink: Arc::clone(&self.milestone_sink),
            skill_activation_source: self.skill_activation_source.clone(),
            project_service: Arc::clone(&self.project_service),
            // Same observer drives both the input hook (on the capability port the
            // refreshing helper builds) and the result hook (on `StagedCapabilityIo`),
            // so the two callbacks correlate by `call_id` for one tool call.
            trajectory_observer: self.trajectory_observer.clone(),
            outbound_preferences_service: self.outbound_preferences_service.clone(),
            outbound_preference_write_requires_approval: self
                .outbound_preference_write_requires_approval,
            approval_settings: Arc::clone(&self.approval_settings),
            approval_requests: Arc::clone(&self.approval_requests),
            capability_leases: Arc::clone(&self.capability_leases),
            gate_record_store: Arc::clone(&self.gate_record_store),
            replay_payload_store: Arc::clone(&self.replay_payload_store),
            external_tool_catalog: Arc::clone(&self.external_tool_catalog),
            // Test-support-only knobs (see each field's doc-comment on
            // `RefreshingCapabilityPortConfig`): always empty here.
            capability_execution_mount_overrides: HashMap::new(),
            additional_provider_trust: BTreeMap::new(),
            capability_id_filter: None,
            unavailable_capability_ids: self.unavailable_capability_ids.clone(),
            additional_capability_grants: Vec::new(),
        })
        .await
    }
}

const CAPABILITY_IO_MAX_STAGED_REFS: usize = 1024;
const CAPABILITY_IO_MAX_STAGED_BYTES: usize = 4 * 1024 * 1024;
/// Maximum canonical result content carried inline to the model.
const RESULT_PREVIEW_MAX_BYTES: usize = ARTIFACT_INLINE_PREVIEW_MAX_BYTES;

struct StagedCapabilityIo {
    inputs: StdMutex<StagedValueStore>,
    results: StdMutex<StagedValueStore>,
    display_previews: Arc<CapabilityDisplayPreviewStore>,
    durable_previews: Option<DurableCapabilityDisplayPreviewSink>,
    artifact_persistence: Option<Arc<dyn AccountedArtifactPersister>>,
    artifact_governor: Option<Arc<dyn ResourceGovernor>>,
    /// Optional consumer hook. This struct drives only the *result* half of the
    /// trajectory observer (via `write_capability_result`); the resolved
    /// tool-call inputs are emitted upstream by `HostRuntimeLoopCapabilityPort`
    /// (the input resolver bypasses this IO for provider tool-call inputs).
    observer: Option<Arc<dyn CapabilityTrajectoryObserver>>,
    tool_diagnostics: HostManagedToolDiagnosticEmitter,
}

#[derive(Clone)]
struct DurableCapabilityDisplayPreviewSink {
    thread_service: Arc<dyn SessionThreadService>,
    /// Fallback owner used only when a run scope carries no explicit owner.
    /// The durable thread scope is otherwise derived per-append from the
    /// run context so previews write under the SAME scope the run's thread
    /// was registered under (see `thread_scope_for_run`).
    fallback_user_id: UserId,
}

impl Default for StagedCapabilityIo {
    fn default() -> Self {
        Self::new(Arc::new(CapabilityDisplayPreviewStore::default()))
    }
}

impl StagedCapabilityIo {
    fn new(display_previews: Arc<CapabilityDisplayPreviewStore>) -> Self {
        Self {
            inputs: StdMutex::new(StagedValueStore::default()),
            results: StdMutex::new(StagedValueStore::default()),
            display_previews,
            durable_previews: None,
            artifact_persistence: None,
            artifact_governor: None,
            observer: None,
            tool_diagnostics: HostManagedToolDiagnosticEmitter::default(),
        }
    }

    fn new_with_durable_previews(
        display_previews: Arc<CapabilityDisplayPreviewStore>,
        thread_service: Arc<dyn SessionThreadService>,
        fallback_user_id: UserId,
        tool_diagnostic_sink: Option<Arc<dyn HostManagedPromptDiagnosticSink>>,
    ) -> Self {
        Self {
            inputs: StdMutex::new(StagedValueStore::default()),
            results: StdMutex::new(StagedValueStore::default()),
            display_previews,
            durable_previews: Some(DurableCapabilityDisplayPreviewSink {
                thread_service,
                fallback_user_id,
            }),
            artifact_persistence: None,
            artifact_governor: None,
            observer: None,
            tool_diagnostics: HostManagedToolDiagnosticEmitter::new(tool_diagnostic_sink),
        }
    }

    /// Attach a trajectory observer (no-op when `None`).
    fn with_observer(mut self, observer: Option<Arc<dyn CapabilityTrajectoryObserver>>) -> Self {
        self.observer = observer;
        self
    }

    fn with_artifact_services(
        mut self,
        persistence: Arc<dyn AccountedArtifactPersister>,
        governor: Arc<dyn ResourceGovernor>,
    ) -> Self {
        self.artifact_persistence = Some(persistence);
        self.artifact_governor = Some(governor);
        self
    }

    #[cfg(test)]
    fn result_output(
        &self,
        result_ref: &str,
    ) -> Result<Option<serde_json::Value>, AgentLoopHostError> {
        self.results
            .lock()
            .map_err(|_| capability_io_error())
            .map(|results| results.get(result_ref).cloned())
    }

    fn durable_tool_result_scope(
        &self,
        run_context: &LoopRunContext,
    ) -> Result<Option<(&DurableCapabilityDisplayPreviewSink, ThreadScope)>, AgentLoopHostError>
    {
        let Some(durable_previews) = &self.durable_previews else {
            return Ok(None);
        };
        let scope = thread_scope_for_run(run_context, &durable_previews.fallback_user_id)
            .ok_or_else(durable_result_scope_error)?;
        Ok(Some((durable_previews, scope)))
    }

    fn stage_result_best_effort(
        &self,
        result_ref: &LoopResultRef,
        output: serde_json::Value,
        serialized_bytes: usize,
    ) {
        let Ok(mut results) = self.results.lock() else {
            tracing::warn!("capability-host result staging lock failed; using durable result only");
            return;
        };
        if let Err(error) = results.insert_with_oldest_eviction(
            result_ref.as_str().to_string(),
            output,
            serialized_bytes,
        ) {
            tracing::debug!(
                result_ref = result_ref.as_str(),
                error = %error.safe_summary,
                "skipping transient capability result staging; durable result remains available"
            );
        }
    }

    async fn try_append_durable_display_preview(
        &self,
        run_context: &LoopRunContext,
        invocation_id: InvocationId,
        capability_id: &CapabilityId,
        status: CapabilityDisplayPreviewStatus,
    ) -> Option<ThreadMessageId> {
        let Some(durable_previews) = &self.durable_previews else {
            return None;
        };
        let Some(record) = self.display_previews.record_for_invocation(invocation_id) else {
            tracing::debug!(
                invocation_id = %invocation_id,
                capability_id = capability_id.as_str(),
                "capability display preview record missing after result staging"
            );
            return None;
        };
        let preview =
            match CapabilityDisplayPreviewEnvelope::new(CapabilityDisplayPreviewEnvelopeInput {
                invocation_id,
                capability_id: capability_id.clone(),
                status,
                title: record.title,
                subtitle: record.subtitle,
                input_summary: record.input_summary,
                output_summary: record.output_summary,
                output_preview: record.output_preview,
                output_kind: record.output_kind,
                output_bytes: record.output_bytes,
                result_ref: record.result_ref,
                truncated: record.truncated,
                updated_at: Utc::now(),
                activity_order: None,
            }) {
                Ok(preview) => preview,
                Err(error) => {
                    tracing::debug!(
                        invocation_id = %invocation_id,
                        capability_id = capability_id.as_str(),
                        error,
                        "capability display preview envelope validation failed"
                    );
                    return None;
                }
            };
        // Derive the durable thread scope from the run context so the preview
        // writes under the SAME scope the run's thread was registered under.
        // A composition-time constant scope can mismatch the run's actual
        // owner/project and surface as a spurious `UnknownThread` on append.
        let Some(thread_scope) =
            thread_scope_for_run(run_context, &durable_previews.fallback_user_id)
        else {
            tracing::debug!(
                invocation_id = %invocation_id,
                capability_id = capability_id.as_str(),
                "capability display preview skipped: run scope has no agent"
            );
            return None;
        };
        let message = match durable_previews
            .thread_service
            .append_capability_display_preview(AppendCapabilityDisplayPreviewRequest {
                scope: thread_scope,
                thread_id: run_context.thread_id.clone(),
                turn_run_id: run_context.run_id.to_string(),
                preview,
            })
            .await
        {
            Ok(message) => message,
            Err(error) => {
                tracing::debug!(
                    invocation_id = %invocation_id,
                    capability_id = capability_id.as_str(),
                    error = %error,
                    "capability display preview durable append failed; continuing with staged capability result"
                );
                return None;
            }
        };
        Some(message.message_id)
    }
}

/// Test-support constructor wired exactly like production's
/// `capability_wiring` (`new_with_durable_previews`): durable previews over
/// the caller's `thread_service` and `fallback_user_id`, no trajectory
/// observer. Returns two `Arc` clones of ONE underlying io object -- input
/// resolver and result writer MUST stay two views of the same object so a
/// call's input-ref and result-ref correlate by `call_id`.
///
/// Lets the integration-test harness drive durable tool-result projection
/// instead of the ephemeral `ProductLiveCapabilityIo` test double, which
/// never persists a durable record. For tests only -- gated behind
/// `test-support`, ships zero bytes in production builds.
#[cfg(feature = "test-support")]
pub(super) fn staged_capability_io_for_test(
    thread_service: Arc<dyn SessionThreadService>,
    fallback_user_id: UserId,
) -> (
    Arc<dyn LoopCapabilityInputResolver>,
    Arc<dyn LoopCapabilityResultWriter>,
) {
    let io = Arc::new(StagedCapabilityIo::new_with_durable_previews(
        Arc::new(CapabilityDisplayPreviewStore::default()),
        thread_service,
        fallback_user_id,
        None,
    ));
    let input_resolver: Arc<dyn LoopCapabilityInputResolver> = io.clone();
    let result_writer: Arc<dyn LoopCapabilityResultWriter> = io;
    (input_resolver, result_writer)
}

#[cfg(feature = "test-support")]
pub(super) fn staged_capability_io_with_observer_for_test(
    thread_service: Arc<dyn SessionThreadService>,
    fallback_user_id: UserId,
    observer: Option<Arc<dyn crate::RebornTrajectoryObserver>>,
) -> (
    Arc<dyn LoopCapabilityInputResolver>,
    Arc<dyn LoopCapabilityResultWriter>,
) {
    let io = Arc::new(
        StagedCapabilityIo::new_with_durable_previews(
            Arc::new(CapabilityDisplayPreviewStore::default()),
            thread_service,
            fallback_user_id,
            None,
        )
        .with_observer(
            observer.map(crate::observability::trajectory_observer::as_capability_observer),
        ),
    );
    let input_resolver: Arc<dyn LoopCapabilityInputResolver> = io.clone();
    let result_writer: Arc<dyn LoopCapabilityResultWriter> = io;
    (input_resolver, result_writer)
}

#[derive(Default)]
struct StagedValueStore {
    values: HashMap<String, StagedValue>,
    // Eviction index only, not an execution queue. Inputs fail closed and never
    // evict; results use this to drop oldest staged refs under byte pressure.
    oldest_refs: VecDeque<String>,
    total_bytes: usize,
}

struct StagedValue {
    value: serde_json::Value,
    bytes: usize,
}

impl StagedValueStore {
    fn get(&self, reference: &str) -> Option<&serde_json::Value> {
        self.values.get(reference).map(|staged| &staged.value)
    }

    fn insert_without_eviction(
        &mut self,
        reference: String,
        value: serde_json::Value,
    ) -> Result<(), AgentLoopHostError> {
        let bytes = staged_value_bytes(&value)?;
        if self.values.len() >= CAPABILITY_IO_MAX_STAGED_REFS
            || self.total_bytes.saturating_add(bytes) > CAPABILITY_IO_MAX_STAGED_BYTES
        {
            return Err(AgentLoopHostError::new(
                AgentLoopHostErrorKind::BudgetExceeded,
                "capability-host staging is full",
            ));
        }
        self.insert_measured(reference, value, bytes);
        Ok(())
    }

    fn insert_with_oldest_eviction(
        &mut self,
        reference: String,
        value: serde_json::Value,
        bytes: usize,
    ) -> Result<(), AgentLoopHostError> {
        if bytes > CAPABILITY_IO_MAX_STAGED_BYTES {
            return Err(AgentLoopHostError::new(
                AgentLoopHostErrorKind::BudgetExceeded,
                "capability-host result exceeds staging budget",
            ));
        }
        while self.values.len() >= CAPABILITY_IO_MAX_STAGED_REFS
            || self.total_bytes.saturating_add(bytes) > CAPABILITY_IO_MAX_STAGED_BYTES
        {
            self.evict_oldest();
        }
        self.insert_measured(reference, value, bytes);
        Ok(())
    }

    fn insert_measured(&mut self, reference: String, value: serde_json::Value, bytes: usize) {
        if let Some(previous) = self.values.remove(&reference) {
            self.total_bytes = self.total_bytes.saturating_sub(previous.bytes);
            self.oldest_refs.retain(|candidate| candidate != &reference);
        }
        self.total_bytes = self.total_bytes.saturating_add(bytes);
        self.oldest_refs.push_back(reference.clone());
        self.values.insert(reference, StagedValue { value, bytes });
    }

    fn evict_oldest(&mut self) {
        while let Some(reference) = self.oldest_refs.pop_front() {
            if let Some(previous) = self.values.remove(&reference) {
                self.total_bytes = self.total_bytes.saturating_sub(previous.bytes);
                return;
            }
        }
    }

    fn remove(&mut self, reference: &str) {
        if let Some(previous) = self.values.remove(reference) {
            self.total_bytes = self.total_bytes.saturating_sub(previous.bytes);
            self.oldest_refs.retain(|candidate| candidate != reference);
        }
    }
}

fn staged_value_bytes(value: &serde_json::Value) -> Result<usize, AgentLoopHostError> {
    serde_json::to_vec(value)
        .map(|bytes| bytes.len())
        .map_err(|error| {
            ironclaw_loop_host::raw_agent_loop_host_error(
                "capability_host_io",
                "measure_payload",
                AgentLoopHostErrorKind::InvalidInvocation,
                "capability payload could not be measured",
                error,
            )
        })
}

#[async_trait::async_trait]
impl LoopCapabilityInputResolver for StagedCapabilityIo {
    async fn resolve_capability_input(
        &self,
        run_context: &LoopRunContext,
        input_ref: &CapabilityInputRef,
    ) -> Result<serde_json::Value, AgentLoopHostError> {
        ensure_ref_scope("input", input_ref.as_str(), run_context)?;
        let inputs = self.inputs.lock().map_err(|_| capability_io_error())?;
        inputs.get(input_ref.as_str()).cloned().ok_or_else(|| {
            AgentLoopHostError::new(
                AgentLoopHostErrorKind::InvalidInvocation,
                "capability input ref was not staged for this loop run",
            )
        })
    }

    async fn register_provider_tool_call_input(
        &self,
        run_context: &LoopRunContext,
        tool_call: &ProviderToolCall,
    ) -> Result<CapabilityInputRef, AgentLoopHostError> {
        let input_ref =
            CapabilityInputRef::new(format!("input:{}:{}", run_context.run_id, Uuid::new_v4()))
                .map_err(|_| {
                    AgentLoopHostError::new(
                        AgentLoopHostErrorKind::Internal,
                        "capability input ref could not be represented",
                    )
                })?;
        let mut inputs = self.inputs.lock().map_err(|_| capability_io_error())?;
        inputs
            .insert_without_eviction(input_ref.as_str().to_string(), tool_call.arguments.clone())?;
        // Record the display-preview input under this staging ref for callers
        // that drive the adapter directly (tests, non-decorated paths). In the
        // production loop the resolver is wrapped by
        // `ProviderToolCallInputResolver`, which owns a different (digest) ref
        // and bypasses this method — that path records via
        // `record_provider_tool_call_display_input` below instead. Trajectory
        // inputs are separately observed at the port level
        // (`HostRuntimeLoopCapabilityPort::invoke_capability`), which forwards
        // the resolved dotted `CapabilityId`.
        self.display_previews.record_input(
            &run_context.run_id.to_string(),
            &input_ref,
            tool_call.name.as_str(),
            &tool_call.arguments,
        );
        self.tool_diagnostics.record_input(
            run_context,
            &input_ref,
            tool_call.name.as_str(),
            &tool_call.arguments,
        );
        Ok(input_ref)
    }

    fn record_provider_tool_call_display_input(
        &self,
        run_context: &LoopRunContext,
        input_ref: &CapabilityInputRef,
        capability_id: &CapabilityId,
        tool_call: &ProviderToolCall,
    ) {
        // Driven by the `ProviderToolCallInputResolver` decorator under the
        // canonical (digest) provider tool-call ref, so the activity-card input
        // summary lands under the same ref `write_capability_result` later uses.
        // Key the display by the resolved dotted `capability_id`, not the lossy
        // provider tool name, so the title and per-tool summary are correct.
        self.display_previews.record_input(
            &run_context.run_id.to_string(),
            input_ref,
            capability_id.as_str(),
            &tool_call.arguments,
        );
        self.tool_diagnostics.record_input(
            run_context,
            input_ref,
            capability_id.as_str(),
            &tool_call.arguments,
        );
    }
}

#[async_trait::async_trait]
impl LoopCapabilityResultWriter for StagedCapabilityIo {
    async fn write_capability_result(
        &self,
        write: CapabilityResultWrite<'_>,
    ) -> Result<CapabilityWriteResult, AgentLoopHostError> {
        let CapabilityResultWrite {
            run_context,
            input_ref,
            invocation_id,
            capability_id,
            output,
            display_preview,
            receipt,
            completed_artifact,
            durable_persistence,
            canonical_output_digest,
            canonical_item_count,
        } = write;
        let result_ref =
            LoopResultRef::new(format!("result:{}.{}", run_context.run_id, Uuid::new_v4()))
                .map_err(|_| {
                    AgentLoopHostError::new(
                        AgentLoopHostErrorKind::Internal,
                        "capability result ref could not be represented",
                    )
                })?;
        let (output_content, output_bytes, item_count, preview, artifact) = if let Some(artifact) =
            completed_artifact
        {
            let accounted_bytes = receipt
                .and_then(|receipt| receipt.actual.as_ref())
                .map(|usage| usage.output_bytes);
            if accounted_bytes != Some(artifact.byte_len) {
                return Err(AgentLoopHostError::new(
                    AgentLoopHostErrorKind::InvalidInvocation,
                    "completed capability artifact does not match accounting evidence",
                ));
            }
            if artifact.byte_len > RESULT_PREVIEW_MAX_BYTES as u64 {
                let (content, text) = if let Some(text) = output.as_str() {
                    (text.as_bytes().to_vec(), text.to_string())
                } else {
                    let content = serialized_result_output(&output)?;
                    let text = std::str::from_utf8(&content)
                        .map_err(|_| {
                            AgentLoopHostError::new(
                                AgentLoopHostErrorKind::InvalidInvocation,
                                "large capability result preview cannot be represented",
                            )
                        })?
                        .to_string();
                    (content, text)
                };
                let preview_len = u64::try_from(content.len()).map_err(|_| {
                    AgentLoopHostError::new(
                        AgentLoopHostErrorKind::InvalidInvocation,
                        "large capability result preview length is unsupported",
                    )
                })?;
                if content.len() > RESULT_PREVIEW_MAX_BYTES || preview_len >= artifact.byte_len {
                    return Err(AgentLoopHostError::new(
                        AgentLoopHostErrorKind::InvalidInvocation,
                        "large capability result preview does not match artifact evidence",
                    ));
                }
                (
                    content,
                    artifact.byte_len,
                    canonical_item_count,
                    Some(FirstLookResultPreview {
                        text,
                        next_offset: Some(preview_len),
                    }),
                    Some(artifact.clone()),
                )
            } else {
                let content = serialized_result_output(&output)?;
                if u64::try_from(content.len()).ok() != Some(artifact.byte_len) {
                    return Err(AgentLoopHostError::new(
                        AgentLoopHostErrorKind::InvalidInvocation,
                        "inline capability result does not match artifact evidence",
                    ));
                }
                let item_count = canonical_item_count
                    .or_else(|| output.as_array().map(|items| items.len() as u64));
                let preview = first_look_result_preview(&content);
                (
                    content,
                    artifact.byte_len,
                    item_count,
                    preview,
                    Some(artifact.clone()),
                )
            }
        } else {
            if receipt.is_some() {
                return Err(AgentLoopHostError::new(
                    AgentLoopHostErrorKind::InvalidInvocation,
                    "completed runtime result is missing its durable artifact",
                ));
            }
            let content = serialized_result_output(&output)?;
            let content_len = u64::try_from(content.len()).map_err(|_| {
                AgentLoopHostError::new(
                    AgentLoopHostErrorKind::BudgetExceeded,
                    "capability result byte length is outside the supported range",
                )
            })?;
            let item_count =
                canonical_item_count.or_else(|| output.as_array().map(|items| items.len() as u64));
            let preview = first_look_result_preview(&content);
            let artifact = if matches!(durable_persistence, DurablePersistence::InlineOnly) {
                None
            } else {
                match (
                    &self.artifact_persistence,
                    &self.artifact_governor,
                    &self.durable_previews,
                ) {
                    (Some(persistence), Some(governor), Some(durable_previews)) => {
                        let scope =
                            resource_scope_for_run(run_context, &durable_previews.fallback_user_id);
                        let reservation = governor
                            .reserve(
                                scope.clone(),
                                ResourceEstimate::default().set_output_bytes(content_len),
                            )
                            .map_err(|_| {
                                AgentLoopHostError::new(
                                    AgentLoopHostErrorKind::BudgetExceeded,
                                    "capability result artifact budget is unavailable",
                                )
                            })?;
                        let fallback_receipt = match governor.reconcile(
                            reservation.id,
                            ResourceUsage::default().set_output_bytes(content_len),
                        ) {
                            Ok(receipt) => receipt,
                            Err(_) => {
                                let _ = governor.release(reservation.id);
                                return Err(AgentLoopHostError::new(
                                    AgentLoopHostErrorKind::BudgetExceeded,
                                    "capability result artifact accounting failed",
                                ));
                            }
                        };
                        Some(
                            persistence
                                .persist(
                                    ArtifactWriteMetadata {
                                        write_key: None,
                                        owner_scope: ArtifactOwnerScope::from_resource_scope(
                                            &scope,
                                        ),
                                        namespace: run_context.effective_artifact_namespace(),
                                        producer_capability_id: capability_id.clone(),
                                        content_type: "application/json".to_string(),
                                        expected_bytes: Some(content_len),
                                    },
                                    &content,
                                    &fallback_receipt,
                                )
                                .await
                                .map_err(artifact_store_error)?,
                        )
                    }
                    _ => None,
                }
            };
            (content, content_len, item_count, preview, artifact)
        };
        let serialized_bytes = output_content.len();
        let diagnostic_result = self
            .tool_diagnostics
            .prepare_result(&output_content, TOOL_RESULT_DIAGNOSTIC_CAPTURE_MAX_BYTES);
        if serialized_bytes <= CAPABILITY_IO_MAX_STAGED_BYTES {
            self.stage_result_best_effort(&result_ref, output.clone(), serialized_bytes);
        }
        self.display_previews.record_result_with_preview(
            CapabilityDisplayPreviewResult {
                run_id: &run_context.run_id.to_string(),
                input_ref,
                invocation_id,
                capability_id,
                result_ref: result_ref.as_str(),
                output: &output,
                output_bytes,
            },
            display_preview.as_ref(),
        );
        if let Some(observer) = &self.observer {
            // Best-effort, inline on the capability hot path: a panicking
            // observer must never unwind capability result staging. (Blocking
            // is the observer's own contract — see `CapabilityTrajectoryObserver`.)
            let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                observer.on_capability_result(input_ref.as_str(), capability_id.as_str(), &output);
            }));
            if caught.is_err() {
                tracing::warn!(
                    capability_id = capability_id.as_str(),
                    "trajectory observer on_capability_result panicked; dropping event"
                );
            }
        }
        if let Some(message_id) = self
            .try_append_durable_display_preview(
                run_context,
                invocation_id,
                capability_id,
                CapabilityDisplayPreviewStatus::Completed,
            )
            .await
        {
            self.display_previews
                .attach_timeline_message_id(invocation_id, message_id);
        }
        self.tool_diagnostics.record_succeeded(
            run_context,
            invocation_id,
            capability_id,
            diagnostic_result,
            output_bytes,
        );
        let mut write_result =
            CapabilityWriteResult::from_output(result_ref, output_bytes, &output);
        let preview_incomplete = preview
            .as_ref()
            .and_then(|preview| preview.next_offset)
            .is_some();
        if let Some(canonical_output_digest) = canonical_output_digest {
            write_result.output_digest = Some(canonical_output_digest);
        } else if preview_incomplete {
            // Synthetic/internal callers without a canonical digest must not
            // mistake the bounded preview's digest for the full result.
            write_result.output_digest = None;
        }
        write_result.model_observation = Some(
            if preview_incomplete && matches!(durable_persistence, DurablePersistence::Persist) {
                let artifact = artifact.as_ref().ok_or_else(|| {
                    AgentLoopHostError::new(
                        AgentLoopHostErrorKind::Unavailable,
                        "durable capability result artifact is unavailable",
                    )
                })?;
                artifact_reference_observation(artifact, preview.as_ref(), item_count)
            } else {
                inline_result_observation(
                    preview.ok_or_else(|| {
                        AgentLoopHostError::new(
                            AgentLoopHostErrorKind::InvalidInvocation,
                            "inline capability result is unavailable",
                        )
                    })?,
                    item_count,
                )
            },
        );
        Ok(write_result)
    }

    fn record_running_invocation(
        &self,
        run_context: &LoopRunContext,
        invocation_id: InvocationId,
        input_ref: &CapabilityInputRef,
    ) {
        self.display_previews
            .record_running_invocation(invocation_id, input_ref);
        self.tool_diagnostics
            .record_started(run_context, invocation_id, input_ref);
    }

    async fn stage_capability_failure_preview(
        &self,
        run_context: &LoopRunContext,
        invocation_id: InvocationId,
        capability_id: &CapabilityId,
        summary: &str,
    ) {
        self.display_previews.record_failure_preview(
            &run_context.run_id.to_string(),
            invocation_id,
            capability_id,
            summary,
        );
        self.tool_diagnostics
            .record_failed(run_context, invocation_id, capability_id, summary);
        // Persist the failure preview to the durable timeline (status Failed)
        // so the detail survives refresh/replay, mirroring the success path in
        // `write_capability_result`.
        if let Some(message_id) = self
            .try_append_durable_display_preview(
                run_context,
                invocation_id,
                capability_id,
                CapabilityDisplayPreviewStatus::Failed,
            )
            .await
        {
            self.display_previews
                .attach_timeline_message_id(invocation_id, message_id);
        }
    }

    async fn update_capability_result(
        &self,
        run_context: &LoopRunContext,
        result_ref: &LoopResultRef,
        output: serde_json::Value,
    ) -> Result<CapabilityResultUpdate, AgentLoopHostError> {
        ensure_ref_scope("result", result_ref.as_str(), run_context)?;
        let content = serialized_result_output(&output)?;
        let byte_len = u64::try_from(content.len()).map_err(|_| {
            AgentLoopHostError::new(
                AgentLoopHostErrorKind::BudgetExceeded,
                "capability result byte length is outside the supported range",
            )
        })?;
        let Some((durable_previews, thread_scope)) = self.durable_tool_result_scope(run_context)?
        else {
            return Err(AgentLoopHostError::new(
                AgentLoopHostErrorKind::Unavailable,
                "synthetic capability settlement requires durable artifact services",
            ));
        };
        let (Some(persistence), Some(governor)) =
            (&self.artifact_persistence, &self.artifact_governor)
        else {
            return Err(AgentLoopHostError::new(
                AgentLoopHostErrorKind::Unavailable,
                "capability result artifact services are unavailable",
            ));
        };
        let resource_scope =
            resource_scope_for_run(run_context, &durable_previews.fallback_user_id);
        let estimate = ResourceEstimate::default().set_output_bytes(byte_len);
        let usage = ResourceUsage::default().set_output_bytes(byte_len);
        let reservation_id = settlement_reservation_id(&resource_scope, run_context, result_ref)?;
        let newly_reserved = match governor.reserve_with_id(
            resource_scope.clone(),
            estimate.clone(),
            reservation_id,
        ) {
            Ok(_) => true,
            Err(ResourceError::ReservationAlreadyExists { .. }) => false,
            Err(_) => {
                return Err(AgentLoopHostError::new(
                    AgentLoopHostErrorKind::BudgetExceeded,
                    "capability result artifact budget is unavailable",
                ));
            }
        };
        let receipt = match governor.reconcile(reservation_id, usage.clone()) {
            Ok(receipt) => receipt,
            Err(ResourceError::ReservationClosed {
                status: ReservationStatus::Reconciled,
                ..
            }) if !newly_reserved => ResourceReceipt {
                id: reservation_id,
                scope: resource_scope.clone(),
                status: ReservationStatus::Reconciled,
                estimate,
                actual: Some(usage),
            },
            Err(_) => {
                if newly_reserved {
                    let _ = governor.release(reservation_id);
                }
                return Err(AgentLoopHostError::new(
                    AgentLoopHostErrorKind::BudgetExceeded,
                    "capability result artifact accounting failed",
                ));
            }
        };
        let producer_capability_id = CapabilityId::new("builtin.spawn_subagent").map_err(|_| {
            AgentLoopHostError::new(
                AgentLoopHostErrorKind::Internal,
                "subagent capability identity is invalid",
            )
        })?;
        let artifact = persistence
            .persist(
                ArtifactWriteMetadata {
                    write_key: None,
                    owner_scope: ArtifactOwnerScope::from_resource_scope(&resource_scope),
                    namespace: run_context.effective_artifact_namespace(),
                    producer_capability_id,
                    content_type: "application/json".to_string(),
                    expected_bytes: Some(byte_len),
                },
                &content,
                &receipt,
            )
            .await
            .map_err(artifact_store_error)?;
        let preview = first_look_result_preview(&content);
        let observation = artifact_reference_observation(
            &artifact,
            preview.as_ref(),
            output.as_array().map(|items| items.len() as u64),
        );
        let model_observation = serde_json::to_value(observation).map_err(|error| {
            ironclaw_loop_host::raw_agent_loop_host_error(
                "capability_host_io",
                "serialize_settled_result_observation",
                AgentLoopHostErrorKind::InvalidInvocation,
                "settled capability result observation could not be serialized",
                error,
            )
        })?;
        durable_previews
            .thread_service
            .update_tool_result_reference(UpdateToolResultReferenceRequest {
                scope: thread_scope,
                thread_id: run_context.thread_id.clone(),
                turn_run_id: run_context.run_id.to_string(),
                result_ref: result_ref.as_str().to_string(),
                provider_call_id: None,
                safe_summary: ToolResultSafeSummary::new("subagent completed").map_err(|_| {
                    AgentLoopHostError::new(
                        AgentLoopHostErrorKind::Internal,
                        "subagent completion summary is invalid",
                    )
                })?,
                model_observation: Some(model_observation),
            })
            .await
            .map_err(durable_result_store_error)?;
        self.stage_result_best_effort(result_ref, output, content.len());
        Ok(CapabilityResultUpdate {
            byte_len,
            completed_artifact: Some(artifact),
        })
    }

    async fn delete_capability_result(
        &self,
        run_context: &LoopRunContext,
        result_ref: &LoopResultRef,
    ) -> Result<(), AgentLoopHostError> {
        ensure_ref_scope("result", result_ref.as_str(), run_context)?;
        self.results
            .lock()
            .map_err(|_| capability_io_error())?
            .remove(result_ref.as_str());
        Ok(())
    }
}

fn serialized_result_output(output: &serde_json::Value) -> Result<Vec<u8>, AgentLoopHostError> {
    let content = serde_json::to_vec(output).map_err(|error| {
        ironclaw_loop_host::raw_agent_loop_host_error(
            "capability_host_io",
            "serialize_result",
            AgentLoopHostErrorKind::InvalidInvocation,
            "capability result could not be serialized",
            error,
        )
    })?;
    Ok(content)
}

/// A bounded, UTF-8-safe first-look slice of a serialized result payload,
/// truncated at `RESULT_PREVIEW_MAX_BYTES`.
struct FirstLookResultPreview {
    text: String,
    /// `None` when `text` already covers the entire payload.
    next_offset: Option<u64>,
}

/// Builds the inline first-look preview from the same serialized bytes the
/// durable artifact stores, so a truncated preview's `next_offset` matches
/// the selector offset used when the coding `read` tool reads the artifact URI.
fn first_look_result_preview(serialized: &[u8]) -> Option<FirstLookResultPreview> {
    let Ok(full_text) = std::str::from_utf8(serialized) else {
        return None;
    };
    if full_text.len() <= RESULT_PREVIEW_MAX_BYTES {
        return Some(FirstLookResultPreview {
            text: full_text.to_string(),
            next_offset: None,
        });
    }
    let end = floor_char_boundary(full_text, RESULT_PREVIEW_MAX_BYTES);
    Some(FirstLookResultPreview {
        text: full_text[..end].to_string(),
        next_offset: Some(end as u64),
    })
}

fn floor_char_boundary(value: &str, index: usize) -> usize {
    if index >= value.len() {
        return value.len();
    }
    let mut index = index;
    while index > 0 && !value.is_char_boundary(index) {
        index -= 1;
    }
    index
}

/// Truncated-artifact observation caption; names the full array's element
/// count when known so the model does not misread a truncated array preview
/// as the whole result.
///
/// The caption must survive the strict `SafeSummary` collapse in
/// `result_preview_parts` (path/payload delimiters and credential markers are
/// rejected), so the `artifact://` reference itself is deliberately NOT
/// interpolated here — it rides the observation's `detail.artifact_ref` and
/// `artifacts` list, which the transcript carries verbatim.
fn truncated_artifact_summary(item_count: Option<u64>) -> String {
    let base = "Tool completed; full output is stored in a durable artifact.";
    match item_count {
        Some(count) => format!("{base} Full result is a JSON array of {count} items."),
        None => base.to_string(),
    }
}

fn inline_result_observation(
    preview: FirstLookResultPreview,
    item_count: Option<u64>,
) -> ModelVisibleToolObservation {
    let byte_len = preview.text.len() as u64;
    ModelVisibleToolObservation {
        schema_version: MODEL_VISIBLE_TOOL_OBSERVATION_SCHEMA_VERSION,
        status: ToolObservationStatus::Success,
        summary: "Tool completed; inline content contains the full result.".to_string(),
        detail: ToolObservationDetail::InlineResult {
            content: preview.text,
            byte_len,
            item_count,
        },
        artifacts: Vec::new(),
        recovery: None,
        trust: ObservationTrust::UntrustedToolOutput,
    }
}

fn artifact_reference_observation(
    artifact: &ironclaw_host_api::artifact::CompletedArtifact,
    preview: Option<&FirstLookResultPreview>,
    item_count: Option<u64>,
) -> ModelVisibleToolObservation {
    let artifact_ref = artifact.artifact_ref.to_string();
    ModelVisibleToolObservation {
        schema_version: MODEL_VISIBLE_TOOL_OBSERVATION_SCHEMA_VERSION,
        status: ToolObservationStatus::Success,
        summary: truncated_artifact_summary(item_count),
        detail: ToolObservationDetail::ArtifactReference {
            artifact_ref: artifact_ref.clone(),
            total_bytes: artifact.byte_len,
            // Credential-bearing preview text is SUPPRESSED (not masked): the
            // model-visible observation must never carry raw capability
            // output that fails the preview content contract, and the durable
            // artifact reference plus `item_count` remain the continuation
            // authority. Benign previews pass through unchanged.
            preview: preview
                .filter(|preview| ModelResultPreview::new(preview.text.clone()).is_ok())
                .map(|preview| preview.text.clone()),
            item_count,
        },
        artifacts: vec![ModelVisibleArtifact {
            artifact_ref,
            summary: "Stored tool output".to_string(),
        }],
        recovery: None,
        trust: ObservationTrust::UntrustedToolOutput,
    }
}

fn settlement_reservation_id(
    resource_scope: &ResourceScope,
    run_context: &LoopRunContext,
    result_ref: &LoopResultRef,
) -> Result<ResourceReservationId, AgentLoopHostError> {
    let mut hasher = Sha256::new();
    hasher.update(resource_scope.tenant_id.as_str().as_bytes());
    hasher.update([0]);
    hasher.update(resource_scope.user_id.as_str().as_bytes());
    for value in [
        resource_scope.agent_id.as_ref().map(|id| id.as_str()),
        resource_scope.project_id.as_ref().map(|id| id.as_str()),
        resource_scope.mission_id.as_ref().map(|id| id.as_str()),
        resource_scope.thread_id.as_ref().map(|id| id.as_str()),
    ] {
        hasher.update([0]);
        if let Some(value) = value {
            hasher.update(value.as_bytes());
        }
    }
    hasher.update(
        run_context
            .effective_artifact_namespace()
            .as_run_id()
            .as_uuid()
            .as_bytes(),
    );
    hasher.update(result_ref.as_str().as_bytes());
    let digest = hasher.finalize();
    let mut uuid_bytes = [0_u8; 16];
    uuid_bytes.copy_from_slice(&digest[..16]);
    Ok(ResourceReservationId::from_uuid(Uuid::from_bytes(
        uuid_bytes,
    )))
}

fn artifact_store_error(
    error: ironclaw_host_api::artifact::ArtifactWriteError,
) -> AgentLoopHostError {
    tracing::warn!(error = %error, "durable tool artifact persistence failed");
    AgentLoopHostError::new(
        AgentLoopHostErrorKind::Unavailable,
        "durable tool artifact storage is unavailable",
    )
}

fn durable_result_store_error(error: ironclaw_threads::SessionThreadError) -> AgentLoopHostError {
    tracing::warn!(error = %error, "durable capability result persistence failed");
    AgentLoopHostError::new(
        AgentLoopHostErrorKind::Unavailable,
        "durable capability result storage is unavailable",
    )
}

fn durable_result_scope_error() -> AgentLoopHostError {
    AgentLoopHostError::new(
        AgentLoopHostErrorKind::Unavailable,
        "durable capability results require an agent-scoped thread",
    )
}

/// The scope a run's workspace grants and skill mounts key off. Delegates to
/// the one contract derivation ([`LoopRunContext::acting_resource_scope`]): a
/// run acts as its user, so its mounts resolve to the same identity as its
/// gates, settings, and deliveries. Since the ephemeral-per-ping remodel a
/// run's owner IS its actor, so there is a single identity to key off.
pub(super) fn resource_scope_for_run(
    run_context: &LoopRunContext,
    fallback_user_id: &UserId,
) -> ResourceScope {
    run_context.acting_resource_scope(fallback_user_id)
}

/// Build the per-run [`ThreadScope`] for durable display-preview appends.
///
/// Seeds the configured fallback owner into the run's tenant/agent/project scope,
/// then delegates owner selection to the canonical resolver. This keeps durable
/// operations on the same per-turn scope as the production loop host. Returns
/// `None` when the run scope carries no agent (durable previews are
/// agent-scoped), in which case the caller skips the durable append.
fn thread_scope_for_run(
    run_context: &LoopRunContext,
    fallback_user_id: &UserId,
) -> Option<ThreadScope> {
    let resource = run_context.scope.to_resource_scope();
    let base = ThreadScope {
        tenant_id: resource.tenant_id,
        agent_id: resource.agent_id?,
        project_id: resource.project_id,
        owner_user_id: Some(fallback_user_id.clone()),
        mission_id: resource.mission_id,
    };
    Some(ThreadScopeResolver::resolve_for_turn(
        &base,
        &run_context.scope,
        run_context.actor(),
    ))
}

struct VisibleCapabilityInputs<'a> {
    workspace_mounts: &'a MountView,
    skill_mounts: &'a MountView,
    memory_mounts: &'a MountView,
    system_extensions_lifecycle_mounts: &'a MountView,
    policy: &'a BuiltinCapabilityPolicy,
    surface_policy: &'a CapabilitySurfacePolicy,
    extension_surface: &'a ExtensionCapabilitySurface,
}

fn visible_capability_request(
    run_context: &LoopRunContext,
    fallback_user_id: &UserId,
    inputs: VisibleCapabilityInputs<'_>,
) -> Result<HostVisibleCapabilityRequest, AgentLoopHostError> {
    let extension_id = loop_driver_execution_extension_id(run_context)?;
    // Resolved BEFORE grant minting: extension grants are filtered per caller
    // (#5459 P1 — user-private installs mint grants only for their owner).
    // The caller is the run user — one contract derivation for grants, mounts,
    // and the gate dance alike (owner == actor, #7377).
    let user_id = run_context.acting_user_id(fallback_user_id);
    let mut grants = inputs.policy.builtin_grants(
        &extension_id,
        inputs.workspace_mounts,
        inputs.skill_mounts,
        inputs.memory_mounts,
        inputs.system_extensions_lifecycle_mounts,
    );
    grants
        .grants
        .extend(inputs.extension_surface.grants(&extension_id, &user_id));
    let mut context = ExecutionContext::local_default(
        user_id,
        extension_id,
        RuntimeKind::FirstParty,
        TrustClass::UserTrusted,
        grants,
        MountView::default(),
    )
    .map_err(host_api_agent_loop_error)?;
    context.tenant_id = run_context.scope.tenant_id.clone();
    context.agent_id = run_context.scope.agent_id.clone();
    context.project_id = run_context.scope.project_id.clone();
    context.thread_id = Some(run_context.thread_id.clone());
    context.resource_scope.tenant_id = context.tenant_id.clone();
    context.resource_scope.agent_id = context.agent_id.clone();
    context.resource_scope.project_id = context.project_id.clone();
    context.resource_scope.thread_id = context.thread_id.clone();
    context.validate().map_err(host_api_agent_loop_error)?;

    let builtin_provider =
        ExtensionId::new(inputs.policy.provider.id.as_str()).map_err(host_api_agent_loop_error)?;
    let mut provider_trust = BTreeMap::new();
    provider_trust.insert(
        builtin_provider,
        TrustDecision {
            effective_trust: EffectiveTrustClass::user_trusted(),
            authority_ceiling: AuthorityCeiling {
                allowed_effects: inputs.policy.provider.authority_effects.clone(),
                max_resource_ceiling: None,
            },
            provenance: TrustProvenance::AdminConfig,
            evaluated_at: Utc::now(),
        },
    );
    // The bound memory provider rides the same always-on first-party lane as
    // builtin (not the catalog extension surface), so every bundled memory
    // provider id is trusted here directly — only the bound one ever has a
    // registered package, so the others stay inert. The authority ceiling is
    // the memory provider's needs only: dispatch + read/write filesystem
    // (matching the builtin provider's first-party effects).
    for provider in ironclaw_host_runtime::memory_native_extension::MEMORY_PROVIDER_PACKAGE_IDS {
        provider_trust.insert(
            ExtensionId::new(*provider).map_err(host_api_agent_loop_error)?,
            TrustDecision {
                effective_trust: EffectiveTrustClass::user_trusted(),
                authority_ceiling: AuthorityCeiling {
                    allowed_effects: vec![
                        EffectKind::DispatchCapability,
                        EffectKind::ReadFilesystem,
                        EffectKind::WriteFilesystem,
                    ],
                    max_resource_ceiling: None,
                },
                provenance: TrustProvenance::AdminConfig,
                evaluated_at: Utc::now(),
            },
        );
    }
    provider_trust.extend(inputs.extension_surface.provider_trust(&context.user_id));

    Ok(HostVisibleCapabilityRequest::new(
        context,
        SurfaceKind::new("agent_loop").map_err(host_api_agent_loop_error)?,
    )
    .with_policy(inputs.surface_policy.clone())
    .with_provider_trust(provider_trust))
}

fn ensure_ref_scope(
    prefix: &str,
    reference: &str,
    run_context: &LoopRunContext,
) -> Result<(), AgentLoopHostError> {
    // Match product_live_adapters' convention: result refs are
    // `result:<run_id>.<uuid>` (dot) so they tokenize cleanly when a uuid
    // contains hyphens, while input refs stay `input:<run_id>:<n>` (colon).
    // Keep this in sync with `ensure_ref_scoped_to_run` in
    // `product_live_adapters.rs`.
    let separator = if prefix == "result" { "." } else { ":" };
    let expected_prefix = format!("{prefix}:{}{separator}", run_context.run_id);
    if reference.starts_with(&expected_prefix) {
        Ok(())
    } else {
        Err(AgentLoopHostError::new(
            AgentLoopHostErrorKind::ScopeMismatch,
            "capability input ref is not scoped to this loop run",
        ))
    }
}

fn capability_io_error() -> AgentLoopHostError {
    AgentLoopHostError::new(
        AgentLoopHostErrorKind::Internal,
        "capability io store is unavailable",
    )
}

fn host_api_agent_loop_error(
    error: impl std::fmt::Debug + std::fmt::Display,
) -> AgentLoopHostError {
    let safe_summary = error.to_string();
    ironclaw_loop_host::raw_agent_loop_host_error(
        "capability_host_api",
        "validate_runtime_input",
        AgentLoopHostErrorKind::InvalidInvocation,
        safe_summary,
        format!("{error:?}"),
    )
}

/// Shared test assertion for the `capability_host` per-capability submodules: the
/// §5.3 collapse maps a recoverable service failure onto `Resolution::Done`
/// carrying a `RecoverableFailure` verdict (the collapse of the old
/// `CapabilityOutcome::Failed`). Consumed by `outbound_delivery`,
/// `project_create`, and further submodules as they migrate to the `Resolution`
/// shape — replacing the byte-identical per-file copies (CodeRabbit #6299).
#[cfg(test)]
pub(crate) fn assert_recoverable_failure(
    resolution: &ironclaw_host_api::resolution::Resolution,
    expected: ironclaw_host_api::result_meta::FailureKind,
) {
    match resolution {
        ironclaw_host_api::resolution::Resolution::Done(outcome) => {
            assert_eq!(outcome.verdict.error_kind(), Some(&expected));
            let detail = outcome
                .verdict
                .diagnostic()
                .and_then(
                    ironclaw_host_api::result_meta::ModelFailureDiagnostic::model_visible_text,
                )
                .expect("recoverable failures must carry a model-visible cause");
            assert!(
                !detail.trim().is_empty(),
                "recoverable failure detail must be actionable"
            );
        }
        other => panic!("expected Resolution::Done recoverable failure, got {other:?}"),
    }
}

#[cfg(test)]
mod tests;
