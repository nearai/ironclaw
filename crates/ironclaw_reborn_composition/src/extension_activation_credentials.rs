use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_extensions::ExtensionPackage;
use ironclaw_host_api::{CredentialStageError, ResourceScope, RuntimeCredentialAuthRequirement};
use ironclaw_product_workflow::ProductWorkflowError;

use crate::extension_credential_requirements::package_runtime_credential_auth_requirements;
use crate::product_auth_runtime_credentials::{
    RuntimeCredentialAccountSelectionService, missing_runtime_credential_auth_requirements,
};

#[async_trait]
pub(crate) trait ExtensionActivationCredentialGate: Send + Sync {
    async fn ensure_credentials(
        &self,
        package: &ExtensionPackage,
    ) -> Result<(), ProductWorkflowError>;
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
        let missing_requirements = self
            .missing_requirements(package_runtime_credential_auth_requirements(package))
            .await
            .map_err(map_activation_credential_stage_error)?;
        if missing_requirements.is_empty() {
            return Ok(());
        }
        Err(missing_activation_credentials_error(package))
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
        CredentialStageError::Backend => ProductWorkflowError::InvalidBindingRequest {
            reason: "extension product auth credential state is invalid".to_string(),
        },
    }
}
