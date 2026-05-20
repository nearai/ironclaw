//! Encryption-at-rest decorator for [`RootFilesystem`] backends.

use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_host_api::VirtualPath;
use serde::{Deserialize, Serialize};

use crate::{
    BackendCapabilities, Capability, CasExpectation, DirEntry, Entry, FilesystemError,
    FilesystemOperation, Filter, IndexValue, RecordVersion, RootFilesystem, SeqNo, TxnCapability,
    VersionedEntry,
};
use crate::{EventRecord, FileStat, IndexSpec, Page};

const ENCRYPTED_BYTES_VERSION: u8 = 1;

pub trait EntryCipher: Send + Sync {
    fn encrypt_entry_bytes(
        &self,
        plaintext: &[u8],
        aad: &[u8],
    ) -> Result<(Vec<u8>, Vec<u8>), String>;

    fn decrypt_entry_bytes(
        &self,
        ciphertext: &[u8],
        salt: &[u8],
        aad: &[u8],
    ) -> Result<Vec<u8>, String>;
}

#[derive(Debug)]
pub struct EncryptedBackend<F, C> {
    inner: Arc<F>,
    cipher: Arc<C>,
}

impl<F, C> EncryptedBackend<F, C> {
    pub fn new(inner: Arc<F>, cipher: Arc<C>) -> Self {
        Self { inner, cipher }
    }
}

#[async_trait]
impl<F, C> RootFilesystem for EncryptedBackend<F, C>
where
    F: RootFilesystem + 'static,
    C: EntryCipher + 'static,
{
    fn capabilities(&self) -> BackendCapabilities {
        let inner = self.inner.capabilities();
        let mut capabilities = BackendCapabilities::empty();
        for capability in inner
            .iter()
            .filter(|capability| *capability != Capability::Events)
        {
            capabilities = capabilities.with(capability);
        }
        let txn = match inner.txn() {
            TxnCapability::None => TxnCapability::None,
            TxnCapability::Cas | TxnCapability::MultiKey => TxnCapability::Cas,
        };
        capabilities.with_txn(txn)
    }

    async fn put(
        &self,
        path: &VirtualPath,
        entry: Entry,
        cas: CasExpectation,
    ) -> Result<RecordVersion, FilesystemError> {
        let entry = self.encrypt_entry(path, entry, FilesystemOperation::WriteFile)?;
        self.inner.put(path, entry, cas).await
    }

    async fn get(&self, path: &VirtualPath) -> Result<Option<VersionedEntry>, FilesystemError> {
        let Some(versioned) = self.inner.get(path).await? else {
            return Ok(None);
        };
        self.decrypt_versioned_entry(versioned, FilesystemOperation::ReadFile)
            .map(Some)
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
        let entries = self.inner.query(path, filter, page).await?;
        entries
            .into_iter()
            .map(|entry| self.decrypt_versioned_entry(entry, FilesystemOperation::Query))
            .collect()
    }

    async fn ensure_index(
        &self,
        path: &VirtualPath,
        spec: &IndexSpec,
    ) -> Result<(), FilesystemError> {
        self.inner.ensure_index(path, spec).await
    }

    async fn stat(&self, path: &VirtualPath) -> Result<FileStat, FilesystemError> {
        self.inner.stat(path).await
    }

    async fn delete(&self, path: &VirtualPath) -> Result<(), FilesystemError> {
        self.inner.delete(path).await
    }

    async fn append(
        &self,
        path: &VirtualPath,
        _payload: Vec<u8>,
    ) -> Result<SeqNo, FilesystemError> {
        Err(FilesystemError::Unsupported {
            path: path.clone(),
            operation: FilesystemOperation::Tail,
        })
    }

    async fn tail(
        &self,
        path: &VirtualPath,
        _from: SeqNo,
    ) -> Result<Vec<EventRecord>, FilesystemError> {
        Err(FilesystemError::Unsupported {
            path: path.clone(),
            operation: FilesystemOperation::Tail,
        })
    }
}

impl<F, C> EncryptedBackend<F, C>
where
    C: EntryCipher,
{
    fn encrypt_entry(
        &self,
        path: &VirtualPath,
        mut entry: Entry,
        operation: FilesystemOperation,
    ) -> Result<Entry, FilesystemError> {
        entry.body = self.encrypt_bytes(path, "body", entry.body, operation)?;
        for (key, value) in entry.indexed.iter_mut() {
            if let IndexValue::Bytes(bytes) = value {
                *bytes = self.encrypt_bytes(
                    path,
                    &format!("indexed:{}", key.as_str()),
                    bytes.clone(),
                    operation,
                )?;
            }
        }
        Ok(entry)
    }

    fn decrypt_versioned_entry(
        &self,
        mut versioned: VersionedEntry,
        operation: FilesystemOperation,
    ) -> Result<VersionedEntry, FilesystemError> {
        versioned.entry.body =
            self.decrypt_bytes(&versioned.path, "body", &versioned.entry.body, operation)?;
        for (key, value) in versioned.entry.indexed.iter_mut() {
            if let IndexValue::Bytes(bytes) = value {
                *bytes = self.decrypt_bytes(
                    &versioned.path,
                    &format!("indexed:{}", key.as_str()),
                    bytes,
                    operation,
                )?;
            }
        }
        Ok(versioned)
    }

    fn encrypt_bytes(
        &self,
        path: &VirtualPath,
        component: &str,
        plaintext: Vec<u8>,
        operation: FilesystemOperation,
    ) -> Result<Vec<u8>, FilesystemError> {
        let aad = encrypted_entry_aad(path, component);
        let (ciphertext, salt) = self
            .cipher
            .encrypt_entry_bytes(&plaintext, &aad)
            .map_err(|reason| encrypted_backend_error(path, operation, reason))?;
        let envelope = EncryptedBytes {
            version: ENCRYPTED_BYTES_VERSION,
            salt,
            ciphertext,
        };
        serde_json::to_vec(&envelope).map_err(|error| {
            encrypted_backend_error(
                path,
                operation,
                format!("serialize encrypted entry: {error}"),
            )
        })
    }

    fn decrypt_bytes(
        &self,
        path: &VirtualPath,
        component: &str,
        encrypted: &[u8],
        operation: FilesystemOperation,
    ) -> Result<Vec<u8>, FilesystemError> {
        let envelope: EncryptedBytes = serde_json::from_slice(encrypted).map_err(|error| {
            encrypted_backend_error(path, operation, format!("parse encrypted entry: {error}"))
        })?;
        if envelope.version != ENCRYPTED_BYTES_VERSION {
            return Err(encrypted_backend_error(
                path,
                operation,
                "unsupported encrypted entry version".to_string(),
            ));
        }
        let aad = encrypted_entry_aad(path, component);
        self.cipher
            .decrypt_entry_bytes(&envelope.ciphertext, &envelope.salt, &aad)
            .map_err(|reason| encrypted_backend_error(path, operation, reason))
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct EncryptedBytes {
    version: u8,
    salt: Vec<u8>,
    ciphertext: Vec<u8>,
}

fn encrypted_entry_aad(path: &VirtualPath, component: &str) -> Vec<u8> {
    format!(
        "ironclaw_filesystem:encrypted-entry:v1\0{}\0{}",
        path.as_str(),
        component
    )
    .into_bytes()
}

fn encrypted_backend_error(
    path: &VirtualPath,
    operation: FilesystemOperation,
    reason: String,
) -> FilesystemError {
    FilesystemError::Backend {
        path: path.clone(),
        operation,
        reason: format!("encrypted filesystem backend error: {reason}"),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use ironclaw_host_api::VirtualPath;

    use super::*;
    use crate::{Entry, InMemoryBackend};

    #[derive(Debug)]
    struct XorCipher;

    impl EntryCipher for XorCipher {
        fn encrypt_entry_bytes(
            &self,
            plaintext: &[u8],
            aad: &[u8],
        ) -> Result<(Vec<u8>, Vec<u8>), String> {
            Ok((xor(plaintext, aad), vec![1]))
        }

        fn decrypt_entry_bytes(
            &self,
            ciphertext: &[u8],
            _salt: &[u8],
            aad: &[u8],
        ) -> Result<Vec<u8>, String> {
            Ok(xor(ciphertext, aad))
        }
    }

    #[tokio::test]
    async fn encrypted_backend_hides_body_from_inner_backend() {
        let inner = Arc::new(InMemoryBackend::new());
        let backend = EncryptedBackend::new(Arc::clone(&inner), Arc::new(XorCipher));
        let path = VirtualPath::new("/secrets/record.json").unwrap();
        backend
            .put(
                &path,
                Entry::record(
                    crate::RecordKind::new("secret_record").unwrap(),
                    &serde_json::json!({"value":"sk-test"}),
                )
                .unwrap(),
                CasExpectation::Absent,
            )
            .await
            .unwrap();

        let raw = inner.get(&path).await.unwrap().unwrap();
        assert!(!String::from_utf8_lossy(&raw.entry.body).contains("sk-test"));

        let decrypted = backend.get(&path).await.unwrap().unwrap();
        let parsed: serde_json::Value = decrypted.entry.parse_json().unwrap();
        assert_eq!(parsed["value"], "sk-test");
    }

    fn xor(bytes: &[u8], aad: &[u8]) -> Vec<u8> {
        bytes
            .iter()
            .enumerate()
            .map(|(index, byte)| byte ^ aad[index % aad.len()])
            .collect()
    }
}
