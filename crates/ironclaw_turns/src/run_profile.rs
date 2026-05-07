use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{
    LoopExit, RunProfileId, RunProfileRequest, RunProfileVersion, TurnCheckpointId, TurnId,
    TurnRunId,
};

macro_rules! profile_ref {
    ($name:ident, $kind:literal) => {
        #[derive(Debug, Clone, PartialEq, Eq, Hash)]
        pub struct $name(String);

        impl $name {
            pub fn new(value: impl Into<String>) -> Result<Self, String> {
                let value = value.into();
                validate_profile_ref($kind, &value)?;
                Ok(Self(value))
            }

            #[allow(dead_code)]
            pub(crate) fn from_trusted_static(value: &'static str) -> Self {
                debug_assert!(validate_profile_ref($kind, value).is_ok());
                Self(value.to_string())
            }

            #[allow(dead_code)]
            pub(crate) fn from_trusted_string(value: String) -> Self {
                debug_assert!(validate_profile_ref($kind, &value).is_ok());
                Self(value)
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl Serialize for $name {
            fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
            where
                S: serde::Serializer,
            {
                serializer.serialize_str(&self.0)
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: serde::Deserializer<'de>,
            {
                let value = String::deserialize(deserializer)?;
                Self::new(value).map_err(serde::de::Error::custom)
            }
        }
    };
}

profile_ref!(RunClassId, "run_class_id");
profile_ref!(LoopDriverId, "loop_driver_id");
profile_ref!(CheckpointSchemaId, "checkpoint_schema_id");
profile_ref!(ModelProfileId, "model_profile_id");
profile_ref!(CapabilitySurfaceProfileId, "capability_surface_profile_id");
profile_ref!(ContextProfileId, "context_profile_id");
profile_ref!(RunnerPoolId, "runner_pool_id");
profile_ref!(SchedulingClass, "scheduling_class");
profile_ref!(ConcurrencyClass, "concurrency_class");
profile_ref!(ResourceBudgetTier, "resource_budget_tier");
profile_ref!(RunProfileFingerprint, "run_profile_fingerprint");
profile_ref!(RunProfileSourceLayer, "run_profile_source_layer");
profile_ref!(RunProfileSourceRef, "run_profile_source_ref");

fn validate_profile_ref(kind: &'static str, value: &str) -> Result<(), String> {
    if value.is_empty() {
        return Err(format!("{kind} must not be empty"));
    }
    if value.len() > 128 {
        return Err(format!("{kind} must be at most 128 bytes"));
    }
    if !value
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-' || c == ':')
    {
        return Err(format!(
            "{kind} must contain only lowercase ASCII letters, digits, _, -, or :"
        ));
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentLoopDriverDescriptor {
    pub id: LoopDriverId,
    pub version: RunProfileVersion,
    pub checkpoint_schema_id: Option<CheckpointSchemaId>,
    pub checkpoint_schema_version: Option<RunProfileVersion>,
}

impl AgentLoopDriverDescriptor {
    pub fn new(id: impl Into<String>, version: RunProfileVersion) -> Result<Self, String> {
        Ok(Self {
            id: LoopDriverId::new(id)?,
            version,
            checkpoint_schema_id: None,
            checkpoint_schema_version: None,
        })
    }

    pub fn with_checkpoint_schema(
        mut self,
        checkpoint_schema_id: impl Into<String>,
        checkpoint_schema_version: RunProfileVersion,
    ) -> Result<Self, String> {
        self.checkpoint_schema_id = Some(CheckpointSchemaId::new(checkpoint_schema_id)?);
        self.checkpoint_schema_version = Some(checkpoint_schema_version);
        Ok(self)
    }
}

/// Minimal host marker for the driver contract.
///
/// The concrete per-run host facade is owned by the AgentLoopHost work. This
/// marker keeps this crate's driver contract loop-neutral: drivers receive a
/// host capability boundary, not raw runtime, provider, process, filesystem,
/// network, secret, approval, grant, or lease handles.
pub trait AgentLoopDriverHost: Send + Sync {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentLoopDriverRunRequest {
    pub turn_id: TurnId,
    pub run_id: TurnRunId,
    pub resolved_run_profile: ResolvedRunProfile,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentLoopDriverResumeRequest {
    pub turn_id: TurnId,
    pub run_id: TurnRunId,
    pub checkpoint_id: TurnCheckpointId,
    pub resolved_run_profile: ResolvedRunProfile,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum AgentLoopDriverError {
    #[error("agent loop driver rejected request: {reason}")]
    InvalidRequest { reason: String },
    #[error("agent loop driver is unavailable: {reason}")]
    Unavailable { reason: String },
    #[error("agent loop driver failed: {reason_kind}")]
    Failed { reason_kind: String },
}

/// Userland loop implementation contract.
///
/// Implementations own loop mechanics and return a [`LoopExit`] handshake to the
/// trusted runner. They do not mutate turn state directly and do not receive raw
/// authority handles.
#[async_trait]
pub trait AgentLoopDriver: Send + Sync {
    fn descriptor(&self) -> AgentLoopDriverDescriptor;

    async fn run(
        &self,
        request: AgentLoopDriverRunRequest,
        host: &(dyn AgentLoopDriverHost + Send + Sync),
    ) -> Result<LoopExit, AgentLoopDriverError>;

    async fn resume(
        &self,
        request: AgentLoopDriverResumeRequest,
        host: &(dyn AgentLoopDriverHost + Send + Sync),
    ) -> Result<LoopExit, AgentLoopDriverError>;
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedRunProfile {
    pub run_class_id: RunClassId,
    pub profile_id: RunProfileId,
    pub profile_version: RunProfileVersion,
    pub loop_driver: AgentLoopDriverDescriptor,
    pub checkpoint_schema_id: CheckpointSchemaId,
    pub checkpoint_schema_version: RunProfileVersion,
    pub model_profile_id: ModelProfileId,
    pub capability_surface_profile_id: CapabilitySurfaceProfileId,
    pub context_profile_id: ContextProfileId,
    pub steering_policy: SteeringPolicy,
    pub cancellation_policy: CancellationPolicy,
    pub checkpoint_policy: CheckpointPolicy,
    pub resource_budget_policy: ResourceBudgetPolicy,
    pub runtime_constraints: RuntimeProfileConstraints,
    pub runner_pool_id: Option<RunnerPoolId>,
    pub scheduling_class: SchedulingClass,
    pub concurrency_class: ConcurrencyClass,
    pub resolution_fingerprint: RunProfileFingerprint,
    pub provenance: RedactedRunProfileProvenance,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SteeringPolicy {
    pub allow_steering: bool,
    pub allow_interrupt: bool,
    pub allow_driver_specific_nudges: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CancellationPolicy {
    pub allow_cancel: bool,
    pub require_checkpoint_before_cancel: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckpointPolicy {
    pub require_before_model: bool,
    pub require_before_side_effect: bool,
    pub require_before_block: bool,
    pub max_checkpoint_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResourceBudgetPolicy {
    pub tier: ResourceBudgetTier,
    pub max_model_calls: u32,
    pub max_capability_invocations: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeProfileConstraints {
    pub allow_raw_runtime_backend_selection: bool,
    pub allow_broad_capability_surface: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RedactedRunProfileProvenance {
    pub sources: Vec<RedactedRunProfileSource>,
    pub effective_privileges: Vec<PrivilegedRunProfileDimension>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RedactedRunProfileSource {
    pub layer: RunProfileSourceLayer,
    pub source_ref: RunProfileSourceRef,
    pub summary: String,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum RunProfileRequestAuthority {
    #[default]
    User,
    ProductSurface,
    Admin,
    System,
}

impl RunProfileRequestAuthority {
    fn allows(self, dimension: PrivilegedRunProfileDimension) -> bool {
        match dimension {
            PrivilegedRunProfileDimension::LongRunningMission
            | PrivilegedRunProfileDimension::SpecialDriver
            | PrivilegedRunProfileDimension::RunnerPool => {
                matches!(self, Self::ProductSurface | Self::Admin | Self::System)
            }
            PrivilegedRunProfileDimension::BroadCapabilitySurface
            | PrivilegedRunProfileDimension::HighBudget => {
                matches!(self, Self::Admin | Self::System)
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PrivilegedRunProfileDimension {
    LongRunningMission,
    BroadCapabilitySurface,
    HighBudget,
    SpecialDriver,
    RunnerPool,
}

impl PrivilegedRunProfileDimension {
    fn category(self) -> &'static str {
        match self {
            Self::LongRunningMission => "long_running_mission",
            Self::BroadCapabilitySurface => "broad_capability_surface",
            Self::HighBudget => "high_budget",
            Self::SpecialDriver => "special_driver",
            Self::RunnerPool => "runner_pool",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum RunProfileResolutionError {
    #[error("run profile request is unauthorized for {dimension:?}")]
    Unauthorized {
        dimension: PrivilegedRunProfileDimension,
    },
    #[error("run profile is unavailable: {profile_id}")]
    ProfileUnavailable { profile_id: String },
    #[error("invalid run profile request: {reason}")]
    InvalidRequest { reason: String },
}

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
}

impl RunProfileDefinition {
    fn resolve(&self, request: &RunProfileResolutionRequest) -> ResolvedRunProfile {
        let mut provenance = provenance_for(self, request);
        let resource_budget_policy = self.resolve_resource_budget_policy(request, &mut provenance);
        let fingerprint = fingerprint_for(self, &resource_budget_policy, &provenance);
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
            resolution_fingerprint: fingerprint,
            provenance,
        }
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
        profile_id: RunProfileId::from_trusted_static("interactive_default"),
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
    }
}

fn long_running_mission_profile() -> RunProfileDefinition {
    let checkpoint_schema_id = CheckpointSchemaId::from_trusted_static("durable_mission_v1");
    let checkpoint_schema_version = RunProfileVersion::new(1);
    RunProfileDefinition {
        profile_id: RunProfileId::from_trusted_static("long_running_mission"),
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
            source_ref: RunProfileSourceRef::from_trusted_static(
                match definition.profile_id.as_str() {
                    "interactive_default" => "builtin:interactive_default:v1",
                    "long_running_mission" => "builtin:long_running_mission:v1",
                    _ => "builtin:unknown:v1",
                },
            ),
            summary: summary.to_string(),
        }],
        effective_privileges: definition.required_privileges.clone(),
    }
}

fn update_bool(value: bool, update: &mut impl FnMut(&str)) {
    update(if value { "true" } else { "false" });
}

fn fingerprint_for(
    definition: &RunProfileDefinition,
    resource_budget_policy: &ResourceBudgetPolicy,
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
