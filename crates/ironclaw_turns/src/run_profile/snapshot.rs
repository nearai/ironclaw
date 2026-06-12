use serde::{Deserialize, Serialize};

use crate::{RunProfileId, RunProfileVersion};

use super::{
    driver::AgentLoopDriverDescriptor,
    policy::{
        CancellationPolicy, CheckpointPolicy, RedactedRunProfileProvenance,
        RedactedRunProfileSource, ResourceBudgetPolicy, RuntimeProfileConstraints, SteeringPolicy,
    },
    refs::{
        CapabilitySurfaceProfileId, CheckpointSchemaId, ConcurrencyClass, ContextProfileId,
        LoopDriverId, ModelProfileId, ResourceBudgetTier, RunClassId, RunProfileFingerprint,
        RunProfileSourceLayer, RunProfileSourceRef, RunnerPoolId, SchedulingClass,
    },
};

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
    #[serde(default)]
    pub personal_context_policy: PersonalContextPolicy,
    pub runtime_constraints: RuntimeProfileConstraints,
    pub runner_pool_id: Option<RunnerPoolId>,
    pub scheduling_class: SchedulingClass,
    pub concurrency_class: ConcurrencyClass,
    pub resolution_fingerprint: RunProfileFingerprint,
    pub provenance: RedactedRunProfileProvenance,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PersonalContextPolicy {
    #[default]
    Excluded,
    Allowed,
}

impl PersonalContextPolicy {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Excluded => "excluded",
            Self::Allowed => "allowed",
        }
    }
}

impl ResolvedRunProfile {
    pub(crate) fn legacy_compatibility(
        profile_id: RunProfileId,
        profile_version: RunProfileVersion,
        allow_steering: bool,
    ) -> Self {
        let checkpoint_schema_id =
            CheckpointSchemaId::from_trusted_static("interactive_checkpoint_v1");
        let checkpoint_schema_version = RunProfileVersion::new(1);
        Self {
            run_class_id: RunClassId::from_trusted_static("legacy_interactive_coding"),
            profile_id,
            profile_version,
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
                allow_steering,
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
                require_final_checkpoint: false,
                allow_no_reply_completion: false,
            },
            resource_budget_policy: ResourceBudgetPolicy {
                tier: ResourceBudgetTier::from_trusted_static("interactive_standard"),
                max_model_calls: 32,
                max_capability_invocations: 64,
            },
            personal_context_policy: PersonalContextPolicy::Excluded,
            runtime_constraints: RuntimeProfileConstraints {
                allow_raw_runtime_backend_selection: false,
                allow_broad_capability_surface: false,
            },
            runner_pool_id: None,
            scheduling_class: SchedulingClass::from_trusted_static("interactive"),
            concurrency_class: ConcurrencyClass::from_trusted_static("thread_serial"),
            resolution_fingerprint: RunProfileFingerprint::from_trusted_static(
                "legacy-persisted-profile-v1",
            ),
            provenance: RedactedRunProfileProvenance {
                sources: vec![RedactedRunProfileSource {
                    layer: RunProfileSourceLayer::from_trusted_static("legacy_persistence"),
                    source_ref: RunProfileSourceRef::from_trusted_static(
                        "turn-run-profile:legacy:v1",
                    ),
                    summary: "legacy persisted turn run profile reconstructed without raw authority handles"
                        .to_string(),
                }],
                effective_privileges: Vec::new(),
            },
        }
    }
}
