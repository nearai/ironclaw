//! Memory profile binding (issue #3537, keystone milestone).
//!
//! Reborn binds memory behavior **per capability profile**, not as one global
//! provider: a `profile_id -> extension_id` map decides which provider serves
//! `memory.context_retrieval.v1`, `memory.interaction_log.v1`, and
//! `memory.document_store.v1`. This module owns the **fail-closed** resolution of
//! that map into a [`MemoryBindingPolicy`] the dispatch sites consult instead of
//! hardwiring `NativeMemoryService::from_filesystem`.
//!
//! Resolution rules (all fail closed — an unresolved or disallowed binding is an
//! error, never a silent fallback to some other provider):
//!
//! - **Default**: an unconfigured required profile binds to the host-bundled
//!   native provider ([`NATIVE_MEMORY_EXTENSION_ID`]) when it is compiled in.
//! - **Native / disabled / third-party** targets are parsed from the configured
//!   `extension_id`. `memory.disabled` is the explicit disable sentinel.
//! - **Production-shaped** deployments (`production`, `migration-dry-run`) reject
//!   `memory.disabled` and reject binding an **unverified third-party** extension
//!   to a required memory profile *unless* an explicit admin override scoped to
//!   `(extension_id, profile_id, deployment_profile)` exists.
//! - Unknown profile ids and duplicate bindings are rejected.
//!
//! Constructing a provider over a per-invocation filesystem still happens at the
//! dispatch site; this policy only decides *which* provider (and whether the
//! binding is permitted at all).

use std::collections::BTreeMap;

use ironclaw_host_api::{CapabilityProfileId, ExtensionId, HostApiError};
use thiserror::Error;

use crate::memory_native_extension::NATIVE_MEMORY_EXTENSION_ID;
use crate::memory_profiles::memory_capability_profiles;

/// Sentinel `extension_id` that explicitly disables a memory profile. Allowed
/// only in non-production deployments.
pub const MEMORY_DISABLED_BINDING_SENTINEL: &str = "memory.disabled";

/// Deployment-profile axis for memory binding resolution and override scoping.
///
/// Mirrors `ironclaw_reborn_config::RebornProfile` by name, but is defined here
/// so `ironclaw_host_runtime` does not depend on the config crate; composition
/// maps one to the other.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryDeploymentProfile {
    LocalDev,
    LocalDevYolo,
    HostedSingleTenant,
    Production,
    MigrationDryRun,
}

impl MemoryDeploymentProfile {
    /// Kebab-case wire name, matching `RebornProfile::as_str`.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::LocalDev => "local-dev",
            Self::LocalDevYolo => "local-dev-yolo",
            Self::HostedSingleTenant => "hosted-single-tenant",
            Self::Production => "production",
            Self::MigrationDryRun => "migration-dry-run",
        }
    }

    /// Parse from the kebab-case wire name. Returns `None` for unknown names.
    pub fn from_wire(name: &str) -> Option<Self> {
        match name {
            "local-dev" => Some(Self::LocalDev),
            "local-dev-yolo" => Some(Self::LocalDevYolo),
            "hosted-single-tenant" => Some(Self::HostedSingleTenant),
            "production" => Some(Self::Production),
            "migration-dry-run" => Some(Self::MigrationDryRun),
            _ => None,
        }
    }

    /// Production-shaped deployments require certified/overridden bindings and
    /// reject `memory.disabled`. `migration-dry-run` validates production shape,
    /// so it enforces the same rules.
    pub fn requires_certified_bindings(self) -> bool {
        matches!(self, Self::Production | Self::MigrationDryRun)
    }
}

/// Resolved provider decision for one memory profile.
///
/// `Default` is [`Native`](Self::Native) so dispatch sites that have not yet
/// been handed a resolved binding preserve the pre-#3537 behavior.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum MemoryProviderBinding {
    /// Host-bundled native provider (`ironclaw.memory.native`).
    #[default]
    Native,
    /// Explicitly disabled (`memory.disabled`); non-production only.
    Disabled,
    /// A third-party provider extension. Permitted in production only with an
    /// admin override; no third-party providers are implemented yet, so the
    /// dispatch site fails closed when it tries to construct one.
    ThirdParty { extension_id: ExtensionId },
}

impl MemoryProviderBinding {
    pub fn is_native(&self) -> bool {
        matches!(self, Self::Native)
    }
}

/// One configured `profile_id -> extension_id` binding (pre-resolution input).
#[derive(Debug, Clone)]
pub struct MemoryProfileBindingEntry {
    pub profile_id: CapabilityProfileId,
    /// `ironclaw.memory.native`, `memory.disabled`, or a third-party id.
    pub extension_id: String,
}

/// One admin override authorizing an otherwise-rejected production binding,
/// scoped to `(extension_id, profile_id, deployment_profile)`.
#[derive(Debug, Clone)]
pub struct MemoryAdminOverrideEntry {
    pub profile_id: CapabilityProfileId,
    pub extension_id: String,
    /// Deployment-profile wire name this override applies to, or `*` for all.
    pub deployment_profile: String,
}

/// An override that was actually applied during resolution. Surfaced (redacted)
/// by startup/doctor diagnostics.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryActiveOverride {
    pub profile_id: CapabilityProfileId,
    pub extension_id: ExtensionId,
    pub deployment_profile: MemoryDeploymentProfile,
}

impl MemoryActiveOverride {
    /// Redacted one-line diagnostic. The extension id is truncated so a verbose
    /// third-party id cannot leak in full through operator-facing output.
    pub fn redacted_summary(&self) -> String {
        let ext = self.extension_id.as_str();
        let redacted_ext = if ext.len() > 12 {
            format!("{}…", &ext[..12])
        } else {
            ext.to_string()
        };
        format!(
            "memory override: profile={} extension={} deployment={}",
            self.profile_id,
            redacted_ext,
            self.deployment_profile.as_str()
        )
    }
}

/// Inputs for resolving a [`MemoryBindingPolicy`].
#[derive(Debug, Clone)]
pub struct MemoryBindingInput {
    pub deployment: MemoryDeploymentProfile,
    /// Whether the host-bundled native provider is compiled in / available.
    pub native_available: bool,
    pub bindings: Vec<MemoryProfileBindingEntry>,
    pub overrides: Vec<MemoryAdminOverrideEntry>,
}

impl MemoryBindingInput {
    /// Default input: no explicit bindings, native available.
    pub fn native_default(deployment: MemoryDeploymentProfile) -> Self {
        Self {
            deployment,
            native_available: true,
            bindings: Vec::new(),
            overrides: Vec::new(),
        }
    }
}

/// Fail-closed reasons a memory binding cannot resolve.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum MemoryBindingError {
    #[error("memory binding references unknown profile '{profile}'")]
    UnknownProfile { profile: String },
    #[error("duplicate memory binding for profile '{profile}'")]
    DuplicateBinding { profile: String },
    #[error("memory.disabled is not allowed for profile '{profile}' in deployment '{deployment}'")]
    DisabledInProduction {
        profile: String,
        deployment: &'static str,
    },
    #[error(
        "binding third-party extension '{extension}' to required profile '{profile}' in deployment '{deployment}' requires an explicit admin override"
    )]
    ThirdPartyRequiresOverride {
        extension: String,
        profile: String,
        deployment: &'static str,
    },
    #[error("native memory provider is required for profile '{profile}' but is not available")]
    NativeUnavailable { profile: String },
    #[error("invalid memory binding extension id '{value}': {reason}")]
    InvalidExtensionId { value: String, reason: String },
}

impl MemoryBindingError {
    fn invalid_extension(value: &str, error: HostApiError) -> Self {
        Self::InvalidExtensionId {
            value: value.to_string(),
            reason: error.to_string(),
        }
    }
}

/// Resolved, validated memory binding policy. The dispatch sites consult
/// [`binding_for`](Self::binding_for) instead of hardwiring the native provider.
#[derive(Debug, Clone)]
pub struct MemoryBindingPolicy {
    bindings: BTreeMap<CapabilityProfileId, MemoryProviderBinding>,
    active_overrides: Vec<MemoryActiveOverride>,
}

impl MemoryBindingPolicy {
    /// Every required memory profile bound to the native provider. The
    /// behavior-preserving default when no `[memory]` config is present.
    pub fn native_default() -> Result<Self, MemoryBindingError> {
        let mut bindings = BTreeMap::new();
        for contract in catalog_profiles()? {
            bindings.insert(contract, MemoryProviderBinding::Native);
        }
        Ok(Self {
            bindings,
            active_overrides: Vec::new(),
        })
    }

    /// Resolve a policy from configured bindings + overrides, failing closed.
    pub fn resolve(input: MemoryBindingInput) -> Result<Self, MemoryBindingError> {
        let known = catalog_profiles()?;

        // Index configured bindings by profile, rejecting unknowns + duplicates.
        let mut configured: BTreeMap<CapabilityProfileId, String> = BTreeMap::new();
        for entry in &input.bindings {
            if !known.contains(&entry.profile_id) {
                return Err(MemoryBindingError::UnknownProfile {
                    profile: entry.profile_id.as_str().to_string(),
                });
            }
            if configured
                .insert(entry.profile_id.clone(), entry.extension_id.clone())
                .is_some()
            {
                return Err(MemoryBindingError::DuplicateBinding {
                    profile: entry.profile_id.as_str().to_string(),
                });
            }
        }

        // Overrides must reference known profiles too (fail closed on typos).
        for over in &input.overrides {
            if !known.contains(&over.profile_id) {
                return Err(MemoryBindingError::UnknownProfile {
                    profile: over.profile_id.as_str().to_string(),
                });
            }
        }

        let mut bindings = BTreeMap::new();
        let mut active_overrides = Vec::new();

        for profile in &known {
            let binding = match configured.get(profile) {
                None => native_binding(profile, input.native_available)?,
                Some(extension_id) if extension_id == NATIVE_MEMORY_EXTENSION_ID => {
                    native_binding(profile, input.native_available)?
                }
                Some(extension_id) if extension_id == MEMORY_DISABLED_BINDING_SENTINEL => {
                    if input.deployment.requires_certified_bindings() {
                        return Err(MemoryBindingError::DisabledInProduction {
                            profile: profile.as_str().to_string(),
                            deployment: input.deployment.as_str(),
                        });
                    }
                    MemoryProviderBinding::Disabled
                }
                Some(extension_id) => {
                    resolve_third_party(profile, extension_id, &input, &mut active_overrides)?
                }
            };
            bindings.insert(profile.clone(), binding);
        }

        Ok(Self {
            bindings,
            active_overrides,
        })
    }

    /// Resolved binding for a profile, if it is a required memory profile.
    pub fn binding_for(&self, profile: &CapabilityProfileId) -> Option<&MemoryProviderBinding> {
        self.bindings.get(profile)
    }

    /// Overrides actually applied during resolution (for redacted diagnostics).
    pub fn active_overrides(&self) -> &[MemoryActiveOverride] {
        &self.active_overrides
    }

    /// Whether any third-party override is active (production diagnostics flag).
    pub fn has_active_overrides(&self) -> bool {
        !self.active_overrides.is_empty()
    }
}

fn catalog_profiles() -> Result<Vec<CapabilityProfileId>, MemoryBindingError> {
    Ok(memory_capability_profiles()
        .map_err(|error| MemoryBindingError::UnknownProfile {
            profile: format!("<catalog build failed: {error}>"),
        })?
        .into_iter()
        .map(|contract| contract.id().clone())
        .collect())
}

fn native_binding(
    profile: &CapabilityProfileId,
    native_available: bool,
) -> Result<MemoryProviderBinding, MemoryBindingError> {
    if native_available {
        Ok(MemoryProviderBinding::Native)
    } else {
        Err(MemoryBindingError::NativeUnavailable {
            profile: profile.as_str().to_string(),
        })
    }
}

fn resolve_third_party(
    profile: &CapabilityProfileId,
    extension_id: &str,
    input: &MemoryBindingInput,
    active_overrides: &mut Vec<MemoryActiveOverride>,
) -> Result<MemoryProviderBinding, MemoryBindingError> {
    let parsed = ExtensionId::new(extension_id)
        .map_err(|error| MemoryBindingError::invalid_extension(extension_id, error))?;

    if input.deployment.requires_certified_bindings() {
        let has_override = input.overrides.iter().any(|over| {
            over.profile_id == *profile
                && over.extension_id == extension_id
                && (over.deployment_profile == input.deployment.as_str()
                    || over.deployment_profile == "*")
        });
        if !has_override {
            return Err(MemoryBindingError::ThirdPartyRequiresOverride {
                extension: extension_id.to_string(),
                profile: profile.as_str().to_string(),
                deployment: input.deployment.as_str(),
            });
        }
        active_overrides.push(MemoryActiveOverride {
            profile_id: profile.clone(),
            extension_id: parsed.clone(),
            deployment_profile: input.deployment,
        });
    }

    Ok(MemoryProviderBinding::ThirdParty {
        extension_id: parsed,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory_profiles::MEMORY_DOCUMENT_STORE_PROFILE_ID;

    fn document_store() -> CapabilityProfileId {
        CapabilityProfileId::new(MEMORY_DOCUMENT_STORE_PROFILE_ID).unwrap()
    }

    fn binding(profile_id: &str, extension_id: &str) -> MemoryProfileBindingEntry {
        MemoryProfileBindingEntry {
            profile_id: CapabilityProfileId::new(profile_id).unwrap(),
            extension_id: extension_id.to_string(),
        }
    }

    #[test]
    fn default_binds_every_profile_to_native() {
        let policy = MemoryBindingPolicy::native_default().expect("default policy");
        assert_eq!(
            policy.binding_for(&document_store()),
            Some(&MemoryProviderBinding::Native)
        );
        assert!(!policy.has_active_overrides());
    }

    #[test]
    fn unconfigured_production_defaults_to_native() {
        let policy = MemoryBindingPolicy::resolve(MemoryBindingInput::native_default(
            MemoryDeploymentProfile::Production,
        ))
        .expect("resolves");
        assert_eq!(
            policy.binding_for(&document_store()),
            Some(&MemoryProviderBinding::Native)
        );
    }

    #[test]
    fn explicit_native_binding_resolves_to_native() {
        let mut input = MemoryBindingInput::native_default(MemoryDeploymentProfile::Production);
        input.bindings = vec![binding(
            MEMORY_DOCUMENT_STORE_PROFILE_ID,
            NATIVE_MEMORY_EXTENSION_ID,
        )];
        let policy = MemoryBindingPolicy::resolve(input).expect("resolves");
        assert_eq!(
            policy.binding_for(&document_store()),
            Some(&MemoryProviderBinding::Native)
        );
    }

    #[test]
    fn disabled_is_allowed_in_dev_but_rejected_in_production() {
        let mut dev = MemoryBindingInput::native_default(MemoryDeploymentProfile::LocalDev);
        dev.bindings = vec![binding(
            MEMORY_DOCUMENT_STORE_PROFILE_ID,
            MEMORY_DISABLED_BINDING_SENTINEL,
        )];
        let policy = MemoryBindingPolicy::resolve(dev).expect("dev allows disabled");
        assert_eq!(
            policy.binding_for(&document_store()),
            Some(&MemoryProviderBinding::Disabled)
        );

        let mut prod = MemoryBindingInput::native_default(MemoryDeploymentProfile::Production);
        prod.bindings = vec![binding(
            MEMORY_DOCUMENT_STORE_PROFILE_ID,
            MEMORY_DISABLED_BINDING_SENTINEL,
        )];
        let err = MemoryBindingPolicy::resolve(prod).expect_err("production rejects disabled");
        assert!(matches!(
            err,
            MemoryBindingError::DisabledInProduction { .. }
        ));
    }

    #[test]
    fn third_party_in_production_requires_override() {
        let mut prod = MemoryBindingInput::native_default(MemoryDeploymentProfile::Production);
        prod.bindings = vec![binding(MEMORY_DOCUMENT_STORE_PROFILE_ID, "acme.honcho")];
        let err = MemoryBindingPolicy::resolve(prod.clone())
            .expect_err("production rejects unverified third-party");
        assert!(matches!(
            err,
            MemoryBindingError::ThirdPartyRequiresOverride { .. }
        ));

        // With a scoped override it resolves and is surfaced for diagnostics.
        prod.overrides = vec![MemoryAdminOverrideEntry {
            profile_id: document_store(),
            extension_id: "acme.honcho".to_string(),
            deployment_profile: "production".to_string(),
        }];
        let policy = MemoryBindingPolicy::resolve(prod).expect("override permits binding");
        match policy.binding_for(&document_store()) {
            Some(MemoryProviderBinding::ThirdParty { extension_id }) => {
                assert_eq!(extension_id.as_str(), "acme.honcho");
            }
            other => panic!("expected third-party binding, got {other:?}"),
        }
        assert_eq!(policy.active_overrides().len(), 1);
        assert!(
            policy.active_overrides()[0]
                .redacted_summary()
                .contains("profile=memory.document_store.v1")
        );
    }

    #[test]
    fn wildcard_override_applies_to_production() {
        let mut prod = MemoryBindingInput::native_default(MemoryDeploymentProfile::Production);
        prod.bindings = vec![binding(MEMORY_DOCUMENT_STORE_PROFILE_ID, "acme.honcho")];
        prod.overrides = vec![MemoryAdminOverrideEntry {
            profile_id: document_store(),
            extension_id: "acme.honcho".to_string(),
            deployment_profile: "*".to_string(),
        }];
        let policy = MemoryBindingPolicy::resolve(prod).expect("wildcard override permits binding");
        assert!(matches!(
            policy.binding_for(&document_store()),
            Some(MemoryProviderBinding::ThirdParty { .. })
        ));
    }

    #[test]
    fn third_party_in_dev_does_not_require_override() {
        let mut dev = MemoryBindingInput::native_default(MemoryDeploymentProfile::LocalDev);
        dev.bindings = vec![binding(MEMORY_DOCUMENT_STORE_PROFILE_ID, "acme.honcho")];
        let policy = MemoryBindingPolicy::resolve(dev).expect("dev allows third-party");
        assert!(matches!(
            policy.binding_for(&document_store()),
            Some(MemoryProviderBinding::ThirdParty { .. })
        ));
        // No override needed in dev, so none is recorded as active.
        assert!(!policy.has_active_overrides());
    }

    #[test]
    fn unknown_profile_binding_is_rejected() {
        let mut input = MemoryBindingInput::native_default(MemoryDeploymentProfile::LocalDev);
        input.bindings = vec![binding(
            "memory.not_a_real_profile.v1",
            NATIVE_MEMORY_EXTENSION_ID,
        )];
        let err = MemoryBindingPolicy::resolve(input).expect_err("unknown profile rejected");
        assert!(matches!(err, MemoryBindingError::UnknownProfile { .. }));
    }

    #[test]
    fn native_unavailable_is_fail_closed() {
        let mut input = MemoryBindingInput::native_default(MemoryDeploymentProfile::LocalDev);
        input.native_available = false;
        let err = MemoryBindingPolicy::resolve(input).expect_err("no provider available");
        assert!(matches!(err, MemoryBindingError::NativeUnavailable { .. }));
    }
}
