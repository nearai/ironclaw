use std::sync::Arc;

use ironclaw_host_api::runtime_policy::EffectiveRuntimePolicy;
use ironclaw_host_runtime::UserSandboxProcessPort;

use crate::input::RebornRuntimeProcessBindingError;
use crate::{RebornCompositionError, RebornRuntimeProcessBinding};

/// Production runtime policy plus the process port required by its process
/// backend.
#[derive(Clone, Debug)]
pub struct RebornProductionRuntimePolicy {
    runtime_policy: EffectiveRuntimePolicy,
    process_binding: RebornRuntimeProcessBinding,
}

impl RebornProductionRuntimePolicy {
    pub fn without_process_port(
        runtime_policy: EffectiveRuntimePolicy,
    ) -> Result<Self, RebornCompositionError> {
        let process_binding = RebornRuntimeProcessBinding::None;
        process_binding
            .validate_for_production_policy(&runtime_policy)
            .map_err(map_process_binding_error)?;
        Ok(Self {
            runtime_policy,
            process_binding,
        })
    }

    pub fn with_user_sandbox_process_port(
        runtime_policy: EffectiveRuntimePolicy,
        process_port: Arc<UserSandboxProcessPort>,
    ) -> Result<Self, RebornCompositionError> {
        let process_binding = RebornRuntimeProcessBinding::user_sandbox(process_port);
        process_binding
            .validate_for_production_policy(&runtime_policy)
            .map_err(map_process_binding_error)?;
        Ok(Self {
            runtime_policy,
            process_binding,
        })
    }

    pub(crate) fn into_parts(self) -> (EffectiveRuntimePolicy, RebornRuntimeProcessBinding) {
        (self.runtime_policy, self.process_binding)
    }
}

fn map_process_binding_error(error: RebornRuntimeProcessBindingError) -> RebornCompositionError {
    match error {
        RebornRuntimeProcessBindingError::MissingUserSandboxProcessPort => {
            RebornCompositionError::MissingUserSandboxProcessPort
        }
        RebornRuntimeProcessBindingError::UnexpectedUserSandboxProcessPort { process_backend } => {
            RebornCompositionError::UnexpectedUserSandboxProcessPort { process_backend }
        }
    }
}
