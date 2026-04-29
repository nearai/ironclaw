use ironclaw_host_api::{
    ApprovalRequestId, CapabilityDispatchResult, CapabilityId, ExecutionContext, ResourceEstimate,
};
use ironclaw_processes::ProcessRecord;
use ironclaw_trust::TrustDecision;
use serde_json::Value;

pub struct CapabilityInvocationRequest {
    pub context: ExecutionContext,
    pub capability_id: CapabilityId,
    pub estimate: ResourceEstimate,
    pub input: Value,
    pub trust_decision: TrustDecision,
}

/// Caller-facing approved capability resume request.
#[derive(Debug, Clone, PartialEq)]
pub struct CapabilityResumeRequest {
    pub context: ExecutionContext,
    pub approval_request_id: ApprovalRequestId,
    pub capability_id: CapabilityId,
    pub estimate: ResourceEstimate,
    pub input: Value,
    pub trust_decision: TrustDecision,
}

/// Caller-facing capability spawn request.
#[derive(Debug, Clone, PartialEq)]
pub struct CapabilitySpawnRequest {
    pub context: ExecutionContext,
    pub capability_id: CapabilityId,
    pub estimate: ResourceEstimate,
    pub input: Value,
    pub trust_decision: TrustDecision,
}

/// Caller-facing capability invocation result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapabilityInvocationResult {
    pub dispatch: CapabilityDispatchResult,
}

/// Caller-facing capability spawn result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapabilitySpawnResult {
    pub process: ProcessRecord,
}
