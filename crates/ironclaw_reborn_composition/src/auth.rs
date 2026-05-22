use std::sync::Arc;

use ironclaw_auth::{
    AuthFlowManager, AuthInteractionService, AuthProviderClient, CredentialAccountService,
    CredentialSetupService, InMemoryAuthProductServices, SecretCleanupService,
};

/// Reborn product-auth service bundle exposed by the composition root.
///
/// This is the single composition seam for product-facing auth flows,
/// credential accounts, secure manual-token interactions, provider exchange,
/// and lifecycle cleanup. It deliberately exposes trait-shaped ports only:
/// WebUI/setup/extension callers should enter here instead of reaching into
/// lower auth stores, provider clients, or route-local state.
#[derive(Clone)]
pub struct RebornProductAuthServices {
    flow_manager: Arc<dyn AuthFlowManager>,
    interaction_service: Arc<dyn AuthInteractionService>,
    credential_setup_service: Arc<dyn CredentialSetupService>,
    credential_account_service: Arc<dyn CredentialAccountService>,
    provider_client: Arc<dyn AuthProviderClient>,
    cleanup_service: Arc<dyn SecretCleanupService>,
}

impl std::fmt::Debug for RebornProductAuthServices {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RebornProductAuthServices")
            .field("flow_manager", &"Arc<dyn AuthFlowManager>")
            .field("interaction_service", &"Arc<dyn AuthInteractionService>")
            .field(
                "credential_setup_service",
                &"Arc<dyn CredentialSetupService>",
            )
            .field(
                "credential_account_service",
                &"Arc<dyn CredentialAccountService>",
            )
            .field("provider_client", &"Arc<dyn AuthProviderClient>")
            .field("cleanup_service", &"Arc<dyn SecretCleanupService>")
            .finish()
    }
}

impl RebornProductAuthServices {
    pub fn new(
        flow_manager: Arc<dyn AuthFlowManager>,
        interaction_service: Arc<dyn AuthInteractionService>,
        credential_setup_service: Arc<dyn CredentialSetupService>,
        credential_account_service: Arc<dyn CredentialAccountService>,
        provider_client: Arc<dyn AuthProviderClient>,
        cleanup_service: Arc<dyn SecretCleanupService>,
    ) -> Self {
        Self {
            flow_manager,
            interaction_service,
            credential_setup_service,
            credential_account_service,
            provider_client,
            cleanup_service,
        }
    }

    pub fn from_shared<T>(services: Arc<T>) -> Self
    where
        T: AuthFlowManager
            + AuthInteractionService
            + CredentialSetupService
            + CredentialAccountService
            + AuthProviderClient
            + SecretCleanupService
            + 'static,
    {
        let flow_manager: Arc<dyn AuthFlowManager> = services.clone();
        let interaction_service: Arc<dyn AuthInteractionService> = services.clone();
        let credential_setup_service: Arc<dyn CredentialSetupService> = services.clone();
        let credential_account_service: Arc<dyn CredentialAccountService> = services.clone();
        let provider_client: Arc<dyn AuthProviderClient> = services.clone();
        let cleanup_service: Arc<dyn SecretCleanupService> = services;

        Self::new(
            flow_manager,
            interaction_service,
            credential_setup_service,
            credential_account_service,
            provider_client,
            cleanup_service,
        )
    }

    pub fn flow_manager(&self) -> Arc<dyn AuthFlowManager> {
        self.flow_manager.clone()
    }

    pub fn interaction_service(&self) -> Arc<dyn AuthInteractionService> {
        self.interaction_service.clone()
    }

    pub fn credential_setup_service(&self) -> Arc<dyn CredentialSetupService> {
        self.credential_setup_service.clone()
    }

    pub fn credential_account_service(&self) -> Arc<dyn CredentialAccountService> {
        self.credential_account_service.clone()
    }

    pub fn provider_client(&self) -> Arc<dyn AuthProviderClient> {
        self.provider_client.clone()
    }

    pub fn cleanup_service(&self) -> Arc<dyn SecretCleanupService> {
        self.cleanup_service.clone()
    }

    pub(crate) fn local_dev_in_memory() -> Self {
        Self::from_shared(Arc::new(InMemoryAuthProductServices::new()))
    }
}
