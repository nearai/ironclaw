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
    trust_policy
        .evaluate(&input)
        .map_err(|_| TrustEvaluationError::Policy)
}

fn trust_policy_input_for_local_manifest(
    package: &ExtensionPackage,
) -> Result<TrustPolicyInput, TrustEvaluationError> {
    package
        .trust_policy_input(local_manifest_source(package), package.manifest_digest(), None)
        .map_err(|_| TrustEvaluationError::TrustInput)
}

fn local_manifest_source(package: &ExtensionPackage) -> PackageSource {
    PackageSource::LocalManifest {
        path: format!(
            "{}/manifest.toml",
            package.root.as_str().trim_end_matches('/')
        ),
    }
}
