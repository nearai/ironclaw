use std::path::{Path, PathBuf};

use ironclaw_host_api::ids::UserId;

use crate::RebornBuildError;
use crate::root::default_system_prompt::seed_default_system_prompt;

const DEFAULT_SYSTEM_PROMPT_PATH: &str = "system/prompts/default-system.md";
#[cfg(all(test, unix))]
pub(crate) use ironclaw_extension_host::bundled_skills::LEGACY_SKILLS_BACKFILL_MARKER;
const STANDALONE_LEGACY_SKILL_TENANTS: [&str; 2] = ["default", "reborn-cli"];

/// Apply the legacy standalone skill-tree migration to every tenant identity
/// this standalone host profile supports.
pub(crate) fn backfill_legacy_user_skills(
    storage_root: &Path,
    owner_user_id: &UserId,
) -> Result<(), RebornBuildError> {
    let legacy_root = storage_root.join("skills");
    for tenant_id in STANDALONE_LEGACY_SKILL_TENANTS {
        let scoped_root = storage_root
            .join("tenants")
            .join(tenant_id)
            .join("users")
            .join(owner_user_id.as_str())
            .join("skills");
        ironclaw_extension_host::bundled_skills::backfill_legacy_skill_tree(
            &legacy_root,
            &scoped_root,
        )?;
    }
    Ok(())
}

/// Move host-disk user skills into the database-backed tree, which is the only tree skills are read
/// from now.
///
/// Two populations need this, and both are silent without it:
///
/// * The legacy backfill above writes to `storage_root/tenants/<t>/users/<u>/skills` on the HOST DISK.
///   Nothing reads that path any more, so a user upgrading with legacy skills would find them gone.
/// * Every skill an agent installed before this change also went to that disk path, because the
///   agent's in-run skill port wrote there while Settings → Skills listed the database — the mount
///   split that is nearai/ironclaw#7168. Those skills are real, the user created them, and an upgrade
///   must not silently drop them.
///
/// Copies, never moves: the disk copy is left in place so a downgrade is not destructive, and a
/// database entry already present always wins, so this cannot clobber a newer edit made through the
/// database.
///
/// Runs ONCE per store, gated on [`SKILL_DISK_IMPORT_MARKER`]. "A database entry already present
/// wins" is not on its own enough to make a per-boot import safe: the disk copy is deliberately
/// left behind, so a skill the user later REMOVED through the database is absent on the next boot
/// and gets copied straight back in. A removal that undoes itself at restart is worse than no
/// migration at all, and the marker is what makes this a migration rather than a standing sync.
/// One-shot marker recording that the host-disk skill import has run for this store.
///
/// Lives under `/system/settings`, which is database-backed on every shape, so it travels with the
/// store it describes rather than with whatever host directory the server happened to boot from.
const SKILL_DISK_IMPORT_MARKER: &str = "/system/settings/skill-disk-import.done";

pub(crate) async fn import_host_disk_skills_into_database(
    storage_root: &Path,
    filesystem: &std::sync::Arc<ironclaw_filesystem::CompositeRootFilesystem>,
) -> Result<(), RebornBuildError> {
    use ironclaw_filesystem::RootFilesystem;
    use ironclaw_host_api::path::VirtualPath;

    let marker = VirtualPath::new(SKILL_DISK_IMPORT_MARKER)?;
    if RootFilesystem::stat(filesystem.as_ref(), &marker)
        .await
        .is_ok()
    {
        return Ok(());
    }

    let tenants_root = storage_root.join("tenants");
    let mut imported = 0usize;
    for (host_path, virtual_path) in disk_skill_files(&tenants_root) {
        let target = VirtualPath::new(&virtual_path)?;
        // A database entry wins: it is either newer or the product of a previous import.
        if RootFilesystem::stat(filesystem.as_ref(), &target)
            .await
            .is_ok()
        {
            continue;
        }
        let Ok(bytes) = std::fs::read(&host_path) else {
            continue;
        };
        if RootFilesystem::write_file(filesystem.as_ref(), &target, &bytes)
            .await
            .is_ok()
        {
            imported += 1;
        }
    }
    if imported > 0 {
        tracing::info!(
            imported,
            "imported host-disk skills into the database-backed skill tree"
        );
    }
    // Written even when nothing was imported: "there was nothing on disk" is as final an answer as
    // "everything was copied", and re-walking the tree on every boot to re-learn it is waste.
    if let Err(error) = RootFilesystem::write_file(filesystem.as_ref(), &marker, b"1").await {
        // Not fatal. The cost of a missing marker is that the next boot re-runs the import; the cost
        // of failing the boot is that the runtime does not start at all.
        tracing::debug!(
            %error,
            "could not record the skill disk-import marker; the import will be retried next boot"
        );
    }
    Ok(())
}

/// Every file under `tenants/<tenant>/users/<user>/skills/**`, paired with its database path.
///
/// Walks only that shape, so nothing else under `tenants/` is copied into the skill tree.
fn disk_skill_files(tenants_root: &Path) -> Vec<(PathBuf, String)> {
    let mut found = Vec::new();
    let Ok(tenants) = std::fs::read_dir(tenants_root) else {
        return found;
    };
    for tenant in tenants.flatten() {
        let tenant_id = tenant.file_name().to_string_lossy().to_string();
        let Ok(users) = std::fs::read_dir(tenant.path().join("users")) else {
            continue;
        };
        for user in users.flatten() {
            let user_id = user.file_name().to_string_lossy().to_string();
            let skills_root = user.path().join("skills");
            collect_files_under(&skills_root, &skills_root, &mut |relative, host_path| {
                found.push((
                    host_path.to_path_buf(),
                    format!("/tenants/{tenant_id}/users/{user_id}/skills/{relative}"),
                ));
            });
        }
    }
    found
}

fn collect_files_under(base: &Path, dir: &Path, visit: &mut impl FnMut(String, &Path)) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_files_under(base, &path, visit);
        } else if path.is_file()
            && let Ok(relative) = path.strip_prefix(base)
        {
            // Forward slashes: this becomes a VirtualPath, not a host path.
            let relative = relative
                .components()
                .map(|component| component.as_os_str().to_string_lossy().to_string())
                .collect::<Vec<_>>()
                .join("/");
            visit(relative, &path);
        }
    }
}

/// Initializes standalone host content after storage roots are prepared.
pub(crate) async fn bootstrap_standalone_host(
    storage_root: &Path,
    owner_user_id: &UserId,
) -> Result<PathBuf, RebornBuildError> {
    let backfill_root = storage_root.to_path_buf();
    let backfill_owner_user_id = owner_user_id.clone();
    tokio::task::spawn_blocking(move || {
        backfill_legacy_user_skills(&backfill_root, &backfill_owner_user_id)
    })
    .await
    .map_err(|error| RebornBuildError::InvalidConfig {
        reason: format!("legacy skill backfill task failed: {error}"),
    })??;

    let default_system_prompt_path = storage_root.join(DEFAULT_SYSTEM_PROMPT_PATH);
    seed_default_system_prompt(storage_root, &default_system_prompt_path).map_err(|error| {
        RebornBuildError::InvalidConfig {
            reason: error.to_string(),
        }
    })?;
    ironclaw_extension_host::bundled_skills::ensure_bundled_reborn_skills_installed(storage_root)
        .await?;

    Ok(default_system_prompt_path)
}
