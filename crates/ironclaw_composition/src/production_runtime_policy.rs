use std::sync::Arc;

use ironclaw_host_api::runtime_policy::EffectiveRuntimePolicy;
use ironclaw_host_runtime::TenantSandboxProcessPort;

use crate::input::IronClawRuntimeProcessBindingError;
use crate::{IronClawCompositionError, IronClawRuntimeProcessBinding};

/// Production runtime policy plus the process port required by its process
/// backend.
#[derive(Clone, Debug)]
pub struct IronClawProductionRuntimePolicy {
    runtime_policy: EffectiveRuntimePolicy,
    process_binding: IronClawRuntimeProcessBinding,
}

impl IronClawProductionRuntimePolicy {
    pub fn without_process_port(
        runtime_policy: EffectiveRuntimePolicy,
    ) -> Result<Self, IronClawCompositionError> {
        let process_binding = IronClawRuntimeProcessBinding::None;
        process_binding
            .validate_for_production_policy(&runtime_policy)
            .map_err(map_process_binding_error)?;
        Ok(Self {
            runtime_policy,
            process_binding,
        })
    }

    pub fn with_tenant_sandbox_process_port(
        runtime_policy: EffectiveRuntimePolicy,
        process_port: Arc<TenantSandboxProcessPort>,
    ) -> Result<Self, IronClawCompositionError> {
        let process_binding = IronClawRuntimeProcessBinding::tenant_sandbox(process_port);
        process_binding
            .validate_for_production_policy(&runtime_policy)
            .map_err(map_process_binding_error)?;
        Ok(Self {
            runtime_policy,
            process_binding,
        })
    }

    pub(crate) fn into_parts(self) -> (EffectiveRuntimePolicy, IronClawRuntimeProcessBinding) {
        (self.runtime_policy, self.process_binding)
    }
}

fn map_process_binding_error(
    error: IronClawRuntimeProcessBindingError,
) -> IronClawCompositionError {
    match error {
        IronClawRuntimeProcessBindingError::MissingTenantSandboxProcessPort => {
            IronClawCompositionError::MissingTenantSandboxProcessPort
        }
        IronClawRuntimeProcessBindingError::UnexpectedTenantSandboxProcessPort {
            process_backend,
        } => IronClawCompositionError::UnexpectedTenantSandboxProcessPort { process_backend },
    }
}
