use std::path::{Path, PathBuf};

use ironclaw_filesystem::{DiskDirectoryCapability, DiskFilesystem, RootFilesystem};
use ironclaw_host_api::ids::UserId;
use ironclaw_host_api::path::{HostPath, VirtualPath};
use ironclaw_skills::MAX_INSTALL_BUNDLE_FILE_BYTES;

use crate::RebornBuildError;
use crate::root::default_system_prompt::seed_default_system_prompt;

const DEFAULT_SYSTEM_PROMPT_PATH: &str = "prompts/default-system.md";
const SYSTEM_SKILLS_ROOT: &str = "/projects/system/skills";
const STANDALONE_LEGACY_SKILL_TENANTS: [&str; 2] = ["default", "reborn-cli"];

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
/// Copies rather than moves, so a downgrade is not destructive, and an existing database entry
/// always wins.
///
/// Per-skill markers under [`SKILL_DISK_IMPORT_MARKER_ROOT`] make interruption
/// recovery idempotent and prevent deleted skills from being resurrected. A
/// versioned snapshot-complete marker then makes normal startup O(1): layout
/// adoption snapshots are immutable, so a completed snapshot must never be
/// rescanned for files that could only have appeared through out-of-band
/// mutation.
///
/// Markers live under `/system/settings`, database-backed on every shape, so they travel with the
/// store rather than the boot directory.
const SKILL_DISK_IMPORT_MARKER_ROOT: &str = "/system/settings/skill-disk-import";
const SKILL_DISK_IMPORT_COMPLETE_MARKER: &str = "/system/settings/skill-disk-import-v1-complete";

async fn record_skill_disk_import(
    filesystem: &ironclaw_filesystem::CompositeRootFilesystem,
    marker: &ironclaw_host_api::path::VirtualPath,
) -> Result<(), RebornBuildError> {
    RootFilesystem::write_file(filesystem, marker, b"1").await?;
    Ok(())
}

pub(crate) async fn import_host_disk_skills_into_database(
    storage_root: &Path,
    owner_user_id: &UserId,
    filesystem: &std::sync::Arc<ironclaw_filesystem::CompositeRootFilesystem>,
) -> Result<(), RebornBuildError> {
    let storage_root = storage_root.to_path_buf();
    let owner_user_id = owner_user_id.clone();
    import_host_disk_skills_into_database_with_collector(filesystem, move |events| {
        stream_legacy_skill_snapshot(&storage_root, &owner_user_id, events)
    })
    .await
}

type SnapshotRead = (
    tokio::sync::oneshot::Sender<Result<Vec<u8>, RebornBuildError>>,
    tokio::sync::oneshot::Receiver<()>,
);
type SnapshotCandidate = (String, tokio::sync::oneshot::Sender<Option<SnapshotRead>>);

async fn import_host_disk_skills_into_database_with_collector<F>(
    filesystem: &std::sync::Arc<ironclaw_filesystem::CompositeRootFilesystem>,
    collect: F,
) -> Result<(), RebornBuildError>
where
    F: FnOnce(tokio::sync::mpsc::Sender<SnapshotCandidate>) -> Result<(), RebornBuildError>
        + Send
        + 'static,
{
    let complete_marker = VirtualPath::new(SKILL_DISK_IMPORT_COMPLETE_MARKER)?;
    match RootFilesystem::stat(filesystem.as_ref(), &complete_marker).await {
        Ok(_) => return Ok(()),
        Err(ironclaw_filesystem::FilesystemError::NotFound { .. }) => {}
        Err(error) => return Err(RebornBuildError::Filesystem(error)),
    }

    // Capacity one plus the consumption acknowledgement prevents the producer
    // from starting another read while the async consumer owns a payload.
    let (event_tx, mut event_rx) = tokio::sync::mpsc::channel(1);
    let collection = tokio::task::spawn_blocking(move || collect(event_tx));
    let mut imported = 0usize;
    let processing = async {
        while let Some((virtual_path, decision)) = event_rx.recv().await {
            let target = VirtualPath::new(&virtual_path)?;
            let marker =
                VirtualPath::new(format!("{SKILL_DISK_IMPORT_MARKER_ROOT}{virtual_path}"))?;
            // Marker-covered disk copies must not even be opened.
            match RootFilesystem::stat(filesystem.as_ref(), &marker).await {
                Ok(_) => {
                    let _ = decision.send(None);
                    continue;
                }
                Err(ironclaw_filesystem::FilesystemError::NotFound { .. }) => {}
                Err(error) => return Err(RebornBuildError::Filesystem(error)),
            }
            // A database entry wins and receives its marker before skipping.
            match RootFilesystem::stat(filesystem.as_ref(), &target).await {
                Ok(_) => {
                    record_skill_disk_import(filesystem, &marker).await?;
                    let _ = decision.send(None);
                    continue;
                }
                Err(ironclaw_filesystem::FilesystemError::NotFound { .. }) => {}
                Err(error) => return Err(RebornBuildError::Filesystem(error)),
            }
            let (payload_tx, payload_rx) = tokio::sync::oneshot::channel();
            let (consumed_tx, consumed_rx) = tokio::sync::oneshot::channel();
            if decision.send(Some((payload_tx, consumed_rx))).is_err() {
                continue;
            }
            let bytes = payload_rx
                .await
                .map_err(|error| RebornBuildError::InvalidConfig {
                    reason: format!("legacy skill snapshot payload task stopped: {error}"),
                })??;
            RootFilesystem::write_file(filesystem.as_ref(), &target, &bytes).await?;
            record_skill_disk_import(filesystem, &marker).await?;
            imported += 1;
            drop(bytes);
            let _ = consumed_tx.send(());
        }
        Ok::<(), RebornBuildError>(())
    }
    .await;
    drop(event_rx);
    let collection = collection
        .await
        .map_err(|error| RebornBuildError::InvalidConfig {
            reason: format!("legacy skill snapshot collection task failed: {error}"),
        })?;
    processing?;
    collection?;
    if imported > 0 {
        tracing::info!(
            imported,
            "imported host-disk skills into the database-backed skill tree"
        );
    }
    record_skill_disk_import(filesystem, &complete_marker).await?;
    Ok(())
}

fn stream_legacy_skill_snapshot(
    storage_root: &Path,
    owner_user_id: &UserId,
    events: tokio::sync::mpsc::Sender<SnapshotCandidate>,
) -> Result<(), RebornBuildError> {
    let snapshot_root = validate_legacy_skill_snapshot_tree(storage_root)?;
    let tenants_root = storage_root.join("tenants");
    let mut skill_files = disk_skill_files(&tenants_root)?;
    skill_files.extend(unscoped_disk_skill_files(storage_root, owner_user_id)?);
    for (host_path, virtual_path) in skill_files {
        let (decision_tx, decision_rx) = tokio::sync::oneshot::channel();
        if events.blocking_send((virtual_path, decision_tx)).is_err() {
            return Ok(());
        }
        let Ok(decision) = decision_rx.blocking_recv() else {
            return Ok(());
        };
        let Some((payload, consumed)) = decision else {
            continue;
        };
        let relative_path =
            host_path
                .strip_prefix(storage_root)
                .map_err(|_| RebornBuildError::InvalidConfig {
                    reason: format!(
                        "legacy skill snapshot file escaped its admitted root: {}",
                        host_path.display()
                    ),
                })?;
        #[cfg(all(test, unix))]
        legacy_skill_snapshot_async_tests::run_before_snapshot_read_hook(&host_path);
        let bytes = ironclaw_filesystem::read_ordinary_host_file(
            &snapshot_root,
            relative_path,
            MAX_INSTALL_BUNDLE_FILE_BYTES,
        )
        .map_err(|error| snapshot_io_error("read legacy skill snapshot file", &host_path, error));
        let read_succeeded = bytes.is_ok();
        if payload.send(bytes).is_err() {
            return Ok(());
        }
        if !read_succeeded || consumed.blocking_recv().is_err() {
            return Ok(());
        }
    }
    Ok(())
}

/// Map the oldest unscoped `skills/` tree exactly as the released standalone
/// backfill did: to the configured owner under both supported tenant aliases.
fn unscoped_disk_skill_files(
    storage_root: &Path,
    owner_user_id: &UserId,
) -> Result<Vec<(PathBuf, String)>, RebornBuildError> {
    let skills_root = storage_root.join("skills");
    let mut files = Vec::new();
    collect_files_under(&skills_root, &skills_root, &mut |relative, host_path| {
        for tenant_id in STANDALONE_LEGACY_SKILL_TENANTS {
            files.push((
                host_path.to_path_buf(),
                format!(
                    "/tenants/{tenant_id}/users/{}/skills/{relative}",
                    owner_user_id.as_str()
                ),
            ));
        }
    })?;
    Ok(files)
}

/// Every file under `tenants/<tenant>/users/<user>/skills/**`, paired with its database path.
///
/// Walks only that shape, so nothing else under `tenants/` is copied into the skill tree.
fn disk_skill_files(tenants_root: &Path) -> Result<Vec<(PathBuf, String)>, RebornBuildError> {
    let mut found = Vec::new();
    let tenants = match std::fs::read_dir(tenants_root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(found),
        Err(error) => {
            return Err(snapshot_io_error(
                "read tenants directory",
                tenants_root,
                error,
            ));
        }
    };
    for tenant in tenants {
        let tenant = tenant.map_err(|error| {
            snapshot_io_error("read tenant directory entry", tenants_root, error)
        })?;
        let tenant_id = tenant.file_name().to_string_lossy().to_string();
        let users_root = tenant.path().join("users");
        let users = match std::fs::read_dir(&users_root) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => {
                return Err(snapshot_io_error(
                    "read users directory",
                    &users_root,
                    error,
                ));
            }
        };
        for user in users {
            let user = user.map_err(|error| {
                snapshot_io_error("read user directory entry", &users_root, error)
            })?;
            let user_id = user.file_name().to_string_lossy().to_string();
            let skills_root = user.path().join("skills");
            collect_files_under(&skills_root, &skills_root, &mut |relative, host_path| {
                found.push((
                    host_path.to_path_buf(),
                    format!("/tenants/{tenant_id}/users/{user_id}/skills/{relative}"),
                ));
            })?;
        }
    }
    Ok(found)
}

fn collect_files_under(
    base: &Path,
    dir: &Path,
    visit: &mut impl FnMut(String, &Path),
) -> Result<(), RebornBuildError> {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(snapshot_io_error("read skill directory", dir, error)),
    };
    for entry in entries {
        let entry =
            entry.map_err(|error| snapshot_io_error("read skill directory entry", dir, error))?;
        let path = entry.path();
        let metadata = std::fs::symlink_metadata(&path)
            .map_err(|error| snapshot_io_error("inspect skill snapshot entry", &path, error))?;
        if metadata.file_type().is_symlink() {
            return Err(snapshot_symlink_error(&path));
        }
        if metadata.is_dir() {
            collect_files_under(base, &path, visit)?;
        } else if metadata.is_file()
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
    Ok(())
}

/// Rejects symlinks in the completed legacy skill roots before any host file is read.
///
/// The migration source is an operator-selected legacy tree, so following even one entry would
/// allow a snapshot to import data from outside that tree. Only selected
/// `skills` roots matter: unrelated archived state in the snapshot is never
/// read and must not block this one-time import.
fn validate_legacy_skill_snapshot_tree(
    storage_root: &Path,
) -> Result<DiskDirectoryCapability, RebornBuildError> {
    validate_legacy_skill_snapshot_directory(storage_root)?;
    let snapshot_root = DiskDirectoryCapability::admit_existing(storage_root).map_err(|error| {
        snapshot_io_error("retain legacy skill snapshot root", storage_root, error)
    })?;
    validate_legacy_skill_snapshot_root(&storage_root.join("skills"))?;

    let tenants_root = storage_root.join("tenants");
    if !validate_legacy_skill_snapshot_directory_if_present(&tenants_root)? {
        return Ok(snapshot_root);
    }
    let tenants = match std::fs::read_dir(&tenants_root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(snapshot_root),
        Err(error) => {
            return Err(snapshot_io_error(
                "read tenants directory",
                &tenants_root,
                error,
            ));
        }
    };
    for tenant in tenants {
        let tenant = tenant.map_err(|error| {
            snapshot_io_error("read tenant directory entry", &tenants_root, error)
        })?;
        validate_legacy_skill_snapshot_directory(&tenant.path())?;
        let users_root = tenant.path().join("users");
        if !validate_legacy_skill_snapshot_directory_if_present(&users_root)? {
            continue;
        }
        for user in std::fs::read_dir(&users_root)
            .map_err(|error| snapshot_io_error("read users directory", &users_root, error))?
        {
            let user = user.map_err(|error| {
                snapshot_io_error("read user directory entry", &users_root, error)
            })?;
            validate_legacy_skill_snapshot_directory(&user.path())?;
            validate_legacy_skill_snapshot_root(&user.path().join("skills"))?;
        }
    }
    Ok(snapshot_root)
}

fn validate_legacy_skill_snapshot_root(path: &Path) -> Result<(), RebornBuildError> {
    if validate_legacy_skill_snapshot_directory_if_present(path)? {
        ironclaw_filesystem::inspect_ordinary_host_tree(&HostPath::from_path_buf(
            path.to_path_buf(),
        ))
        .map_err(|error| {
            snapshot_io_error("validate ordinary legacy skill snapshot tree", path, error)
        })?;
    }
    Ok(())
}

fn validate_legacy_skill_snapshot_directory_if_present(
    path: &Path,
) -> Result<bool, RebornBuildError> {
    match std::fs::symlink_metadata(path) {
        Ok(_) => {
            validate_legacy_skill_snapshot_directory(path)?;
            Ok(true)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(snapshot_io_error(
            "inspect legacy skill snapshot",
            path,
            error,
        )),
    }
}

fn validate_legacy_skill_snapshot_directory(path: &Path) -> Result<(), RebornBuildError> {
    let metadata = std::fs::symlink_metadata(path)
        .map_err(|error| snapshot_io_error("inspect legacy skill snapshot", path, error))?;
    if metadata.file_type().is_symlink() {
        return Err(snapshot_symlink_error(path));
    }
    if metadata.is_dir() {
        Ok(())
    } else {
        Err(RebornBuildError::InvalidConfig {
            reason: format!(
                "legacy skill snapshot entry is not a directory: {}",
                path.display()
            ),
        })
    }
}

fn snapshot_symlink_error(path: &Path) -> RebornBuildError {
    RebornBuildError::InvalidConfig {
        reason: format!(
            "legacy skill snapshot must not contain symlinks: {}",
            path.display()
        ),
    }
}

fn snapshot_io_error(context: &str, path: &Path, error: std::io::Error) -> RebornBuildError {
    RebornBuildError::InvalidConfig {
        reason: format!("failed to {context} at {}: {error}", path.display()),
    }
}

/// Initializes standalone host content after storage roots are prepared.
pub(crate) async fn bootstrap_standalone_host(
    system_root: &Path,
    _owner_user_id: &UserId,
) -> Result<PathBuf, RebornBuildError> {
    let default_system_prompt_path = system_root.join(DEFAULT_SYSTEM_PROMPT_PATH);
    seed_default_system_prompt(system_root, &default_system_prompt_path).map_err(|error| {
        RebornBuildError::InvalidConfig {
            reason: error.to_string(),
        }
    })?;
    let filesystem = standalone_system_skills_filesystem(&system_root.join("skills")).await?;
    let system_skills_root = VirtualPath::new(SYSTEM_SKILLS_ROOT)?;
    ironclaw_extension_host::bundled_skills::ensure_bundled_reborn_skills_installed_in(
        &filesystem,
        &system_skills_root,
    )
    .await?;

    Ok(default_system_prompt_path)
}

/// Builds the narrowly scoped host-disk filesystem used only to seed standalone system skills.
///
/// The exact host root is validated before it is mounted so bundle installation cannot follow a
/// symlink outside the standalone system tree.
async fn standalone_system_skills_filesystem(
    system_skills_root: &Path,
) -> Result<DiskFilesystem, RebornBuildError> {
    let virtual_system_skills_root = VirtualPath::new(SYSTEM_SKILLS_ROOT)?;
    let mut filesystem = DiskFilesystem::new();
    filesystem
        .mount_local_create(
            virtual_system_skills_root,
            HostPath::from_path_buf(system_skills_root.to_path_buf()),
        )
        .await
        .map_err(RebornBuildError::Filesystem)?;
    Ok(filesystem)
}

#[cfg(test)]
mod bootstrap_tests {
    use std::error::Error as _;

    use ironclaw_host_api::ids::UserId;

    use super::bootstrap_standalone_host;

    #[tokio::test]
    async fn bundled_skills_install_at_the_exact_system_skills_root() {
        let root = tempfile::tempdir().expect("tempdir");
        let system_root = root
            .path()
            .canonicalize()
            .expect("canonical tempdir")
            .join("system");
        let owner = UserId::new("bootstrap-owner").expect("valid owner");
        std::fs::create_dir_all(system_root.join("prompts"))
            .expect("host-access system prompt root");

        bootstrap_standalone_host(&system_root, &owner)
            .await
            .expect("standalone bootstrap succeeds");

        assert!(
            system_root.join("skills/coding/SKILL.md").is_file(),
            "bundled skills must be installed under system/skills"
        );
        assert!(
            !system_root.join("system").exists(),
            "an exact system root must never be interpreted as an installation root"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn bundled_skills_reject_a_symlinked_system_skills_root() {
        use std::os::unix::fs::symlink;

        let root = tempfile::tempdir().expect("tempdir");
        let base = root.path().canonicalize().expect("canonical tempdir");
        let system_root = base.join("system");
        let outside_root = base.join("outside-skills");
        let owner = UserId::new("bootstrap-owner").expect("valid owner");
        std::fs::create_dir_all(system_root.join("prompts"))
            .expect("host-access system prompt root");
        std::fs::create_dir_all(&outside_root).expect("outside skills root");
        symlink(&outside_root, system_root.join("skills")).expect("symlink system skills root");

        let error = bootstrap_standalone_host(&system_root, &owner)
            .await
            .expect_err("standalone bootstrap must not follow a symlinked skills root");

        assert!(matches!(error, crate::RebornBuildError::Filesystem(_)));
        assert!(error.source().is_some());
        assert!(error.source().and_then(std::error::Error::source).is_some());

        assert_eq!(error.to_string(), "reborn filesystem build failed");
        assert!(
            error
                .source()
                .expect("filesystem source")
                .to_string()
                .contains("filesystem backend rejected local directory capability")
        );
        assert!(
            !error
                .to_string()
                .contains(outside_root.to_string_lossy().as_ref())
        );
        assert!(
            !outside_root.join("coding").exists(),
            "a rejected skills-root symlink must not receive bundled content"
        );
    }
}

#[cfg(test)]
mod legacy_skill_snapshot_async_tests;

#[cfg(test)]
mod skill_disk_import_tests {
    use std::path::Path;
    use std::sync::Arc;

    use ironclaw_filesystem::{InMemoryBackend, RootFilesystem};
    use ironclaw_host_api::{ids::UserId, path::VirtualPath};

    use super::import_host_disk_skills_into_database;

    const TENANT: &str = "import-tenant";
    const USER: &str = "import-user";

    fn owner() -> UserId {
        UserId::new(USER).expect("owner user id")
    }

    fn virtual_skill_path(name: &str) -> VirtualPath {
        VirtualPath::new(format!(
            "/tenants/{TENANT}/users/{USER}/skills/{name}/SKILL.md"
        ))
        .expect("virtual skill path")
    }

    fn seed_skill_on_disk(storage_root: &Path, name: &str) {
        let dir = storage_root
            .join("tenants")
            .join(TENANT)
            .join("users")
            .join(USER)
            .join("skills")
            .join(name);
        std::fs::create_dir_all(&dir).expect("skill dir");
        std::fs::write(
            dir.join("SKILL.md"),
            format!("---\nname: {name}\ndescription: {name}\n---\n\nbody\n"),
        )
        .expect("skill body");
    }

    fn seed_unscoped_legacy_skill_on_disk(storage_root: &Path, name: &str) {
        let dir = storage_root.join("skills").join(name);
        std::fs::create_dir_all(&dir).expect("legacy skill dir");
        std::fs::write(
            dir.join("SKILL.md"),
            format!("---\nname: {name}\ndescription: {name}\n---\n\nbody\n"),
        )
        .expect("legacy skill body");
    }

    #[tokio::test]
    async fn unscoped_legacy_skills_keep_the_released_owner_and_tenant_mapping() {
        let storage = tempfile::tempdir().expect("temp storage root");
        let filesystem = database_filesystem();
        seed_unscoped_legacy_skill_on_disk(storage.path(), "unscoped");

        import_host_disk_skills_into_database(storage.path(), &owner(), &filesystem)
            .await
            .expect("legacy unscoped import runs");

        for tenant in ["default", "reborn-cli"] {
            let path = VirtualPath::new(format!(
                "/tenants/{tenant}/users/{USER}/skills/unscoped/SKILL.md"
            ))
            .expect("mapped skill path");
            assert!(
                RootFilesystem::stat(filesystem.as_ref(), &path)
                    .await
                    .is_ok(),
                "released backfill mapping is preserved for {tenant}"
            );
        }
    }

    fn database_filesystem() -> Arc<ironclaw_filesystem::CompositeRootFilesystem> {
        crate::filesystem_assembly::production_database_root_filesystem(
            Arc::new(InMemoryBackend::new()),
            "skill-disk-import-test",
        )
        .expect("database root filesystem builds")
    }

    /// A completed immutable adoption snapshot is never rescanned on later
    /// starts. Out-of-band mutation cannot become a standing synchronization
    /// channel from host disk back into authoritative database state.
    #[tokio::test]
    async fn a_skill_appearing_after_snapshot_completion_is_not_imported() {
        let storage = tempfile::tempdir().expect("temp storage root");
        let filesystem = database_filesystem();

        seed_skill_on_disk(storage.path(), "first");
        import_host_disk_skills_into_database(storage.path(), &owner(), &filesystem)
            .await
            .expect("first import runs");

        seed_skill_on_disk(storage.path(), "second");
        import_host_disk_skills_into_database(storage.path(), &owner(), &filesystem)
            .await
            .expect("second import runs");

        assert!(
            RootFilesystem::stat(filesystem.as_ref(), &virtual_skill_path("second"))
                .await
                .is_err(),
            "completed adoption snapshots are immutable and must not be rescanned on every startup"
        );
    }

    /// ...but re-running the import must NOT undo a deletion.
    ///
    /// This is the reason the marker existed. The disk copy stays behind when a user removes a
    /// skill through the product, so an import that only checks "is it already in the database?"
    /// copies it straight back. Per-skill markers keep the migration one-shot PER SKILL, which is
    /// what lets the test above pass without resurrecting anything.
    #[tokio::test]
    async fn an_imported_skill_deleted_from_the_database_is_not_resurrected() {
        let storage = tempfile::tempdir().expect("temp storage root");
        let filesystem = database_filesystem();

        seed_skill_on_disk(storage.path(), "removed-later");
        import_host_disk_skills_into_database(storage.path(), &owner(), &filesystem)
            .await
            .expect("first import runs");

        let path = virtual_skill_path("removed-later");
        RootFilesystem::delete(filesystem.as_ref(), &path)
            .await
            .expect("user deletes the skill through the product");

        import_host_disk_skills_into_database(storage.path(), &owner(), &filesystem)
            .await
            .expect("second import runs");

        assert!(
            RootFilesystem::stat(filesystem.as_ref(), &path)
                .await
                .is_err(),
            "a skill the user deleted must stay deleted; the disk copy outlives the deletion, so an \
             import that re-reads it resurrects a skill the user removed"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn legacy_skill_snapshot_import_rejects_a_symlinked_snapshot_root() {
        use std::os::unix::fs::symlink;

        let storage = tempfile::tempdir().expect("temp storage root");
        let outside = tempfile::tempdir().expect("outside snapshot root");
        let filesystem = database_filesystem();
        seed_skill_on_disk(outside.path(), "escaped-root");
        let snapshot_alias = storage.path().join("snapshot");
        symlink(outside.path(), &snapshot_alias).expect("snapshot alias");

        let error = import_host_disk_skills_into_database(&snapshot_alias, &owner(), &filesystem)
            .await
            .expect_err("a legacy snapshot root symlink must fail closed");

        assert!(error.to_string().contains("symlink"), "{error}");
        assert!(
            RootFilesystem::stat(filesystem.as_ref(), &virtual_skill_path("escaped-root"))
                .await
                .is_err(),
            "a rejected snapshot must not import an outside skill"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn legacy_skill_snapshot_import_rejects_a_symlinked_tenants_root() {
        use std::os::unix::fs::symlink;

        let storage = tempfile::tempdir().expect("temp storage root");
        let outside = tempfile::tempdir().expect("outside tenants root");
        let filesystem = database_filesystem();
        seed_skill_on_disk(outside.path(), "escaped-tenants-root");
        symlink(
            outside.path().join("tenants"),
            storage.path().join("tenants"),
        )
        .expect("tenants root alias");

        let error = import_host_disk_skills_into_database(storage.path(), &owner(), &filesystem)
            .await
            .expect_err("a tenants-root symlink must fail before directory traversal");

        assert!(error.to_string().contains("symlink"), "{error}");
        assert!(
            RootFilesystem::stat(
                filesystem.as_ref(),
                &virtual_skill_path("escaped-tenants-root")
            )
            .await
            .is_err(),
            "a rejected tenants-root alias must not import an outside skill"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn legacy_skill_snapshot_import_rejects_a_symlinked_subtree() {
        use std::os::unix::fs::symlink;

        let storage = tempfile::tempdir().expect("temp storage root");
        let outside = tempfile::tempdir().expect("outside skill subtree");
        let filesystem = database_filesystem();
        seed_skill_on_disk(outside.path(), "escaped-subtree");
        let skills_root = storage
            .path()
            .join("tenants")
            .join(TENANT)
            .join("users")
            .join(USER)
            .join("skills");
        std::fs::create_dir_all(&skills_root).expect("skills root");
        let outside_skill = outside
            .path()
            .join("tenants")
            .join(TENANT)
            .join("users")
            .join(USER)
            .join("skills")
            .join("escaped-subtree");
        symlink(&outside_skill, skills_root.join("linked")).expect("skill subtree symlink");

        let error = import_host_disk_skills_into_database(storage.path(), &owner(), &filesystem)
            .await
            .expect_err("a legacy snapshot subtree symlink must fail closed");

        assert!(error.to_string().contains("symlink"), "{error}");
        assert!(
            RootFilesystem::stat(filesystem.as_ref(), &virtual_skill_path("linked"))
                .await
                .is_err(),
            "a rejected snapshot must not import an outside subtree"
        );
    }

    #[tokio::test]
    async fn legacy_skill_snapshot_import_rejects_a_tree_beyond_the_shared_depth_bound() {
        let storage = tempfile::tempdir().expect("temp storage root");
        let filesystem = database_filesystem();
        let mut deepest = storage
            .path()
            .join("tenants")
            .join(TENANT)
            .join("users")
            .join(USER)
            .join("skills");
        std::fs::create_dir_all(&deepest).expect("skills root");
        for level in 0..=ironclaw_filesystem::MAX_ORDINARY_HOST_TREE_DEPTH {
            deepest = deepest.join(format!("level-{level}"));
            std::fs::create_dir(&deepest).expect("nested skill directory");
        }
        std::fs::write(deepest.join("SKILL.md"), b"nested skill").expect("nested skill file");

        let error = import_host_disk_skills_into_database(storage.path(), &owner(), &filesystem)
            .await
            .expect_err("an unbounded legacy snapshot must fail closed");

        assert!(error.to_string().contains("depth"), "{error}");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn legacy_skill_snapshot_import_ignores_symlinks_outside_selected_skill_roots() {
        use std::os::unix::fs::symlink;

        let storage = tempfile::tempdir().expect("temp storage root");
        let outside = tempfile::tempdir().expect("outside unrelated state root");
        let filesystem = database_filesystem();
        seed_skill_on_disk(storage.path(), "kept-skill");
        symlink(outside.path(), storage.path().join("state")).expect("unrelated state symlink");

        import_host_disk_skills_into_database(storage.path(), &owner(), &filesystem)
            .await
            .expect("only imported skill roots should be validated");

        assert!(
            RootFilesystem::stat(filesystem.as_ref(), &virtual_skill_path("kept-skill"))
                .await
                .is_ok(),
            "an unrelated snapshot alias must not prevent importing a selected skill root"
        );
    }
}
