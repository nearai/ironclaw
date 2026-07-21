use std::collections::BTreeSet;

use ironclaw_extensions::{ExtensionInstallation, ExtensionInstallationStore, InstallationOwner};
use ironclaw_host_api::{ExtensionId, TenantId, UserId};
use serde::Serialize;

use crate::error::MigrationError;
use crate::options::TargetStore;
use crate::target::ExtensionOwnershipTarget;

const USER_PAGE_SIZE: usize = 200;

/// Inputs for the explicit, one-time extension ownership rewrite.
#[derive(Clone)]
pub struct ExtensionOwnershipMigrationOptions {
    pub target: TargetStore,
    pub tenant_id: TenantId,
    pub include_users: BTreeSet<UserId>,
    pub dry_run: bool,
}

/// Auditable output containing identifiers only; no credentials or profiles.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ExtensionOwnershipMigrationReport {
    pub tenant_id: TenantId,
    pub user_ids: Vec<UserId>,
    pub installed_extension_ids: Vec<ExtensionId>,
    pub changed_extension_ids: Vec<ExtensionId>,
    pub dry_run: bool,
}

pub async fn run_extension_ownership_migration(
    options: ExtensionOwnershipMigrationOptions,
) -> Result<ExtensionOwnershipMigrationReport, MigrationError> {
    let target = ExtensionOwnershipTarget::open(&options.target, &options.tenant_id).await?;
    let mut user_ids = list_all_user_ids(&target, &options.tenant_id).await?;
    user_ids.extend(options.include_users);

    rewrite_store(
        target.extension_store.as_ref(),
        options.tenant_id,
        user_ids,
        options.dry_run,
    )
    .await
}

async fn list_all_user_ids(
    target: &ExtensionOwnershipTarget,
    tenant_id: &TenantId,
) -> Result<BTreeSet<UserId>, MigrationError> {
    let mut user_ids = BTreeSet::new();
    let mut after = None;
    loop {
        let page = target
            .user_directory
            .list_users(tenant_id, None, after.as_ref(), USER_PAGE_SIZE)
            .await
            .map_err(|error| MigrationError::ReadTarget {
                domain: "user directory".to_string(),
                reason: error.to_string(),
            })?;
        let Some(last) = page.last() else {
            break;
        };
        after = Some(last.user_id.clone());
        user_ids.extend(page.into_iter().map(|user| user.user_id));
    }
    Ok(user_ids)
}

async fn rewrite_store(
    store: &dyn ExtensionInstallationStore,
    tenant_id: TenantId,
    user_ids: BTreeSet<UserId>,
    dry_run: bool,
) -> Result<ExtensionOwnershipMigrationReport, MigrationError> {
    let installations =
        store
            .list_installations()
            .await
            .map_err(|error| MigrationError::ReadTarget {
                domain: "extension installations".to_string(),
                reason: error.to_string(),
            })?;
    if !installations.is_empty() && user_ids.is_empty() {
        return Err(MigrationError::InvalidInput(
            "cannot assign installed extensions because no tenant users were found; pass at least one --include-user"
                .to_string(),
        ));
    }

    let mut installed_extension_ids = installations
        .iter()
        .map(|installation| installation.extension_id().clone())
        .collect::<Vec<_>>();
    installed_extension_ids.sort();
    installed_extension_ids.dedup();

    let rewritten = rewrite_installation_owners(installations.clone(), &user_ids);
    let mut changed_extension_ids = Vec::new();
    for (before, after) in installations.into_iter().zip(rewritten) {
        if before.owner() == after.owner() {
            continue;
        }
        changed_extension_ids.push(after.extension_id().clone());
        if !dry_run {
            store.upsert_installation(after).await.map_err(|error| {
                MigrationError::WriteTarget {
                    domain: "extension ownership".to_string(),
                    reason: error.to_string(),
                }
            })?;
        }
    }
    changed_extension_ids.sort();
    changed_extension_ids.dedup();

    Ok(ExtensionOwnershipMigrationReport {
        tenant_id,
        user_ids: user_ids.into_iter().collect(),
        installed_extension_ids,
        changed_extension_ids,
        dry_run,
    })
}

fn rewrite_installation_owners(
    installations: Vec<ExtensionInstallation>,
    user_ids: &BTreeSet<UserId>,
) -> Vec<ExtensionInstallation> {
    let desired = InstallationOwner::Users {
        user_ids: user_ids.clone(),
    };
    installations
        .into_iter()
        .map(|installation| {
            if installation.owner() == &desired {
                installation
            } else {
                installation.with_owner(desired.clone())
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use std::{path::Path, sync::Arc};

    use chrono::Utc;
    use ironclaw_extensions::{
        ExtensionActivationState, ExtensionInstallation, ExtensionInstallationId,
        ExtensionInstallationStore, ExtensionManifestRecord, ExtensionManifestRef,
        InMemoryExtensionInstallationStore, InstallationOwner, ManifestSource,
    };
    use ironclaw_filesystem::{LibSqlRootFilesystem, RootFilesystem, ScopedFilesystem};
    use ironclaw_host_api::AgentId;
    use ironclaw_host_api::{ExtensionId, HostPortCatalog, TenantId, UserId};
    use ironclaw_reborn_identity::{
        FilesystemRebornIdentityStore, RebornUserDirectory, RebornUserRole,
    };

    use super::{ExtensionOwnershipMigrationOptions, run_extension_ownership_migration};
    use super::{rewrite_installation_owners, rewrite_store};
    use crate::options::TargetStore;

    fn installation(id: &str, owner: InstallationOwner) -> ExtensionInstallation {
        let extension_id = ExtensionId::new(id.to_string()).expect("extension id");
        ExtensionInstallation::new(
            ExtensionInstallationId::new(id.to_string()).expect("installation id"),
            extension_id.clone(),
            ExtensionActivationState::Installed,
            ExtensionManifestRef::new(extension_id, None),
            Vec::new(),
            Utc::now(),
            owner,
        )
        .expect("installation")
    }

    fn manifest(id: &str) -> ExtensionManifestRecord {
        ExtensionManifestRecord::from_toml(
            format!(
                r#"
schema_version = "reborn.extension_manifest.v2"
id = "{id}"
name = "{id}"
version = "0.1.0"
description = "migration fixture"
trust = "third_party"

[runtime]
kind = "wasm"
module = "wasm/{id}.wasm"

[[capabilities]]
id = "{id}.read"
description = "read"
effects = ["network"]
default_permission = "ask"
visibility = "model"
input_schema_ref = "schemas/read.input.json"
output_schema_ref = "schemas/read.output.json"
"#
            ),
            ManifestSource::HostBundled,
            &HostPortCatalog::empty(),
            None,
        )
        .expect("manifest")
    }

    #[test]
    fn rewrites_every_installation_to_the_complete_user_set() {
        let alice = UserId::new("alice").expect("alice");
        let bob = UserId::new("bob").expect("bob");
        let users = BTreeSet::from([alice.clone(), bob.clone()]);
        let installations = vec![
            installation("github", InstallationOwner::Tenant),
            installation("gmail", InstallationOwner::user(alice)),
        ];

        let rewritten = rewrite_installation_owners(installations, &users);

        assert_eq!(rewritten.len(), 2);
        for installation in rewritten {
            assert_eq!(
                installation.owner(),
                &InstallationOwner::Users {
                    user_ids: users.clone(),
                }
            );
        }
    }

    #[tokio::test]
    async fn dry_run_apply_and_rerun_are_safe_and_idempotent() {
        let store = InMemoryExtensionInstallationStore::default();
        let alice = UserId::new("alice").expect("alice");
        let bob = UserId::new("bob").expect("bob");
        let bootstrap = UserId::new("bootstrap-operator").expect("bootstrap");
        let users = BTreeSet::from([alice.clone(), bob, bootstrap.clone()]);
        store
            .upsert_manifest_and_installation(
                manifest("github"),
                installation("github", InstallationOwner::Tenant),
            )
            .await
            .expect("seed github");
        store
            .upsert_manifest_and_installation(
                manifest("gmail"),
                installation("gmail", InstallationOwner::user(alice)),
            )
            .await
            .expect("seed gmail");
        let tenant_id = TenantId::new("fixture-tenant").expect("tenant");

        let dry_run = rewrite_store(&store, tenant_id.clone(), users.clone(), true)
            .await
            .expect("dry run");
        assert_eq!(dry_run.changed_extension_ids.len(), 2);
        assert!(dry_run.user_ids.contains(&bootstrap));
        assert!(
            store
                .list_installations()
                .await
                .expect("list after dry run")
                .iter()
                .any(|installation| installation.owner() == &InstallationOwner::Tenant),
            "dry run must not write"
        );

        let applied = rewrite_store(&store, tenant_id.clone(), users.clone(), false)
            .await
            .expect("apply");
        assert_eq!(applied.changed_extension_ids.len(), 2);
        for installation in store.list_installations().await.expect("list after apply") {
            assert_eq!(installation.owner().members(), Some(&users));
        }

        let rerun = rewrite_store(&store, tenant_id, users, false)
            .await
            .expect("rerun");
        assert!(rerun.changed_extension_ids.is_empty());
    }

    #[tokio::test]
    async fn public_migration_reads_tenant_users_and_tenant_installation_store() {
        let temp = tempfile::tempdir().expect("tempdir");
        let database_path = temp.path().join("reborn.db");
        let tenant_id = TenantId::new("tenant-a").expect("tenant");
        let actor = UserId::new("seed-operator").expect("actor");

        let root = open_root(&database_path).await;
        let identity_filesystem = Arc::new(ScopedFilesystem::new(
            root.clone(),
            ironclaw_reborn_composition::invocation_mount_view,
        ));
        let directory = FilesystemRebornIdentityStore::new(
            identity_filesystem,
            tenant_id.clone(),
            actor.clone(),
            AgentId::new("migration-test").expect("agent"),
            None,
        );
        let alice = directory
            .create_user(
                &tenant_id,
                None,
                Some("Alice".to_string()),
                RebornUserRole::Member,
                &actor,
            )
            .await
            .expect("create alice")
            .user_id;
        let bob = directory
            .create_user(
                &tenant_id,
                None,
                Some("Bob".to_string()),
                RebornUserRole::Member,
                &actor,
            )
            .await
            .expect("create bob")
            .user_id;
        let root_dyn: Arc<dyn RootFilesystem> = root.clone();
        let store = ironclaw_reborn_composition::extension_installation_store_for_migration(
            root_dyn,
            Some(&tenant_id),
        )
        .await
        .expect("tenant extension store");
        store
            .upsert_manifest_and_installation(
                manifest("github"),
                installation("github", InstallationOwner::Tenant),
            )
            .await
            .expect("seed github");
        drop(store);
        drop(directory);
        drop(root);

        let include_users = BTreeSet::from([actor.clone()]);
        let dry_run = run_extension_ownership_migration(ExtensionOwnershipMigrationOptions {
            target: TargetStore::LibSql {
                path: database_path.clone(),
            },
            tenant_id: tenant_id.clone(),
            include_users: include_users.clone(),
            dry_run: true,
        })
        .await
        .expect("dry run");
        assert_eq!(dry_run.changed_extension_ids.len(), 1);
        assert_eq!(
            dry_run.user_ids.into_iter().collect::<BTreeSet<_>>(),
            BTreeSet::from([actor.clone(), alice.clone(), bob.clone()])
        );
        assert_eq!(
            read_github_owner(&database_path, &tenant_id).await,
            InstallationOwner::Tenant,
            "dry run does not write"
        );

        let applied = run_extension_ownership_migration(ExtensionOwnershipMigrationOptions {
            target: TargetStore::LibSql {
                path: database_path.clone(),
            },
            tenant_id: tenant_id.clone(),
            include_users: include_users.clone(),
            dry_run: false,
        })
        .await
        .expect("apply");
        assert_eq!(applied.changed_extension_ids.len(), 1);
        assert_eq!(
            read_github_owner(&database_path, &tenant_id).await,
            InstallationOwner::Users {
                user_ids: BTreeSet::from([actor, alice, bob])
            }
        );

        let rerun = run_extension_ownership_migration(ExtensionOwnershipMigrationOptions {
            target: TargetStore::LibSql {
                path: database_path,
            },
            tenant_id,
            include_users,
            dry_run: false,
        })
        .await
        .expect("rerun");
        assert!(rerun.changed_extension_ids.is_empty());
    }

    async fn open_root(path: &Path) -> Arc<LibSqlRootFilesystem> {
        let database = Arc::new(
            libsql::Builder::new_local(path)
                .build()
                .await
                .expect("open libsql"),
        );
        let root = Arc::new(LibSqlRootFilesystem::new(database));
        root.run_migrations().await.expect("root migrations");
        root
    }

    async fn read_github_owner(path: &Path, tenant_id: &TenantId) -> InstallationOwner {
        let root = open_root(path).await;
        let root_dyn: Arc<dyn RootFilesystem> = root;
        let store = ironclaw_reborn_composition::extension_installation_store_for_migration(
            root_dyn,
            Some(tenant_id),
        )
        .await
        .expect("reopen tenant extension store");
        store
            .get_installation(&ExtensionInstallationId::new("github").expect("installation id"))
            .await
            .expect("read github")
            .expect("github installation")
            .owner()
            .clone()
    }
}
