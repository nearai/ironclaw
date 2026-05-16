//! Product-live adapter bundle for planned AgentLoop composition.
//!
//! This module does not cut app or gateway traffic over to Reborn. It provides
//! the explicit adapter bundle the eventual app/gateway entrypoint can pass
//! into `ironclaw_reborn::runtime::build_product_live_planned_runtime` once
//! durable thread/checkpoint stores are selected by that caller.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use chrono::Utc;
use thiserror::Error;
use uuid::Uuid;

use ironclaw_host_api::{
    CapabilityId, CapabilitySet, EffectKind, ExecutionContext, ExtensionId, MountView, RuntimeKind,
    TrustClass, UserId,
};
use ironclaw_host_runtime::{CapabilitySurfacePolicy, SurfaceKind, VisibleCapabilityRequest};
use ironclaw_loop_support::{
    CapabilityAllowSet, CapabilityResolveError, CapabilitySurfaceProfileResolver,
    HostIdentityContextSource, HostInputQueue, HostRuntimeLoopCapabilityPortFactory,
    LoopCapabilityInputResolver, LoopCapabilityResultWriter, RunCancellationFactory,
};
use ironclaw_reborn::{
    loop_driver_host::LoopCapabilityPortFactory,
    model_routes::{
        ModelRoute, ModelRouteError, ModelRoutePolicy, ModelRouteResolver, ModelSelectionMode,
        ModelSlot, StaticModelRouteResolver,
    },
};
use ironclaw_trust::{AuthorityCeiling, EffectiveTrustClass, TrustDecision, TrustProvenance};
use ironclaw_turns::{
    LoopResultRef,
    run_profile::{
        AgentLoopHostError, AgentLoopHostErrorKind, CapabilityInputRef, InstructionSafetyContext,
        LoopHostMilestoneSink, LoopModelBudgetAccountant, LoopModelPolicyGuard, LoopRunContext,
    },
};

use crate::RebornServices;

#[derive(Debug, Error)]
pub enum ProductLivePlannedRuntimeAdapterError {
    #[error("product-live planned runtime adapters require a host runtime facade")]
    MissingHostRuntime,
    #[error("product-live model route is invalid: {0}")]
    ModelRoute(#[from] ModelRouteError),
    #[error("product-live capability execution scope is invalid: {reason}")]
    InvalidCapabilityScope { reason: String },
}

#[derive(Default)]
pub struct ProductLiveCapabilityIo {
    inputs: Mutex<HashMap<String, StagedCapabilityInput>>,
    results: Mutex<HashMap<String, StagedCapabilityResult>>,
}

#[derive(Clone)]
struct StagedCapabilityInput {
    run_id: String,
    payload: serde_json::Value,
}

#[derive(Clone)]
struct StagedCapabilityResult {
    run_id: String,
    output: serde_json::Value,
}

impl ProductLiveCapabilityIo {
    pub fn stage_input(
        &self,
        run_context: &LoopRunContext,
        payload: serde_json::Value,
    ) -> Result<CapabilityInputRef, AgentLoopHostError> {
        let input_ref =
            CapabilityInputRef::new(format!("input:{}:{}", run_context.run_id, Uuid::new_v4()))
                .map_err(|_| {
                    AgentLoopHostError::new(
                        AgentLoopHostErrorKind::Internal,
                        "capability input ref could not be represented",
                    )
                })?;
        self.inputs
            .lock()
            .map_err(|_| capability_io_internal_error())?
            .insert(
                input_ref.as_str().to_string(),
                StagedCapabilityInput {
                    run_id: run_context.run_id.to_string(),
                    payload,
                },
            );
        Ok(input_ref)
    }

    pub fn result_for_ref(
        &self,
        run_context: &LoopRunContext,
        result_ref: &LoopResultRef,
    ) -> Result<serde_json::Value, AgentLoopHostError> {
        ensure_ref_scoped_to_run("result", result_ref.as_str(), run_context)?;
        let results = self
            .results
            .lock()
            .map_err(|_| capability_io_internal_error())?;
        let result = results.get(result_ref.as_str()).ok_or_else(|| {
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
}

#[async_trait]
impl LoopCapabilityInputResolver for ProductLiveCapabilityIo {
    async fn resolve_capability_input(
        &self,
        run_context: &LoopRunContext,
        input_ref: &CapabilityInputRef,
    ) -> Result<serde_json::Value, AgentLoopHostError> {
        ensure_ref_scoped_to_run("input", input_ref.as_str(), run_context)?;
        let inputs = self
            .inputs
            .lock()
            .map_err(|_| capability_io_internal_error())?;
        let input = inputs.get(input_ref.as_str()).ok_or_else(|| {
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
}

#[async_trait]
impl LoopCapabilityResultWriter for ProductLiveCapabilityIo {
    async fn write_capability_result(
        &self,
        run_context: &LoopRunContext,
        _capability_id: &CapabilityId,
        output: serde_json::Value,
    ) -> Result<LoopResultRef, AgentLoopHostError> {
        let result_ref =
            LoopResultRef::new(format!("result:{}.{}", run_context.run_id, Uuid::new_v4()))
                .map_err(|_| {
                    AgentLoopHostError::new(
                        AgentLoopHostErrorKind::Internal,
                        "capability result ref could not be represented",
                    )
                })?;
        self.results
            .lock()
            .map_err(|_| capability_io_internal_error())?
            .insert(
                result_ref.as_str().to_string(),
                StagedCapabilityResult {
                    run_id: run_context.run_id.to_string(),
                    output,
                },
            );
        Ok(result_ref)
    }
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

#[derive(Clone)]
pub struct ProductLiveVisibleCapabilityRequestConfig {
    user_id: UserId,
    extension_id: ExtensionId,
    runtime: RuntimeKind,
    trust: TrustClass,
    grants: CapabilitySet,
    surface_kind: SurfaceKind,
    policy: CapabilitySurfacePolicy,
    provider_trust: BTreeMap<ExtensionId, TrustDecision>,
}

impl ProductLiveVisibleCapabilityRequestConfig {
    pub fn new(
        user_id: UserId,
        extension_id: ExtensionId,
        runtime: RuntimeKind,
        trust: TrustClass,
        surface_kind: SurfaceKind,
        policy: CapabilitySurfacePolicy,
    ) -> Self {
        Self {
            user_id,
            extension_id,
            runtime,
            trust,
            grants: CapabilitySet::default(),
            surface_kind,
            policy,
            provider_trust: BTreeMap::new(),
        }
    }

    pub fn with_grants(mut self, grants: CapabilitySet) -> Self {
        self.grants = grants;
        self
    }

    pub fn with_provider_trust(
        mut self,
        provider: ExtensionId,
        effective_trust: EffectiveTrustClass,
    ) -> Self {
        self.provider_trust.insert(
            provider,
            TrustDecision {
                effective_trust,
                authority_ceiling: AuthorityCeiling {
                    allowed_effects: vec![EffectKind::DispatchCapability],
                    max_resource_ceiling: None,
                },
                provenance: TrustProvenance::AdminConfig,
                evaluated_at: Utc::now(),
            },
        );
        self
    }
}

pub fn visible_capability_request_for_run(
    run_context: &LoopRunContext,
    config: ProductLiveVisibleCapabilityRequestConfig,
) -> Result<VisibleCapabilityRequest, ProductLivePlannedRuntimeAdapterError> {
    let mut context = ExecutionContext::local_default(
        config.user_id,
        config.extension_id,
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

#[derive(Debug, Clone, PartialEq, Eq)]
struct ProductLiveModelRoute {
    provider_id: String,
    model_id: String,
}

impl ProductLiveModelRoute {
    fn new(
        provider_id: impl Into<String>,
        model_id: impl Into<String>,
    ) -> Result<Self, ModelRouteError> {
        let route = ModelRoute::new(provider_id, model_id)?;
        Ok(Self {
            provider_id: route.provider_id().to_string(),
            model_id: route.model_id().to_string(),
        })
    }

    fn to_model_route(&self) -> Result<ModelRoute, ModelRouteError> {
        ModelRoute::new(self.provider_id.clone(), self.model_id.clone())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProductLiveModelRouteSettings {
    selection_mode: ModelSelectionMode,
    default_route: ProductLiveModelRoute,
    mission_route: Option<ProductLiveModelRoute>,
}

impl ProductLiveModelRouteSettings {
    pub fn new(
        provider_id: impl Into<String>,
        model_id: impl Into<String>,
    ) -> Result<Self, ModelRouteError> {
        Ok(Self {
            selection_mode: ModelSelectionMode::ManagedOnly,
            default_route: ProductLiveModelRoute::new(provider_id, model_id)?,
            mission_route: None,
        })
    }

    pub fn with_selection_mode(mut self, selection_mode: ModelSelectionMode) -> Self {
        self.selection_mode = selection_mode;
        self
    }

    pub fn with_mission_route(
        mut self,
        provider_id: impl Into<String>,
        model_id: impl Into<String>,
    ) -> Result<Self, ModelRouteError> {
        self.mission_route = Some(ProductLiveModelRoute::new(provider_id, model_id)?);
        Ok(self)
    }

    fn into_resolver(self) -> Result<StaticModelRouteResolver, ModelRouteError> {
        let default_route = self.default_route.to_model_route()?;
        let mission_route = self
            .mission_route
            .map(|route| route.to_model_route())
            .transpose()?;

        let mut policy =
            ModelRoutePolicy::new(self.selection_mode).with_approved_route(default_route.clone());
        if let Some(route) = mission_route.clone() {
            policy = policy.with_approved_route(route);
        }

        let mut resolver =
            StaticModelRouteResolver::new(policy).with_route(ModelSlot::Default, default_route);
        if let Some(route) = mission_route {
            resolver = resolver.with_route(ModelSlot::Mission, route);
        }
        Ok(resolver)
    }
}

pub struct ProductLivePlannedRuntimeAdapterConfig {
    pub visible_capability_request: VisibleCapabilityRequest,
    pub capability_input_resolver: Arc<dyn LoopCapabilityInputResolver>,
    pub capability_result_writer: Arc<dyn LoopCapabilityResultWriter>,
    pub capability_allow_set: CapabilityAllowSet,
    pub model_routes: ProductLiveModelRouteSettings,
    pub cancellation_factory: Arc<dyn RunCancellationFactory>,
    pub input_queue: Arc<dyn HostInputQueue>,
    pub identity_context_source: Arc<dyn HostIdentityContextSource>,
    pub model_policy_guard: Arc<dyn LoopModelPolicyGuard>,
    pub model_budget_accountant: Arc<dyn LoopModelBudgetAccountant>,
    pub safety_context: InstructionSafetyContext,
    pub milestone_sink: Option<Arc<dyn LoopHostMilestoneSink>>,
}

#[derive(Clone)]
pub struct ProductLivePlannedRuntimeAdapters {
    pub capability_factory: Arc<dyn LoopCapabilityPortFactory>,
    pub capability_surface_resolver: Arc<dyn CapabilitySurfaceProfileResolver>,
    pub model_route_resolver: Arc<dyn ModelRouteResolver>,
    pub cancellation_factory: Arc<dyn RunCancellationFactory>,
    pub input_queue: Arc<dyn HostInputQueue>,
    pub identity_context_source: Arc<dyn HostIdentityContextSource>,
    pub model_policy_guard: Arc<dyn LoopModelPolicyGuard>,
    pub model_budget_accountant: Arc<dyn LoopModelBudgetAccountant>,
    pub safety_context: InstructionSafetyContext,
}

impl ProductLivePlannedRuntimeAdapters {
    pub fn from_services(
        services: &RebornServices,
        config: ProductLivePlannedRuntimeAdapterConfig,
    ) -> Result<Self, ProductLivePlannedRuntimeAdapterError> {
        let host_runtime = services
            .host_runtime
            .clone()
            .ok_or(ProductLivePlannedRuntimeAdapterError::MissingHostRuntime)?;

        let capability_factory = HostRuntimeLoopCapabilityPortFactory::new(
            host_runtime,
            config.visible_capability_request,
            config.capability_input_resolver,
            config.capability_result_writer,
            config.milestone_sink,
        );
        let model_route_resolver: Arc<dyn ModelRouteResolver> =
            Arc::new(config.model_routes.into_resolver()?);

        Ok(Self {
            capability_factory: Arc::new(capability_factory),
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

pub fn capability_allowlist(ids: impl IntoIterator<Item = CapabilityId>) -> CapabilityAllowSet {
    CapabilityAllowSet::allowlist(ids)
}
