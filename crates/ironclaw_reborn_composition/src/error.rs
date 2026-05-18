use thiserror::Error;

#[derive(Debug, Error)]
pub enum RebornBuildError {
    #[error("invalid reborn composition configuration: {reason}")]
    InvalidConfig { reason: String },
    #[error("reborn composition requires database handle for {backend}")]
    MissingDatabaseHandle { backend: &'static str },
    #[error("reborn composition requires configured production trust policy")]
    MissingProductionTrustPolicy,
    #[error("reborn composition production trust policy must contain at least one source")]
    EmptyProductionTrustPolicy,
    #[error("reborn composition requires live turn scheduler wake notifier")]
    MissingTurnRunWakeNotifier,
    #[error("reborn planned run-profile resolver build failed: {reason}")]
    PlannedRunProfileResolver { reason: String },
    #[error("reborn composition failed production validation")]
    ProductionWiring {
        report: ironclaw_host_runtime::ProductionWiringReport,
    },
    #[error("reborn host runtime build failed")]
    HostRuntime(#[from] ironclaw_host_runtime::HostRuntimeError),
    #[error("reborn event store build failed")]
    EventStore(#[from] ironclaw_reborn_event_store::RebornEventStoreError),
    #[error("reborn secret store build failed")]
    Secret(#[from] ironclaw_secrets::SecretError),
    #[error("reborn filesystem build failed")]
    Filesystem(#[from] ironclaw_filesystem::FilesystemError),
    #[error("reborn resource governor build failed")]
    Resource(#[from] ironclaw_resources::ResourceError),
    #[error("reborn run state build failed")]
    RunState(#[from] ironclaw_run_state::RunStateError),
    #[error("reborn capability lease store build failed")]
    CapabilityLease(#[from] ironclaw_authorization::CapabilityLeaseError),
    #[error("reborn turn state build failed")]
    Turn(#[from] ironclaw_turns::TurnError),
}

impl From<ironclaw_host_runtime::ProductionWiringReport> for RebornBuildError {
    fn from(report: ironclaw_host_runtime::ProductionWiringReport) -> Self {
        Self::ProductionWiring { report }
    }
}

impl From<crate::RebornCompositionError> for RebornBuildError {
    fn from(error: crate::RebornCompositionError) -> Self {
        match error {
            crate::RebornCompositionError::MissingSecretMasterKey => Self::InvalidConfig {
                reason: "reborn production composition requires explicit secret master key"
                    .to_string(),
            },
            crate::RebornCompositionError::Filesystem(error) => Self::Filesystem(error),
            crate::RebornCompositionError::Resource(error) => Self::Resource(error),
            crate::RebornCompositionError::RunState(error) => Self::RunState(error),
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
        }
    }
}
