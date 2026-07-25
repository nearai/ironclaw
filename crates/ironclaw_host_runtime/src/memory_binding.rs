//! Memory provider binding (issue #3537).
//!
//! Memory is a single host adapter surface (the `ironclaw.memory.*` tools plus
//! the retrieve-before / record-after lifecycle) backed by exactly one provider
//! chosen at compose time. This module owns the **fail-closed** resolution of
//! the configured `[memory]` provider selection into a [`MemoryBindingPolicy`]
//! the dispatch site consults instead of hardwiring `NativeMemoryService`.
//!
//! Resolution rules (all fail closed — an unresolved or disallowed binding is an
//! error, never a silent fallback):
//!
//! - **Default**: an unconfigured deployment binds the host-bundled native
//!   provider ([`NATIVE_MEMORY_EXTENSION_ID`]) when it is compiled in.
//! - **Native / disabled / third-party** targets are parsed from the configured
//!   provider extension id. `memory.disabled` is the explicit disable sentinel.
//! - **Production-shaped** deployments (`production`, `migration-dry-run`) reject
//!   `memory.disabled` and reject binding an **unverified third-party** provider
//!   *unless* an explicit admin override scoped to
//!   `(extension_id, deployment_profile)` exists.
//!
//! Constructing a provider over a per-invocation filesystem still happens at the
//! dispatch site; this policy only decides *which* provider (and whether the
//! binding is permitted at all). The provider is immutable for the runtime's
//! lifetime — there is no runtime swap.

use ironclaw_host_api::{ExtensionId, HostApiError};
use thiserror::Error;

use crate::memory_native_extension::NATIVE_MEMORY_EXTENSION_ID;

/// Sentinel `extension_id` that explicitly disables memory. Allowed only in
/// non-production deployments.
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

/// Resolved provider decision for memory.
///
/// `Default` is [`Native`](Self::Native) so dispatch sites that have not yet
/// been handed a resolved binding preserve the default behavior.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum MemoryProviderBinding {
    /// Host-bundled native provider (`ironclaw.memory`).
    #[default]
    Native,
    /// Explicitly disabled (`memory.disabled`); non-production only.
    Disabled,
    /// A third-party provider extension (e.g. `mem0`). Permitted in production
    /// only with an admin override; the dispatch site fails closed if it cannot
    /// construct the named provider.
    ThirdParty { extension_id: ExtensionId },
}

impl MemoryProviderBinding {
    pub fn is_native(&self) -> bool {
        matches!(self, Self::Native)
    }
}

/// One admin override authorizing an otherwise-rejected production binding,
/// scoped to `(extension_id, deployment_profile)`.
#[derive(Debug, Clone)]
pub struct MemoryAdminOverrideEntry {
    pub extension_id: String,
    /// Deployment-profile wire name this override applies to, or `*` for all.
    pub deployment_profile: String,
}

/// An override that was actually applied during resolution. Surfaced (redacted)
/// by startup/doctor diagnostics.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryActiveOverride {
    pub extension_id: ExtensionId,
    pub deployment_profile: MemoryDeploymentProfile,
}

impl MemoryActiveOverride {
    /// Redacted one-line diagnostic. The extension id is truncated so a verbose
    /// third-party id cannot leak in full through operator-facing output.
    pub fn redacted_summary(&self) -> String {
        let ext = self.extension_id.as_str();
        // Truncate by characters, not bytes (repo no-byte-slice rule).
        let redacted_ext = if ext.chars().count() > 12 {
            let head: String = ext.chars().take(12).collect();
            format!("{head}…")
        } else {
            ext.to_string()
        };
        format!(
            "memory override: extension={} deployment={}",
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
    /// The configured memory provider extension id (`ironclaw.memory`,
    /// `memory.disabled`, or a third-party id). `None` binds native by default.
    pub provider: Option<String>,
    pub overrides: Vec<MemoryAdminOverrideEntry>,
}

impl MemoryBindingInput {
    /// Default input: no explicit provider, native available.
    pub fn native_default(deployment: MemoryDeploymentProfile) -> Self {
        Self {
            deployment,
            native_available: true,
            provider: None,
            overrides: Vec::new(),
        }
    }
}

/// Fail-closed reasons a memory binding cannot resolve.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum MemoryBindingError {
    #[error("memory.disabled is not allowed in deployment '{deployment}'")]
    DisabledInProduction { deployment: &'static str },
    #[error(
        "binding third-party memory extension '{extension}' in deployment '{deployment}' requires an explicit admin override"
    )]
    ThirdPartyRequiresOverride {
        extension: String,
        deployment: &'static str,
    },
    #[error("native memory provider is required but is not available")]
    NativeUnavailable,
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

/// Resolved, validated memory binding policy. The dispatch site consults
/// [`binding`](Self::binding) instead of hardwiring the native provider.
#[derive(Debug, Clone)]
pub struct MemoryBindingPolicy {
    binding: MemoryProviderBinding,
    /// The applied override (0 or 1), stored as a slice for redacted diagnostics.
    active_overrides: Vec<MemoryActiveOverride>,
}

impl MemoryBindingPolicy {
    /// Memory bound to the native provider — the behavior-preserving default
    /// when no `[memory]` config is present.
    pub fn native_default() -> Self {
        Self {
            binding: MemoryProviderBinding::Native,
            active_overrides: Vec::new(),
        }
    }

    /// Resolve a policy from the configured provider selection + overrides,
    /// failing closed.
    pub fn resolve(input: MemoryBindingInput) -> Result<Self, MemoryBindingError> {
        let mut active_overrides = Vec::new();
        let binding = match input.provider.as_deref() {
            None => native_binding(input.native_available)?,
            Some(extension_id) if extension_id == NATIVE_MEMORY_EXTENSION_ID => {
                native_binding(input.native_available)?
            }
            Some(extension_id) if extension_id == MEMORY_DISABLED_BINDING_SENTINEL => {
                if input.deployment.requires_certified_bindings() {
                    return Err(MemoryBindingError::DisabledInProduction {
                        deployment: input.deployment.as_str(),
                    });
                }
                MemoryProviderBinding::Disabled
            }
            Some(extension_id) => resolve_third_party(extension_id, &input, &mut active_overrides)?,
        };
        Ok(Self {
            binding,
            active_overrides,
        })
    }

    /// The resolved memory provider binding.
    pub fn binding(&self) -> &MemoryProviderBinding {
        &self.binding
    }

    /// Overrides actually applied during resolution (for redacted diagnostics).
    pub fn active_overrides(&self) -> &[MemoryActiveOverride] {
        &self.active_overrides
    }

    /// Whether a third-party override is active (production diagnostics flag).
    pub fn has_active_overrides(&self) -> bool {
        !self.active_overrides.is_empty()
    }
}

fn native_binding(native_available: bool) -> Result<MemoryProviderBinding, MemoryBindingError> {
    if native_available {
        Ok(MemoryProviderBinding::Native)
    } else {
        Err(MemoryBindingError::NativeUnavailable)
    }
}

fn resolve_third_party(
    extension_id: &str,
    input: &MemoryBindingInput,
    active_overrides: &mut Vec<MemoryActiveOverride>,
) -> Result<MemoryProviderBinding, MemoryBindingError> {
    let parsed = ExtensionId::new(extension_id)
        .map_err(|error| MemoryBindingError::invalid_extension(extension_id, error))?;

    if input.deployment.requires_certified_bindings() {
        let has_override = input.overrides.iter().any(|over| {
            over.extension_id == extension_id
                && (over.deployment_profile == input.deployment.as_str()
                    || over.deployment_profile == "*")
        });
        if !has_override {
            return Err(MemoryBindingError::ThirdPartyRequiresOverride {
                extension: extension_id.to_string(),
                deployment: input.deployment.as_str(),
            });
        }
        active_overrides.push(MemoryActiveOverride {
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

    const MEM0: &str = "mem0";

    #[test]
    fn native_default_binds_native() {
        let policy = MemoryBindingPolicy::native_default();
        assert_eq!(policy.binding(), &MemoryProviderBinding::Native);
        assert!(!policy.has_active_overrides());
    }

    #[test]
    fn unconfigured_resolves_to_native() {
        let policy = MemoryBindingPolicy::resolve(MemoryBindingInput::native_default(
            MemoryDeploymentProfile::LocalDev,
        ))
        .unwrap();
        assert_eq!(policy.binding(), &MemoryProviderBinding::Native);
    }

    #[test]
    fn explicit_native_id_resolves_to_native() {
        let policy = MemoryBindingPolicy::resolve(MemoryBindingInput {
            provider: Some(NATIVE_MEMORY_EXTENSION_ID.to_string()),
            ..MemoryBindingInput::native_default(MemoryDeploymentProfile::Production)
        })
        .unwrap();
        assert_eq!(policy.binding(), &MemoryProviderBinding::Native);
    }

    #[test]
    fn disabled_allowed_outside_production() {
        let policy = MemoryBindingPolicy::resolve(MemoryBindingInput {
            provider: Some(MEMORY_DISABLED_BINDING_SENTINEL.to_string()),
            ..MemoryBindingInput::native_default(MemoryDeploymentProfile::LocalDev)
        })
        .unwrap();
        assert_eq!(policy.binding(), &MemoryProviderBinding::Disabled);
    }

    #[test]
    fn disabled_rejected_in_production() {
        let err = MemoryBindingPolicy::resolve(MemoryBindingInput {
            provider: Some(MEMORY_DISABLED_BINDING_SENTINEL.to_string()),
            ..MemoryBindingInput::native_default(MemoryDeploymentProfile::Production)
        })
        .unwrap_err();
        assert!(matches!(
            err,
            MemoryBindingError::DisabledInProduction { .. }
        ));
    }

    #[test]
    fn third_party_allowed_outside_production_without_override() {
        let policy = MemoryBindingPolicy::resolve(MemoryBindingInput {
            provider: Some(MEM0.to_string()),
            ..MemoryBindingInput::native_default(MemoryDeploymentProfile::LocalDev)
        })
        .unwrap();
        assert!(matches!(
            policy.binding(),
            MemoryProviderBinding::ThirdParty { extension_id } if extension_id.as_str() == MEM0
        ));
        assert!(!policy.has_active_overrides());
    }

    #[test]
    fn third_party_rejected_in_production_without_override() {
        let err = MemoryBindingPolicy::resolve(MemoryBindingInput {
            provider: Some(MEM0.to_string()),
            ..MemoryBindingInput::native_default(MemoryDeploymentProfile::Production)
        })
        .unwrap_err();
        assert!(matches!(
            err,
            MemoryBindingError::ThirdPartyRequiresOverride { .. }
        ));
    }

    #[test]
    fn third_party_allowed_in_production_with_override() {
        let policy = MemoryBindingPolicy::resolve(MemoryBindingInput {
            provider: Some(MEM0.to_string()),
            overrides: vec![MemoryAdminOverrideEntry {
                extension_id: MEM0.to_string(),
                deployment_profile: "production".to_string(),
            }],
            ..MemoryBindingInput::native_default(MemoryDeploymentProfile::Production)
        })
        .unwrap();
        assert!(matches!(
            policy.binding(),
            MemoryProviderBinding::ThirdParty { .. }
        ));
        assert!(policy.has_active_overrides());
        assert_eq!(policy.active_overrides()[0].extension_id.as_str(), MEM0);
    }

    #[test]
    fn wildcard_override_applies_in_production() {
        let policy = MemoryBindingPolicy::resolve(MemoryBindingInput {
            provider: Some(MEM0.to_string()),
            overrides: vec![MemoryAdminOverrideEntry {
                extension_id: MEM0.to_string(),
                deployment_profile: "*".to_string(),
            }],
            ..MemoryBindingInput::native_default(MemoryDeploymentProfile::MigrationDryRun)
        })
        .unwrap();
        assert!(matches!(
            policy.binding(),
            MemoryProviderBinding::ThirdParty { .. }
        ));
    }

    #[test]
    fn native_unavailable_is_an_error() {
        let err = MemoryBindingPolicy::resolve(MemoryBindingInput {
            native_available: false,
            ..MemoryBindingInput::native_default(MemoryDeploymentProfile::LocalDev)
        })
        .unwrap_err();
        assert!(matches!(err, MemoryBindingError::NativeUnavailable));
    }

    #[test]
    fn invalid_extension_id_is_rejected() {
        let err = MemoryBindingPolicy::resolve(MemoryBindingInput {
            provider: Some("Not A Valid Id".to_string()),
            ..MemoryBindingInput::native_default(MemoryDeploymentProfile::LocalDev)
        })
        .unwrap_err();
        assert!(matches!(err, MemoryBindingError::InvalidExtensionId { .. }));
    }
}
