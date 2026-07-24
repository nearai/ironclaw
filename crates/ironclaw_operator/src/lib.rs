//! Host/operator control-plane services for Reborn.

pub mod llm_admin;
pub mod operator_logs;
pub mod operator_service_lifecycle;
pub mod route_mounts;

pub use ironclaw_host_api::operator_llm::{
    DetectedEnvLlm, EXAMPLE_OVERLAY_PROVIDER_ID, ProviderMenuEntry, RebornModelRoutesState,
    RebornProviderInfo, RebornProviderList, RebornProviderMetadata, RebornProviderSelection,
    RebornProviderStatus, RebornProviderWriteOutcome, RebornV1State,
};
pub use llm_admin::{
    LlmKeyStore, LlmKeyStoreError, LlmReloadTrigger, ProviderActiveModelReader,
    ProviderProbeOutcome, ProviderRepo, ProviderRepoError, RebornLlmConfigService,
    RebornLlmReloadAdapter, RebornProviderAdmin, RebornProviderAdminError,
    RebornProviderAdminProductCommandService, RebornProviderFactory, ResolvedRebornLlm,
    apply_stored_api_key, resolve_reborn_runtime_llm,
};
pub use operator_logs::{OperatorLogLayer, capture_tracing_log, operator_log_buffer};
pub use operator_service_lifecycle::OperatorServiceLifecycle;
