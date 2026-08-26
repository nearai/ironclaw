//! CLI-facing durable-storage boot orchestration.
//!
//! This module maps an active deployment profile onto the pure transition
//! machinery in [`super::storage_layout`]. It owns command and startup
//! sequencing; `storage_layout` owns filesystem-state admission and the
//! one-shot legacy migration.

use ironclaw_composition::deployment::DeploymentConfig;
use ironclaw_config::{
    DurableStateKind, LayoutRequirement, RebornBootConfig, RebornProfile, RebornStoragePaths,
};

use super::{effective_profile, read_config_file, storage_layout};

pub(super) fn storage_layout_requirement_for_profile(
    profile: RebornProfile,
) -> anyhow::Result<LayoutRequirement> {
    storage_layout_requirement_for_profile_with_workspace_scope(profile, false)
}

fn storage_layout_requirement_for_profile_with_workspace_scope(
    profile: RebornProfile,
    require_per_caller_workspace: bool,
) -> anyhow::Result<LayoutRequirement> {
    let deployment = DeploymentConfig::for_profile(profile.into(), false)
        .with_workspace_scoped_per_caller(require_per_caller_workspace);
    deployment.storage_layout_requirement().ok_or_else(|| {
        anyhow::anyhow!(
            "profile {} has no durable filesystem layout to migrate",
            profile
        )
    })
}

pub(super) fn storage_migration_policy_from_environment()
-> anyhow::Result<storage_layout::StorageMigrationPolicy> {
    let value = match std::env::var(storage_layout::StorageMigrationPolicy::ENV) {
        Ok(value) => Some(value),
        Err(std::env::VarError::NotPresent) => None,
        Err(std::env::VarError::NotUnicode(_)) => anyhow::bail!(
            "{} is invalid: the value must be valid UTF-8",
            storage_layout::StorageMigrationPolicy::ENV
        ),
    };
    storage_layout::StorageMigrationPolicy::from_environment_value(value.as_deref())
}

pub(crate) fn ensure_ready_layout_for_profile(
    config: &RebornBootConfig,
    profile: RebornProfile,
) -> anyhow::Result<RebornStoragePaths> {
    let requirement = storage_layout_requirement_for_profile(profile)?;
    if profile == RebornProfile::MigrationDryRun {
        return storage_layout::inspect_ready_layout(config.home(), requirement);
    }
    storage_layout::ensure_ready_layout(config.home(), requirement)
}

pub(super) fn ensure_startup_layout(
    config: &RebornBootConfig,
    profile: RebornProfile,
    require_per_caller_workspace: bool,
) -> anyhow::Result<RebornStoragePaths> {
    let requirement = storage_layout_requirement_for_profile_with_workspace_scope(
        profile,
        require_per_caller_workspace,
    )?;
    if profile == RebornProfile::MigrationDryRun {
        return storage_layout::inspect_ready_layout(config.home(), requirement);
    }
    match storage_layout::admit_startup_layout(config.home(), requirement)? {
        storage_layout::StartupLayoutAdmission::Ready(paths) => Ok(paths),
        storage_layout::StartupLayoutAdmission::MigrationRequired(candidates) => {
            let policy = storage_migration_policy_from_environment()?;
            storage_layout::migrate_legacy_layout(config.home(), requirement, policy, candidates)?;
            storage_layout::ensure_ready_layout(config.home(), requirement)
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OnboardingSecretStoreMode {
    Embedded,
    HostedExternal,
}

/// Admit the active layout before onboarding writes home-local configuration.
/// Only embedded profiles may provision the standalone master key or encrypted
/// secret store; hosted onboarding leaves credentials to its hosted surface.
pub(crate) fn prepare_onboarding_layout(
    config: &RebornBootConfig,
    replace_existing_config: bool,
) -> anyhow::Result<OnboardingSecretStoreMode> {
    // `onboard --force` promises to replace an existing config file, including
    // one that is malformed. Admit against the already validated env/default
    // profile in that case so the broken file cannot prevent its own repair.
    // The non-force path still parses before any write and fails closed.
    let profile = if replace_existing_config {
        config.profile()
    } else {
        let config_file = read_config_file(config)?;
        effective_profile(config, config_file.as_ref())?
    };
    let requirement = storage_layout_requirement_for_profile(profile)?;
    ensure_ready_layout_for_profile(config, profile)?;
    Ok(match requirement.durable_state {
        DurableStateKind::EmbeddedLibSql => OnboardingSecretStoreMode::Embedded,
        DurableStateKind::ExternalPostgres => OnboardingSecretStoreMode::HostedExternal,
    })
}

/// Admit the active profile's durable layout before a CLI command opens a
/// stateful store outside the runtime assembly path.
#[cfg(test)]
pub(crate) fn ensure_ready_layout_for_active_profile(
    config: &RebornBootConfig,
) -> anyhow::Result<RebornStoragePaths> {
    let config_file = read_config_file(config)?;
    let profile = effective_profile(config, config_file.as_ref())?;
    ensure_ready_layout_for_profile(config, profile)
}

/// Admit a CLI secret write only when it can open the same embedded store as
/// the selected runtime. Hosted PostgreSQL writes must use a PostgreSQL-aware
/// operator surface; silently creating a local libSQL database would report a
/// credential as saved while `serve` reads a different backend.
pub(crate) fn ensure_embedded_secret_store_for_active_profile(
    config: &RebornBootConfig,
) -> anyhow::Result<RebornStoragePaths> {
    let config_file = read_config_file(config)?;
    let profile = effective_profile(config, config_file.as_ref())?;
    let requirement = storage_layout_requirement_for_profile(profile)?;
    if requirement.durable_state != DurableStateKind::EmbeddedLibSql {
        anyhow::bail!(
            "profile {profile} uses external PostgreSQL durable state; this CLI secret command cannot safely write that backend. Configure the credential through the hosted operator/WebUI surface or deployment secret environment"
        );
    }
    ensure_ready_layout_for_profile(config, profile)
}

#[cfg(test)]
mod tests {
    #[cfg(unix)]
    #[test]
    fn storage_migration_policy_rejects_a_non_utf8_value() {
        use std::os::unix::ffi::OsStringExt as _;

        use super::super::test_env::{EnvGuard, lock_runtime_env};
        use super::{storage_layout, storage_migration_policy_from_environment};

        let _lock = lock_runtime_env();
        let invalid = std::ffi::OsString::from_vec(vec![0xff, 0xfe]);
        let _policy = EnvGuard::set_os(storage_layout::StorageMigrationPolicy::ENV, &invalid);

        let error = storage_migration_policy_from_environment()
            .expect_err("non-UTF-8 migration policy must fail loudly");
        assert!(
            error
                .to_string()
                .contains(storage_layout::StorageMigrationPolicy::ENV),
            "{error:#}"
        );
        assert!(error.to_string().contains("UTF-8"), "{error:#}");
    }
}
