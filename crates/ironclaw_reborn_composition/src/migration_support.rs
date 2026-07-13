//! Non-activating target configuration resolution for migration.
//!
//! This module deliberately returns configuration only. It does not open the
//! target filesystem, run schema migrations, construct workers, or start
//! ingress. PostgreSQL locator/key selection is shared with runtime
//! composition, and local target paths use the canonical profile layout.
//! Profile and identity precedence mirror the standalone CLI and are parity
//! tested here; they are not yet selected through one shared CLI/composition
//! entry point. The migration companion owns the legacy bridge and maps these
//! values into its target selector.

#[cfg(feature = "libsql")]
use std::path::Path;
#[cfg(feature = "libsql")]
use std::path::PathBuf;

use ironclaw_host_api::{AgentId, TenantId};
use ironclaw_reborn_config::{
    REBORN_PROFILE_ENV, RebornBootConfig, RebornConfigFile, RebornProfile,
};
use ironclaw_secrets::SecretMaterial;
#[cfg(feature = "postgres")]
use secrecy::ExposeSecret as _;

use crate::{RebornBuildError, RebornRuntimeIdentity};
#[cfg(feature = "postgres")]
use crate::{RebornCompositionProfile, input};

/// Production-selected durable target, expressed without a dependency on the
/// migration crate (which itself depends on composition).
#[derive(Clone)]
pub enum RebornMigrationTargetStore {
    /// Canonical local/volume-backed target database path.
    #[cfg(feature = "libsql")]
    LibSql {
        /// Database path selected by the production profile layout.
        path: PathBuf,
    },
    /// Secret-bearing production PostgreSQL locator. Callers must not log or
    /// serialize it into migration artifacts.
    #[cfg(feature = "postgres")]
    Postgres {
        /// Production target URL held in redacting secret material.
        url: SecretMaterial,
    },
}

/// Resolved migration target scope and encryption material.
#[derive(Clone)]
pub struct RebornMigrationTargetConfig {
    /// Effective production profile.
    pub profile: RebornProfile,
    /// Config-only target selector; resolving it does not open the store.
    pub store: RebornMigrationTargetStore,
    /// Target tenant scope.
    pub tenant_id: TenantId,
    /// Target agent scope.
    pub agent_id: AgentId,
    /// PostgreSQL resolves its configured key without opening the database.
    /// Local libSQL resolves/generates the cached production key only after
    /// apply preconditions, when the migration target is opened.
    pub target_master_key: Option<SecretMaterial>,
}

/// Resolve migration target configuration without activating the runtime.
///
/// Storage locator/key rules are shared with runtime composition. Profile and
/// identity precedence intentionally mirror the standalone CLI until the CLI
/// routes both live boot and migration through a single resolver.
pub fn resolve_reborn_migration_target(
    boot: &RebornBootConfig,
) -> Result<RebornMigrationTargetConfig, RebornBuildError> {
    let config_file = RebornConfigFile::load(&boot.home().config_file_path()).map_err(|error| {
        RebornBuildError::InvalidConfig {
            reason: format!("Reborn migration target config could not be loaded: {error}"),
        }
    })?;
    let profile = effective_profile(boot, config_file.as_ref())?;
    if config_file
        .as_ref()
        .and_then(|file| file.storage.as_ref())
        .is_some()
        && matches!(
            profile,
            RebornProfile::LocalDev
                | RebornProfile::LocalDevYolo
                | RebornProfile::HostedSingleTenantVolume
        )
    {
        return Err(RebornBuildError::InvalidConfig {
            reason: format!(
                "config file [storage] is not wired for profile={profile}; migration target resolution refuses to ignore it"
            ),
        });
    }
    let identity = runtime_identity(config_file.as_ref());
    let tenant_id =
        TenantId::new(identity.tenant_id).map_err(|error| RebornBuildError::InvalidConfig {
            reason: format!("invalid migration target tenant identity: {error}"),
        })?;
    let agent_id =
        AgentId::new(identity.agent_id).map_err(|error| RebornBuildError::InvalidConfig {
            reason: format!("invalid migration target agent identity: {error}"),
        })?;
    let (store, target_master_key) = match profile {
        RebornProfile::LocalDev
        | RebornProfile::LocalDevYolo
        | RebornProfile::HostedSingleTenantVolume => {
            #[cfg(feature = "libsql")]
            {
                let root = boot
                    .home()
                    .path()
                    .join(profile.local_runtime_storage_subdir());
                (
                    RebornMigrationTargetStore::LibSql {
                        path: root.join(crate::factory::LOCAL_DEV_DB_FILENAME),
                    },
                    None,
                )
            }
            #[cfg(not(feature = "libsql"))]
            {
                return Err(RebornBuildError::InvalidConfig {
                    reason: format!(
                        "profile={profile} migration target requires a binary built with libsql"
                    ),
                });
            }
        }
        RebornProfile::HostedSingleTenant
        | RebornProfile::Production
        | RebornProfile::MigrationDryRun => {
            #[cfg(feature = "postgres")]
            {
                let composition_profile = profile
                    .as_str()
                    .parse::<RebornCompositionProfile>()
                    .map_err(|error| RebornBuildError::InvalidConfig {
                        reason: format!("invalid migration composition profile: {error}"),
                    })?;
                let target = input::resolve_postgres_migration_target(
                    composition_profile,
                    config_file.as_ref(),
                )?;
                (
                    RebornMigrationTargetStore::Postgres { url: target.url },
                    Some(target.secret_master_key),
                )
            }
            #[cfg(not(feature = "postgres"))]
            {
                return Err(RebornBuildError::InvalidConfig {
                    reason: format!(
                        "profile={profile} migration target requires a binary built with postgres"
                    ),
                });
            }
        }
    };

    Ok(RebornMigrationTargetConfig {
        profile,
        store,
        tenant_id,
        agent_id,
        target_master_key,
    })
}

/// Compute the redacted local target identity consumed by the migration
/// companion and CLI activation guard. The composition public-surface snapshot
/// pins this shared boundary so both callers compare the same locator.
#[cfg(feature = "libsql")]
pub fn migration_libsql_locator_fingerprint(path: &Path) -> String {
    use std::path::Component;

    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(path)
    };
    let mut normalized = PathBuf::new();
    for component in absolute.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            other => normalized.push(other.as_os_str()),
        }
    }
    let mut ancestor = normalized.clone();
    let mut missing_suffix = Vec::new();
    while !ancestor.exists() {
        let Some(name) = ancestor.file_name() else {
            break;
        };
        missing_suffix.push(name.to_os_string());
        if !ancestor.pop() {
            break;
        }
    }
    let mut resolved = ancestor.canonicalize().unwrap_or(ancestor);
    for component in missing_suffix.into_iter().rev() {
        resolved.push(component);
    }
    ironclaw_common::hashing::sha256_hex(resolved.as_os_str().as_encoded_bytes())
}

/// Compute the credential-free PostgreSQL target identity consumed by the
/// migration companion and CLI activation guard.
#[cfg(feature = "postgres")]
pub fn migration_postgres_locator_fingerprint(
    locator: &SecretMaterial,
) -> Result<String, RebornBuildError> {
    use deadpool_postgres::tokio_postgres::config::Host;

    let config = locator
        .expose_secret()
        .parse::<deadpool_postgres::tokio_postgres::Config>()
        .map_err(|_| RebornBuildError::InvalidConfig {
            reason: "PostgreSQL migration target locator is invalid (details redacted)".to_string(),
        })?;
    if config.get_options().is_some() {
        return Err(RebornBuildError::InvalidConfig {
            reason: "PostgreSQL connection options are not supported for migration target identity (details redacted)".to_string(),
        });
    }
    let mut material = Vec::new();
    append_locator_field(&mut material, b"schema", b"postgres-locator-v1");
    append_locator_field(
        &mut material,
        b"database",
        config
            .get_dbname()
            .or_else(|| config.get_user())
            .unwrap_or_default()
            .as_bytes(),
    );
    for (index, host) in config.get_hosts().iter().enumerate() {
        let label = format!("host-{index}");
        match host {
            Host::Tcp(host) => {
                append_locator_field(&mut material, label.as_bytes(), host.as_bytes())
            }
            #[cfg(unix)]
            Host::Unix(path) => append_locator_field(
                &mut material,
                label.as_bytes(),
                path.as_os_str().as_encoded_bytes(),
            ),
        }
        let port = config.get_ports().get(index).copied().unwrap_or(5432);
        append_locator_field(
            &mut material,
            format!("port-{index}").as_bytes(),
            port.to_string().as_bytes(),
        );
    }
    for (index, address) in config.get_hostaddrs().iter().enumerate() {
        append_locator_field(
            &mut material,
            format!("hostaddr-{index}").as_bytes(),
            address.to_string().as_bytes(),
        );
    }
    Ok(ironclaw_common::hashing::sha256_hex(&material))
}

#[cfg(feature = "postgres")]
fn append_locator_field(material: &mut Vec<u8>, label: &[u8], value: &[u8]) {
    material.extend_from_slice(label.len().to_string().as_bytes());
    material.push(b':');
    material.extend_from_slice(label);
    material.extend_from_slice(value.len().to_string().as_bytes());
    material.push(b':');
    material.extend_from_slice(value);
}

/// Resolve or create the cached local-runtime master key. Call this only from
/// the apply path after all source/manifest preconditions have passed and the
/// target database parent directory exists.
#[cfg(feature = "libsql")]
pub fn resolve_local_migration_target_key(
    target_database_path: &Path,
) -> Result<SecretMaterial, RebornBuildError> {
    let root = target_database_path
        .parent()
        .ok_or_else(|| RebornBuildError::InvalidConfig {
            reason: "local migration target database has no parent directory".to_string(),
        })?;
    crate::factory::resolve_local_dev_secret_master_key(root)
}

fn effective_profile(
    boot: &RebornBootConfig,
    config_file: Option<&RebornConfigFile>,
) -> Result<RebornProfile, RebornBuildError> {
    if std::env::var_os(REBORN_PROFILE_ENV).is_some() {
        return Ok(boot.profile());
    }
    let Some(profile) = config_file
        .and_then(|file| file.boot.as_ref())
        .and_then(|section| section.profile.as_deref())
    else {
        return Ok(boot.profile());
    };
    profile
        .parse::<RebornProfile>()
        .map_err(|error| RebornBuildError::InvalidConfig {
            reason: format!("config file [boot].profile `{profile}` is invalid: {error}"),
        })
}

fn runtime_identity(config_file: Option<&RebornConfigFile>) -> RebornRuntimeIdentity {
    let default = RebornRuntimeIdentity::reborn_cli();
    let Some(identity) = config_file.and_then(|file| file.identity.as_ref()) else {
        return default;
    };
    RebornRuntimeIdentity {
        tenant_id: identity
            .tenant
            .clone()
            .unwrap_or_else(|| default.tenant_id.clone()),
        agent_id: identity
            .default_agent
            .clone()
            .unwrap_or_else(|| default.agent_id.clone()),
        source_binding_id: default.source_binding_id,
        reply_target_binding_id: default.reply_target_binding_id,
    }
}

#[cfg(test)]
mod tests {
    #[cfg(feature = "libsql")]
    use std::fs;

    #[cfg(feature = "libsql")]
    use ironclaw_reborn_config::{RebornBootConfig, RebornHome, RebornProfile};

    #[cfg(feature = "libsql")]
    use super::{RebornMigrationTargetStore, resolve_reborn_migration_target};

    #[cfg(feature = "libsql")]
    #[test]
    fn local_target_resolution_matches_runtime_layout_without_creating_state() {
        let temp = tempfile::tempdir().unwrap();
        let home_path = temp.path().join("reborn-home");
        let home = RebornHome::resolve_from_env_parts(
            Some(home_path.clone().into_os_string()),
            None,
            None,
        )
        .unwrap();
        let boot = RebornBootConfig::new(home, RebornProfile::LocalDev);

        let resolved = resolve_reborn_migration_target(&boot).unwrap();

        let path = match resolved.store {
            RebornMigrationTargetStore::LibSql { path } => path,
            #[cfg(feature = "postgres")]
            RebornMigrationTargetStore::Postgres { .. } => {
                panic!("local-dev migration target must be libSQL")
            }
        };
        assert_eq!(path, home_path.join("local-dev/reborn-local-dev.db"));
        assert_eq!(resolved.tenant_id.as_str(), "reborn-cli");
        assert_eq!(resolved.agent_id.as_str(), "reborn-cli-agent");
        assert!(resolved.target_master_key.is_none());
        assert!(
            !home_path.exists(),
            "resolution must not create target state"
        );
    }

    #[cfg(feature = "libsql")]
    #[test]
    fn config_profile_and_identity_select_the_same_local_target_scope_as_boot() {
        let temp = tempfile::tempdir().unwrap();
        let home_path = temp.path().join("reborn-home");
        fs::create_dir_all(&home_path).unwrap();
        fs::write(
            home_path.join("config.toml"),
            r#"
[boot]
profile = "hosted-single-tenant-volume"

[identity]
tenant = "migrated-tenant"
default_agent = "migrated-agent"
"#,
        )
        .unwrap();
        let home = RebornHome::resolve_from_env_parts(
            Some(home_path.clone().into_os_string()),
            None,
            None,
        )
        .unwrap();
        let boot = RebornBootConfig::new(home, RebornProfile::LocalDev);

        let resolved = resolve_reborn_migration_target(&boot).unwrap();

        let path = match resolved.store {
            RebornMigrationTargetStore::LibSql { path } => path,
            #[cfg(feature = "postgres")]
            RebornMigrationTargetStore::Postgres { .. } => {
                panic!("hosted volume migration target must be libSQL")
            }
        };
        assert_eq!(
            path,
            home_path.join("hosted-single-tenant-volume/reborn-local-dev.db")
        );
        assert_eq!(resolved.profile, RebornProfile::HostedSingleTenantVolume);
        assert_eq!(resolved.tenant_id.as_str(), "migrated-tenant");
        assert_eq!(resolved.agent_id.as_str(), "migrated-agent");
    }

    #[cfg(feature = "libsql")]
    #[test]
    fn local_profile_rejects_storage_config_instead_of_ignoring_it() {
        let temp = tempfile::tempdir().unwrap();
        let home_path = temp.path().join("reborn-home");
        fs::create_dir_all(&home_path).unwrap();
        fs::write(
            home_path.join("config.toml"),
            r#"
[storage]
backend = "postgres"
url_env = "IGNORED_DATABASE_URL"
"#,
        )
        .unwrap();
        let home = RebornHome::resolve_from_env_parts(Some(home_path.into_os_string()), None, None)
            .unwrap();
        let boot = RebornBootConfig::new(home, RebornProfile::LocalDev);

        let error = resolve_reborn_migration_target(&boot)
            .err()
            .expect("storage config must fail closed for a local target");

        assert!(error.to_string().contains("refuses to ignore it"));
    }
}
