use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::{RunProfileId, RunProfileRequest, RunProfileVersion};

use super::{
    driver::AgentLoopDriverDescriptor,
    model_route::{
        InMemoryModelRouteResolver, ModelRoute, ModelRouteResolver, ModelSelectionPolicy,
        ModelSlot, ResolvedModelRoute,
    },
    policy::{
        CancellationPolicy, CheckpointPolicy, PrivilegedRunProfileDimension,
        RedactedRunProfileProvenance, RedactedRunProfileSource, ResourceBudgetPolicy,
        RunProfileRequestAuthority, RunProfileResolutionError, RuntimeProfileConstraints,
        SteeringPolicy,
    },
    refs::{
        CapabilitySurfaceProfileId, CheckpointSchemaId, ConcurrencyClass, ContextProfileId,
        LoopDriverId, ModelProfileId, ResourceBudgetTier, RunClassId, RunProfileFingerprint,
        RunProfileSourceLayer, RunProfileSourceRef, RunnerPoolId, SchedulingClass,
    },
    snapshot::ResolvedRunProfile,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunProfileResolutionRequest {
    pub requested_run_profile: Option<RunProfileRequest>,
    #[serde(skip, default)]
    pub authority: RunProfileRequestAuthority,
}

impl RunProfileResolutionRequest {
    pub fn interactive_default() -> Self {
        Self {
            requested_run_profile: None,
            authority: RunProfileRequestAuthority::User,
        }
    }

    pub fn with_requested_run_profile(mut self, requested: RunProfileRequest) -> Self {
        self.requested_run_profile = Some(requested);
        self
    }

    pub fn with_authority(mut self, authority: RunProfileRequestAuthority) -> Self {
        self.authority = authority;
        self
    }
}

#[async_trait]
pub trait RunProfileResolver: Send + Sync {
    async fn resolve_run_profile(
        &self,
        request: RunProfileResolutionRequest,
    ) -> Result<ResolvedRunProfile, RunProfileResolutionError>;
}

#[derive(Debug, Clone)]
pub struct InMemoryRunProfileResolver {
    registry: InMemoryRunProfileRegistry,
}

impl Default for InMemoryRunProfileResolver {
    fn default() -> Self {
        Self {
            registry: InMemoryRunProfileRegistry::with_builtin_profiles(),
        }
    }
}

impl InMemoryRunProfileResolver {
    pub fn new(registry: InMemoryRunProfileRegistry) -> Self {
        Self { registry }
    }
}

#[async_trait]
impl RunProfileResolver for InMemoryRunProfileResolver {
    async fn resolve_run_profile(
        &self,
        request: RunProfileResolutionRequest,
    ) -> Result<ResolvedRunProfile, RunProfileResolutionError> {
        let requested = request
            .requested_run_profile
            .as_ref()
            .map(RunProfileRequest::as_str)
            .unwrap_or("interactive_default");
        let profile_key = if requested == "default" {
            "interactive_default"
        } else {
            requested
        };
        let definition = self.registry.profile(profile_key).ok_or_else(|| {
            RunProfileResolutionError::ProfileUnavailable {
                profile_id: requested.to_string(),
            }
        })?;

        for &dimension in &definition.required_privileges {
            if !request.authority.allows(dimension) {
                return Err(RunProfileResolutionError::Unauthorized { dimension });
            }
        }

        Ok(definition.resolve(&request))
    }
}

#[derive(Debug, Clone)]
pub struct InMemoryRunProfileRegistry {
    profiles: Vec<RunProfileDefinition>,
}

impl InMemoryRunProfileRegistry {
    pub fn with_builtin_profiles() -> Self {
        Self {
            profiles: vec![interactive_profile(), long_running_mission_profile()],
        }
    }

    pub fn profile(&self, profile_id: &str) -> Option<&RunProfileDefinition> {
        self.profiles
            .iter()
            .find(|definition| definition.profile_id.as_str() == profile_id)
    }
}

#[derive(Debug, Clone)]
pub struct RunProfileDefinition {
    profile_id: RunProfileId,
    profile_version: RunProfileVersion,
    run_class_id: RunClassId,
    loop_driver: AgentLoopDriverDescriptor,
    checkpoint_schema_id: CheckpointSchemaId,
    checkpoint_schema_version: RunProfileVersion,
    model_profile_id: ModelProfileId,
    capability_surface_profile_id: CapabilitySurfaceProfileId,
    context_profile_id: ContextProfileId,
    steering_policy: SteeringPolicy,
    cancellation_policy: CancellationPolicy,
    checkpoint_policy: CheckpointPolicy,
    resource_budget_policy: ResourceBudgetPolicy,
    runtime_constraints: RuntimeProfileConstraints,
    runner_pool_id: Option<RunnerPoolId>,
    scheduling_class: SchedulingClass,
    concurrency_class: ConcurrencyClass,
    required_privileges: Vec<PrivilegedRunProfileDimension>,
    default_model_route: Option<ModelRoute>,
    model_selection_policy: ModelSelectionPolicy,
}

impl RunProfileDefinition {
    fn resolve(&self, request: &RunProfileResolutionRequest) -> ResolvedRunProfile {
        let mut provenance = provenance_for(self, request);
        let resource_budget_policy = self.resolve_resource_budget_policy(request, &mut provenance);
        let resolved_model_route = self.resolve_default_model_route();
        let fingerprint = fingerprint_for(
            self,
            &resource_budget_policy,
            &resolved_model_route,
            &provenance,
        );
        ResolvedRunProfile {
            run_class_id: self.run_class_id.clone(),
            profile_id: self.profile_id.clone(),
            profile_version: self.profile_version,
            loop_driver: self.loop_driver.clone(),
            checkpoint_schema_id: self.checkpoint_schema_id.clone(),
            checkpoint_schema_version: self.checkpoint_schema_version,
            model_profile_id: self.model_profile_id.clone(),
            capability_surface_profile_id: self.capability_surface_profile_id.clone(),
            context_profile_id: self.context_profile_id.clone(),
            steering_policy: self.steering_policy.clone(),
            cancellation_policy: self.cancellation_policy.clone(),
            checkpoint_policy: self.checkpoint_policy.clone(),
            resource_budget_policy,
            runtime_constraints: self.runtime_constraints.clone(),
            runner_pool_id: self.runner_pool_id.clone(),
            scheduling_class: self.scheduling_class.clone(),
            concurrency_class: self.concurrency_class.clone(),
            resolved_model_route,
            resolution_fingerprint: fingerprint,
            provenance,
        }
    }

    fn resolve_default_model_route(&self) -> Option<ResolvedModelRoute> {
        let default_route = self.default_model_route.as_ref()?;
        let resolver = InMemoryModelRouteResolver;
        resolver
            .resolve_model_route(
                &ModelSlot::Default,
                None,
                &self.model_selection_policy,
                Some(default_route),
            )
            .ok()
    }

    fn resolve_resource_budget_policy(
        &self,
        request: &RunProfileResolutionRequest,
        provenance: &mut RedactedRunProfileProvenance,
    ) -> ResourceBudgetPolicy {
        if self.resource_budget_policy.tier.as_str() == "mission_high"
            && !request
                .authority
                .allows(PrivilegedRunProfileDimension::HighBudget)
        {
            provenance.sources.push(RedactedRunProfileSource {
                layer: RunProfileSourceLayer::from_trusted_static("policy_ceiling"),
                source_ref: RunProfileSourceRef::from_trusted_static("builtin:budget-ceiling:v1"),
                summary: "resource budget clamped to mission_standard by policy ceiling"
                    .to_string(),
            });
            return ResourceBudgetPolicy {
                tier: ResourceBudgetTier::from_trusted_static("mission_standard"),
                max_model_calls: self.resource_budget_policy.max_model_calls.min(128),
                max_capability_invocations: self
                    .resource_budget_policy
                    .max_capability_invocations
                    .min(512),
            };
        }

        self.resource_budget_policy.clone()
    }
}

fn interactive_profile() -> RunProfileDefinition {
    let checkpoint_schema_id = CheckpointSchemaId::from_trusted_static("interactive_checkpoint_v1");
    let checkpoint_schema_version = RunProfileVersion::new(1);
    RunProfileDefinition {
        profile_id: RunProfileId::interactive_default(),
        profile_version: RunProfileVersion::new(1),
        run_class_id: RunClassId::from_trusted_static("interactive_coding"),
        loop_driver: AgentLoopDriverDescriptor {
            id: LoopDriverId::from_trusted_static("lightweight_loop"),
            version: RunProfileVersion::new(1),
            checkpoint_schema_id: Some(checkpoint_schema_id.clone()),
            checkpoint_schema_version: Some(checkpoint_schema_version),
        },
        checkpoint_schema_id,
        checkpoint_schema_version,
        model_profile_id: ModelProfileId::from_trusted_static("interactive_model"),
        capability_surface_profile_id: CapabilitySurfaceProfileId::from_trusted_static(
            "interactive_tools",
        ),
        context_profile_id: ContextProfileId::from_trusted_static("interactive_context"),
        steering_policy: SteeringPolicy {
            allow_steering: true,
            allow_interrupt: true,
            allow_driver_specific_nudges: false,
        },
        cancellation_policy: CancellationPolicy {
            allow_cancel: true,
            require_checkpoint_before_cancel: false,
        },
        checkpoint_policy: CheckpointPolicy {
            require_before_model: false,
            require_before_side_effect: true,
            require_before_block: true,
            max_checkpoint_bytes: 64 * 1024,
        },
        resource_budget_policy: ResourceBudgetPolicy {
            tier: ResourceBudgetTier::from_trusted_static("interactive_standard"),
            max_model_calls: 32,
            max_capability_invocations: 64,
        },
        runtime_constraints: RuntimeProfileConstraints {
            allow_raw_runtime_backend_selection: false,
            allow_broad_capability_surface: false,
        },
        runner_pool_id: None,
        scheduling_class: SchedulingClass::from_trusted_static("interactive"),
        concurrency_class: ConcurrencyClass::from_trusted_static("thread_serial"),
        required_privileges: Vec::new(),
        default_model_route: None,
        model_selection_policy: ModelSelectionPolicy::DeveloperAnyConfigured,
    }
}

fn long_running_mission_profile() -> RunProfileDefinition {
    let checkpoint_schema_id = CheckpointSchemaId::from_trusted_static("durable_mission_v1");
    let checkpoint_schema_version = RunProfileVersion::new(1);
    RunProfileDefinition {
        profile_id: RunProfileId::long_running_mission(),
        profile_version: RunProfileVersion::new(1),
        run_class_id: RunClassId::from_trusted_static("long_running_mission"),
        loop_driver: AgentLoopDriverDescriptor {
            id: LoopDriverId::from_trusted_static("codeact_loop"),
            version: RunProfileVersion::new(1),
            checkpoint_schema_id: Some(checkpoint_schema_id.clone()),
            checkpoint_schema_version: Some(checkpoint_schema_version),
        },
        checkpoint_schema_id,
        checkpoint_schema_version,
        model_profile_id: ModelProfileId::from_trusted_static("mission_model"),
        capability_surface_profile_id: CapabilitySurfaceProfileId::from_trusted_static(
            "mission_tools",
        ),
        context_profile_id: ContextProfileId::from_trusted_static("mission_context"),
        steering_policy: SteeringPolicy {
            allow_steering: true,
            allow_interrupt: true,
            allow_driver_specific_nudges: false,
        },
        cancellation_policy: CancellationPolicy {
            allow_cancel: true,
            require_checkpoint_before_cancel: true,
        },
        checkpoint_policy: CheckpointPolicy {
            require_before_model: true,
            require_before_side_effect: true,
            require_before_block: true,
            max_checkpoint_bytes: 256 * 1024,
        },
        resource_budget_policy: ResourceBudgetPolicy {
            tier: ResourceBudgetTier::from_trusted_static("mission_high"),
            max_model_calls: 256,
            max_capability_invocations: 1024,
        },
        runtime_constraints: RuntimeProfileConstraints {
            allow_raw_runtime_backend_selection: false,
            allow_broad_capability_surface: false,
        },
        runner_pool_id: Some(RunnerPoolId::from_trusted_static("mission_workers")),
        scheduling_class: SchedulingClass::from_trusted_static("background"),
        concurrency_class: ConcurrencyClass::from_trusted_static("mission_serial"),
        required_privileges: vec![
            PrivilegedRunProfileDimension::LongRunningMission,
            PrivilegedRunProfileDimension::SpecialDriver,
            PrivilegedRunProfileDimension::RunnerPool,
        ],
        default_model_route: None,
        model_selection_policy: ModelSelectionPolicy::DeveloperAnyConfigured,
    }
}

fn provenance_for(
    definition: &RunProfileDefinition,
    request: &RunProfileResolutionRequest,
) -> RedactedRunProfileProvenance {
    let summary = if request.requested_run_profile.is_some() {
        "requested profile accepted within policy ceiling"
    } else {
        "system default profile selected"
    };
    RedactedRunProfileProvenance {
        sources: vec![RedactedRunProfileSource {
            layer: RunProfileSourceLayer::from_trusted_static("system_default"),
            source_ref: source_ref_for(definition),
            summary: summary.to_string(),
        }],
        effective_privileges: definition.required_privileges.clone(),
    }
}

fn source_ref_for(definition: &RunProfileDefinition) -> RunProfileSourceRef {
    if definition.profile_id == RunProfileId::interactive_default() {
        RunProfileSourceRef::from_trusted_static("builtin:interactive_default:v1")
    } else if definition.profile_id == RunProfileId::long_running_mission() {
        RunProfileSourceRef::from_trusted_static("builtin:long_running_mission:v1")
    } else {
        RunProfileSourceRef::from_trusted_static("builtin:unknown:v1")
    }
}

fn update_bool(value: bool, update: &mut impl FnMut(&str)) {
    update(if value { "true" } else { "false" });
}

fn fingerprint_for(
    definition: &RunProfileDefinition,
    resource_budget_policy: &ResourceBudgetPolicy,
    resolved_model_route: &Option<ResolvedModelRoute>,
    provenance: &RedactedRunProfileProvenance,
) -> RunProfileFingerprint {
    let mut hash = 0xcbf29ce484222325_u64;
    let mut update = |value: &str| {
        for byte in value.as_bytes() {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(0x100000001b3);
        }
    };
    update(definition.profile_id.as_str());
    update(&definition.profile_version.as_u64().to_string());
    update(definition.run_class_id.as_str());
    update(definition.loop_driver.id.as_str());
    update(&definition.loop_driver.version.as_u64().to_string());
    update(
        definition
            .loop_driver
            .checkpoint_schema_id
            .as_ref()
            .map(CheckpointSchemaId::as_str)
            .unwrap_or("none"),
    );
    update(
        &definition
            .loop_driver
            .checkpoint_schema_version
            .map(RunProfileVersion::as_u64)
            .unwrap_or_default()
            .to_string(),
    );
    update(definition.checkpoint_schema_id.as_str());
    update(&definition.checkpoint_schema_version.as_u64().to_string());
    update(definition.model_profile_id.as_str());
    update(definition.capability_surface_profile_id.as_str());
    update(definition.context_profile_id.as_str());
    update_bool(definition.steering_policy.allow_steering, &mut update);
    update_bool(definition.steering_policy.allow_interrupt, &mut update);
    update_bool(
        definition.steering_policy.allow_driver_specific_nudges,
        &mut update,
    );
    update_bool(definition.cancellation_policy.allow_cancel, &mut update);
    update_bool(
        definition
            .cancellation_policy
            .require_checkpoint_before_cancel,
        &mut update,
    );
    update_bool(
        definition.checkpoint_policy.require_before_model,
        &mut update,
    );
    update_bool(
        definition.checkpoint_policy.require_before_side_effect,
        &mut update,
    );
    update_bool(
        definition.checkpoint_policy.require_before_block,
        &mut update,
    );
    update(
        &definition
            .checkpoint_policy
            .max_checkpoint_bytes
            .to_string(),
    );
    update(resource_budget_policy.tier.as_str());
    update(&resource_budget_policy.max_model_calls.to_string());
    update(
        &resource_budget_policy
            .max_capability_invocations
            .to_string(),
    );
    update_bool(
        definition
            .runtime_constraints
            .allow_raw_runtime_backend_selection,
        &mut update,
    );
    update_bool(
        definition
            .runtime_constraints
            .allow_broad_capability_surface,
        &mut update,
    );
    update(
        definition
            .runner_pool_id
            .as_ref()
            .map(RunnerPoolId::as_str)
            .unwrap_or("none"),
    );
    update(definition.scheduling_class.as_str());
    update(definition.concurrency_class.as_str());
    if let Some(route) = resolved_model_route {
        update(route.route.provider_id.as_str());
        update(route.route.model_id.as_str());
    }
    for dimension in &provenance.effective_privileges {
        update(dimension.category());
    }
    for source in &provenance.sources {
        update(source.layer.as_str());
        update(source.source_ref.as_str());
        update(&source.summary);
    }
    RunProfileFingerprint::from_trusted_string(format!("fp:{hash:016x}"))
}
