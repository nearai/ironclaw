use std::path::{Path, PathBuf};
use std::sync::Arc;

use ironclaw_filesystem::{
    BackendCapabilities, BackendId, BackendKind, CompositeRootFilesystem, ContentKind,
    DiskFilesystem, IndexPolicy, LibSqlRootFilesystem, MountDescriptor, PostgresRootFilesystem,
    RootFilesystem, StorageClass,
};
use ironclaw_host_api::path::{HostPath, VirtualPath};

use crate::RebornBuildError;
use crate::host_access_assembly::{HostDiskMountCapabilities, HostHomeRoot};

/// Compatibility filename for the embedded standalone database.
///
/// Existing installations already persist this path, so the legacy filename is
/// intentionally stable even though the code-level profile terminology is not.
pub(crate) const STANDALONE_DB_FILENAME: &str = "reborn-local-dev.db";

pub fn standalone_db_path(root: &Path) -> PathBuf {
    root.join(STANDALONE_DB_FILENAME)
}

/// Read a file back out of the standalone database, for tests that assert WHERE a skill landed.
/// Skill writes go to the DB-backed filesystem, so a `storage_root.join(...).exists()` check asks
/// the wrong question (nearai/ironclaw#7168).
#[cfg(test)]
pub(crate) async fn database_file_bytes(
    storage_root: &Path,
    virtual_path: &str,
) -> Option<Vec<u8>> {
    let db = Arc::new(
        libsql::Builder::new_local(standalone_db_path(storage_root))
            .build()
            .await
            .expect("open standalone libsql database"),
    );
    let vfs = LibSqlRootFilesystem::new(db).expect("libsql root filesystem");
    vfs.run_migrations().await.expect("libsql migrations");
    let path = VirtualPath::new(virtual_path).expect("virtual path");
    ironclaw_filesystem::RootFilesystem::read_file(&vfs, &path)
        .await
        .ok()
}

/// Seed a file into the standalone database, for tests that need a skill the runtime can find.
#[cfg(any(test, feature = "test-support"))]
pub(crate) async fn write_database_file_for_test(
    storage_root: &Path,
    virtual_path: &str,
    contents: &[u8],
) {
    std::fs::create_dir_all(storage_root).expect("storage root");
    let db = Arc::new(
        libsql::Builder::new_local(standalone_db_path(storage_root))
            .build()
            .await
            .expect("open standalone libsql database"),
    );
    let vfs = LibSqlRootFilesystem::new(db).expect("libsql root filesystem");
    vfs.run_migrations().await.expect("libsql migrations");
    let path = VirtualPath::new(virtual_path).expect("virtual path");
    ironclaw_filesystem::RootFilesystem::write_file(&vfs, &path, contents)
        .await
        .expect("write seeded file into the database");
}

pub(crate) struct FilesystemAssembly {
    pub(crate) filesystem: Arc<CompositeRootFilesystem>,
    pub(crate) durable_backend: DurableBackend,
}

pub(crate) enum DurableBackend {
    LibSql {
        runtime: Arc<ironclaw_libsql_runtime::LibSqlRuntime>,
        filesystem: Arc<LibSqlRootFilesystem>,
    },
    Postgres(deadpool_postgres::Pool),
}

pub(crate) enum DurableStorageInput {
    EmbeddedLibsql,
    Postgres(deadpool_postgres::Pool),
}

/// Builds the storage substrate selected by already-resolved configuration.
pub(crate) async fn build_filesystem(
    state_root: &Path,
    system_root: &Path,
    workspace_root: &Path,
    host_home_root: Option<&HostHomeRoot>,
    admitted_disk_mounts: Option<&HostDiskMountCapabilities>,
    durable_storage: DurableStorageInput,
) -> Result<FilesystemAssembly, RebornBuildError> {
    let disk = Arc::new(host_disk_filesystem(
        system_root,
        workspace_root,
        host_home_root,
        admitted_disk_mounts,
    )?);
    let mut composite = CompositeRootFilesystem::new();
    let durable_backend = match durable_storage {
        DurableStorageInput::Postgres(pool) => {
            let database = Arc::new(PostgresRootFilesystem::new(pool.clone()));
            database.run_migrations().await?;
            mount_database_roots(&mut composite, database)?;
            DurableBackend::Postgres(pool)
        }
        DurableStorageInput::EmbeddedLibsql => {
            build_default_database_roots(state_root, &mut composite).await?
        }
    };
    mount_host_disk_roots(&mut composite, disk, host_home_root.is_some())?;
    Ok(FilesystemAssembly {
        filesystem: Arc::new(composite),
        durable_backend,
    })
}

/// Open the compatibility-path embedded database without mounting it.
pub(crate) async fn open_standalone_libsql_database(
    root: &Path,
) -> Result<Arc<libsql::Database>, RebornBuildError> {
    let db_path = standalone_db_path(root);
    Ok(Arc::new(
        libsql::Builder::new_local(&db_path)
            .build()
            .await
            .map_err(|error| RebornBuildError::InvalidConfig {
                reason: format!("standalone libSQL database could not be opened: {error}"),
            })?,
    ))
}

pub(crate) async fn build_default_database_roots(
    root: &Path,
    composite: &mut CompositeRootFilesystem,
) -> Result<DurableBackend, RebornBuildError> {
    let db = open_standalone_libsql_database(root).await?;
    let runtime = Arc::new(ironclaw_libsql_runtime::LibSqlRuntime::new(db)?);
    let database = Arc::new(LibSqlRootFilesystem::from_runtime(Arc::clone(&runtime)));
    database.run_migrations().await?;
    mount_database_roots(composite, Arc::clone(&database))?;
    Ok(DurableBackend::LibSql {
        runtime,
        filesystem: database,
    })
}

fn host_disk_filesystem(
    system_root: &Path,
    workspace_root: &Path,
    host_home_root: Option<&HostHomeRoot>,
    admitted: Option<&HostDiskMountCapabilities>,
) -> Result<DiskFilesystem, RebornBuildError> {
    let mut filesystem = DiskFilesystem::new();
    if let Some(admitted) = admitted {
        filesystem.mount_local_capability(
            VirtualPath::new("/projects/workspace")?,
            HostPath::from_path_buf(workspace_root.to_path_buf()),
            admitted.workspace.clone(),
        )?;
        filesystem.mount_local_capability(
            VirtualPath::new("/system/extensions")?,
            HostPath::from_path_buf(system_root.join("extensions")),
            admitted.system_extensions.clone(),
        )?;
        filesystem.mount_local_capability(
            VirtualPath::new("/system/prompts")?,
            HostPath::from_path_buf(system_root.join("prompts")),
            admitted.system_prompts.clone(),
        )?;
        filesystem.mount_local_capability(
            VirtualPath::new("/system/skills")?,
            HostPath::from_path_buf(system_root.join("skills")),
            admitted.system_skills.clone(),
        )?;
    } else {
        filesystem.mount_local(
            VirtualPath::new("/projects/workspace")?,
            HostPath::from_path_buf(workspace_root.to_path_buf()),
        )?;
        filesystem.mount_local(
            VirtualPath::new("/system/extensions")?,
            HostPath::from_path_buf(system_root.join("extensions")),
        )?;
        filesystem.mount_local(
            VirtualPath::new("/system/prompts")?,
            HostPath::from_path_buf(system_root.join("prompts")),
        )?;
        filesystem.mount_local(
            VirtualPath::new("/system/skills")?,
            HostPath::from_path_buf(system_root.join("skills")),
        )?;
    }
    if let Some(host_home_root) = host_home_root {
        filesystem.mount_local(
            VirtualPath::new("/projects/host")?,
            HostPath::from_path_buf(host_home_root.canonical_root().to_path_buf()),
        )?;
    }
    Ok(filesystem)
}

fn mount_memory_root<F>(
    root: &mut CompositeRootFilesystem,
    backend: Arc<F>,
) -> Result<(), RebornBuildError>
where
    F: RootFilesystem + 'static,
{
    root.mount(
        mount_descriptor(
            "/memory",
            "standalone-memory",
            BackendKind::MemoryDocuments,
            StorageClass::StructuredRecords,
            ContentKind::MemoryDocument,
            IndexPolicy::FullTextAndVector,
            backend.capabilities(),
        )?,
        backend,
    )?;
    Ok(())
}

/// A root filesystem the process journal writes through, over its own backend
/// handle.
///
/// The journal's heartbeat is the liveness signal a run's lease depends on.
/// While it shared one connection pool with event-store, trigger, and
/// result-read traffic, a busy turn could starve its own heartbeat until the
/// lease expired underneath it — the run then failed `lease_expired` while it
/// was still healthy. Giving the journal its own backend handle means a
/// heartbeat never queues behind data-plane work.
///
/// The mount set is exactly [`mount_database_roots`]', so the journal resolves
/// the same virtual paths to the same rows the shared filesystem would have
/// written. Only the connection it travels over differs.
pub(crate) fn process_journal_root_filesystem<F>(
    backend: Arc<F>,
) -> Result<Arc<CompositeRootFilesystem>, RebornBuildError>
where
    F: RootFilesystem + 'static,
{
    let mut root = CompositeRootFilesystem::new();
    mount_database_roots(&mut root, backend)?;
    Ok(Arc::new(root))
}

/// Build the journal filesystem over libSQL's bounded secondary write lane.
/// `mount_roots` preserves the data-plane mount layout and row identity; the
/// runtime owns the writer-admission invariant for #7714.
pub(crate) fn libsql_journal_lane_filesystem(
    runtime: &ironclaw_libsql_runtime::LibSqlRuntime,
    mount_roots: impl FnOnce(
        Arc<LibSqlRootFilesystem>,
    ) -> Result<Arc<CompositeRootFilesystem>, RebornBuildError>,
) -> Result<Arc<CompositeRootFilesystem>, RebornBuildError> {
    let lane_runtime = Arc::new(runtime.split_journal_lane()?);
    // The data-plane handle already migrated this database.
    mount_roots(Arc::new(LibSqlRootFilesystem::from_runtime(lane_runtime)))
}

pub(crate) fn mount_database_roots<F>(
    root: &mut CompositeRootFilesystem,
    database: Arc<F>,
) -> Result<(), RebornBuildError>
where
    F: RootFilesystem + 'static,
{
    for (virtual_root, backend_id, content_kind, index_policy) in [
        (
            "/tenants",
            "standalone-reborn-state",
            ContentKind::StructuredRecord,
            IndexPolicy::NotIndexed,
        ),
        (
            "/system/extensions/.installations",
            "standalone-extension-installation-state",
            ContentKind::SystemState,
            IndexPolicy::BackendDefined,
        ),
        (
            "/system/settings",
            "standalone-system-settings",
            ContentKind::SystemState,
            IndexPolicy::BackendDefined,
        ),
    ] {
        root.mount(
            mount_descriptor(
                virtual_root,
                backend_id,
                BackendKind::DatabaseFilesystem,
                StorageClass::StructuredRecords,
                content_kind,
                index_policy,
                database.capabilities(),
            )?,
            Arc::clone(&database),
        )?;
    }
    mount_memory_root(root, Arc::clone(&database))?;
    root.mount(
        mount_descriptor(
            "/events",
            "standalone-events",
            BackendKind::DatabaseFilesystem,
            StorageClass::StructuredRecords,
            ContentKind::StructuredRecord,
            IndexPolicy::NotIndexed,
            database.capabilities(),
        )?,
        database,
    )?;
    Ok(())
}

pub(crate) fn production_database_root_filesystem<F>(
    backend: Arc<F>,
    backend_id: &str,
) -> Result<Arc<CompositeRootFilesystem>, RebornBuildError>
where
    F: RootFilesystem + 'static,
{
    let mut root = CompositeRootFilesystem::new();
    for virtual_root in [
        "/tenants",
        "/events",
        "/memory",
        "/projects",
        "/system/extensions",
        "/system/settings",
        "/system/skills",
    ] {
        let mount_id = format!(
            "{backend_id}-{}",
            virtual_root
                .trim_start_matches('/')
                .replace(['/', '.'], "-")
        );
        root.mount(
            mount_descriptor(
                virtual_root,
                &mount_id,
                BackendKind::DatabaseFilesystem,
                StorageClass::StructuredRecords,
                ContentKind::StructuredRecord,
                IndexPolicy::BackendDefined,
                backend.capabilities(),
            )?,
            Arc::clone(&backend),
        )?;
    }
    Ok(Arc::new(root))
}

fn mount_host_disk_roots(
    root: &mut CompositeRootFilesystem,
    disk: Arc<DiskFilesystem>,
    include_host_home: bool,
) -> Result<(), RebornBuildError> {
    for (virtual_root, backend_id, content_kind) in [
        (
            "/projects/workspace",
            "standalone-workspace-files",
            ContentKind::ProjectFile,
        ),
        (
            "/system/extensions",
            "standalone-system-extensions",
            ContentKind::ExtensionPackage,
        ),
        (
            "/system/prompts",
            "standalone-system-prompts",
            ContentKind::GenericFile,
        ),
        (
            "/system/skills",
            "standalone-system-skills",
            ContentKind::GenericFile,
        ),
    ] {
        root.mount(
            mount_descriptor(
                virtual_root,
                backend_id,
                BackendKind::DiskFilesystem,
                StorageClass::FileContent,
                content_kind,
                IndexPolicy::NotIndexed,
                BackendCapabilities::bytes_only(),
            )?,
            Arc::clone(&disk),
        )?;
    }
    if include_host_home {
        root.mount(
            mount_descriptor(
                "/projects/host",
                "standalone-host-home",
                BackendKind::DiskFilesystem,
                StorageClass::FileContent,
                ContentKind::ProjectFile,
                IndexPolicy::NotIndexed,
                BackendCapabilities::bytes_only(),
            )?,
            disk,
        )?;
    }
    Ok(())
}

pub(crate) fn mount_descriptor(
    virtual_root: &str,
    backend_id: &str,
    backend_kind: BackendKind,
    storage_class: StorageClass,
    content_kind: ContentKind,
    index_policy: IndexPolicy,
    capabilities: BackendCapabilities,
) -> Result<MountDescriptor, RebornBuildError> {
    Ok(MountDescriptor {
        virtual_root: VirtualPath::new(virtual_root)?,
        backend_id: BackendId::new(backend_id)?,
        backend_kind,
        storage_class,
        content_kind,
        index_policy,
        capabilities,
    })
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::{
        DurableStorageInput, STANDALONE_DB_FILENAME, build_filesystem, mount_host_disk_roots,
    };
    use ironclaw_filesystem::{CompositeRootFilesystem, DiskFilesystem, RootFilesystem};
    use ironclaw_host_api::path::{HostPath, VirtualPath};

    #[tokio::test]
    async fn filesystem_assembly_keeps_state_system_and_workspace_content_in_separate_roots() {
        let temp = tempfile::tempdir().expect("temporary Reborn home");
        let home = temp.path().join("reborn-home");
        let state = home.join("state");
        let system = home.join("system");
        let workspaces = home.join("workspaces");
        std::fs::create_dir_all(&state).expect("create state root");
        std::fs::create_dir_all(&workspaces).expect("create workspace root");
        std::fs::create_dir_all(system.join("extensions")).expect("create system extensions root");
        std::fs::create_dir_all(system.join("prompts")).expect("create system prompts root");
        std::fs::create_dir_all(system.join("skills")).expect("create system skills root");

        let assembly = build_filesystem(
            &state,
            &system,
            &workspaces,
            None,
            None,
            DurableStorageInput::EmbeddedLibsql,
        )
        .await
        .expect("filesystem assembly");

        let mounted_roots = assembly
            .filesystem
            .mounts()
            .await
            .expect("mount catalog")
            .into_iter()
            .map(|mount| mount.virtual_root.as_str().to_owned())
            .collect::<Vec<_>>();
        assert!(mounted_roots.contains(&"/projects/workspace".to_string()));
        assert!(
            !mounted_roots.contains(&"/projects".to_string()),
            "the trusted disk catalog must not expose a broad projects root"
        );
        assert!(
            !mounted_roots.contains(&"/projects/host".to_string()),
            "a catalog without a confirmed host home must not advertise one"
        );

        let project_file = VirtualPath::new("/projects/workspace/only-workspace.txt")
            .expect("workspace file virtual path");
        RootFilesystem::write_file(assembly.filesystem.as_ref(), &project_file, b"workspace")
            .await
            .expect("workspace write");
        for (virtual_path, contents, disk_path) in [
            (
                "/system/extensions/example.toml",
                b"extension".as_slice(),
                system.join("extensions/example.toml"),
            ),
            (
                "/system/prompts/default-system.md",
                b"prompt".as_slice(),
                system.join("prompts/default-system.md"),
            ),
            (
                "/system/skills/example/SKILL.md",
                b"skill".as_slice(),
                system.join("skills/example/SKILL.md"),
            ),
        ] {
            let path = VirtualPath::new(virtual_path).expect("system file virtual path");
            RootFilesystem::write_file(assembly.filesystem.as_ref(), &path, contents)
                .await
                .expect("system write");
            assert_eq!(
                std::fs::read(disk_path).expect("system file is mounted at system root"),
                contents
            );
        }

        assert!(
            state.join(STANDALONE_DB_FILENAME).is_file(),
            "the embedded database must be created in state/"
        );
        assert!(
            !home.join(STANDALONE_DB_FILENAME).exists(),
            "the embedded database must not be created at the Reborn home"
        );
        assert_eq!(
            std::fs::read(workspaces.join("only-workspace.txt"))
                .expect("workspace file is on the host workspace root"),
            b"workspace"
        );
        assert!(
            !home.join("only-workspace.txt").exists(),
            "/projects/workspace must not map to the Reborn home"
        );
        assert!(
            system.is_dir(),
            "the reviewed system root is part of the explicit layout"
        );
    }

    #[tokio::test]
    async fn host_disk_catalog_routes_confirmed_host_home_files() {
        let temp = tempfile::tempdir().expect("temporary Reborn home");
        let host_home = temp.path().join("host-home");
        std::fs::create_dir_all(&host_home).expect("create confirmed host home");

        let mut disk = DiskFilesystem::new();
        disk.mount_local(
            VirtualPath::new("/projects/host").expect("host home virtual path"),
            HostPath::from_path_buf(host_home.clone()),
        )
        .expect("mount confirmed host home on disk filesystem");
        let disk = Arc::new(disk);
        let mut catalog = CompositeRootFilesystem::new();
        mount_host_disk_roots(&mut catalog, Arc::clone(&disk), true)
            .expect("mount host disk roots in composite catalog");

        let host_file =
            VirtualPath::new("/projects/host/safe.txt").expect("host file virtual path");
        RootFilesystem::write_file(&catalog, &host_file, b"host file")
            .await
            .expect("composite catalog routes confirmed host home writes");

        assert_eq!(
            std::fs::read(host_home.join("safe.txt")).expect("host file written to disk"),
            b"host file",
        );
    }
}
