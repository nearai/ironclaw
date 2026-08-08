use std::path::{Path, PathBuf};
use std::sync::Arc;

use ironclaw_filesystem::{
    BackendCapabilities, BackendId, BackendKind, CompositeRootFilesystem, ContentKind,
    DiskFilesystem, IndexPolicy, LibSqlRootFilesystem, MountDescriptor, PostgresRootFilesystem,
    RootFilesystem, StorageClass,
};
use ironclaw_host_api::path::{HostPath, VirtualPath};

use crate::RebornBuildError;
use crate::host_access_assembly::HostHomeRoot;

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
#[cfg(test)]
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
    storage_root: &Path,
    workspace_root: &Path,
    host_home_root: Option<&HostHomeRoot>,
    durable_storage: DurableStorageInput,
) -> Result<FilesystemAssembly, RebornBuildError> {
    let disk = Arc::new(host_disk_filesystem(
        storage_root,
        workspace_root,
        host_home_root,
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
            build_default_database_roots(storage_root, &mut composite).await?
        }
    };
    mount_host_disk_roots(&mut composite, disk)?;
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
    root: &Path,
    workspace_root: &Path,
    host_home_root: Option<&HostHomeRoot>,
) -> Result<DiskFilesystem, RebornBuildError> {
    let mut filesystem = DiskFilesystem::new();
    filesystem.mount_local(
        VirtualPath::new("/projects")?,
        HostPath::from_path_buf(root.to_path_buf()),
    )?;
    filesystem.mount_local(
        VirtualPath::new("/projects/workspace")?,
        HostPath::from_path_buf(workspace_root.to_path_buf()),
    )?;
    filesystem.mount_local(
        VirtualPath::new("/system/extensions")?,
        HostPath::from_path_buf(root.join("system/extensions")),
    )?;
    filesystem.mount_local(
        VirtualPath::new("/system/skills")?,
        HostPath::from_path_buf(root.join("system/skills")),
    )?;
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
) -> Result<(), RebornBuildError> {
    for (virtual_root, backend_id, content_kind) in [
        (
            "/projects",
            "standalone-project-files",
            ContentKind::ProjectFile,
        ),
        (
            "/system/extensions",
            "standalone-system-extensions",
            ContentKind::ExtensionPackage,
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
