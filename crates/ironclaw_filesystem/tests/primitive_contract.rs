use std::collections::BTreeMap;
use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_filesystem::{
    BackendCapabilities, BackendId, BackendKind, BatchPut, CasExpectation, CompositeRootFilesystem,
    ContentKind, DirEntry, Entry, FileStat, FilesystemError, Filter, InMemoryBackend, IndexKey,
    IndexPolicy, IndexValue, MountDescriptor, Page, RecordVersion, RootFilesystem, StorageClass,
    VersionedEntry,
};
use ironclaw_host_api::VirtualPath;

fn vpath(path: impl Into<String>) -> VirtualPath {
    VirtualPath::new(path).unwrap()
}

fn batch_entry(body: &[u8]) -> Entry {
    Entry::bytes(body.to_vec())
}

fn child(prefix: &str, leaf: &str) -> VirtualPath {
    vpath(format!("{prefix}/{leaf}"))
}

async fn single_key_put_batch_contract<F>(fs: &F, path: VirtualPath)
where
    F: RootFilesystem + ?Sized,
{
    let versions = fs
        .put_batch(vec![BatchPut {
            path: path.clone(),
            entry: batch_entry(b"single"),
            cas: CasExpectation::Absent,
        }])
        .await
        .unwrap();

    assert_eq!(versions.len(), 1);
    let got = fs.get(&path).await.unwrap().unwrap();
    assert_eq!(got.version, versions[0]);
    assert_eq!(got.entry.body, b"single");
}

async fn put_batch_all_or_nothing_contract<F>(fs: &F, prefix: &str)
where
    F: RootFilesystem + ?Sized,
{
    let existing = child(prefix, "existing.json");
    let original = fs
        .put(
            &existing,
            batch_entry(b"already committed"),
            CasExpectation::Absent,
        )
        .await
        .unwrap();
    let new_a = child(prefix, "a.json");
    let new_b = child(prefix, "b.json");
    let any = child(prefix, "any.json");

    let versions = fs
        .put_batch(vec![
            BatchPut {
                path: new_a.clone(),
                entry: batch_entry(b"a"),
                cas: CasExpectation::Absent,
            },
            BatchPut {
                path: existing.clone(),
                entry: batch_entry(b"updated"),
                cas: CasExpectation::Version(original),
            },
            BatchPut {
                path: new_b.clone(),
                entry: batch_entry(b"b"),
                cas: CasExpectation::Absent,
            },
            BatchPut {
                path: any.clone(),
                entry: batch_entry(b"any"),
                cas: CasExpectation::Any,
            },
        ])
        .await
        .unwrap();
    assert_eq!(versions.len(), 4);

    assert_eq!(fs.get(&new_a).await.unwrap().unwrap().entry.body, b"a");
    assert_eq!(fs.get(&new_b).await.unwrap().unwrap().entry.body, b"b");
    assert_eq!(fs.get(&any).await.unwrap().unwrap().entry.body, b"any");
    let after_update = fs.get(&existing).await.unwrap().unwrap();
    assert_eq!(after_update.entry.body, b"updated");
    assert!(after_update.version > original);

    let should_not_write = child(prefix, "should_not_write.json");
    let also_should_not_write = child(prefix, "also_should_not_write.json");
    let err = fs
        .put_batch(vec![
            BatchPut {
                path: should_not_write.clone(),
                entry: batch_entry(b"nope"),
                cas: CasExpectation::Absent,
            },
            BatchPut {
                path: existing.clone(),
                entry: batch_entry(b"stale"),
                cas: CasExpectation::Version(original),
            },
            BatchPut {
                path: also_should_not_write.clone(),
                entry: batch_entry(b"nope"),
                cas: CasExpectation::Absent,
            },
        ])
        .await
        .unwrap_err();

    assert!(
        matches!(
            &err,
            FilesystemError::VersionMismatch {
                path,
                expected: Some(version),
                ..
            } if *path == existing && *version == original
        ),
        "expected stale VersionMismatch on {existing:?}, got {err:?}"
    );
    assert!(fs.get(&should_not_write).await.unwrap().is_none());
    assert!(fs.get(&also_should_not_write).await.unwrap().is_none());
    assert_eq!(
        fs.get(&existing).await.unwrap().unwrap().entry.body,
        b"updated",
        "failed batch must leave the previously committed row untouched"
    );
}

async fn drive_equivalence_sequence<F>(fs: &F, prefix: &str)
where
    F: RootFilesystem + ?Sized,
{
    fs.put_batch(vec![
        BatchPut {
            path: child(prefix, "a.json"),
            entry: batch_entry(b"a"),
            cas: CasExpectation::Absent,
        },
        BatchPut {
            path: child(prefix, "b.json"),
            entry: batch_entry(b"b"),
            cas: CasExpectation::Absent,
        },
    ])
    .await
    .unwrap();
}

async fn dump_records<F>(
    fs: &F,
    prefix: &str,
) -> Vec<(String, Vec<u8>, BTreeMap<IndexKey, IndexValue>, u64)>
where
    F: RootFilesystem + ?Sized,
{
    let mut rows: Vec<_> = fs
        .query(&vpath(prefix), &Filter::All, Page::first(100))
        .await
        .unwrap()
        .into_iter()
        .map(|versioned| {
            (
                versioned.path.as_str().to_string(),
                versioned.entry.body,
                versioned.entry.indexed,
                versioned.version.get(),
            )
        })
        .collect();
    rows.sort_by(|left, right| left.0.cmp(&right.0));
    rows
}

#[tokio::test]
async fn in_memory_put_batch_single_key_uses_default_fast_path() {
    let fs = InMemoryBackend::new();
    single_key_put_batch_contract(&fs, vpath("/secrets/leases/single.json")).await;
}

#[tokio::test]
async fn in_memory_put_batch_is_all_or_nothing_for_multi_key_default_path() {
    let fs = InMemoryBackend::new();
    put_batch_all_or_nothing_contract(&fs, "/secrets/leases/inmem_batch").await;
}

#[tokio::test]
async fn composite_put_batch_rejects_cross_mount_batch_before_writing() {
    let left = Arc::new(InMemoryBackend::new());
    let right = Arc::new(InMemoryBackend::new());
    let mut root = CompositeRootFilesystem::new();
    root.mount(descriptor("/secrets", "left"), Arc::clone(&left))
        .unwrap();
    root.mount(descriptor("/memory", "right"), Arc::clone(&right))
        .unwrap();

    let left_path = vpath("/secrets/a.json");
    let right_path = vpath("/memory/b.json");
    let err = root
        .put_batch(vec![
            BatchPut {
                path: left_path.clone(),
                entry: batch_entry(b"a"),
                cas: CasExpectation::Absent,
            },
            BatchPut {
                path: right_path.clone(),
                entry: batch_entry(b"b"),
                cas: CasExpectation::Absent,
            },
        ])
        .await
        .unwrap_err();

    assert!(matches!(err, FilesystemError::PathOutsideMount { .. }));
    assert!(left.get(&left_path).await.unwrap().is_none());
    assert!(right.get(&right_path).await.unwrap().is_none());
}

#[tokio::test]
async fn native_vs_default_wrapper_equivalence_uses_non_overriding_newtype() {
    let native = InMemoryBackend::new();
    let default_inner = Arc::new(InMemoryBackend::new());
    let default = DefaultOnlyBackend {
        inner: Arc::clone(&default_inner),
    };
    let prefix = "/resources/accounts/equivalence";

    drive_equivalence_sequence(&native, prefix).await;
    drive_equivalence_sequence(&default, prefix).await;

    assert_eq!(
        dump_records(&native, prefix).await,
        dump_records(&default, prefix).await
    );
}

fn descriptor(virtual_root: &str, backend_id: &str) -> MountDescriptor {
    MountDescriptor {
        virtual_root: vpath(virtual_root),
        backend_id: BackendId::new(backend_id).unwrap(),
        backend_kind: BackendKind::MemoryDocuments,
        storage_class: StorageClass::StructuredRecords,
        content_kind: ContentKind::StructuredRecord,
        index_policy: IndexPolicy::NotIndexed,
        capabilities: BackendCapabilities::in_memory_full(),
    }
}

struct DefaultOnlyBackend {
    inner: Arc<InMemoryBackend>,
}

#[async_trait]
impl RootFilesystem for DefaultOnlyBackend {
    fn capabilities(&self) -> BackendCapabilities {
        self.inner.capabilities()
    }

    async fn put(
        &self,
        path: &VirtualPath,
        entry: Entry,
        cas: CasExpectation,
    ) -> Result<RecordVersion, FilesystemError> {
        self.inner.put(path, entry, cas).await
    }

    async fn get(&self, path: &VirtualPath) -> Result<Option<VersionedEntry>, FilesystemError> {
        self.inner.get(path).await
    }

    async fn list_dir(&self, path: &VirtualPath) -> Result<Vec<DirEntry>, FilesystemError> {
        self.inner.list_dir(path).await
    }

    async fn query(
        &self,
        path: &VirtualPath,
        filter: &Filter,
        page: Page,
    ) -> Result<Vec<VersionedEntry>, FilesystemError> {
        self.inner.query(path, filter, page).await
    }

    async fn stat(&self, path: &VirtualPath) -> Result<FileStat, FilesystemError> {
        self.inner.stat(path).await
    }

    async fn begin(
        &self,
        path: &VirtualPath,
    ) -> Result<Box<dyn ironclaw_filesystem::StorageTxn>, FilesystemError> {
        self.inner.begin(path).await
    }
}

#[cfg(feature = "libsql")]
mod libsql_tests {
    use super::*;
    use ironclaw_filesystem::LibSqlRootFilesystem;

    struct TestLibSqlRootFilesystem {
        filesystem: LibSqlRootFilesystem,
        _dir: tempfile::TempDir,
    }

    impl std::ops::Deref for TestLibSqlRootFilesystem {
        type Target = LibSqlRootFilesystem;

        fn deref(&self) -> &Self::Target {
            &self.filesystem
        }
    }

    async fn libsql_root() -> TestLibSqlRootFilesystem {
        let db_dir = tempfile::tempdir().unwrap();
        let db_path = db_dir.path().join("root-filesystem.db");
        let db = Arc::new(libsql::Builder::new_local(db_path).build().await.unwrap());
        let filesystem = LibSqlRootFilesystem::new(db);
        filesystem.run_migrations().await.unwrap();
        TestLibSqlRootFilesystem {
            filesystem,
            _dir: db_dir,
        }
    }

    #[tokio::test]
    async fn libsql_put_batch_single_key_uses_default_fast_path() {
        let fs = libsql_root().await;
        single_key_put_batch_contract(&*fs, vpath("/secrets/leases/libsql_single.json")).await;
    }
}

#[cfg(feature = "postgres")]
mod postgres_tests {
    use super::*;
    use ironclaw_filesystem::PostgresRootFilesystem;

    async fn postgres_pool() -> Option<deadpool_postgres::Pool> {
        if std::env::var("IRONCLAW_SKIP_POSTGRES_TESTS").is_ok() {
            return None;
        }
        let url = std::env::var("IRONCLAW_FILESYSTEM_POSTGRES_URL")
            .or_else(|_| std::env::var("DATABASE_URL"))
            .ok()?;
        let config = url.parse::<tokio_postgres::Config>().ok()?;
        let manager = deadpool_postgres::Manager::new(config, tokio_postgres::NoTls);
        deadpool_postgres::Pool::builder(manager)
            .max_size(4)
            .build()
            .ok()
    }

    async fn postgres_root() -> Option<(PostgresRootFilesystem, String)> {
        let pool = postgres_pool().await?;
        let fs = PostgresRootFilesystem::new(pool);
        fs.run_migrations().await.ok()?;
        let prefix = format!(
            "/secrets/leases/pg_primitive_{}",
            uuid::Uuid::new_v4().simple()
        );
        Some((fs, prefix))
    }

    #[tokio::test]
    async fn postgres_put_batch_single_key_uses_default_fast_path() {
        let Some((fs, prefix)) = postgres_root().await else {
            return;
        };
        single_key_put_batch_contract(&fs, child(&prefix, "single.json")).await;
    }

    #[tokio::test]
    async fn postgres_put_batch_is_all_or_nothing_for_multi_key_default_path() {
        let Some((fs, prefix)) = postgres_root().await else {
            return;
        };
        put_batch_all_or_nothing_contract(&fs, &prefix).await;
    }
}
