use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_extensions::ExtensionPackage;
use ironclaw_host_api::{
    decision::RuntimeCredentialAuthRequirement, dispatch::CredentialStageError,
    resource::ResourceScope,
};
use ironclaw_product_contracts::error::ProductOperationFailure;

use ironclaw_auth::product_auth::credentials::runtime_credentials::{
    RuntimeCredentialAccountSelectionService, missing_runtime_credential_auth_requirements,
};
use ironclaw_extension_host::{
    ExtensionActivationCredentialGate, ExtensionActivationCredentialReadiness,
    missing_activation_credentials_error, package_runtime_credential_auth_requirements,
};

#[derive(Clone)]
pub struct RuntimeExtensionActivationCredentialGate {
    scope: ResourceScope,
    credential_accounts: Arc<dyn RuntimeCredentialAccountSelectionService>,
}

impl RuntimeExtensionActivationCredentialGate {
    pub fn new(
        scope: ResourceScope,
        credential_accounts: Arc<dyn RuntimeCredentialAccountSelectionService>,
    ) -> Self {
        Self {
            scope,
            credential_accounts,
        }
    }

    pub async fn missing_requirements(
        &self,
        requirements: Vec<RuntimeCredentialAuthRequirement>,
    ) -> Result<Vec<RuntimeCredentialAuthRequirement>, CredentialStageError> {
        missing_runtime_credential_auth_requirements(
            self.credential_accounts.as_ref(),
            &self.scope,
            requirements,
        )
        .await
    }
}

#[async_trait]
impl ExtensionActivationCredentialGate for RuntimeExtensionActivationCredentialGate {
    async fn ensure_credentials(
        &self,
        package: &ExtensionPackage,
    ) -> Result<(), ProductOperationFailure> {
        match self.credential_readiness(package).await? {
            ExtensionActivationCredentialReadiness::Ready => Ok(()),
            ExtensionActivationCredentialReadiness::Missing(_) => {
                Err(missing_activation_credentials_error(package))
            }
        }
    }

    async fn credential_readiness(
        &self,
        package: &ExtensionPackage,
    ) -> Result<ExtensionActivationCredentialReadiness, ProductOperationFailure> {
        let missing = self
            .missing_requirements(package_runtime_credential_auth_requirements(package))
            .await
            .map_err(map_activation_credential_stage_error)?;
        if missing.is_empty() {
            Ok(ExtensionActivationCredentialReadiness::Ready)
        } else {
            Ok(ExtensionActivationCredentialReadiness::Missing(missing))
        }
    }
}

fn map_activation_credential_stage_error(error: CredentialStageError) -> ProductOperationFailure {
    match error {
        CredentialStageError::AuthRequired => ProductOperationFailure::InvalidBindingRequest {
            reason: "extension requires product auth credentials before activation".to_string(),
        },
        CredentialStageError::Backend => ProductOperationFailure::Transient {
            reason: "extension product auth credential state is temporarily unavailable"
                .to_string(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::map_activation_credential_stage_error;
    use ironclaw_host_api::dispatch::CredentialStageError;
    use ironclaw_product_contracts::error::ProductOperationFailure;

    /// The activation gate reads credential state before letting an extension
    /// go active. "The user has not connected an account yet" and "we could not
    /// read the credential store" are opposite outcomes: the first must send
    /// the user to the connect flow, the second must be retried. Collapsing
    /// them would either send users to reconnect an already-connected account
    /// during an outage, or leave a genuinely unconnected extension retrying
    /// forever.
    #[test]
    fn credential_staging_separates_missing_auth_from_a_credential_store_outage() {
        assert_eq!(
            map_activation_credential_stage_error(CredentialStageError::AuthRequired),
            ProductOperationFailure::InvalidBindingRequest {
                reason: "extension requires product auth credentials before activation".to_string(),
            },
            "a missing connection is the caller's to resolve, via the connect flow"
        );
        assert_eq!(
            map_activation_credential_stage_error(CredentialStageError::Backend),
            ProductOperationFailure::Transient {
                reason: "extension product auth credential state is temporarily unavailable"
                    .to_string(),
            },
            "a credential-store outage must stay retryable, not read as 'not connected'"
        );
    }
}
