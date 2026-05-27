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
    #[error("reborn composition requires live turn scheduler wake notifier")]
    MissingTurnRunWakeNotifier,
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
    #[error("reborn mount view construction failed")]
    Mount(#[from] ironclaw_host_api::HostApiError),
}

impl From<ironclaw_host_runtime::ProductionWiringReport> for RebornBuildError {
    fn from(report: ironclaw_host_runtime::ProductionWiringReport) -> Self {
        Self::ProductionWiring { report }
    }
}
