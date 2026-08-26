use std::path::Path;
use std::sync::Arc;

use ironclaw_filesystem::LibSqlRootFilesystem;
use ironclaw_host_api::ids::UserId;
use ironclaw_skills::ScopedSkillManagementPort;

/// List standalone user skills from the same canonical libSQL filesystem the
/// runtime uses, then merge the embedded bundled catalog.
pub async fn list_reborn_local_skills_from_state(
    owner_id: impl Into<String>,
    state_root: impl AsRef<Path>,
) -> Result<
    Vec<ironclaw_skills::SkillSummary>,
    ironclaw_extension_host::skill_listing::RebornSkillListError,
> {
    let state_root = state_root.as_ref();
    let database_path = crate::filesystem_assembly::standalone_db_path(state_root);
    if !database_path.try_exists().map_err(|_inspection_error| {
        ironclaw_extension_host::skill_listing::RebornSkillListError::Unavailable {
            reason: "standalone skill database could not be inspected".to_string(),
        }
    })? {
        return ironclaw_extension_host::skill_listing::list_reborn_skills_from_management(None)
            .await;
    }

    // Keep the rejected external identity out of the model-visible error.
    let owner_user_id = UserId::new(owner_id.into()).map_err(|_invalid_owner| {
        ironclaw_extension_host::skill_listing::RebornSkillListError::InvalidRequest {
            reason: "skill list owner is invalid".to_string(),
        }
    })?;
    let database = crate::filesystem_assembly::open_standalone_libsql_database(state_root)
        .await
        .map_err(|_database_error| {
            ironclaw_extension_host::skill_listing::RebornSkillListError::Unavailable {
                reason: "standalone skill database is unavailable".to_string(),
            }
        })?;
    let runtime = Arc::new(
        ironclaw_libsql_runtime::LibSqlRuntime::new(database).map_err(|_runtime_error| {
            ironclaw_extension_host::skill_listing::RebornSkillListError::Unavailable {
                reason: "standalone skill database is unavailable".to_string(),
            }
        })?,
    );
    let filesystem = Arc::new(LibSqlRootFilesystem::from_runtime(runtime));
    let skill_management = Arc::new(ScopedSkillManagementPort::new_with_mount_resolver(
        owner_user_id,
        filesystem,
        Arc::new(
            crate::factory::production_backend_assembly::production_skill_management_mount_view,
        ),
    ));

    ironclaw_extension_host::skill_listing::list_reborn_skills_from_management(Some(
        skill_management,
    ))
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn invalid_owner_error_is_sanitized() {
        let directory = tempfile::tempdir().expect("tempdir");
        let database =
            crate::filesystem_assembly::open_standalone_libsql_database(directory.path())
                .await
                .expect("create database file");
        let connection = database.connect().expect("create database file");
        drop(connection);
        drop(database);

        let error = list_reborn_local_skills_from_state("", directory.path())
            .await
            .expect_err("empty owner must be rejected");

        assert!(matches!(
            error,
            ironclaw_extension_host::skill_listing::RebornSkillListError::InvalidRequest {
                reason
            } if reason == "skill list owner is invalid"
        ));
    }

    #[tokio::test]
    async fn database_inspection_error_is_sanitized() {
        let directory = tempfile::tempdir().expect("tempdir");
        let overlong_component = "x".repeat(300);
        let state_root = directory.path().join(overlong_component);

        let error = list_reborn_local_skills_from_state("owner", state_root)
            .await
            .expect_err("overlong database path must be rejected");

        assert!(matches!(
            error,
            ironclaw_extension_host::skill_listing::RebornSkillListError::Unavailable {
                reason
            } if reason == "standalone skill database could not be inspected"
        ));
    }

    #[tokio::test]
    async fn listing_an_uninitialized_existing_database_is_read_only() {
        let directory = tempfile::tempdir().expect("tempdir");
        let database =
            crate::filesystem_assembly::open_standalone_libsql_database(directory.path())
                .await
                .expect("create empty database file");
        let connection = database.connect().expect("connect empty database");
        drop(connection);
        drop(database);

        list_reborn_local_skills_from_state("owner", directory.path())
            .await
            .expect_err("inspection must not initialize or migrate an existing database");

        let database =
            crate::filesystem_assembly::open_standalone_libsql_database(directory.path())
                .await
                .expect("reopen empty database");
        let connection = database.connect().expect("connect empty database");
        let mut rows = connection
            .query(
                "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'root_filesystem_entries'",
                (),
            )
            .await
            .expect("inspect empty database schema");
        assert!(
            rows.next().await.expect("read schema row").is_none(),
            "skills list must not create the filesystem schema"
        );
    }
}
