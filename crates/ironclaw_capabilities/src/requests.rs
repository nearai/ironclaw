use ironclaw_host_api::dispatch::CapabilityDispatchResult;
use ironclaw_processes::ProcessRecord;

/// Caller-facing capability spawn request.
#[derive(Debug, Clone, PartialEq)]
pub struct CapabilitySpawnRequest {
    pub context: ironclaw_host_api::scope::ExecutionContext,
    pub capability_id: ironclaw_host_api::ids::CapabilityId,
    pub estimate: ironclaw_host_api::resource::ResourceEstimate,
    pub input: serde_json::Value,
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
