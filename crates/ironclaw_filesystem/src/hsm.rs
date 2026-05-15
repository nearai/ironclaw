//! Placeholder HSM-style backend demonstrating the universal dispatch seam.
//!
//! `HsmBackend` is intentionally minimal: it shows what a backend looks like
//! when its storage primitives sit behind an external boundary (a hardware
//! security module, a TEE-resident KMS, an OS keychain). A production HSM
//! implementation would replace [`HsmBackend::new`] with constructors that
//! accept an HSM session handle and would route [`put`](RootFilesystem::put) /
//! [`get`](RootFilesystem::get) / [`delete`](RootFilesystem::delete) through
//! the HSM's encrypt/decrypt API.
//!
//! The point of having it in-tree is to prove that adding a new backend is a
//! single-file change. The seam this demonstrates:
//!
//! 1. **One trait.** `HsmBackend` implements `RootFilesystem` and nothing
//!    else; the composite dispatcher routes through it like any other mount.
//! 2. **Declared capabilities.** The HSM exposes only the encrypted-bytes
//!    surface (`Read` / `Write` / `Stat` / `Delete`) — no records, no query,
//!    no index, no events, no transactions. `BackendCapabilities` advertises
//!    that up front; `CompositeRootFilesystem::mount_dyn` then refuses any
//!    `MountDescriptor` that claims more than the HSM delivers
//!    (`FilesystemError::DescriptorOverclaims`). Consumers cannot accidentally
//!    attach a query-requiring store onto an HSM mount.
//! 3. **Swap by wiring, not by consumer edits.** A consumer that holds a
//!    `ScopedFilesystem` bound to `/system/secrets` does not change when the
//!    mount swaps from `LibSqlRootFilesystem` to `HsmBackend` — only the
//!    `mount()` call at startup changes.
//!
//! The placeholder stores ciphertext in process memory so the trait can be
//! exercised end-to-end in tests. It is not a security boundary. Real HSM
//! backends are sealed behind external infrastructure.

use async_trait::async_trait;
use ironclaw_host_api::VirtualPath;

use crate::in_memory::InMemoryBackend;
use crate::{
    BackendCapabilities, Capability, CasExpectation, DirEntry, Entry, FileStat, FilesystemError,
    FilesystemOperation, RecordVersion, RootFilesystem, TxnCapability, VersionedEntry,
};

/// Placeholder HSM-style backend. See the module-level docs.
pub struct HsmBackend {
    inner: InMemoryBackend,
}

impl HsmBackend {
    pub fn new() -> Self {
        Self {
            inner: InMemoryBackend::new(),
        }
    }

    fn declared_capabilities() -> BackendCapabilities {
        BackendCapabilities::empty()
            .with(Capability::Read)
            .with(Capability::Write)
            .with(Capability::Stat)
            .with(Capability::Delete)
            .with_txn(TxnCapability::Cas)
    }
}

impl Default for HsmBackend {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl RootFilesystem for HsmBackend {
    fn capabilities(&self) -> BackendCapabilities {
        Self::declared_capabilities()
    }

    async fn put(
        &self,
        path: &VirtualPath,
        entry: Entry,
        cas: CasExpectation,
    ) -> Result<RecordVersion, FilesystemError> {
        if entry.kind.is_some() || !entry.indexed.is_empty() {
            return Err(FilesystemError::Unsupported {
                path: path.clone(),
                operation: FilesystemOperation::WriteFile,
            });
        }
        self.inner.put(path, entry, cas).await
    }

    async fn get(&self, path: &VirtualPath) -> Result<Option<VersionedEntry>, FilesystemError> {
        self.inner.get(path).await
    }

    async fn list_dir(&self, path: &VirtualPath) -> Result<Vec<DirEntry>, FilesystemError> {
        self.inner.list_dir(path).await
    }

    async fn stat(&self, path: &VirtualPath) -> Result<FileStat, FilesystemError> {
        self.inner.stat(path).await
    }

    async fn delete(&self, path: &VirtualPath) -> Result<(), FilesystemError> {
        self.inner.delete(path).await
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use ironclaw_host_api::VirtualPath;

    use crate::{
        BackendCapabilities, BackendId, BackendKind, Capability, CasExpectation,
        CompositeRootFilesystem, ContentKind, Entry, FilesystemError, IndexKey, IndexKind,
        IndexName, IndexPolicy, IndexSpec, IndexValue, MountDescriptor, RootFilesystem,
        StorageClass, TxnCapability,
    };

    use super::HsmBackend;

    fn vpath(value: &str) -> VirtualPath {
        VirtualPath::new(value).unwrap()
    }

    fn hsm_descriptor(capabilities: BackendCapabilities) -> MountDescriptor {
        MountDescriptor {
            virtual_root: vpath("/secrets"),
            backend_id: BackendId::new("hsm-secrets").unwrap(),
            backend_kind: BackendKind::Custom("hsm".into()),
            storage_class: StorageClass::FileContent,
            content_kind: ContentKind::SystemState,
            index_policy: IndexPolicy::NotIndexed,
            capabilities,
        }
    }

    #[tokio::test]
    async fn hsm_supports_encrypted_bytes_round_trip() {
        let hsm = HsmBackend::new();
        let path = vpath("/secrets/account/api-key");

        let version = hsm
            .put(
                &path,
                Entry::bytes(b"ciphertext-blob".to_vec()),
                CasExpectation::Absent,
            )
            .await
            .unwrap();

        let read_back = hsm.get(&path).await.unwrap().expect("entry present");
        assert_eq!(read_back.entry.body, b"ciphertext-blob");
        assert_eq!(read_back.version, version);
    }

    #[tokio::test]
    async fn hsm_rejects_structured_records() {
        let hsm = HsmBackend::new();
        let path = vpath("/secrets/account/api-key");
        let entry = Entry::record(
            crate::RecordKind::new("credential_lease").unwrap(),
            &serde_json::json!({"scope": "team"}),
        )
        .unwrap();

        let err = hsm
            .put(&path, entry, CasExpectation::Any)
            .await
            .unwrap_err();
        assert!(matches!(err, FilesystemError::Unsupported { .. }));
    }

    #[tokio::test]
    async fn hsm_rejects_query_and_index_ops() {
        let hsm = HsmBackend::new();
        let path = vpath("/secrets");

        let query_err = hsm
            .query(&path, &crate::Filter::All, crate::Page::new(0, 10))
            .await
            .unwrap_err();
        assert!(matches!(query_err, FilesystemError::Unsupported { .. }));

        let spec = IndexSpec::new(
            IndexName::new("by_scope").unwrap(),
            vec![IndexKey::new("scope").unwrap()],
            IndexKind::Exact,
        );
        let index_err = hsm.ensure_index(&path, &spec).await.unwrap_err();
        assert!(matches!(index_err, FilesystemError::Unsupported { .. }));
    }

    #[tokio::test]
    async fn composite_rejects_overclaimed_hsm_descriptor() {
        let mut composite = CompositeRootFilesystem::new();
        let over_claimed = BackendCapabilities::empty()
            .with(Capability::Read)
            .with(Capability::Write)
            .with(Capability::Stat)
            .with(Capability::Delete)
            .with(Capability::Query)
            .with(Capability::IndexExact);

        let err = composite
            .mount_dyn(hsm_descriptor(over_claimed), Arc::new(HsmBackend::new()))
            .unwrap_err();

        match err {
            FilesystemError::DescriptorOverclaims { missing, .. } => {
                assert!(missing.contains(&Capability::Query));
                assert!(missing.contains(&Capability::IndexExact));
            }
            other => panic!("expected DescriptorOverclaims, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn composite_routes_to_hsm_under_secrets_mount() {
        // Demonstrates the swap-by-wiring acceptance gate: a /secrets mount
        // can point at HsmBackend with no consumer changes. Consumer code
        // sees the same RootFilesystem trait — capability validation at
        // mount time ensures the descriptor never over-claims.
        let mut composite = CompositeRootFilesystem::new();
        let honest_descriptor = hsm_descriptor(
            BackendCapabilities::empty()
                .with(Capability::Read)
                .with(Capability::Write)
                .with(Capability::Stat)
                .with(Capability::Delete)
                .with_txn(TxnCapability::Cas),
        );
        composite
            .mount_dyn(honest_descriptor, Arc::new(HsmBackend::new()))
            .unwrap();

        let path = vpath("/secrets/account/key");
        composite
            .put(
                &path,
                Entry::bytes(b"ciphertext".to_vec()),
                CasExpectation::Absent,
            )
            .await
            .unwrap();

        // Read back via the composite — same surface the consumer would use.
        let read = composite.get(&path).await.unwrap().expect("entry present");
        assert_eq!(read.entry.body, b"ciphertext");

        // Indexed values are still rejected because the HSM declared no
        // index/query capability — even though the consumer code path
        // didn't change.
        let with_index = Entry::bytes(b"more".to_vec()).with_indexed(
            IndexKey::new("scope").unwrap(),
            IndexValue::Text("team".into()),
        );
        let err = composite
            .put(&path, with_index, CasExpectation::Any)
            .await
            .unwrap_err();
        assert!(matches!(err, FilesystemError::Unsupported { .. }));
    }
}
