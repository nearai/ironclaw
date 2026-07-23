use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_extensions::ExtensionPackage;
use ironclaw_host_api::{CredentialStageError, ResourceScope, RuntimeCredentialAuthRequirement};
use ironclaw_product::ProductWorkflowError;

pub(crate) use ironclaw_extension_host::activation_transaction::ExtensionActivationCredentialReadiness;

use crate::extension_host::extension_credential_requirements::package_runtime_credential_auth_requirements;
use crate::product_auth::credentials::runtime_credentials::{
    RuntimeCredentialAccountSelectionService, missing_runtime_credential_auth_requirements,
    runtime_credential_accounts_and_missing_requirements,
};

#[async_trait]
pub(crate) trait ExtensionActivationCredentialGate: Send + Sync {
    async fn ensure_credentials(
        &self,
        package: &ExtensionPackage,
    ) -> Result<(), ProductWorkflowError>;

    async fn credential_readiness(
        &self,
        package: &ExtensionPackage,
    ) -> Result<ExtensionActivationCredentialReadiness, ProductWorkflowError> {
        self.ensure_credentials(package).await?;
        Ok(ExtensionActivationCredentialReadiness::Ready(Vec::new()))
    }
}

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
    ) -> Result<(), ProductWorkflowError> {
        match self.credential_readiness(package).await? {
            ExtensionActivationCredentialReadiness::Ready(_) => Ok(()),
            ExtensionActivationCredentialReadiness::Missing(_) => {
                Err(missing_activation_credentials_error(package))
            }
        }
    }

    async fn credential_readiness(
        &self,
        package: &ExtensionPackage,
    ) -> Result<ExtensionActivationCredentialReadiness, ProductWorkflowError> {
        let (selected_accounts, missing) = runtime_credential_accounts_and_missing_requirements(
            self.credential_accounts.as_ref(),
            &self.scope,
            package_runtime_credential_auth_requirements(package),
        )
        .await
        .map_err(map_activation_credential_stage_error)?;
        if missing.is_empty() {
            Ok(ExtensionActivationCredentialReadiness::Ready(
                selected_accounts,
            ))
        } else {
            Ok(ExtensionActivationCredentialReadiness::Missing(missing))
        }
    }
}

pub(crate) struct UnavailableExtensionActivationCredentialGate;

#[async_trait]
impl ExtensionActivationCredentialGate for UnavailableExtensionActivationCredentialGate {
    async fn ensure_credentials(
        &self,
        package: &ExtensionPackage,
    ) -> Result<(), ProductWorkflowError> {
        if package_runtime_credential_auth_requirements(package).is_empty() {
            return Ok(());
        }
        Err(missing_activation_credentials_error(package))
    }

    async fn credential_readiness(
        &self,
        package: &ExtensionPackage,
    ) -> Result<ExtensionActivationCredentialReadiness, ProductWorkflowError> {
        let missing = package_runtime_credential_auth_requirements(package);
        if missing.is_empty() {
            Ok(ExtensionActivationCredentialReadiness::Ready(Vec::new()))
        } else {
            Ok(ExtensionActivationCredentialReadiness::Missing(missing))
        }
    }
}

#[cfg(test)]
pub(crate) struct PrecheckedExtensionActivationCredentialGate;

#[cfg(test)]
#[async_trait]
impl ExtensionActivationCredentialGate for PrecheckedExtensionActivationCredentialGate {
    async fn ensure_credentials(
        &self,
        _package: &ExtensionPackage,
    ) -> Result<(), ProductWorkflowError> {
        Ok(())
    }
}

fn missing_activation_credentials_error(package: &ExtensionPackage) -> ProductWorkflowError {
    ProductWorkflowError::InvalidBindingRequest {
        reason: format!(
            "extension {} requires product auth credentials before activation",
            package.manifest.id.as_str()
        ),
    }
}

fn map_activation_credential_stage_error(error: CredentialStageError) -> ProductWorkflowError {
    match error {
        CredentialStageError::AuthRequired => ProductWorkflowError::InvalidBindingRequest {
            reason: "extension requires product auth credentials before activation".to_string(),
        },
        CredentialStageError::Backend => ProductWorkflowError::Transient {
            reason: "extension product auth credential state is temporarily unavailable"
                .to_string(),
        },
    }
}
