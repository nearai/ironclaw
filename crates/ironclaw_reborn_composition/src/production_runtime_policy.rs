use ironclaw_host_api::runtime_policy::{EffectiveRuntimePolicy, ProcessBackendKind};
use ironclaw_host_runtime::VerifiedTenantSandboxProcessPort;

use crate::RebornCompositionError;

/// Production runtime policy plus the process port required by its process
/// backend.
#[derive(Clone, Debug)]
pub struct RebornProductionRuntimePolicy {
    runtime_policy: EffectiveRuntimePolicy,
    tenant_sandbox_process_port: Option<VerifiedTenantSandboxProcessPort>,
}

impl RebornProductionRuntimePolicy {
    pub fn without_process_port(
        runtime_policy: EffectiveRuntimePolicy,
    ) -> Result<Self, RebornCompositionError> {
        if runtime_policy.process_backend == ProcessBackendKind::TenantSandbox {
            return Err(RebornCompositionError::MissingTenantSandboxProcessPort);
        }
        Ok(Self {
            runtime_policy,
            tenant_sandbox_process_port: None,
        })
    }

    pub fn with_tenant_sandbox_process_port(
        runtime_policy: EffectiveRuntimePolicy,
        process_port: VerifiedTenantSandboxProcessPort,
    ) -> Result<Self, RebornCompositionError> {
        if runtime_policy.process_backend != ProcessBackendKind::TenantSandbox {
            return Err(RebornCompositionError::UnexpectedTenantSandboxProcessPort {
                process_backend: runtime_policy.process_backend,
            });
        }
        Ok(Self {
            runtime_policy,
            tenant_sandbox_process_port: Some(process_port),
        })
    }

    pub(crate) fn into_parts(
        self,
    ) -> (
        EffectiveRuntimePolicy,
        Option<VerifiedTenantSandboxProcessPort>,
    ) {
        (self.runtime_policy, self.tenant_sandbox_process_port)
    }
}
