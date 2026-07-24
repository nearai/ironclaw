use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_extensions::ExtensionPackage;
use ironclaw_host_api::{CredentialStageError, ResourceScope, RuntimeCredentialAuthRequirement};
use ironclaw_product::ProductSurfaceFailure;

use crate::product_auth::credentials::runtime_credentials::{
    RuntimeCredentialAccountSelectionService, missing_runtime_credential_auth_requirements,
};
use ironclaw_extension_host::{
    ExtensionActivationCredentialGate, ExtensionActivationCredentialReadiness,
    missing_activation_credentials_error, package_runtime_credential_auth_requirements,
};

#[derive(Clone)]
pub(crate) struct RuntimeExtensionActivationCredentialGate {
    scope: ResourceScope,
    credential_accounts: Arc<dyn RuntimeCredentialAccountSelectionService>,
}

impl RuntimeExtensionActivationCredentialGate {
    pub(crate) fn new(
        scope: ResourceScope,
        credential_accounts: Arc<dyn RuntimeCredentialAccountSelectionService>,
    ) -> Self {
        Self {
            scope,
            credential_accounts,
        }
    }

    pub(crate) async fn missing_requirements(
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
    ) -> Result<(), ProductSurfaceFailure> {
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
    ) -> Result<ExtensionActivationCredentialReadiness, ProductSurfaceFailure> {
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

fn map_activation_credential_stage_error(error: CredentialStageError) -> ProductSurfaceFailure {
    match error {
        CredentialStageError::AuthRequired => ProductSurfaceFailure::InvalidBindingRequest {
            reason: "extension requires product auth credentials before activation".to_string(),
        },
        CredentialStageError::Backend => ProductSurfaceFailure::Transient {
            reason: "extension product auth credential state is temporarily unavailable"
                .to_string(),
        },
    }
}
