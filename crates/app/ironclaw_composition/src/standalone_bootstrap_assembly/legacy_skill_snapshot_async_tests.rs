use std::{path::Path, sync::Arc};

#[cfg(unix)]
use std::{path::PathBuf, sync::Mutex};

use ironclaw_filesystem::{InMemoryBackend, RootFilesystem};
use ironclaw_host_api::{ids::UserId, path::VirtualPath};

use super::{
    MAX_INSTALL_BUNDLE_FILE_BYTES, SKILL_DISK_IMPORT_MARKER_ROOT,
    import_host_disk_skills_into_database, import_host_disk_skills_into_database_with_collector,
};

const TENANT: &str = "import-tenant";
const USER: &str = "import-user";

#[cfg(unix)]
struct BeforeSnapshotReadHook {
    path: PathBuf,
    action: Box<dyn FnOnce(&Path) + Send + 'static>,
}

#[cfg(unix)]
static SWAP_TEST_SERIALIZER: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
#[cfg(unix)]
static BEFORE_SNAPSHOT_READ_HOOK: Mutex<Option<BeforeSnapshotReadHook>> = Mutex::new(None);

#[cfg(unix)]
struct BeforeSnapshotReadHookGuard;

#[cfg(unix)]
impl Drop for BeforeSnapshotReadHookGuard {
    fn drop(&mut self) {
        *BEFORE_SNAPSHOT_READ_HOOK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
    }
}

#[cfg(unix)]
fn install_before_snapshot_read_hook(
    path: PathBuf,
    hook: impl FnOnce(&Path) + Send + 'static,
) -> BeforeSnapshotReadHookGuard {
    let mut selected = BEFORE_SNAPSHOT_READ_HOOK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    assert!(
        selected.is_none(),
        "snapshot read hook is already installed"
    );
    *selected = Some(BeforeSnapshotReadHook {
        path,
        action: Box::new(hook),
    });
    BeforeSnapshotReadHookGuard
}

#[cfg(unix)]
pub(super) fn run_before_snapshot_read_hook(path: &Path) {
    let mut selected = BEFORE_SNAPSHOT_READ_HOOK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if selected
        .as_ref()
        .is_none_or(|hook| hook.path.as_path() != path)
    {
        return;
    }
    let hook = selected.take();
    drop(selected);
    if let Some(hook) = hook {
        (hook.action)(path);
    }
}

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
    std::fs::write(dir.join("SKILL.md"), b"snapshot skill").expect("skill body");
}

fn database_filesystem() -> Arc<ironclaw_filesystem::CompositeRootFilesystem> {
    crate::filesystem_assembly::production_database_root_filesystem(
        Arc::new(InMemoryBackend::new()),
        "skill-disk-import-async-test",
    )
    .expect("database root filesystem builds")
}

#[tokio::test(flavor = "current_thread")]
async fn snapshot_collection_does_not_block_unrelated_executor_work() {
    let filesystem = database_filesystem();
    let (collection_started_tx, collection_started_rx) = tokio::sync::oneshot::channel();
    let (executor_progress_tx, executor_progress_rx) = std::sync::mpsc::sync_channel(1);
    let (release_collection_tx, release_collection_rx) = std::sync::mpsc::sync_channel(1);
    let coordinator = std::thread::spawn(move || {
        // Success is signal-driven. The generous timeout only releases a
        // broken inline implementation instead of deadlocking the suite.
        let executor_progressed = executor_progress_rx
            .recv_timeout(std::time::Duration::from_secs(10))
            .is_ok();
        release_collection_tx
            .send(())
            .expect("release snapshot collection");
        executor_progressed
    });
    let progress = tokio::spawn(async move {
        collection_started_rx
            .await
            .expect("snapshot collector reports it started");
        let _ = executor_progress_tx.send(());
    });

    import_host_disk_skills_into_database_with_collector(&filesystem, move |_events| {
        collection_started_tx
            .send(())
            .map_err(|_| crate::RebornBuildError::InvalidConfig {
                reason: "snapshot progress observer dropped".to_string(),
            })?;
        release_collection_rx
            .recv()
            .map_err(|error| crate::RebornBuildError::InvalidConfig {
                reason: format!("wait to release snapshot collection: {error}"),
            })?;
        Ok(())
    })
    .await
    .expect("empty snapshot import succeeds");
    progress.await.expect("executor progress task joins");

    assert!(
        coordinator.join().expect("progress coordinator joins"),
        "snapshot collection blocked the current-thread Tokio executor"
    );
}

#[tokio::test]
async fn snapshot_collection_join_failure_keeps_stage_context() {
    let filesystem = database_filesystem();

    let error = import_host_disk_skills_into_database_with_collector(&filesystem, |_events| {
        panic!("snapshot collector panic");
    })
    .await
    .expect_err("a panicked snapshot collection task must fail startup");

    assert!(
        matches!(
            error,
            crate::RebornBuildError::InvalidConfig { ref reason }
                if reason.contains("legacy skill snapshot collection task failed")
                    && reason.contains("panicked")
        ),
        "{error}"
    );
}

#[tokio::test]
async fn marked_snapshot_file_is_skipped_before_an_oversized_read() {
    let storage = tempfile::tempdir().expect("temp storage root");
    let filesystem = database_filesystem();
    seed_skill_on_disk(storage.path(), "already-marked");
    let selected = storage
        .path()
        .join("tenants")
        .join(TENANT)
        .join("users")
        .join(USER)
        .join("skills/already-marked/SKILL.md");
    std::fs::OpenOptions::new()
        .write(true)
        .open(&selected)
        .expect("open marked snapshot file")
        .set_len((MAX_INSTALL_BUNDLE_FILE_BYTES as u64).saturating_add(1))
        .expect("make marked snapshot file oversized");
    let virtual_path = virtual_skill_path("already-marked");
    let marker = VirtualPath::new(format!(
        "{SKILL_DISK_IMPORT_MARKER_ROOT}{}",
        virtual_path.as_str()
    ))
    .expect("per-skill marker path");
    RootFilesystem::write_file(filesystem.as_ref(), &marker, b"1")
        .await
        .expect("seed per-skill marker");

    import_host_disk_skills_into_database(storage.path(), &owner(), &filesystem)
        .await
        .expect("marked oversized snapshot file is skipped before read");

    assert!(
        RootFilesystem::stat(filesystem.as_ref(), &virtual_path)
            .await
            .is_err(),
        "a marker-covered disk copy must not be re-imported"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn collected_skill_file_replaced_by_symlink_is_rejected_at_verified_read() {
    use std::os::unix::fs::symlink;

    let _serial = SWAP_TEST_SERIALIZER.lock().await;
    let storage = tempfile::tempdir().expect("temp storage root");
    let filesystem = database_filesystem();
    seed_skill_on_disk(storage.path(), "replace-before-read");
    let selected = storage
        .path()
        .join("tenants")
        .join(TENANT)
        .join("users")
        .join(USER)
        .join("skills/replace-before-read/SKILL.md");
    let outside = storage.path().join("outside.txt");
    std::fs::write(&outside, b"outside bytes").expect("outside file");
    let _hook = install_before_snapshot_read_hook(selected.clone(), move |path| {
        assert_eq!(path, selected);
        std::fs::remove_file(path).expect("remove collected file");
        symlink(&outside, path).expect("replace collected file with symlink");
    });

    let error = import_host_disk_skills_into_database(storage.path(), &owner(), &filesystem)
        .await
        .expect_err("verified import must reject the replacement symlink");
    assert!(error.to_string().contains("symlink"), "{error}");
    assert_rejected_import_left_no_state(&filesystem, "replace-before-read").await;
}

#[cfg(unix)]
#[tokio::test]
async fn collected_skill_directory_replaced_by_symlink_is_rejected_at_verified_read() {
    use std::os::unix::fs::symlink;

    let _serial = SWAP_TEST_SERIALIZER.lock().await;
    let storage = tempfile::tempdir().expect("temp storage root");
    let outside = tempfile::tempdir().expect("outside snapshot root");
    let filesystem = database_filesystem();
    seed_skill_on_disk(storage.path(), "replace-directory-before-read");
    seed_skill_on_disk(outside.path(), "replace-directory-before-read");
    let selected = storage
        .path()
        .join("tenants")
        .join(TENANT)
        .join("users")
        .join(USER)
        .join("skills/replace-directory-before-read/SKILL.md");
    let selected_directory = selected.parent().expect("selected skill directory");
    let selected_directory = selected_directory.to_path_buf();
    let retired_directory = storage.path().join("retired-skill-directory");
    let outside_directory = outside
        .path()
        .join("tenants")
        .join(TENANT)
        .join("users")
        .join(USER)
        .join("skills/replace-directory-before-read");
    let _hook = install_before_snapshot_read_hook(selected.clone(), move |path| {
        assert_eq!(path, selected);
        std::fs::rename(&selected_directory, &retired_directory)
            .expect("retire validated skill directory");
        symlink(&outside_directory, &selected_directory)
            .expect("replace validated skill directory with outside symlink");
    });

    let error = import_host_disk_skills_into_database(storage.path(), &owner(), &filesystem)
        .await
        .expect_err("verified import must reject a replaced ancestor directory");
    assert!(
        error.to_string().contains("without following links"),
        "{error}"
    );
    assert_rejected_import_left_no_state(&filesystem, "replace-directory-before-read").await;
}

async fn assert_rejected_import_left_no_state(
    filesystem: &ironclaw_filesystem::CompositeRootFilesystem,
    skill_name: &str,
) {
    let target = virtual_skill_path(skill_name);
    let marker = VirtualPath::new(format!(
        "{SKILL_DISK_IMPORT_MARKER_ROOT}{}",
        target.as_str()
    ))
    .expect("per-skill marker path");
    assert!(
        matches!(
            RootFilesystem::stat(filesystem, &target).await,
            Err(ironclaw_filesystem::FilesystemError::NotFound { .. })
        ),
        "rejected snapshot must not create its target"
    );
    assert!(
        matches!(
            RootFilesystem::stat(filesystem, &marker).await,
            Err(ironclaw_filesystem::FilesystemError::NotFound { .. })
        ),
        "rejected snapshot must not create its import marker"
    );
}
