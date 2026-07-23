use std::{
    collections::{BTreeMap, HashMap, VecDeque},
    sync::{Arc, Mutex as StdMutex},
};

use chrono::Utc;
use uuid::Uuid;

use ironclaw_authorization::CapabilityLeaseStore;
use ironclaw_host_api::{
    CapabilityId, EffectKind, ExecutionContext, ExtensionId, InvocationId, MountView,
    ResourceScope, RuntimeKind, TrustClass, UserId,
};
use ironclaw_host_runtime::{
    CapabilitySurfacePolicy, HostRuntime, SurfaceKind,
    VisibleCapabilityRequest as HostVisibleCapabilityRequest,
};
use ironclaw_loop_host::{
    CapabilityResultWrite, CapabilityWriteResult, DurablePersistence, HostManagedModelGateway,
    LoopCapabilityInputResolver, LoopCapabilityPortFactory, LoopCapabilityResultWriter,
    loop_driver_execution_extension_id,
};
use ironclaw_product::{OutboundPreferencesProductFacade, ProjectService};
use ironclaw_runner::thread_scope::ThreadScopeResolver;

use ironclaw_run_state::ApprovalRequestStore;
use ironclaw_threads::{
    AppendCapabilityDisplayPreviewRequest, CapabilityDisplayPreviewEnvelope,
    CapabilityDisplayPreviewEnvelopeInput, CapabilityDisplayPreviewStatus, SessionThreadService,
    TOOL_RESULT_RECORD_READ_MAX_BYTES, ThreadMessageId, ThreadScope,
};
use ironclaw_trust::{AuthorityCeiling, EffectiveTrustClass, TrustDecision, TrustProvenance};
use ironclaw_turns::{
    ExternalToolCatalog, LoopResultRef,
    run_profile::{
        AgentLoopHostError, AgentLoopHostErrorKind, CapabilityInputRef, LoopCapabilityPort,
        LoopHostMilestoneSink, LoopRunContext, MODEL_VISIBLE_TOOL_OBSERVATION_SCHEMA_VERSION,
        ModelVisibleArtifact, ModelVisibleToolObservation, ObservationTrust, ProviderToolCall,
        ToolObservationDetail, ToolObservationStatus,
    },
};

use crate::builtin_capability_policy::BuiltinCapabilityPolicy;
use crate::factory::RebornRuntimeStores;
use crate::local_dev_authorization::{
    StoreApprovalSettingsProvider, local_dev_effects_require_approval,
};
use crate::local_dev_mounts::scoped_skill_management_mount_view;
use crate::profile_approval_authorization::ApprovalSettingsProvider;
use crate::{
    projection::{CapabilityDisplayPreviewResult, CapabilityDisplayPreviewStore},
    runtime::ComposedSelectableSkillContextSource,
};

mod external_tool_capability;
mod outbound_delivery;
mod project_create;
mod refreshing_capability_port;
mod result_read;
#[cfg(test)]
mod shell_tests;
mod skill_activation;
mod surface_disclosure;
mod synthetic_capability;

use super::extension_surface::{ExtensionCapabilitySurface, ExtensionCapabilitySurfaceSource};
#[cfg(test)]
pub(crate) use crate::outbound::{
    OUTBOUND_DELIVERY_TARGET_SET_CAPABILITY_ID, OUTBOUND_DELIVERY_TARGETS_LIST_CAPABILITY_ID,
};
#[cfg(any(test, feature = "test-support"))]
pub(crate) use project_create::PROJECT_CREATE_CAPABILITY_ID;
use refreshing_capability_port::{
    RefreshingCapabilityPortConfig, create_refreshing_capability_port,
};
#[cfg(feature = "test-support")]
pub(crate) use result_read::RESULT_READ_CAPABILITY_ID_FOR_TEST;
#[cfg(any(test, feature = "test-support"))]
pub(crate) use skill_activation::SKILL_ACTIVATE_CAPABILITY_ID;

/// Test-only bridge (result_read seam, issue #5838), co-located with the
/// capability it wraps and re-exported here for the `runtime` caller.
#[cfg(feature = "test-support")]
pub(super) use refreshing_capability_port::create_refreshing_capability_port_for_test;
#[cfg(feature = "test-support")]
pub(super) use result_read::wrap_result_read_capability_for_test;

pub(super) struct CapabilityPortWiring {
    pub(super) capability_factory: Arc<dyn LoopCapabilityPortFactory>,
    pub(super) capability_input_resolver: Arc<dyn LoopCapabilityInputResolver>,
    pub(super) capability_result_writer: Arc<dyn LoopCapabilityResultWriter>,
    pub(super) model_gateway: Arc<dyn HostManagedModelGateway>,
    pub(super) display_previews: Arc<CapabilityDisplayPreviewStore>,
}

#[allow(clippy::too_many_arguments)]
pub(super) fn capability_wiring(
    services: &RebornRuntimeStores,
    thread_service: Arc<dyn SessionThreadService>,
    fallback_user_id: UserId,
    policy: Arc<BuiltinCapabilityPolicy>,
    model_gateway: Arc<dyn HostManagedModelGateway>,
    milestone_sink: Arc<dyn LoopHostMilestoneSink>,
    skill_activation_source: Option<Arc<ComposedSelectableSkillContextSource>>,
    outbound_preferences_facade: Option<Arc<dyn OutboundPreferencesProductFacade>>,
    trajectory_observer: Option<Arc<dyn crate::RebornTrajectoryObserver>>,
) -> Option<CapabilityPortWiring> {
    let runtime = services.host_runtime.clone();
    let workspace_mounts = services.workspace_mounts.clone();
    let memory_mounts = services.memory_mounts.clone();
    let system_extensions_lifecycle_mounts = services.system_extensions_lifecycle_mounts.clone();
    let approval_requests: Arc<dyn ApprovalRequestStore> = services.approval_requests.clone();
    let capability_leases: Arc<dyn CapabilityLeaseStore> = services.capability_leases.clone();
    let tool_permission_overrides: Arc<dyn ironclaw_approvals::ToolPermissionOverrideStore> =
        services.tool_permission_overrides.clone();
    let auto_approve_settings: Arc<dyn ironclaw_approvals::AutoApproveSettingStore> =
        services.auto_approve_settings.clone();
    let approval_settings: Arc<dyn ApprovalSettingsProvider> =
        Arc::new(StoreApprovalSettingsProvider::new(
            tool_permission_overrides,
            auto_approve_settings,
            services.persistent_approval_policies.clone(),
        ));
    let outbound_delivery_target_set_requires_approval = local_dev_effects_require_approval(
        services.runtime_policy.as_ref(),
        policy.as_ref(),
        &[EffectKind::ExternalWrite],
    );
    let extension_surface_source = ExtensionCapabilitySurfaceSource::new(Some(
        super::approval_surface_lifecycle_facade(services),
    ));
    // First-class project creation reuses the same access-controlled
    // `ProjectService` facade the WebUI v2 surface wires (composition owns the
    // service, never the raw repository), so an agent-created project is a real
    // entity that appears in the Projects list.
    let project_service: Arc<dyn ProjectService> = Arc::clone(&services.project_service);
    let display_previews = Arc::new(CapabilityDisplayPreviewStore::default());
    let capability_io = Arc::new(
        StagedCapabilityIo::new_with_durable_previews(
            Arc::clone(&display_previews),
            Arc::clone(&thread_service),
            fallback_user_id.clone(),
        )
        .with_observer(trajectory_observer.clone()),
    );
    let capability_input_resolver: Arc<dyn LoopCapabilityInputResolver> = capability_io.clone();
    let capability_result_writer: Arc<dyn LoopCapabilityResultWriter> = capability_io.clone();
    // Shared per-runtime catalog (owned by the composed runtime) so the
    // OpenAI-compatible Responses surface and this loop host see the same
    // run-scoped external-tool state.
    let external_tool_catalog: Arc<dyn ExternalToolCatalog> =
        services.external_tool_catalog.clone();
    // Wire the durable gate-record and host-private replay-payload stores over
    // the composition-owned scoped filesystem (same backend + per-user mount view
    // as every other durable store; `extension_filesystem` is the shared composite
    // root). Before this, the loop-host capability port defaulted both to
    // no-op/fail-closed, so production gate records never persisted (the #6245 gap)
    // and a gate/auth resume had no host-side replay payload to reconstitute
    // {input, estimate} from (arch-simplification §5.3 Stage 2a-i).
    let capability_store_filesystem =
        crate::wrap_scoped(Arc::clone(&services.extension_filesystem));
    let gate_record_store: Arc<dyn ironclaw_run_state::GateRecordStore> =
        Arc::new(ironclaw_run_state::FilesystemGateRecordStore::new(
            Arc::clone(&capability_store_filesystem),
        ));
    let replay_payload_store: Arc<dyn ironclaw_capabilities::ReplayPayloadStore> = Arc::new(
        ironclaw_capabilities::FilesystemReplayPayloadStore::new(capability_store_filesystem),
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
            thread_service,
            trajectory_observer,
            outbound_preferences_facade,
            outbound_delivery_target_set_requires_approval,
            approval_settings,
            approval_requests,
            capability_leases,
            gate_record_store,
            replay_payload_store,
            external_tool_catalog,
        });
    Some(CapabilityPortWiring {
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
    workspace_mounts: MountView,
    memory_mounts: MountView,
    system_extensions_lifecycle_mounts: MountView,
    extension_surface_source: ExtensionCapabilitySurfaceSource,
    input_resolver: Arc<dyn LoopCapabilityInputResolver>,
    result_writer: Arc<dyn LoopCapabilityResultWriter>,
    milestone_sink: Arc<dyn LoopHostMilestoneSink>,
    skill_activation_source: Option<Arc<ComposedSelectableSkillContextSource>>,
    project_service: Arc<dyn ProjectService>,
    thread_service: Arc<dyn SessionThreadService>,
    trajectory_observer: Option<Arc<dyn crate::RebornTrajectoryObserver>>,
    outbound_preferences_facade: Option<Arc<dyn OutboundPreferencesProductFacade>>,
    outbound_delivery_target_set_requires_approval: bool,
    approval_settings: Arc<dyn ApprovalSettingsProvider>,
    approval_requests: Arc<dyn ApprovalRequestStore>,
    capability_leases: Arc<dyn CapabilityLeaseStore>,
    /// Durable model-visible gate-record store; one instance per runtime, shared
    /// by reference into every port this factory builds.
    gate_record_store: Arc<dyn ironclaw_run_state::GateRecordStore>,
    /// Durable host-private replay-payload store (§5.3 Stage 2a-i); one instance
    /// per runtime, shared by reference into every port this factory builds.
    replay_payload_store: Arc<dyn ironclaw_capabilities::ReplayPayloadStore>,
    /// Per-runtime catalog of client-supplied ("external") tools. Shared across
    /// all runs in this runtime so a parked external-tool call and its later
    /// client-submitted output (across a pause/resume) hit the same store.
    external_tool_catalog: Arc<dyn ExternalToolCatalog>,
}

#[async_trait::async_trait]
impl LoopCapabilityPortFactory for RefreshingLoopCapabilityPortFactory {
    async fn create_capability_port(
        &self,
        run_context: &LoopRunContext,
    ) -> Result<Arc<dyn LoopCapabilityPort>, AgentLoopHostError> {
        let skill_mounts = scoped_skill_management_mount_view(&local_dev_resource_scope_for_run(
            run_context,
            &self.fallback_user_id,
        ))
        .map_err(host_api_agent_loop_error)?;
        create_refreshing_capability_port(RefreshingCapabilityPortConfig {
            runtime: Arc::clone(&self.runtime),
            run_context: run_context.clone(),
            fallback_user_id: self.fallback_user_id.clone(),
            policy: Arc::clone(&self.policy),
            workspace_mounts: self.workspace_mounts.clone(),
            skill_mounts,
            memory_mounts: self.memory_mounts.clone(),
            system_extensions_lifecycle_mounts: self.system_extensions_lifecycle_mounts.clone(),
            extension_surface_source: self.extension_surface_source.clone(),
            input_resolver: Arc::clone(&self.input_resolver),
            result_writer: Arc::clone(&self.result_writer),
            milestone_sink: Arc::clone(&self.milestone_sink),
            skill_activation_source: self.skill_activation_source.clone(),
            project_service: Arc::clone(&self.project_service),
            thread_service: Arc::clone(&self.thread_service),
            // Same observer drives both the input hook (on the capability port the
            // refreshing helper builds) and the result hook (on `StagedCapabilityIo`),
            // so the two callbacks correlate by `call_id` for one tool call.
            trajectory_observer: self.trajectory_observer.clone(),
            outbound_preferences_facade: self.outbound_preferences_facade.clone(),
            outbound_delivery_target_set_requires_approval: self
                .outbound_delivery_target_set_requires_approval,
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
            additional_capability_grants: Vec::new(),
        })
        .await
    }
}

const LOCAL_DEV_CAPABILITY_IO_MAX_STAGED_REFS: usize = 1024;
const LOCAL_DEV_CAPABILITY_IO_MAX_STAGED_BYTES: usize = 4 * 1024 * 1024;
const LOCAL_DEV_DURABLE_TOOL_RESULT_MAX_BYTES: usize = 4 * 1024 * 1024;
/// First-look preview bound on the initial result-reference observation.
/// Matches `result_read`'s max chunk size so the preview is exactly the
/// first chunk `result_read` would itself return at `offset: 0` — a model
/// that pages past `next_offset` sees no gap or overlap.
const LOCAL_DEV_RESULT_PREVIEW_MAX_BYTES: usize = TOOL_RESULT_RECORD_READ_MAX_BYTES;

struct StagedCapabilityIo {
    inputs: StdMutex<StagedValueStore>,
    results: StdMutex<StagedValueStore>,
    display_previews: Arc<CapabilityDisplayPreviewStore>,
    durable_previews: Option<DurableCapabilityDisplayPreviewSink>,
    /// Optional consumer hook. This struct drives only the *result* half of the
    /// trajectory observer (via `write_capability_result`); the resolved
    /// tool-call inputs are emitted upstream by `HostRuntimeLoopCapabilityPort`
    /// (the input resolver bypasses this IO for provider tool-call inputs).
    observer: Option<Arc<dyn crate::RebornTrajectoryObserver>>,
}

#[derive(Clone)]
struct DurableCapabilityDisplayPreviewSink {
    thread_service: Arc<dyn SessionThreadService>,
    /// Fallback owner used only when a run scope carries no explicit owner.
    /// The durable thread scope is otherwise derived per-append from the
    /// run context so previews write under the SAME scope the run's thread
    /// was registered under (see `local_dev_thread_scope_for_run`).
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
            observer: None,
        }
    }

    fn new_with_durable_previews(
        display_previews: Arc<CapabilityDisplayPreviewStore>,
        thread_service: Arc<dyn SessionThreadService>,
        fallback_user_id: UserId,
    ) -> Self {
        Self {
            inputs: StdMutex::new(StagedValueStore::default()),
            results: StdMutex::new(StagedValueStore::default()),
            display_previews,
            durable_previews: Some(DurableCapabilityDisplayPreviewSink {
                thread_service,
                fallback_user_id,
            }),
            observer: None,
        }
    }

    /// Attach a trajectory observer (no-op when `None`).
    fn with_observer(mut self, observer: Option<Arc<dyn crate::RebornTrajectoryObserver>>) -> Self {
        self.observer = observer;
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

    async fn persist_tool_result(
        &self,
        run_context: &LoopRunContext,
        result_ref: &LoopResultRef,
        content: Vec<u8>,
    ) -> Result<(), AgentLoopHostError> {
        let Some((durable_previews, scope)) = self.durable_tool_result_scope(run_context)? else {
            return Ok(());
        };
        match durable_previews
            .thread_service
            .put_tool_result_record(ironclaw_threads::PutToolResultRecordRequest {
                scope,
                thread_id: run_context.thread_id.clone(),
                result_ref: result_ref.as_str().to_string(),
                content,
            })
            .await
        {
            Ok(()) => Ok(()),
            Err(error) => Err(durable_result_store_error(error)),
        }
    }

    async fn update_persisted_tool_result(
        &self,
        run_context: &LoopRunContext,
        result_ref: &LoopResultRef,
        content: Vec<u8>,
    ) -> Result<(), AgentLoopHostError> {
        let Some((durable_previews, scope)) = self.durable_tool_result_scope(run_context)? else {
            return Ok(());
        };
        match durable_previews
            .thread_service
            .update_tool_result_record(ironclaw_threads::UpdateToolResultRecordRequest {
                scope,
                thread_id: run_context.thread_id.clone(),
                result_ref: result_ref.as_str().to_string(),
                content,
            })
            .await
        {
            Ok(()) => Ok(()),
            Err(ironclaw_threads::SessionThreadError::UnknownThread { thread_id }) => {
                tracing::debug!(
                    thread_id = %thread_id,
                    result_ref = result_ref.as_str(),
                    "local-dev durable tool result update skipped: thread is unknown"
                );
                Ok(())
            }
            Err(error) => Err(durable_result_store_error(error)),
        }
    }

    fn durable_tool_result_scope(
        &self,
        run_context: &LoopRunContext,
    ) -> Result<Option<(&DurableCapabilityDisplayPreviewSink, ThreadScope)>, AgentLoopHostError>
    {
        let Some(durable_previews) = &self.durable_previews else {
            return Ok(None);
        };
        let scope = local_dev_thread_scope_for_run(run_context, &durable_previews.fallback_user_id)
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
            tracing::warn!(
                "local-dev capability result staging lock failed; using durable result only"
            );
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
            local_dev_thread_scope_for_run(run_context, &durable_previews.fallback_user_id)
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
        )
        .with_observer(observer),
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
        if self.values.len() >= LOCAL_DEV_CAPABILITY_IO_MAX_STAGED_REFS
            || self.total_bytes.saturating_add(bytes) > LOCAL_DEV_CAPABILITY_IO_MAX_STAGED_BYTES
        {
            return Err(AgentLoopHostError::new(
                AgentLoopHostErrorKind::BudgetExceeded,
                "local-dev capability staging is full",
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
        if bytes > LOCAL_DEV_CAPABILITY_IO_MAX_STAGED_BYTES {
            return Err(AgentLoopHostError::new(
                AgentLoopHostErrorKind::BudgetExceeded,
                "local-dev capability result exceeds staging budget",
            ));
        }
        while self.values.len() >= LOCAL_DEV_CAPABILITY_IO_MAX_STAGED_REFS
            || self.total_bytes.saturating_add(bytes) > LOCAL_DEV_CAPABILITY_IO_MAX_STAGED_BYTES
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
                "local_dev_capability_io",
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
        ensure_local_dev_ref_scope("input", input_ref.as_str(), run_context)?;
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
            durable_persistence,
        } = write;
        let result_ref =
            LoopResultRef::new(format!("result:{}.{}", run_context.run_id, Uuid::new_v4()))
                .map_err(|_| {
                    AgentLoopHostError::new(
                        AgentLoopHostErrorKind::Internal,
                        "capability result ref could not be represented",
                    )
                })?;
        let output_content = serialized_result_output(&output)?;
        let item_count = output.as_array().map(|items| items.len() as u64);
        let serialized_bytes = output_content.len();
        let output_bytes = serialized_bytes.try_into().unwrap_or(u64::MAX);
        // Snapshot the first-look preview from the same bytes the durable
        // record stores, before `output_content` is moved into persistence,
        // so its offsets line up exactly with what `result_read` returns.
        let preview = first_look_result_preview(&output_content);
        // See `DurablePersistence` doc comment for the Persist/InlineOnly split.
        if matches!(durable_persistence, DurablePersistence::Persist) {
            self.persist_tool_result(run_context, &result_ref, output_content)
                .await?;
        }
        self.stage_result_best_effort(&result_ref, output.clone(), serialized_bytes);
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
            // is the observer's own contract — see `RebornTrajectoryObserver`.)
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
        let mut write_result =
            CapabilityWriteResult::from_output(result_ref, output_bytes, &output);
        write_result.model_observation = Some(local_dev_result_reference_observation(
            &write_result.result_ref,
            write_result.byte_len,
            preview,
            item_count,
        ));
        Ok(write_result)
    }

    fn record_running_invocation(
        &self,
        _run_context: &LoopRunContext,
        invocation_id: InvocationId,
        input_ref: &CapabilityInputRef,
    ) {
        self.display_previews
            .record_running_invocation(invocation_id, input_ref);
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
    ) -> Result<u64, AgentLoopHostError> {
        ensure_local_dev_ref_scope("result", result_ref.as_str(), run_context)?;
        let content = serialized_result_output(&output)?;
        let bytes = content.len();
        self.update_persisted_tool_result(run_context, result_ref, content)
            .await?;
        self.stage_result_best_effort(result_ref, output, bytes);
        Ok(bytes as u64)
    }

    async fn delete_capability_result(
        &self,
        run_context: &LoopRunContext,
        result_ref: &LoopResultRef,
    ) -> Result<(), AgentLoopHostError> {
        ensure_local_dev_ref_scope("result", result_ref.as_str(), run_context)?;
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
            "local_dev_capability_io",
            "serialize_result",
            AgentLoopHostErrorKind::InvalidInvocation,
            "capability result could not be serialized",
            error,
        )
    })?;
    if content.len() > LOCAL_DEV_DURABLE_TOOL_RESULT_MAX_BYTES {
        return Err(AgentLoopHostError::new(
            AgentLoopHostErrorKind::BudgetExceeded,
            "capability result exceeds the durable storage limit",
        ));
    }
    Ok(content)
}

/// A bounded, UTF-8-safe first-look slice of a serialized result payload,
/// truncated at `LOCAL_DEV_RESULT_PREVIEW_MAX_BYTES`.
struct FirstLookResultPreview {
    text: String,
    /// `None` when `text` already covers the entire payload.
    next_offset: Option<u64>,
}

/// Builds the inline first-look preview from the same serialized bytes the
/// durable record stores, so a truncated preview's `next_offset` matches
/// exactly what `result_read` would return continuing from that offset.
fn first_look_result_preview(serialized: &[u8]) -> Option<FirstLookResultPreview> {
    let Ok(full_text) = std::str::from_utf8(serialized) else {
        return None;
    };
    if full_text.len() <= LOCAL_DEV_RESULT_PREVIEW_MAX_BYTES {
        return Some(FirstLookResultPreview {
            text: full_text.to_string(),
            next_offset: None,
        });
    }
    let end = floor_char_boundary(full_text, LOCAL_DEV_RESULT_PREVIEW_MAX_BYTES);
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

/// Truncated-preview summary text; names the full array's element count when
/// known so the model doesn't misread a truncated array preview as the whole
/// result (issue: byte-slice lands mid-JSON-array).
fn truncated_preview_summary(next_offset: u64, item_count: Option<u64>) -> String {
    let base = format!(
        "Tool completed; preview truncated, use result_read with the result \
         reference and offset {next_offset} for more output."
    );
    match item_count {
        Some(count) => format!("{base} Full result is a JSON array of {count} items."),
        None => base,
    }
}

fn local_dev_result_reference_observation(
    result_ref: &LoopResultRef,
    byte_len: u64,
    preview: Option<FirstLookResultPreview>,
    item_count: Option<u64>,
) -> ModelVisibleToolObservation {
    let (summary, preview_text, total_bytes, next_offset, item_count) = match preview {
        Some(FirstLookResultPreview {
            text,
            next_offset: Some(next_offset),
        }) => (
            truncated_preview_summary(next_offset, item_count),
            Some(text),
            Some(byte_len),
            Some(next_offset),
            item_count,
        ),
        Some(FirstLookResultPreview {
            text,
            next_offset: None,
        }) => (
            "Tool completed; preview contains the full result.".to_string(),
            Some(text),
            Some(byte_len),
            None,
            None,
        ),
        None => (
            "Tool completed; use result_read with the result reference for more output."
                .to_string(),
            None,
            None,
            None,
            None,
        ),
    };
    ModelVisibleToolObservation {
        schema_version: MODEL_VISIBLE_TOOL_OBSERVATION_SCHEMA_VERSION,
        status: ToolObservationStatus::Success,
        summary,
        detail: ToolObservationDetail::ResultReference {
            result_ref: result_ref.as_str().to_string(),
            byte_len,
            preview: preview_text,
            total_bytes,
            next_offset,
            item_count,
        },
        artifacts: vec![ModelVisibleArtifact {
            artifact_ref: result_ref.as_str().to_string(),
            summary: "Stored tool result".to_string(),
        }],
        recovery: None,
        trust: ObservationTrust::UntrustedToolOutput,
    }
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

pub(super) fn local_dev_resource_scope_for_run(
    run_context: &LoopRunContext,
    fallback_user_id: &UserId,
) -> ResourceScope {
    let mut scope = run_context.scope.to_resource_scope();
    scope.user_id = run_context
        .scope
        .explicit_owner_user_id()
        .cloned()
        .or_else(|| run_context.actor().map(|actor| actor.user_id.clone()))
        .unwrap_or_else(|| fallback_user_id.clone());
    scope
}

/// Build the per-run [`ThreadScope`] for durable display-preview appends.
///
/// Seeds the LocalDev fallback owner into the run's tenant/agent/project scope,
/// then delegates owner selection to the canonical resolver. This keeps durable
/// operations on the same per-turn scope as the production loop host. Returns
/// `None` when the run scope carries no agent (durable previews are
/// agent-scoped), in which case the caller skips the durable append.
fn local_dev_thread_scope_for_run(
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
    let user_id = run_context
        .scope
        .explicit_owner_user_id()
        .cloned()
        .or_else(|| run_context.actor().map(|actor| actor.user_id.clone()))
        .unwrap_or_else(|| fallback_user_id.clone());
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
    provider_trust.extend(inputs.extension_surface.provider_trust(&context.user_id));

    Ok(HostVisibleCapabilityRequest::new(
        context,
        SurfaceKind::new("agent_loop").map_err(host_api_agent_loop_error)?,
    )
    .with_policy(CapabilitySurfacePolicy::allow_all())
    .with_provider_trust(provider_trust))
}

fn ensure_local_dev_ref_scope(
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
        "local_dev_host_api",
        "validate_local_dev_runtime_input",
        AgentLoopHostErrorKind::InvalidInvocation,
        safe_summary,
        format!("{error:?}"),
    )
}

/// Shared test assertion for the `local_dev` per-capability submodules: the
/// §5.3 collapse maps a recoverable service failure onto `Resolution::Done`
/// carrying a `RecoverableFailure` verdict (the collapse of the old
/// `CapabilityOutcome::Failed`). Consumed by `outbound_delivery`,
/// `project_create`, and further submodules as they migrate to the `Resolution`
/// shape — replacing the byte-identical per-file copies (CodeRabbit #6299).
#[cfg(test)]
pub(crate) fn assert_recoverable_failure(
    resolution: &ironclaw_host_api::Resolution,
    expected: ironclaw_host_api::FailureKind,
) {
    match resolution {
        ironclaw_host_api::Resolution::Done(outcome) => {
            let expected_verdict = ironclaw_host_api::ToolVerdict::recoverable_failure(expected);
            assert_eq!(outcome.verdict, expected_verdict); // safety: test-only assertion helper
        }
        other => panic!("expected Resolution::Done recoverable failure, got {other:?}"),
    }
}

#[cfg(test)]
mod tests;
