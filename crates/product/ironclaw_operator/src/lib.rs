//! Host/operator control-plane services for Reborn.

pub mod llm_admin;
pub mod operator_logs;
pub mod operator_service_lifecycle;

pub use ironclaw_product_contracts::operator_llm::{
    DetectedEnvLlm, EXAMPLE_OVERLAY_PROVIDER_ID, ProviderMenuEntry, RebornModelRoutesState,
    RebornProviderInfo, RebornProviderList, RebornProviderMetadata, RebornProviderSelection,
    RebornProviderStatus, RebornProviderWriteOutcome, RebornV1State,
};
pub use llm_admin::{
    FilesystemModelSelectionPolicyStore, FilesystemUserModelPreferenceStore, LlmKeyStore,
    LlmKeyStoreError, LlmReloadTrigger, ProviderActiveModelReader, ProviderProbeOutcome,
    ProviderRepo, ProviderRepoError, RebornLlmConfigService, RebornLlmReloadAdapter,
    RebornProviderAdmin, RebornProviderAdminError, RebornProviderFactory, ResolvedRebornLlm,
    apply_stored_api_key, nearai_login_callback_mount, resolve_reborn_runtime_llm,
};
pub use operator_logs::{OperatorLogLayer, capture_tracing_log, operator_log_buffer};
pub use operator_service_lifecycle::OperatorServiceLifecycle;
