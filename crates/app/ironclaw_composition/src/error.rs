use thiserror::Error;

#[derive(Debug, Error)]
pub enum RebornBuildError {
    #[error("invalid reborn composition configuration: {reason}")]
    InvalidConfig { reason: String },
    #[error("reborn composition requires database handle for {backend}")]
    MissingDatabaseHandle { backend: &'static str },
    #[error("reborn composition requires configured production trust policy")]
    MissingProductionTrustPolicy,
    #[error("reborn composition requires resolved runtime policy")]
    MissingRuntimePolicy,
    #[error("reborn composition production trust policy must contain at least one source")]
    EmptyProductionTrustPolicy,
    #[error(
        "reborn production composition requires a configured or keychain-resolvable secret master key"
    )]
    MissingSecretMasterKey,
    #[error("reborn planned run-profile resolver build failed: {reason}")]
    PlannedRunProfileResolver { reason: String },
    #[error("reborn composition failed production validation")]
    ProductionWiring {
        report: ironclaw_host_runtime::ProductionWiringReport,
    },
    #[error("reborn host runtime build failed")]
    HostRuntime(#[from] ironclaw_host_runtime::HostRuntimeError),
    #[error("reborn event store build failed")]
    EventStore(#[from] ironclaw_event_store::RebornEventStoreError),
    #[error("reborn secret store build failed")]
    Secret(#[from] ironclaw_secrets::SecretError),
    #[error("reborn filesystem build failed")]
    Filesystem(#[from] ironclaw_filesystem::FilesystemError),
    #[error("reborn libSQL runtime build failed")]
    LibSqlRuntime(#[from] ironclaw_libsql_runtime::LibSqlRuntimeError),
    #[error("reborn resource governor build failed")]
    Resource(#[from] ironclaw_resources::ResourceError),
    #[error("reborn approval store build failed")]
    ApprovalStore(#[from] ironclaw_approvals::ApprovalStoreError),
    #[error("reborn capability lease store build failed")]
    CapabilityLease(#[from] ironclaw_authorization::CapabilityLeaseError),
    #[error("reborn turn state build failed")]
    Turn(#[from] ironclaw_turns::TurnError),
    #[error("reborn mount view construction failed")]
    Mount(#[from] ironclaw_host_api::error::HostApiError),
}

impl From<ironclaw_extension_host::RebornExtensionHostBuildError> for RebornBuildError {
    fn from(error: ironclaw_extension_host::RebornExtensionHostBuildError) -> Self {
        match error {
            ironclaw_extension_host::RebornExtensionHostBuildError::InvalidConfig { reason } => {
                Self::InvalidConfig { reason }
            }
            ironclaw_extension_host::RebornExtensionHostBuildError::Filesystem(error) => {
                Self::Filesystem(error)
            }
            ironclaw_extension_host::RebornExtensionHostBuildError::Mount(error) => {
                Self::Mount(error)
            }
        }
    }
}

impl From<ironclaw_host_runtime::ProductionWiringReport> for crate::RebornCompositionError {
    fn from(report: ironclaw_host_runtime::ProductionWiringReport) -> Self {
        Self::ProductionWiring { report }
    }
}

impl From<ironclaw_host_runtime::ProductionWiringReport> for RebornBuildError {
    fn from(report: ironclaw_host_runtime::ProductionWiringReport) -> Self {
        Self::ProductionWiring { report }
    }
}

impl From<crate::RebornCompositionError> for RebornBuildError {
    fn from(error: crate::RebornCompositionError) -> Self {
        match error {
            crate::RebornCompositionError::InvalidConfig { reason } => {
                Self::InvalidConfig { reason }
            }
            crate::RebornCompositionError::MissingSecretMasterKey => Self::MissingSecretMasterKey,
            crate::RebornCompositionError::Mount(error) => Self::Mount(error),
            crate::RebornCompositionError::Filesystem(error) => Self::Filesystem(error),
            crate::RebornCompositionError::Resource(error) => Self::Resource(error),
            crate::RebornCompositionError::ApprovalStore(error) => Self::ApprovalStore(error),
            crate::RebornCompositionError::CapabilityLease(error) => Self::CapabilityLease(error),
            crate::RebornCompositionError::Secret(error) => Self::Secret(error),
            crate::RebornCompositionError::EventStore(error) => Self::EventStore(error),
            crate::RebornCompositionError::Turn(error) => Self::Turn(error),
            crate::RebornCompositionError::RunProfile(error) => Self::PlannedRunProfileResolver {
                reason: error.to_string(),
            },
            crate::RebornCompositionError::ProductionWiring { report } => {
                Self::ProductionWiring { report }
            }
            error @ crate::RebornCompositionError::MissingTenantSandboxProcessPort
            | error @ crate::RebornCompositionError::UnexpectedTenantSandboxProcessPort { .. } => {
                Self::InvalidConfig {
                    reason: error.to_string(),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::RebornBuildError;

    #[test]
    fn composition_missing_secret_master_key_stays_typed_for_service_errors() {
        let error = RebornBuildError::from(crate::RebornCompositionError::MissingSecretMasterKey);

        assert!(matches!(error, RebornBuildError::MissingSecretMasterKey));
    }

    #[test]
    fn composition_missing_tenant_sandbox_process_port_becomes_invalid_config() {
        let error =
            RebornBuildError::from(crate::RebornCompositionError::MissingTenantSandboxProcessPort);

        assert!(
            matches!(error, RebornBuildError::InvalidConfig { reason } if reason == "production tenant-sandbox process backend requires a tenant sandbox process binding")
        );
    }

    #[test]
    fn composition_unexpected_tenant_sandbox_process_port_becomes_invalid_config() {
        let error = RebornBuildError::from(
            crate::RebornCompositionError::UnexpectedTenantSandboxProcessPort {
                process_backend: ironclaw_host_api::runtime_policy::ProcessBackendKind::LocalHost,
            },
        );

        assert!(
            matches!(error, RebornBuildError::InvalidConfig { reason } if reason == "production runtime policy uses LocalHost but a tenant sandbox process binding was supplied")
        );
    }

    #[test]
    fn composition_run_profile_becomes_planned_run_profile_resolver() {
        let error = RebornBuildError::from(crate::RebornCompositionError::RunProfile(
            ironclaw_loop_contracts::RunProfileRegistryError::InvalidProfile {
                reason: "broken run profile".to_string(),
            },
        ));

        assert!(
            matches!(error, RebornBuildError::PlannedRunProfileResolver { reason } if reason == "invalid run profile: broken run profile")
        );
    }
}
