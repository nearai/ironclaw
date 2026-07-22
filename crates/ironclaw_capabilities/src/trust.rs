//! Provider-trust classification, run inside `authorize()`.
//!
//! Relocated from `ironclaw_host_runtime::DefaultHostRuntime::evaluate_invocation_trust`
//! (arch-simplification §5.3.2/§9): trust is now *computed by the kernel* rather
//! than received as a pre-stamped `trust_decision` request field. The kernel
//! already depends on `ironclaw_trust` (the `TrustPolicy` trait it reuses) and on
//! `ironclaw_extensions` (the registry it walks), so this move adds no dependency
//! edge — it just relocates a pure classification over those inputs to the single
//! authority site.

use ironclaw_extensions::{ExtensionPackage, ExtensionRegistry};
use ironclaw_host_api::{CapabilityId, PackageSource};
use ironclaw_trust::{TrustDecision, TrustPolicy, TrustPolicyInput};
use tracing::debug;

/// Why trust classification refused to produce a `TrustDecision`.
///
/// Mirrors the variants of host-runtime's former `TrustEvaluationError` so the
/// kernel→host result mapping can reproduce today's exact `Failed` outcome kind
/// (`MissingRuntime` for [`Self::UnknownCapability`], `Authorization` for the
/// rest — see [`Self::is_unknown_capability`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TrustEvaluationError {
    UnknownCapability,
    MissingPackage,
    StalePackageDescriptor,
    ConflictingPackageDescriptor,
    TrustInput,
    Policy,
}

impl TrustEvaluationError {
    /// Whether this failure is the "capability not in the registry" case, which
    /// the host maps to `RuntimeFailureKind::MissingRuntime`; every other variant
    /// maps to `RuntimeFailureKind::Authorization` (behavior-preserving).
    pub(crate) fn is_unknown_capability(self) -> bool {
        matches!(self, Self::UnknownCapability)
    }

    pub(crate) fn message(self) -> &'static str {
        match self {
            Self::UnknownCapability => "unknown capability",
            Self::MissingPackage => "capability provider package is missing",
            Self::StalePackageDescriptor => "capability descriptor is stale for its package",
            Self::ConflictingPackageDescriptor => {
                "capability descriptor conflicts with its package descriptor"
            }
            Self::TrustInput => "could not build trust policy input from the package manifest",
            Self::Policy => "host trust policy evaluation failed",
        }
    }
}

/// Classify provider trust for `capability_id` against the host trust policy.
///
/// Pure over the registry snapshot + the policy — no I/O, no host services. The
/// returned [`TrustDecision`] feeds the trust-aware authorizer and is frozen into
/// the sealed `Authorized` witness; it is never model-supplied.
pub(crate) fn evaluate_invocation_trust(
    registry: &ExtensionRegistry,
    trust_policy: &dyn TrustPolicy,
    capability_id: &CapabilityId,
) -> Result<TrustDecision, TrustEvaluationError> {
    let descriptor = registry
        .get_capability(capability_id)
        .ok_or(TrustEvaluationError::UnknownCapability)?;
    let package = registry
        .get_extension(&descriptor.provider)
        .ok_or(TrustEvaluationError::MissingPackage)?;
    let package_descriptor = package
        .capabilities
        .iter()
        .find(|candidate| candidate.id == *capability_id)
        .ok_or(TrustEvaluationError::StalePackageDescriptor)?;
    if package_descriptor != descriptor {
        return Err(TrustEvaluationError::ConflictingPackageDescriptor);
    }
    let input = trust_policy_input_for_local_manifest(package)?;
    trust_policy.evaluate(&input).map_err(|error| {
        // The kernel→host mapping collapses this to `Policy`; log the bound
        // `TrustError` so the underlying policy refusal is recoverable
        // server-side. `debug!` avoids corrupting the REPL/TUI.
        debug!(%error, "host trust policy evaluation refused a decision");
        TrustEvaluationError::Policy
    })
}

fn trust_policy_input_for_local_manifest(
    package: &ExtensionPackage,
) -> Result<TrustPolicyInput, TrustEvaluationError> {
    package
        .trust_policy_input(
            local_manifest_source(package),
            package.manifest_digest(),
            None,
        )
        .map_err(|error| {
            // Collapsed to `TrustInput` for the host mapping; log the bound
            // `ExtensionError` so the manifest defect is recoverable
            // server-side. `debug!` avoids corrupting the REPL/TUI.
            debug!(%error, "could not build trust policy input from package manifest");
            TrustEvaluationError::TrustInput
        })
}

fn local_manifest_source(package: &ExtensionPackage) -> PackageSource {
    PackageSource::LocalManifest {
        path: format!(
            "{}/manifest.toml",
            package.root.as_str().trim_end_matches('/')
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironclaw_extensions::{ExtensionManifest, ManifestSource};
    use ironclaw_host_api::{HostPortCatalog, VirtualPath, sha256_digest_token};

    // Relocated from `ironclaw_host_runtime::production` alongside the trust
    // evaluation this crate now owns (§5.3.2/§9): the local-manifest trust
    // policy input must carry the manifest path as `PackageSource::LocalManifest`
    // and the manifest digest, so the host trust policy classifies the package
    // from its on-disk identity.
    fn capability_provider_contracts() -> ironclaw_extensions::HostApiContractRegistry {
        let mut contracts = ironclaw_extensions::HostApiContractRegistry::new();
        contracts
            .register(std::sync::Arc::new(
                ironclaw_extensions::CapabilityProviderHostApiContract::new()
                    .expect("capability provider contract"),
            ))
            .expect("register capability provider contract");
        contracts
    }

    #[test]
    fn local_manifest_trust_input_includes_manifest_digest() {
        const MANIFEST: &str = r#"
schema_version = "reborn.extension_manifest.v2"
id = "test"
name = "Test"
version = "0.1.0"
description = "test extension"
trust = "third_party"

[runtime]
kind = "script"
runner = "sandboxed_process"
command = "echo"

[[host_api]]
id = "ironclaw.capability_provider/v1"
section = "capability_provider.tools"

[capability_provider.tools]

[[capability_provider.tools.capabilities]]
id = "test.cap"
description = "Test capability"
effects = ["network"]
default_permission = "ask"
visibility = "model"
input_schema_ref = "schemas/test.input.json"
output_schema_ref = "schemas/test.output.json"
"#;
        let manifest = ExtensionManifest::parse(
            MANIFEST,
            ManifestSource::HostBundled,
            &HostPortCatalog::empty(),
            &capability_provider_contracts(),
        )
        .unwrap();
        let package = ExtensionPackage::from_manifest_toml(
            manifest,
            VirtualPath::new("/system/extensions/test").unwrap(),
            MANIFEST,
        )
        .unwrap();

        let input = trust_policy_input_for_local_manifest(&package).unwrap();

        assert_eq!(
            input.identity.source,
            PackageSource::LocalManifest {
                path: "/system/extensions/test/manifest.toml".to_string()
            }
        );
        let expected_digest = sha256_digest_token(MANIFEST.as_bytes());
        assert_eq!(
            input.identity.digest.as_deref(),
            Some(expected_digest.as_str())
        );
    }
}
