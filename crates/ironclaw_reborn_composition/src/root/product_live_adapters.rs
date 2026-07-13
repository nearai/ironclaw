//! Product-live adapter bundle for planned AgentLoop composition.
//!
//! This module does not cut app or gateway traffic over to Reborn. It provides
//! the explicit adapter bundle the eventual app/gateway entrypoint can pass
//! into `ironclaw_runner::runtime::build_product_live_planned_runtime` once
//! durable thread/checkpoint stores are selected by that caller.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use chrono::Utc;
use thiserror::Error;
use uuid::Uuid;

use ironclaw_host_api::{
    CapabilityId, CapabilitySet, EffectKind, ExecutionContext, ExtensionId, InvocationId,
    MountView, RuntimeKind, TrustClass, UserId,
};
use ironclaw_host_runtime::{
    CapabilitySurfacePolicy, HostRuntime, SurfaceKind, VisibleCapabilityRequest,
};
use ironclaw_loop_support::{
    CapabilityAllowSet, CapabilityResolveError, CapabilityResultWrite,
    CapabilitySurfaceProfileResolver, CapabilityWriteResult, HostIdentityContextSource,
    HostInputQueue, HostRuntimeLoopCapabilityPortFactory, LoopCapabilityInputResolver,
    LoopCapabilityPortFactory, LoopCapabilityResultWriter, RunCancellationFactory,
    loop_driver_execution_extension_id,
};
use ironclaw_runner::model_routes::{
    ModelRoute, ModelRouteError, ModelRoutePolicy, ModelRouteResolver, ModelSelectionMode,
    ModelSlot, StaticModelRouteResolver,
};
use ironclaw_trust::{AuthorityCeiling, EffectiveTrustClass, TrustDecision, TrustProvenance};
use ironclaw_turns::{
    LoopResultRef,
    run_profile::{
        AgentLoopHostError, AgentLoopHostErrorKind, CapabilityInputRef, InstructionSafetyContext,
        LoopCapabilityPort, LoopHostMilestoneSink, LoopModelBudgetAccountant, LoopModelPolicyGuard,
        LoopRunContext, ProviderToolCall,
    },
};

use crate::{
    RebornServices,
    projection::{CapabilityDisplayPreviewResult, CapabilityDisplayPreviewStore},
};

#[derive(Debug, Error)]
pub enum ProductLivePlannedRuntimeAdapterError {
    #[error("product-live planned runtime adapters require a host runtime facade")]
    MissingHostRuntime,
    #[error("product-live model route is invalid: {0}")]
    ModelRoute(#[from] ModelRouteError),
    #[error("product-live capability execution scope is invalid: {reason}")]
    InvalidCapabilityScope { reason: String },
}

/// In-memory capability I/O staging used by the product-live planned runtime adapters.
///
/// Also records bounded display previews for product composition. Local-dev WebUI wires
/// this preview store into projection today; production durable/live fanout still needs
/// an entrypoint-level hook to pass the same store into its projection services.
///
/// Inputs and results are keyed by run-scoped refs so provider tool-call payloads and
/// runtime outputs cannot be read across loop runs. Staged refs are consumed on successful read.
/// Each store is capped at 1024 staged refs and 4 MiB of serialized JSON; callers should still
/// prune entries when a run completes to clear refs that were staged but never consumed.
pub struct ProductLiveCapabilityIo {
    inputs: Mutex<HashMap<String, StagedCapabilityInput>>,
    results: Mutex<HashMap<String, StagedCapabilityResult>>,
    display_previews: Arc<CapabilityDisplayPreviewStore>,
}

impl Default for ProductLiveCapabilityIo {
    fn default() -> Self {
        Self::new(Arc::new(CapabilityDisplayPreviewStore::default()))
    }
}

const PRODUCT_LIVE_CAPABILITY_IO_MAX_STAGED_REFS: usize = 1024;
const PRODUCT_LIVE_CAPABILITY_IO_MAX_STAGED_BYTES: usize = 4 * 1024 * 1024;

#[derive(Clone)]
struct StagedCapabilityInput {
    run_id: String,
    payload: serde_json::Value,
    byte_len: usize,
}

#[derive(Clone)]
struct StagedCapabilityResult {
    run_id: String,
    output: serde_json::Value,
    byte_len: usize,
}

impl ProductLiveCapabilityIo {
    pub(crate) fn new(display_previews: Arc<CapabilityDisplayPreviewStore>) -> Self {
        Self {
            inputs: Mutex::new(HashMap::new()),
            results: Mutex::new(HashMap::new()),
            display_previews,
        }
    }

    /// Stages provider tool-call input for one loop run and returns its run-scoped ref.
    pub fn stage_input(
        &self,
        run_context: &LoopRunContext,
        payload: serde_json::Value,
    ) -> Result<CapabilityInputRef, AgentLoopHostError> {
        let byte_len = serialized_json_len(&payload, "capability input")?;
        let input_ref =
            CapabilityInputRef::new(format!("input:{}:{}", run_context.run_id, Uuid::new_v4()))
                .map_err(|_| {
                    AgentLoopHostError::new(
                        AgentLoopHostErrorKind::Internal,
                        "capability input ref could not be represented",
                    )
                })?;
        let mut inputs = self
            .inputs
            .lock()
            .map_err(|_| capability_io_internal_error())?;
        ensure_staging_capacity(
            "capability input",
            inputs.len(),
            inputs.values().map(|input| input.byte_len).sum(),
            byte_len,
        )?;
        inputs.insert(
            input_ref.as_str().to_string(),
            StagedCapabilityInput {
                run_id: run_context.run_id.to_string(),
                payload,
                byte_len,
            },
        );
        Ok(input_ref)
    }

    /// Returns and consumes a staged capability result after verifying the ref belongs to the run.
    pub fn result_for_ref(
        &self,
        run_context: &LoopRunContext,
        result_ref: &LoopResultRef,
    ) -> Result<serde_json::Value, AgentLoopHostError> {
        ensure_ref_scoped_to_run("result", result_ref.as_str(), run_context)?;
        let mut results = self
            .results
            .lock()
            .map_err(|_| capability_io_internal_error())?;
        let result = results.remove(result_ref.as_str()).ok_or_else(|| {
            AgentLoopHostError::new(
                AgentLoopHostErrorKind::InvalidInvocation,
                "capability result ref was not staged for this loop run",
            )
        })?;
        if result.run_id != run_context.run_id.to_string() {
            return Err(cross_run_ref_error("capability result ref"));
        }
        Ok(result.output.clone())
    }

    /// Drops all staged inputs and results for the supplied loop run.
    pub fn prune_run(&self, run_context: &LoopRunContext) -> Result<(), AgentLoopHostError> {
        self.prune_run_id(&run_context.run_id.to_string())
    }

    /// Drops all staged inputs and results whose stored run id matches `run_id`.
    pub fn prune_run_id(&self, run_id: &str) -> Result<(), AgentLoopHostError> {
        self.inputs
            .lock()
            .map_err(|_| capability_io_internal_error())?
            .retain(|_, input| input.run_id != run_id);
        self.results
            .lock()
            .map_err(|_| capability_io_internal_error())?
            .retain(|_, result| result.run_id != run_id);
        self.display_previews.prune_run(run_id);
        Ok(())
    }
}

#[async_trait]
impl LoopCapabilityInputResolver for ProductLiveCapabilityIo {
    async fn resolve_capability_input(
        &self,
        run_context: &LoopRunContext,
        input_ref: &CapabilityInputRef,
    ) -> Result<serde_json::Value, AgentLoopHostError> {
        ensure_ref_scoped_to_run("input", input_ref.as_str(), run_context)?;
        let mut inputs = self
            .inputs
            .lock()
            .map_err(|_| capability_io_internal_error())?;
        let input = inputs.remove(input_ref.as_str()).ok_or_else(|| {
            AgentLoopHostError::new(
                AgentLoopHostErrorKind::InvalidInvocation,
                "capability input ref was not staged for this loop run",
            )
        })?;
        if input.run_id != run_context.run_id.to_string() {
            return Err(cross_run_ref_error("capability input ref"));
        }
        Ok(input.payload.clone())
    }

    async fn register_provider_tool_call_input(
        &self,
        run_context: &LoopRunContext,
        tool_call: &ProviderToolCall,
    ) -> Result<CapabilityInputRef, AgentLoopHostError> {
        let input_ref = self.stage_input(run_context, tool_call.arguments.clone())?;
        // Records under this staging ref for direct callers. In the production
        // loop the resolver is wrapped by `ProviderToolCallInputResolver`, which
        // owns the canonical (digest) ref and bypasses this method — that path
        // records via `record_provider_tool_call_display_input` below instead.
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

#[async_trait]
impl LoopCapabilityResultWriter for ProductLiveCapabilityIo {
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
            // `ProductLiveCapabilityIo` is an ephemeral in-memory test fixture
            // (see crate CLAUDE.md / #5902) that never durably persists a
            // result, so the durable-vs-inline distinction does not apply here.
            durable_persistence: _,
        } = write;
        let byte_len = serialized_json_len(&output, "capability result")?;
        let result_ref =
            LoopResultRef::new(format!("result:{}.{}", run_context.run_id, Uuid::new_v4()))
                .map_err(|_| {
                    AgentLoopHostError::new(
                        AgentLoopHostErrorKind::Internal,
                        "capability result ref could not be represented",
                    )
                })?;
        let mut results = self
            .results
            .lock()
            .map_err(|_| capability_io_internal_error())?;
        ensure_staging_capacity(
            "capability result",
            results.len(),
            results.values().map(|result| result.byte_len).sum(),
            byte_len,
        )?;
        results.insert(
            result_ref.as_str().to_string(),
            StagedCapabilityResult {
                run_id: run_context.run_id.to_string(),
                output: output.clone(),
                byte_len,
            },
        );
        self.display_previews.record_result_with_preview(
            CapabilityDisplayPreviewResult {
                run_id: &run_context.run_id.to_string(),
                input_ref,
                invocation_id,
                capability_id,
                result_ref: result_ref.as_str(),
                output: &output,
                output_bytes: byte_len.try_into().unwrap_or(u64::MAX),
            },
            display_preview.as_ref(),
        );
        Ok(CapabilityWriteResult::from_output(
            result_ref,
            byte_len as u64,
            &output,
        ))
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
        // In-memory only, matching this writer's success path
        // (`write_capability_result` does not persist previews durably here).
        self.display_previews.record_failure_preview(
            &run_context.run_id.to_string(),
            invocation_id,
            capability_id,
            summary,
        );
    }

    async fn update_capability_result(
        &self,
        run_context: &LoopRunContext,
        result_ref: &LoopResultRef,
        output: serde_json::Value,
    ) -> Result<u64, AgentLoopHostError> {
        ensure_ref_scoped_to_run("result", result_ref.as_str(), run_context)?;
        let byte_len = serialized_json_len(&output, "capability result")?;
        let mut results = self
            .results
            .lock()
            .map_err(|_| capability_io_internal_error())?;
        let previous_byte_len = if let Some(previous) = results.get(result_ref.as_str()) {
            if previous.run_id != run_context.run_id.to_string() {
                return Err(cross_run_ref_error("capability result ref"));
            }
            previous.byte_len
        } else {
            0
        };
        if previous_byte_len == 0 && results.len() >= PRODUCT_LIVE_CAPABILITY_IO_MAX_STAGED_REFS {
            return Err(AgentLoopHostError::new(
                AgentLoopHostErrorKind::BudgetExceeded,
                format!(
                    "capability result staging exceeds {} staged refs",
                    PRODUCT_LIVE_CAPABILITY_IO_MAX_STAGED_REFS
                ),
            ));
        }
        if !result_ref
            .as_str()
            .starts_with(&format!("result:{}.", run_context.run_id))
        {
            return Err(cross_run_ref_error("capability result ref"));
        }
        let total_without_previous = results
            .values()
            .map(|result| result.byte_len)
            .sum::<usize>()
            .saturating_sub(previous_byte_len);
        ensure_staging_capacity(
            "capability result",
            results
                .len()
                .saturating_sub(usize::from(previous_byte_len > 0)),
            total_without_previous,
            byte_len,
        )?;
        results.insert(
            result_ref.as_str().to_string(),
            StagedCapabilityResult {
                run_id: run_context.run_id.to_string(),
                output,
                byte_len,
            },
        );
        Ok(byte_len as u64)
    }

    async fn delete_capability_result(
        &self,
        run_context: &LoopRunContext,
        result_ref: &LoopResultRef,
    ) -> Result<(), AgentLoopHostError> {
        ensure_ref_scoped_to_run("result", result_ref.as_str(), run_context)?;
        let mut results = self
            .results
            .lock()
            .map_err(|_| capability_io_internal_error())?;
        if let Some(previous) = results.get(result_ref.as_str())
            && previous.run_id != run_context.run_id.to_string()
        {
            return Err(cross_run_ref_error("capability result ref"));
        }
        results.remove(result_ref.as_str());
        Ok(())
    }
}

fn serialized_json_len(
    value: &serde_json::Value,
    label: &'static str,
) -> Result<usize, AgentLoopHostError> {
    serde_json::to_vec(value)
        .map(|bytes| bytes.len())
        .map_err(|error| {
            AgentLoopHostError::new(
                AgentLoopHostErrorKind::InvalidInvocation,
                format!("{label} could not be serialized: {error}"),
            )
        })
}

fn ensure_staging_capacity(
    label: &'static str,
    current_entries: usize,
    current_bytes: usize,
    new_bytes: usize,
) -> Result<(), AgentLoopHostError> {
    if current_entries >= PRODUCT_LIVE_CAPABILITY_IO_MAX_STAGED_REFS {
        return Err(AgentLoopHostError::new(
            AgentLoopHostErrorKind::BudgetExceeded,
            format!(
                "{label} staging exceeds {} staged refs",
                PRODUCT_LIVE_CAPABILITY_IO_MAX_STAGED_REFS
            ),
        ));
    }
    let Some(total_bytes) = current_bytes.checked_add(new_bytes) else {
        return Err(capability_io_capacity_error(label));
    };
    if total_bytes > PRODUCT_LIVE_CAPABILITY_IO_MAX_STAGED_BYTES {
        return Err(capability_io_capacity_error(label));
    }
    Ok(())
}

fn capability_io_capacity_error(label: &'static str) -> AgentLoopHostError {
    AgentLoopHostError::new(
        AgentLoopHostErrorKind::BudgetExceeded,
        format!(
            "{label} staging exceeds {} serialized bytes",
            PRODUCT_LIVE_CAPABILITY_IO_MAX_STAGED_BYTES
        ),
    )
}

fn ensure_ref_scoped_to_run(
    prefix: &str,
    reference: &str,
    run_context: &LoopRunContext,
) -> Result<(), AgentLoopHostError> {
    let separator = if prefix == "result" { "." } else { ":" };
    let expected_prefix = format!("{prefix}:{}{separator}", run_context.run_id);
    if reference.starts_with(&expected_prefix) {
        Ok(())
    } else {
        Err(cross_run_ref_error(match prefix {
            "input" => "capability input ref",
            "result" => "capability result ref",
            _ => "capability ref",
        }))
    }
}

fn cross_run_ref_error(ref_name: &'static str) -> AgentLoopHostError {
    AgentLoopHostError::new(
        AgentLoopHostErrorKind::ScopeMismatch,
        format!("{ref_name} is not scoped to this loop run"),
    )
}

fn capability_io_internal_error() -> AgentLoopHostError {
    AgentLoopHostError::new(
        AgentLoopHostErrorKind::Internal,
        "capability io store is unavailable",
    )
}

/// Configuration used to build the visible capability request for a product-live loop run.
///
/// The request context is scoped to the run and intentionally starts with no caller-supplied
/// mounts. Execution mounts are passed separately by the authority resolver and applied only
/// when invoking a selected capability.
#[derive(Clone)]
pub struct ProductLiveVisibleCapabilityRequestConfig {
    user_id: UserId,
    runtime: RuntimeKind,
    trust: TrustClass,
    grants: CapabilitySet,
    mounts: MountView,
    surface_kind: SurfaceKind,
    policy: CapabilitySurfacePolicy,
    provider_trust: BTreeMap<ExtensionId, TrustDecision>,
}

impl ProductLiveVisibleCapabilityRequestConfig {
    /// Creates base visible-request config for the user, runtime, trust, surface, and policy.
    pub fn new(
        user_id: UserId,
        runtime: RuntimeKind,
        trust: TrustClass,
        surface_kind: SurfaceKind,
        policy: CapabilitySurfacePolicy,
    ) -> Self {
        Self {
            user_id,
            runtime,
            trust,
            grants: CapabilitySet::default(),
            mounts: MountView::default(),
            surface_kind,
            policy,
            provider_trust: BTreeMap::new(),
        }
    }

    /// Replaces capability grants made visible to the loop driver.
    pub fn with_grants(mut self, grants: CapabilitySet) -> Self {
        self.grants = grants;
        self
    }

    /// Stores host-authorized execution mounts to pass to capability invocations.
    pub fn with_mounts(mut self, mounts: MountView) -> Self {
        self.mounts = mounts;
        self
    }

    /// Grants one provider dispatch authority using the default dispatch-capability effect.
    pub fn with_provider_trust(
        mut self,
        provider: ExtensionId,
        effective_trust: EffectiveTrustClass,
    ) -> Self {
        self = self.with_provider_trust_for_effects(
            provider,
            effective_trust,
            vec![EffectKind::DispatchCapability],
        );
        self
    }

    /// Grants one provider dispatch authority constrained to the supplied allowed effects.
    pub fn with_provider_trust_for_effects(
        mut self,
        provider: ExtensionId,
        effective_trust: EffectiveTrustClass,
        allowed_effects: Vec<EffectKind>,
    ) -> Self {
        self.provider_trust.insert(
            provider,
            TrustDecision {
                effective_trust,
                authority_ceiling: AuthorityCeiling {
                    allowed_effects,
                    max_resource_ceiling: None,
                },
                provenance: TrustProvenance::AdminConfig,
                evaluated_at: Utc::now(),
            },
        );
        self
    }

    /// Inserts a precomputed trust decision for one capability provider.
    pub fn with_provider_trust_decision(
        mut self,
        provider: ExtensionId,
        trust_decision: TrustDecision,
    ) -> Self {
        self.provider_trust.insert(provider, trust_decision);
        self
    }
}

/// Builds the host-runtime visible capability request for one loop run.
///
/// The loop driver id is converted into the execution extension id, run scope fields are copied
/// into both context and resource scope, and invalid loop-driver ids or execution scopes are
/// reported as `InvalidCapabilityScope` errors.
pub fn visible_capability_request_for_run(
    run_context: &LoopRunContext,
    config: ProductLiveVisibleCapabilityRequestConfig,
) -> Result<VisibleCapabilityRequest, ProductLivePlannedRuntimeAdapterError> {
    let extension_id = loop_driver_execution_extension_id(run_context).map_err(|error| {
        ProductLivePlannedRuntimeAdapterError::InvalidCapabilityScope {
            reason: error.to_string(),
        }
    })?;
    let mut context = ExecutionContext::local_default(
        config.user_id,
        extension_id,
        config.runtime,
        config.trust,
        config.grants,
        MountView::default(),
    )
    .map_err(
        |error| ProductLivePlannedRuntimeAdapterError::InvalidCapabilityScope {
            reason: error.to_string(),
        },
    )?;
    context.tenant_id = run_context.scope.tenant_id.clone();
    context.agent_id = run_context.scope.agent_id.clone();
    context.project_id = run_context.scope.project_id.clone();
    context.thread_id = Some(run_context.thread_id.clone());
    context.resource_scope.tenant_id = context.tenant_id.clone();
    context.resource_scope.agent_id = context.agent_id.clone();
    context.resource_scope.project_id = context.project_id.clone();
    context.resource_scope.thread_id = context.thread_id.clone();
    context.validate().map_err(|error| {
        ProductLivePlannedRuntimeAdapterError::InvalidCapabilityScope {
            reason: error.to_string(),
        }
    })?;
    Ok(VisibleCapabilityRequest::new(context, config.surface_kind)
        .with_policy(config.policy)
        .with_provider_trust(config.provider_trust))
}

/// Resolves run-scoped capability authority for the product-live loop driver.
#[async_trait]
pub trait ProductLiveCapabilityAuthorityResolver: Send + Sync {
    /// Returns visible capability request config and execution mounts for one loop run.
    async fn resolve_capability_authority(
        &self,
        run_context: &LoopRunContext,
    ) -> Result<ProductLiveVisibleCapabilityRequestConfig, ProductLivePlannedRuntimeAdapterError>;
}

/// Model route configuration for the product-live planned runtime.
///
/// The default route is always approved and used for the default slot; an optional mission route
/// can be approved and bound to the mission slot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProductLiveModelRouteSettings {
    selection_mode: ModelSelectionMode,
    default_route: ModelRoute,
    mission_route: Option<ModelRoute>,
}

impl ProductLiveModelRouteSettings {
    /// Creates managed-only model routing with one approved default route.
    pub fn new(
        provider_id: impl Into<String>,
        model_id: impl Into<String>,
    ) -> Result<Self, ModelRouteError> {
        Ok(Self {
            selection_mode: ModelSelectionMode::ManagedOnly,
            default_route: ModelRoute::new(provider_id, model_id)?,
            mission_route: None,
        })
    }

    /// Replaces model-selection mode for the generated static route policy.
    pub fn with_selection_mode(mut self, selection_mode: ModelSelectionMode) -> Self {
        self.selection_mode = selection_mode;
        self
    }

    /// Adds an approved mission-slot route.
    pub fn with_mission_route(
        mut self,
        provider_id: impl Into<String>,
        model_id: impl Into<String>,
    ) -> Result<Self, ModelRouteError> {
        self.mission_route = Some(ModelRoute::new(provider_id, model_id)?);
        Ok(self)
    }

    fn into_resolver(self) -> StaticModelRouteResolver {
        let mut policy = ModelRoutePolicy::new(self.selection_mode)
            .with_approved_route(self.default_route.clone());
        if let Some(route) = self.mission_route.clone() {
            policy = policy.with_approved_route(route);
        }

        let mut resolver = StaticModelRouteResolver::new(policy)
            .with_route(ModelSlot::Default, self.default_route);
        if let Some(route) = self.mission_route {
            resolver = resolver.with_route(ModelSlot::Mission, route);
        }
        resolver
    }
}

/// Adapter dependencies required to assemble the product-live planned runtime.
pub struct ProductLivePlannedRuntimeAdapterConfig {
    /// Resolves run-scoped capability authority and execution mounts.
    pub capability_authority_resolver: Arc<dyn ProductLiveCapabilityAuthorityResolver>,
    /// Resolves staged capability inputs from loop refs.
    pub capability_input_resolver: Arc<dyn LoopCapabilityInputResolver>,
    /// Persists capability outputs and returns loop result refs.
    pub capability_result_writer: Arc<dyn LoopCapabilityResultWriter>,
    /// Static allow-set exposed by the capability surface resolver.
    pub capability_allow_set: CapabilityAllowSet,
    /// Static model routing config for default and optional mission slots.
    pub model_routes: ProductLiveModelRouteSettings,
    /// Factory used to create cancellation handles for loop runs.
    pub cancellation_factory: Arc<dyn RunCancellationFactory>,
    /// Host input queue used by the planned runtime.
    pub input_queue: Arc<dyn HostInputQueue>,
    /// Source for host identity context.
    pub identity_context_source: Arc<dyn HostIdentityContextSource>,
    /// Policy guard for loop model use.
    pub model_policy_guard: Arc<dyn LoopModelPolicyGuard>,
    /// Budget accountant for loop model use.
    pub model_budget_accountant: Arc<dyn LoopModelBudgetAccountant>,
    /// Instruction-safety context passed into the planned runtime.
    pub safety_context: InstructionSafetyContext,
    /// Sink for capability invocation milestones.
    pub milestone_sink: Arc<dyn LoopHostMilestoneSink>,
}

/// Adapter bundle consumed by `build_product_live_planned_runtime`.
#[derive(Clone)]
pub struct ProductLivePlannedRuntimeAdapters {
    /// Capability port factory backed by the host runtime facade.
    pub capability_factory: Arc<dyn LoopCapabilityPortFactory>,
    /// Capability input resolver shared with synthetic host capabilities.
    pub capability_input_resolver: Arc<dyn LoopCapabilityInputResolver>,
    /// Capability result writer shared with runtime completion observers.
    pub capability_result_writer: Arc<dyn LoopCapabilityResultWriter>,
    /// Capability surface resolver exposing the configured allow-set.
    pub capability_surface_resolver: Arc<dyn CapabilitySurfaceProfileResolver>,
    /// Model route resolver generated from product-live route settings.
    pub model_route_resolver: Arc<dyn ModelRouteResolver>,
    /// Factory used to create cancellation handles for loop runs.
    pub cancellation_factory: Arc<dyn RunCancellationFactory>,
    /// Host input queue used by the planned runtime.
    pub input_queue: Arc<dyn HostInputQueue>,
    /// Source for host identity context.
    pub identity_context_source: Arc<dyn HostIdentityContextSource>,
    /// Policy guard for loop model use.
    pub model_policy_guard: Arc<dyn LoopModelPolicyGuard>,
    /// Budget accountant for loop model use.
    pub model_budget_accountant: Arc<dyn LoopModelBudgetAccountant>,
    /// Instruction-safety context passed into the planned runtime.
    pub safety_context: InstructionSafetyContext,
}

impl ProductLivePlannedRuntimeAdapters {
    /// Builds the adapter bundle from `RebornServices` and explicit product-live dependencies.
    ///
    /// Returns `MissingHostRuntime` when the service graph has no host runtime facade.
    pub fn from_services(
        services: &RebornServices,
        config: ProductLivePlannedRuntimeAdapterConfig,
    ) -> Result<Self, ProductLivePlannedRuntimeAdapterError> {
        let host_runtime = services
            .host_runtime
            .clone()
            .ok_or(ProductLivePlannedRuntimeAdapterError::MissingHostRuntime)?;

        let capability_factory = ProductLiveLoopCapabilityPortFactory::new(
            host_runtime,
            config.capability_authority_resolver,
            Arc::clone(&config.capability_input_resolver),
            Arc::clone(&config.capability_result_writer),
            config.milestone_sink,
        );
        let model_route_resolver: Arc<dyn ModelRouteResolver> =
            Arc::new(config.model_routes.into_resolver());

        Ok(Self {
            capability_factory: Arc::new(capability_factory),
            capability_input_resolver: config.capability_input_resolver,
            capability_result_writer: config.capability_result_writer,
            capability_surface_resolver: Arc::new(StaticCapabilitySurfaceResolver::new(
                config.capability_allow_set,
            )),
            model_route_resolver,
            cancellation_factory: config.cancellation_factory,
            input_queue: config.input_queue,
            identity_context_source: config.identity_context_source,
            model_policy_guard: config.model_policy_guard,
            model_budget_accountant: config.model_budget_accountant,
            safety_context: config.safety_context,
        })
    }
}

#[derive(Clone)]
struct ProductLiveLoopCapabilityPortFactory {
    runtime: Arc<dyn HostRuntime>,
    authority_resolver: Arc<dyn ProductLiveCapabilityAuthorityResolver>,
    input_resolver: Arc<dyn LoopCapabilityInputResolver>,
    result_writer: Arc<dyn LoopCapabilityResultWriter>,
    milestone_sink: Arc<dyn LoopHostMilestoneSink>,
}

impl ProductLiveLoopCapabilityPortFactory {
    fn new(
        runtime: Arc<dyn HostRuntime>,
        authority_resolver: Arc<dyn ProductLiveCapabilityAuthorityResolver>,
        input_resolver: Arc<dyn LoopCapabilityInputResolver>,
        result_writer: Arc<dyn LoopCapabilityResultWriter>,
        milestone_sink: Arc<dyn LoopHostMilestoneSink>,
    ) -> Self {
        Self {
            runtime,
            authority_resolver,
            input_resolver,
            result_writer,
            milestone_sink,
        }
    }
}

#[async_trait]
impl LoopCapabilityPortFactory for ProductLiveLoopCapabilityPortFactory {
    async fn create_capability_port(
        &self,
        run_context: &LoopRunContext,
    ) -> Result<Arc<dyn LoopCapabilityPort>, AgentLoopHostError> {
        let authority = self
            .authority_resolver
            .resolve_capability_authority(run_context)
            .await
            .map_err(adapter_error)?;
        let execution_mounts = authority.mounts.clone();
        let visible_request =
            visible_capability_request_for_run(run_context, authority).map_err(adapter_error)?;
        let factory = HostRuntimeLoopCapabilityPortFactory::new(
            Arc::clone(&self.runtime),
            visible_request,
            Arc::clone(&self.input_resolver),
            Arc::clone(&self.result_writer),
            Arc::clone(&self.milestone_sink),
        )
        .with_execution_mounts(execution_mounts);
        Ok(factory.for_run_context(run_context.clone()))
    }
}

fn adapter_error(error: ProductLivePlannedRuntimeAdapterError) -> AgentLoopHostError {
    let safe_summary = error.to_string();
    ironclaw_loop_support::raw_agent_loop_host_error(
        "product_live_planned_runtime_adapter",
        "build_capability_port",
        AgentLoopHostErrorKind::InvalidInvocation,
        safe_summary,
        error,
    )
}

struct StaticCapabilitySurfaceResolver {
    allow_set: CapabilityAllowSet,
}

impl StaticCapabilitySurfaceResolver {
    fn new(allow_set: CapabilityAllowSet) -> Self {
        Self { allow_set }
    }
}

#[async_trait]
impl CapabilitySurfaceProfileResolver for StaticCapabilitySurfaceResolver {
    async fn resolve(
        &self,
        _run_context: &LoopRunContext,
    ) -> Result<CapabilityAllowSet, CapabilityResolveError> {
        Ok(self.allow_set.clone())
    }
}

/// Convenience constructor for a capability allow-set.
pub fn capability_allowlist(ids: impl IntoIterator<Item = CapabilityId>) -> CapabilityAllowSet {
    CapabilityAllowSet::allowlist(ids)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironclaw_host_api::{
        AgentId, CapabilityDisplayOutputPreview, InvocationId, ProviderToolName, TenantId, ThreadId,
    };
    use ironclaw_loop_support::DurablePersistence;
    use ironclaw_runner::planned_driver_factory::default_planned_run_profile_resolver;
    use ironclaw_turns::{
        RunProfileResolutionRequest, RunProfileResolver, TurnId, TurnRunId, TurnScope,
    };

    #[tokio::test]
    async fn capability_io_records_read_file_display_preview() {
        let io = ProductLiveCapabilityIo::default();
        let run_context = loop_run_context().await;
        let tool_call = ProviderToolCall {
            provider_id: "provider".to_string(),
            provider_model_id: "model".to_string(),
            turn_id: Some("turn_1".to_string()),
            id: "call_1".to_string(),
            name: ProviderToolName::new("read_file").expect("provider tool name"),
            arguments: serde_json::json!({"path": "src/main.rs", "api_key": "sk-secret"}),
            response_reasoning: None,
            reasoning: None,
            signature: None,
        };
        let input_ref = io
            .register_provider_tool_call_input(&run_context, &tool_call)
            .await
            .expect("input staged");
        let invocation_id = InvocationId::new();
        let capability_id = CapabilityId::new("builtin.read_file").unwrap();
        io.write_capability_result(CapabilityResultWrite {
            run_context: &run_context,
            input_ref: &input_ref,
            invocation_id,
            capability_id: &capability_id,
            output: serde_json::json!({"content": "fn main() {}"}),
            display_preview: None,
            durable_persistence: DurablePersistence::Persist,
        })
        .await
        .map(|_| ())
        .expect("result staged");

        let record = io
            .display_previews
            .record_for_invocation(invocation_id)
            .expect("preview recorded");
        assert_eq!(record.title, "read_file");
        assert_eq!(record.subtitle.as_deref(), Some("src/main.rs"));
        assert!(
            record
                .input_summary
                .as_deref()
                .is_some_and(|summary| summary.contains("path: src/main.rs"))
        );
        assert_eq!(record.output_preview.as_deref(), Some("fn main() {}"));
        assert_eq!(record.output_kind.as_deref(), Some("text"));
        let rendered = serde_json::to_string(&record.input_summary).unwrap();
        assert!(!rendered.contains("sk-secret"));
    }

    /// Regression (#activity-card-args): in production the loop wraps this
    /// resolver with `ProviderToolCallInputResolver`, which owns the canonical
    /// (digest) ref and bypasses `register_provider_tool_call_input` — it drives
    /// the display-preview recording via `record_provider_tool_call_display_input`
    /// instead. Recording under that canonical ref must still surface
    /// `input_summary` when the result is written under the same ref; otherwise
    /// the activity card shows the tool with no arguments.
    #[tokio::test]
    async fn capability_io_records_display_input_via_provider_tool_call_hook() {
        let io = ProductLiveCapabilityIo::default();
        let run_context = loop_run_context().await;
        // The model-facing provider tool name is the lossy `__` encoding; the
        // resolved capability id is the dotted form. The display must key off
        // the capability id so the card title and per-tool summary are correct.
        let tool_call = ProviderToolCall {
            provider_id: "provider".to_string(),
            provider_model_id: "model".to_string(),
            turn_id: Some("turn_1".to_string()),
            id: "call_1".to_string(),
            name: ProviderToolName::new("nearai__web_search").expect("provider tool name"),
            arguments: serde_json::json!({"query": "deploy status"}),
            response_reasoning: None,
            reasoning: None,
            signature: None,
        };
        let capability_id = CapabilityId::new("nearai.web_search").unwrap();
        // The decorator owns this ref; it is NOT produced by
        // `register_provider_tool_call_input` here.
        let input_ref = CapabilityInputRef::new(format!(
            "input:provider-tool-call:{}:digest",
            run_context.run_id
        ))
        .expect("valid ref");

        io.record_provider_tool_call_display_input(
            &run_context,
            &input_ref,
            &capability_id,
            &tool_call,
        );

        let invocation_id = InvocationId::new();
        io.write_capability_result(CapabilityResultWrite {
            run_context: &run_context,
            input_ref: &input_ref,
            invocation_id,
            capability_id: &capability_id,
            output: serde_json::json!({"results": []}),
            display_preview: None,
            durable_persistence: DurablePersistence::Persist,
        })
        .await
        .map(|_| ())
        .expect("result staged");

        let record = io
            .display_previews
            .record_for_invocation(invocation_id)
            .expect("preview recorded");
        // Title is the dotted capability id, not the `__` provider tool name.
        assert_eq!(record.title, "nearai.web_search");
        // The per-tool web_search matcher fires (query summarized), rather than
        // falling back to a raw JSON dump.
        assert!(
            record
                .input_summary
                .as_deref()
                .is_some_and(|summary| summary.contains("query: deploy status")),
            "input summary should use the web_search matcher, got {:?}",
            record.input_summary,
        );
    }

    #[tokio::test]
    async fn capability_io_records_display_preview_side_channel() {
        let io = ProductLiveCapabilityIo::default();
        let run_context = loop_run_context().await;
        let input_ref = io
            .stage_input(
                &run_context,
                serde_json::json!({"path": "/workspace/main.rs"}),
            )
            .expect("input staged");
        let invocation_id = InvocationId::new();
        let capability_id = CapabilityId::new("builtin.write_file").unwrap();
        io.write_capability_result(CapabilityResultWrite {
            run_context: &run_context,
            input_ref: &input_ref,
            invocation_id,
            capability_id: &capability_id,
            output: serde_json::json!({"success": true}),
            display_preview: Some(CapabilityDisplayOutputPreview {
                output_summary: Some("Edited 1 file: +1/-1".to_string()),
                output_preview:
                    "--- a/workspace/main.rs\n+++ b/workspace/main.rs\n@@ -1,1 +1,1 @@\n-old\n+new\n"
                        .to_string(),
                output_kind: "unified_diff".to_string(),
                subtitle: Some("/workspace/main.rs".to_string()),
                truncated: false,
            }),
            durable_persistence: DurablePersistence::Persist,
        })
        .await
        .map(|_| ())
        .expect("result staged");
        let record = io
            .display_previews
            .record_for_invocation(invocation_id)
            .expect("preview recorded");
        assert_eq!(record.output_kind.as_deref(), Some("unified_diff"));
        assert_eq!(
            record.output_summary.as_deref(),
            Some("Edited 1 file: +1/-1")
        );
        assert!(
            record
                .output_preview
                .as_deref()
                .is_some_and(|preview| preview.contains("+new"))
        );
        assert_eq!(record.subtitle.as_deref(), Some("/workspace/main.rs"));
    }

    #[tokio::test]
    async fn capability_io_prunes_display_preview_with_run() {
        let io = ProductLiveCapabilityIo::default();
        let run_context = loop_run_context().await;
        let input_ref = io
            .stage_input(&run_context, serde_json::json!({"text": "ok"}))
            .expect("input staged");
        let invocation_id = InvocationId::new();
        let capability_id = CapabilityId::new("demo.echo").unwrap();
        io.write_capability_result(CapabilityResultWrite {
            run_context: &run_context,
            input_ref: &input_ref,
            invocation_id,
            capability_id: &capability_id,
            output: serde_json::json!({"reply": "ok"}),
            display_preview: None,
            durable_persistence: DurablePersistence::Persist,
        })
        .await
        .map(|_| ())
        .expect("result staged");
        assert!(
            io.display_previews
                .record_for_invocation(invocation_id)
                .is_some()
        );

        io.prune_run(&run_context).expect("run pruned");
        assert!(
            io.display_previews
                .record_for_invocation(invocation_id)
                .is_none()
        );
    }

    async fn loop_run_context() -> LoopRunContext {
        let resolved = default_planned_run_profile_resolver()
            .unwrap()
            .resolve_run_profile(RunProfileResolutionRequest::interactive_default())
            .await
            .unwrap();
        LoopRunContext::new(
            TurnScope::new(
                TenantId::new("preview-tenant").unwrap(),
                Some(AgentId::new("preview-agent").unwrap()),
                None,
                ThreadId::new("preview-thread").unwrap(),
            ),
            TurnId::new(),
            TurnRunId::new(),
            resolved,
        )
    }
}
