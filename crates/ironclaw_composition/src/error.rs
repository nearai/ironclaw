use thiserror::Error;

#[derive(Debug, Error)]
pub enum IronClawBuildError {
    #[error("invalid IronClaw composition configuration: {reason}")]
    InvalidConfig { reason: String },
    #[error("IronClaw composition requires database handle for {backend}")]
    MissingDatabaseHandle { backend: &'static str },
    #[error("IronClaw composition requires configured production trust policy")]
    MissingProductionTrustPolicy,
    #[error("IronClaw composition requires resolved runtime policy")]
    MissingRuntimePolicy,
    #[error("IronClaw composition production trust policy must contain at least one source")]
    EmptyProductionTrustPolicy,
    #[error(
        "IronClaw production composition requires a configured or keychain-resolvable secret master key"
    )]
    MissingSecretMasterKey,
    #[error("IronClaw planned run-profile resolver build failed: {reason}")]
    PlannedRunProfileResolver { reason: String },
    #[error("IronClaw composition failed production validation")]
    ProductionWiring {
        report: ironclaw_host_runtime::ProductionWiringReport,
    },
    #[error("IronClaw host runtime build failed")]
    HostRuntime(#[from] ironclaw_host_runtime::HostRuntimeError),
    #[error("IronClaw event store build failed")]
    EventStore(#[from] ironclaw_event_store::IronClawEventStoreError),
    #[error("IronClaw secret store build failed")]
    Secret(#[from] ironclaw_secrets::SecretError),
    #[error("IronClaw filesystem build failed")]
    Filesystem(#[from] ironclaw_filesystem::FilesystemError),
    #[error("IronClaw resource governor build failed")]
    Resource(#[from] ironclaw_resources::ResourceError),
    #[error("IronClaw run state build failed")]
    RunState(#[from] ironclaw_run_state::RunStateError),
    #[error("IronClaw capability lease store build failed")]
    CapabilityLease(#[from] ironclaw_authorization::CapabilityLeaseError),
    #[error("IronClaw turn state build failed")]
    Turn(#[from] ironclaw_turns::TurnError),
    #[error("IronClaw mount view construction failed")]
    Mount(#[from] ironclaw_host_api::HostApiError),
}

impl From<ironclaw_host_runtime::ProductionWiringReport> for crate::IronClawCompositionError {
    fn from(report: ironclaw_host_runtime::ProductionWiringReport) -> Self {
        Self::ProductionWiring { report }
    }
}

impl From<ironclaw_host_runtime::ProductionWiringReport> for IronClawBuildError {
    fn from(report: ironclaw_host_runtime::ProductionWiringReport) -> Self {
        Self::ProductionWiring { report }
    }
}

impl From<crate::IronClawCompositionError> for IronClawBuildError {
    fn from(error: crate::IronClawCompositionError) -> Self {
        match error {
            crate::IronClawCompositionError::InvalidConfig { reason } => {
                Self::InvalidConfig { reason }
            }
            crate::IronClawCompositionError::MissingSecretMasterKey => Self::MissingSecretMasterKey,
            crate::IronClawCompositionError::Mount(error) => Self::Mount(error),
            crate::IronClawCompositionError::Filesystem(error) => Self::Filesystem(error),
            crate::IronClawCompositionError::Resource(error) => Self::Resource(error),
            crate::IronClawCompositionError::RunState(error) => Self::RunState(error),
            crate::IronClawCompositionError::CapabilityLease(error) => Self::CapabilityLease(error),
            crate::IronClawCompositionError::Secret(error) => Self::Secret(error),
            crate::IronClawCompositionError::EventStore(error) => Self::EventStore(error),
            crate::IronClawCompositionError::Turn(error) => Self::Turn(error),
            crate::IronClawCompositionError::RunProfile(error) => Self::PlannedRunProfileResolver {
                reason: error.to_string(),
            },
            crate::IronClawCompositionError::ProductionWiring { report } => {
                Self::ProductionWiring { report }
            }
            error @ crate::IronClawCompositionError::MissingTenantSandboxProcessPort
            | error @ crate::IronClawCompositionError::UnexpectedTenantSandboxProcessPort {
                ..
            } => Self::InvalidConfig {
                reason: error.to_string(),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::IronClawBuildError;

    #[test]
    fn composition_missing_secret_master_key_stays_typed_for_facade_errors() {
        let error =
            IronClawBuildError::from(crate::IronClawCompositionError::MissingSecretMasterKey);

        assert!(matches!(error, IronClawBuildError::MissingSecretMasterKey));
    }

    #[test]
    fn composition_missing_tenant_sandbox_process_port_becomes_invalid_config() {
        let error = IronClawBuildError::from(
            crate::IronClawCompositionError::MissingTenantSandboxProcessPort,
        );

        assert!(
            matches!(error, IronClawBuildError::InvalidConfig { reason } if reason == "production tenant-sandbox process backend requires a tenant sandbox process binding")
        );
    }

    #[test]
    fn composition_unexpected_tenant_sandbox_process_port_becomes_invalid_config() {
        let error = IronClawBuildError::from(
            crate::IronClawCompositionError::UnexpectedTenantSandboxProcessPort {
                process_backend: ironclaw_host_api::ProcessBackendKind::LocalHost,
            },
        );

        assert!(
            matches!(error, IronClawBuildError::InvalidConfig { reason } if reason == "production runtime policy uses LocalHost but a tenant sandbox process binding was supplied")
        );
    }

    #[test]
    fn composition_run_profile_becomes_planned_run_profile_resolver() {
        let error = IronClawBuildError::from(crate::IronClawCompositionError::RunProfile(
            ironclaw_turns::run_profile::RunProfileRegistryError::InvalidProfile {
                reason: "broken run profile".to_string(),
            },
        ));

        assert!(
            matches!(error, IronClawBuildError::PlannedRunProfileResolver { reason } if reason == "invalid run profile: broken run profile")
        );
    }
}
