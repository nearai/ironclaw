//! Custody tests: pre-scoping, the revision gate, and compare-and-swap.

use std::collections::BTreeMap;
use std::sync::Mutex;

use super::*;

/// A material seam that records every key it is asked about and keeps blobs in
/// process memory. Deliberately not named `InMemory*Store`: the extensions
/// family bans that name for test doubles living under `src/`.
#[derive(Default)]
struct RecordingLinkedSessionMaterial {
    records: Mutex<BTreeMap<LinkedSessionKey, MaterialRecord>>,
    loads: Mutex<Vec<LinkedSessionKey>>,
    replaces: Mutex<Vec<LinkedSessionKey>>,
}

struct MaterialRecord {
    link_revision: u64,
    blob: Option<Vec<u8>>,
    version: u64,
}

impl RecordingLinkedSessionMaterial {
    fn with_account(self, key: LinkedSessionKey, link_revision: u64, blob: Option<&[u8]>) -> Self {
        self.records.lock().expect("records").insert(
            key,
            MaterialRecord {
                link_revision,
                blob: blob.map(<[u8]>::to_vec),
                version: if blob.is_some() { 1 } else { 0 },
            },
        );
        self
    }

    fn load_keys(&self) -> Vec<LinkedSessionKey> {
        self.loads.lock().expect("loads").clone()
    }

    fn replace_keys(&self) -> Vec<LinkedSessionKey> {
        self.replaces.lock().expect("replaces").clone()
    }

    fn stored_blob(&self, key: &LinkedSessionKey) -> Option<Vec<u8>> {
        self.records
            .lock()
            .expect("records")
            .get(key)
            .and_then(|record| record.blob.clone())
    }
}

fn version_token(version: u64) -> LinkedSessionVersion {
    if version == 0 {
        LinkedSessionVersion::absent()
    } else {
        LinkedSessionVersion::new(format!("v{version}")).expect("version token")
    }
}

#[async_trait]
impl LinkedSessionMaterialStore for RecordingLinkedSessionMaterial {
    async fn load(
        &self,
        key: &LinkedSessionKey,
    ) -> Result<Option<StoredLinkedSession>, LinkedSessionError> {
        self.loads.lock().expect("loads").push(key.clone());
        let records = self.records.lock().expect("records");
        let Some(record) = records.get(key) else {
            return Ok(None);
        };
        let snapshot = match &record.blob {
            Some(bytes) => Some(LinkedSessionSnapshot {
                blob: SessionBytes::new(bytes.clone())?,
                version: version_token(record.version),
            }),
            None => None,
        };
        Ok(Some(StoredLinkedSession {
            link_revision: record.link_revision,
            snapshot,
        }))
    }

    async fn replace(
        &self,
        key: &LinkedSessionKey,
        expected: LinkedSessionVersion,
        blob: SessionBytes,
    ) -> Result<LinkedSessionVersion, LinkedSessionError> {
        self.replaces.lock().expect("replaces").push(key.clone());
        let mut records = self.records.lock().expect("records");
        let Some(record) = records.get_mut(key) else {
            return Err(LinkedSessionError::Revoked);
        };
        let current = version_token(record.version);
        if current != expected {
            return Err(LinkedSessionError::VersionConflict { current });
        }
        record.blob = Some(blob.expose().to_vec());
        record.version += 1;
        Ok(version_token(record.version))
    }
}

fn extension(id: &str) -> ExtensionId {
    ExtensionId::new(id).expect("extension id")
}

fn account(value: &str) -> LinkedAccountRef {
    LinkedAccountRef::new(value).expect("account ref")
}

fn key(extension_id: &str, account_ref: &str) -> LinkedSessionKey {
    LinkedSessionKey::new(extension(extension_id), account(account_ref))
}

fn blob(bytes: &[u8]) -> SessionBytes {
    SessionBytes::new(bytes.to_vec()).expect("session bytes")
}

#[tokio::test]
async fn a_handle_addresses_only_the_account_its_grant_named() {
    let material = Arc::new(
        RecordingLinkedSessionMaterial::default()
            .with_account(key("acme-link", "acct-a"), 1, Some(b"alpha"))
            .with_account(key("acme-link", "acct-b"), 1, Some(b"beta")),
    );
    let store =
        LinkedSessionStore::new(Arc::clone(&material) as Arc<dyn LinkedSessionMaterialStore>);
    let custody = store.custody_for(extension("acme-link"));

    let first = custody.open(&LinkedAccountGrant::new(account("acct-a"), 1));
    let second = custody.open(&LinkedAccountGrant::new(account("acct-b"), 1));

    let first_blob = first
        .load()
        .await
        .expect("first load")
        .expect("first snapshot");
    let second_blob = second
        .load()
        .await
        .expect("second load")
        .expect("second snapshot");

    assert_eq!(first_blob.blob.expose(), b"alpha");
    assert_eq!(second_blob.blob.expose(), b"beta");
    assert_eq!(
        material.load_keys(),
        vec![key("acme-link", "acct-a"), key("acme-link", "acct-b")],
        "each handle reached exactly the key its grant named"
    );
}

#[tokio::test]
async fn one_account_ref_under_two_extensions_is_two_records() {
    let material = Arc::new(
        RecordingLinkedSessionMaterial::default()
            .with_account(key("acme-link", "acct-a"), 1, Some(b"acme"))
            .with_account(key("other-link", "acct-a"), 1, Some(b"other")),
    );
    let store =
        LinkedSessionStore::new(Arc::clone(&material) as Arc<dyn LinkedSessionMaterialStore>);

    let grant = LinkedAccountGrant::new(account("acct-a"), 1);
    let acme = store
        .custody_for(extension("acme-link"))
        .open(&grant)
        .load()
        .await
        .expect("acme load")
        .expect("acme snapshot");
    let other = store
        .custody_for(extension("other-link"))
        .open(&grant)
        .load()
        .await
        .expect("other load")
        .expect("other snapshot");

    assert_eq!(acme.blob.expose(), b"acme");
    assert_eq!(other.blob.expose(), b"other");
}

#[tokio::test]
async fn opening_a_handle_performs_no_io() {
    let material = Arc::new(RecordingLinkedSessionMaterial::default().with_account(
        key("acme-link", "acct-a"),
        1,
        Some(b"alpha"),
    ));
    let store =
        LinkedSessionStore::new(Arc::clone(&material) as Arc<dyn LinkedSessionMaterialStore>);

    let _handle = store
        .custody_for(extension("acme-link"))
        .open(&LinkedAccountGrant::new(account("acct-a"), 1));

    assert!(
        material.load_keys().is_empty() && material.replace_keys().is_empty(),
        "bind stays side-effect-free: opening a handle only allocates"
    );
}

#[tokio::test]
async fn a_stale_link_revision_is_revoked_rather_than_served() {
    let material = Arc::new(RecordingLinkedSessionMaterial::default().with_account(
        key("acme-link", "acct-a"),
        2,
        Some(b"relinked"),
    ));
    let store =
        LinkedSessionStore::new(Arc::clone(&material) as Arc<dyn LinkedSessionMaterialStore>);
    // A handle minted before the relink still names revision 1.
    let stale = store
        .custody_for(extension("acme-link"))
        .open(&LinkedAccountGrant::new(account("acct-a"), 1));

    assert_eq!(
        stale.load().await.expect_err("stale load"),
        LinkedSessionError::Revoked
    );
    assert_eq!(
        stale
            .save(version_token(1), blob(b"stale write"))
            .await
            .expect_err("stale save"),
        LinkedSessionError::Revoked
    );
    assert!(
        material.replace_keys().is_empty(),
        "a stale handle never reaches the material seam"
    );
    assert_eq!(
        material.stored_blob(&key("acme-link", "acct-a")),
        Some(b"relinked".to_vec()),
        "the current credential is untouched"
    );
}

#[tokio::test]
async fn a_missing_account_is_revoked_not_empty() {
    let material = Arc::new(RecordingLinkedSessionMaterial::default());
    let store =
        LinkedSessionStore::new(Arc::clone(&material) as Arc<dyn LinkedSessionMaterialStore>);
    let handle = store
        .custody_for(extension("acme-link"))
        .open(&LinkedAccountGrant::new(account("acct-a"), 1));

    assert_eq!(
        handle.load().await.expect_err("missing account"),
        LinkedSessionError::Revoked,
        "an unlinked account must not read as `nothing stored yet`"
    );
}

#[tokio::test]
async fn a_first_write_expects_the_absent_version() {
    let material = Arc::new(RecordingLinkedSessionMaterial::default().with_account(
        key("acme-link", "acct-a"),
        1,
        None,
    ));
    let store =
        LinkedSessionStore::new(Arc::clone(&material) as Arc<dyn LinkedSessionMaterialStore>);
    let handle = store
        .custody_for(extension("acme-link"))
        .open(&LinkedAccountGrant::new(account("acct-a"), 1));

    assert!(
        handle.load().await.expect("load").is_none(),
        "a minted account with no blob yet reads as absent"
    );
    let version = handle
        .save(LinkedSessionVersion::absent(), blob(b"first"))
        .await
        .expect("first write");
    assert_eq!(version, version_token(1));
    assert_eq!(
        material.stored_blob(&key("acme-link", "acct-a")),
        Some(b"first".to_vec())
    );
}

#[tokio::test]
async fn a_lost_compare_and_swap_returns_the_current_version() {
    let material = Arc::new(RecordingLinkedSessionMaterial::default().with_account(
        key("acme-link", "acct-a"),
        1,
        Some(b"current"),
    ));
    let store =
        LinkedSessionStore::new(Arc::clone(&material) as Arc<dyn LinkedSessionMaterialStore>);
    let handle = store
        .custody_for(extension("acme-link"))
        .open(&LinkedAccountGrant::new(account("acct-a"), 1));

    let error = handle
        .save(LinkedSessionVersion::absent(), blob(b"clobber"))
        .await
        .expect_err("stale expectation loses the swap");
    assert_eq!(
        error,
        LinkedSessionError::VersionConflict {
            current: version_token(1)
        },
        "the loser learns the current version so it can reload and merge"
    );
    assert_eq!(
        material.stored_blob(&key("acme-link", "acct-a")),
        Some(b"current".to_vec()),
        "last-writer-wins would have killed the link"
    );
}

#[tokio::test]
async fn custody_that_is_not_wired_fails_closed() {
    let store = LinkedSessionStore::unavailable();
    let handle = store
        .custody_for(extension("acme-link"))
        .open(&LinkedAccountGrant::new(account("acct-a"), 1));

    assert!(matches!(
        handle.load().await.expect_err("unwired load"),
        LinkedSessionError::Unavailable { .. }
    ));
    assert!(matches!(
        handle
            .save(LinkedSessionVersion::absent(), blob(b"anything"))
            .await
            .expect_err("unwired save"),
        LinkedSessionError::Unavailable { .. }
    ));
}
