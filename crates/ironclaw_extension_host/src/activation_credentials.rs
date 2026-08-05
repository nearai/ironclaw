use async_trait::async_trait;
use ironclaw_extensions::ExtensionPackage;
use ironclaw_host_api::decision::RuntimeCredentialAuthRequirement;
use ironclaw_product_contracts::error::ProductOperationFailure;

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
    ) -> Result<(), ProductOperationFailure>;

    async fn credential_readiness(
        &self,
        package: &ExtensionPackage,
    ) -> Result<ExtensionActivationCredentialReadiness, ProductOperationFailure> {
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
    ) -> Result<(), ProductOperationFailure> {
        if package_runtime_credential_auth_requirements(package).is_empty() {
            return Ok(());
        }
        Err(missing_activation_credentials_error(package))
    }

    async fn credential_readiness(
        &self,
        package: &ExtensionPackage,
    ) -> Result<ExtensionActivationCredentialReadiness, ProductOperationFailure> {
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
    ) -> Result<(), ProductOperationFailure> {
        Ok(())
    }
}

pub fn missing_activation_credentials_error(package: &ExtensionPackage) -> ProductOperationFailure {
    ProductOperationFailure::InvalidBindingRequest {
        reason: format!(
            "extension {} requires product auth credentials before activation",
            package.manifest.id.as_str()
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ExtensionActivationCredentialGate, ExtensionActivationCredentialReadiness,
        UnavailableExtensionActivationCredentialGate, missing_activation_credentials_error,
    };
    use ironclaw_extensions::{ExtensionManifest, ExtensionPackage, ManifestSource};
    use ironclaw_host_api::path::VirtualPath;
    use ironclaw_product_contracts::error::ProductOperationFailure;

    const NO_CREDENTIAL_MANIFEST: &str = r#"
schema_version = "reborn.extension_manifest.v2"
id = "credentialless"
name = "Credentialless Extension"
version = "0.1.0"
description = "Activation gate fixture with no credential requirements"
trust = "first_party_requested"

[runtime]
kind = "wasm"
module = "wasm/fixture.wasm"

[[host_api]]
id = "ironclaw.capability_provider/v1"
section = "capability_provider.tools"

[capability_provider.tools]

[[capability_provider.tools.capabilities]]
id = "credentialless.search"
description = "Search without credentials"
effects = ["network"]
default_permission = "ask"
visibility = "model"
input_schema_ref = "schemas/search.input.json"
output_schema_ref = "schemas/search.output.json"
"#;

    /// Same shape as the fixture above, but one capability declares a
    /// **required** `product_auth_account` credential. That is the only
    /// difference between the two packages, and it is what
    /// `package_runtime_credential_auth_requirements` keys on — so it is the
    /// discriminating input for every assertion below.
    const CREDENTIAL_REQUIRED_MANIFEST: &str = r#"
schema_version = "reborn.extension_manifest.v2"
id = "credentialed"
name = "Credentialed Extension"
version = "0.1.0"
description = "Activation gate fixture that requires a product auth account"
trust = "first_party_requested"

[runtime]
kind = "wasm"
module = "wasm/fixture.wasm"

[[host_api]]
id = "ironclaw.capability_provider/v1"
section = "capability_provider.tools"

[capability_provider.tools]

[[capability_provider.tools.capabilities]]
id = "credentialed.search"
description = "Search with a connected account"
effects = ["network", "use_secret"]
default_permission = "ask"
visibility = "model"
input_schema_ref = "schemas/search.input.json"
output_schema_ref = "schemas/search.output.json"

[[capability_provider.tools.capabilities.runtime_credentials]]
handle = "credentialed_account"
source = { type = "product_auth_account", provider = "google" }
audience = { scheme = "https", host_pattern = "api.example.com" }
target = { type = "header", name = "authorization" }
required = true
"#;

    fn package_from(manifest_toml: &str, id: &str) -> ExtensionPackage {
        let contracts =
            crate::product_extension_host_api_contract_registry().expect("host API contracts");
        let manifest = ExtensionManifest::parse(
            manifest_toml,
            ManifestSource::HostBundled,
            &ironclaw_host_api::host_port::default_host_port_catalog().expect("host ports"),
            &contracts,
        )
        .expect("fixture manifest");
        let root = VirtualPath::new(format!("/system/extensions/{id}")).expect("extension root");
        ExtensionPackage::from_manifest_toml(manifest, root, manifest_toml)
            .expect("fixture package")
    }

    fn package() -> ExtensionPackage {
        package_from(NO_CREDENTIAL_MANIFEST, "credentialless")
    }

    fn credentialed_package() -> ExtensionPackage {
        package_from(CREDENTIAL_REQUIRED_MANIFEST, "credentialed")
    }

    /// `UnavailableExtensionActivationCredentialGate` is the stand-in used when
    /// no credential service is wired. It must fail *closed*: an extension that
    /// declares credential requirements cannot activate without one. An
    /// extension that declares none is not gated at all, which is what makes
    /// the deployment usable — so both halves are asserted, and the
    /// discriminating input is the package's own requirements.
    ///
    /// Both halves go through the **trait methods**, not through
    /// `missing_activation_credentials_error` directly: the fail-closed
    /// property belongs to the gate, and a gate that admitted everyone would
    /// still leave a direct call to the error constructor green.
    #[tokio::test]
    async fn the_unavailable_gate_admits_only_credentialless_extensions() {
        let package = package();
        assert!(
            super::package_runtime_credential_auth_requirements(&package).is_empty(),
            "fixture must declare no credential requirements for this to prove anything"
        );

        assert!(
            UnavailableExtensionActivationCredentialGate
                .ensure_credentials(&package)
                .await
                .is_ok(),
            "an extension needing no credentials must not be blocked by a missing service"
        );
        assert_eq!(
            UnavailableExtensionActivationCredentialGate
                .credential_readiness(&package)
                .await
                .expect("readiness is not an error for a credentialless package"),
            ExtensionActivationCredentialReadiness::Ready,
        );

        // The other half of "fail closed", which the assertions above cannot
        // reach: a package that DOES declare a required product-auth account
        // must be refused, and its readiness must name what is missing rather
        // than reporting Ready.
        let credentialed = credentialed_package();
        let declared = super::package_runtime_credential_auth_requirements(&credentialed);
        assert_eq!(
            declared.len(),
            1,
            "fixture must declare exactly one required product-auth credential, got {declared:?}"
        );

        let refusal = UnavailableExtensionActivationCredentialGate
            .ensure_credentials(&credentialed)
            .await
            .expect_err("a credential-requiring extension cannot activate with no service wired");
        assert_eq!(
            refusal,
            ProductOperationFailure::InvalidBindingRequest {
                reason: "extension credentialed requires product auth credentials before \
                         activation"
                    .to_string(),
            },
            "the refusal must name the extension and be the caller's to fix, not retryable"
        );

        assert_eq!(
            UnavailableExtensionActivationCredentialGate
                .credential_readiness(&credentialed)
                .await
                .expect("readiness reports what is missing rather than erroring"),
            ExtensionActivationCredentialReadiness::Missing(declared),
            "readiness must hand back the unmet requirements so the UI can offer the connect step"
        );
    }

    #[test]
    fn the_missing_credentials_error_names_the_extension_and_is_caller_fixable() {
        assert_eq!(
            missing_activation_credentials_error(&package()),
            ProductOperationFailure::InvalidBindingRequest {
                reason: "extension credentialless requires product auth credentials before \
                         activation"
                    .to_string(),
            },
            "the caller must be told which extension needs connecting"
        );
    }
}
