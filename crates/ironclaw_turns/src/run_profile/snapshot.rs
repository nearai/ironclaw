use serde::{Deserialize, Serialize};

use crate::{RunProfileId, RunProfileVersion};

use super::{
    driver::AgentLoopDriverDescriptor,
    policy::{
        CancellationPolicy, CheckpointPolicy, RedactedRunProfileProvenance, ResourceBudgetPolicy,
        RuntimeProfileConstraints, SteeringPolicy,
    },
    refs::{
        CapabilitySurfaceProfileId, CheckpointSchemaId, ConcurrencyClass, ContextProfileId,
        ModelProfileId, RunClassId, RunProfileFingerprint, RunnerPoolId, SchedulingClass,
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
    pub runtime_constraints: RuntimeProfileConstraints,
    pub runner_pool_id: Option<RunnerPoolId>,
    pub scheduling_class: SchedulingClass,
    pub concurrency_class: ConcurrencyClass,
    pub resolution_fingerprint: RunProfileFingerprint,
    pub provenance: RedactedRunProfileProvenance,
}
