//! Composition glue for the memory profile binding (issue #3537).
//!
//! Maps the `[memory]` config section + the active deployment profile into a
//! resolved [`MemoryBindingPolicy`], failing closed (a production deployment
//! that binds a required memory profile to `memory.disabled` or an unverified
//! third-party extension without an admin override is a startup error). Also
//! renders the redacted active-override diagnostics for startup/doctor.

use ironclaw_host_runtime::memory_binding::{
    MemoryAdminOverrideEntry, MemoryBindingInput, MemoryBindingPolicy, MemoryDeploymentProfile,
};
use ironclaw_reborn_config::MemorySection;

use crate::RebornBuildError;
use crate::root::profile::RebornCompositionProfile;

fn deployment_profile(profile: RebornCompositionProfile) -> MemoryDeploymentProfile {
    match profile {
        // `Disabled` never reaches memory composition; treat it as the safest
        // non-production profile so a stray call cannot relax production rules.
        RebornCompositionProfile::Disabled | RebornCompositionProfile::LocalDev => {
            MemoryDeploymentProfile::LocalDev
        }
        RebornCompositionProfile::LocalDevYolo => MemoryDeploymentProfile::LocalDevYolo,
        // Volume-backed hosted single-tenant shares the same single-tenant trust
        // model as plain hosted-single-tenant, so it gets the same memory
        // deployment classification (and the same binding-certification rules).
        RebornCompositionProfile::HostedSingleTenant
        | RebornCompositionProfile::HostedSingleTenantVolume => {
            MemoryDeploymentProfile::HostedSingleTenant
        }
        RebornCompositionProfile::Production => MemoryDeploymentProfile::Production,
        RebornCompositionProfile::MigrationDryRun => MemoryDeploymentProfile::MigrationDryRun,
    }
}

/// Resolve the memory binding policy from config + deployment profile,
/// fail-closed. `None` config binds the native provider by default.
pub fn resolve_memory_binding_policy(
    memory: Option<&MemorySection>,
    profile: RebornCompositionProfile,
) -> Result<MemoryBindingPolicy, RebornBuildError> {
    let Some(memory) = memory else {
        return Ok(MemoryBindingPolicy::native_default());
    };

    let mut overrides = Vec::with_capacity(memory.admin_overrides.len());
    for over in &memory.admin_overrides {
        overrides.push(MemoryAdminOverrideEntry {
            extension_id: over.extension_id.clone(),
            deployment_profile: over.deployment_profile.clone(),
        });
    }

    let input = MemoryBindingInput {
        deployment: deployment_profile(profile),
        native_available: true,
        provider: memory.provider.clone(),
        overrides,
    };
    MemoryBindingPolicy::resolve(input).map_err(map_binding_error)
}

fn map_binding_error(
    error: ironclaw_host_runtime::memory_binding::MemoryBindingError,
) -> RebornBuildError {
    RebornBuildError::InvalidConfig {
        reason: format!("memory binding resolution failed: {error}"),
    }
}

/// Redacted one-line diagnostics for the active third-party binding overrides,
/// for startup logging and the doctor surface.
pub fn memory_binding_diagnostics(policy: &MemoryBindingPolicy) -> Vec<String> {
    policy
        .active_overrides()
        .iter()
        .map(|over| over.redacted_summary())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironclaw_reborn_config::MemoryAdminOverride;

    fn section(provider: Option<&str>, admin_overrides: Vec<MemoryAdminOverride>) -> MemorySection {
        MemorySection {
            provider: provider.map(|value| value.to_string()),
            admin_overrides,
            ..Default::default()
        }
    }

    #[test]
    fn none_config_resolves_to_native_default() {
        let policy = resolve_memory_binding_policy(None, RebornCompositionProfile::Production)
            .expect("default native resolves in production");
        assert!(!policy.has_active_overrides());
    }

    #[test]
    fn production_disabled_binding_fails_startup() {
        let memory = section(Some("memory.disabled"), Vec::new());
        let err =
            resolve_memory_binding_policy(Some(&memory), RebornCompositionProfile::Production)
                .expect_err("production must reject memory.disabled");
        assert!(matches!(err, RebornBuildError::InvalidConfig { .. }));
    }

    #[test]
    fn production_third_party_without_override_fails_startup() {
        let memory = section(Some("acme.honcho"), Vec::new());
        let err =
            resolve_memory_binding_policy(Some(&memory), RebornCompositionProfile::Production)
                .expect_err("production must reject unverified third-party");
        assert!(matches!(err, RebornBuildError::InvalidConfig { .. }));
    }

    #[test]
    fn production_third_party_with_override_resolves_and_reports_redacted() {
        let memory = section(
            Some("acme.honcho"),
            vec![MemoryAdminOverride {
                extension_id: "acme.honcho".to_string(),
                deployment_profile: "production".to_string(),
            }],
        );
        let policy =
            resolve_memory_binding_policy(Some(&memory), RebornCompositionProfile::Production)
                .expect("override permits binding");
        let diagnostics = memory_binding_diagnostics(&policy);
        assert_eq!(diagnostics.len(), 1);
        assert!(diagnostics[0].contains("memory override"));
        assert!(diagnostics[0].contains("deployment=production"));
    }

    #[test]
    fn invalid_provider_id_fails_startup() {
        let memory = section(Some("not a valid id"), Vec::new());
        let err = resolve_memory_binding_policy(Some(&memory), RebornCompositionProfile::LocalDev)
            .expect_err("invalid provider id rejected");
        assert!(matches!(err, RebornBuildError::InvalidConfig { .. }));
    }
}
