use async_trait::async_trait;
use ironclaw_extensions::ExtensionPackage;
use ironclaw_host_api::decision::RuntimeCredentialAuthRequirement;
use ironclaw_product::ProductSurfaceFailure;

use crate::package_runtime_credential_auth_requirements;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExtensionActivationCredentialReadiness {
    Ready,
    Missing(Vec<RuntimeCredentialAuthRequirement>),
}

#[async_trait]
pub trait ExtensionActivationCredentialGate: Send + Sync {
    async fn ensure_credentials(
        &self,
        package: &ExtensionPackage,
    ) -> Result<(), ProductSurfaceFailure>;

    async fn credential_readiness(
        &self,
        package: &ExtensionPackage,
    ) -> Result<ExtensionActivationCredentialReadiness, ProductSurfaceFailure> {
        self.ensure_credentials(package).await?;
        Ok(ExtensionActivationCredentialReadiness::Ready)
    }
}

pub struct UnavailableExtensionActivationCredentialGate;

#[async_trait]
impl ExtensionActivationCredentialGate for UnavailableExtensionActivationCredentialGate {
    async fn ensure_credentials(
        &self,
        package: &ExtensionPackage,
    ) -> Result<(), ProductSurfaceFailure> {
        if package_runtime_credential_auth_requirements(package).is_empty() {
            return Ok(());
        }
        Err(missing_activation_credentials_error(package))
    }

    async fn credential_readiness(
        &self,
        package: &ExtensionPackage,
    ) -> Result<ExtensionActivationCredentialReadiness, ProductSurfaceFailure> {
        let missing = package_runtime_credential_auth_requirements(package);
        if missing.is_empty() {
            Ok(ExtensionActivationCredentialReadiness::Ready)
        } else {
            Ok(ExtensionActivationCredentialReadiness::Missing(missing))
        }
    }
}

pub struct PrecheckedExtensionActivationCredentialGate;

#[async_trait]
impl ExtensionActivationCredentialGate for PrecheckedExtensionActivationCredentialGate {
    async fn ensure_credentials(
        &self,
        _package: &ExtensionPackage,
    ) -> Result<(), ProductSurfaceFailure> {
        Ok(())
    }
}

pub fn missing_activation_credentials_error(package: &ExtensionPackage) -> ProductSurfaceFailure {
    ProductSurfaceFailure::InvalidBindingRequest {
        reason: format!(
            "extension {} requires product auth credentials before activation",
            package.manifest.id.as_str()
        ),
    }
}
