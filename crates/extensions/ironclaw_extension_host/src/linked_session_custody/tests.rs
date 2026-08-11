//! Custody tests: pre-scoping, the two custody spaces (provisional vs
//! durable), the directory, and the revision gate over the real auth fake.

use std::collections::BTreeMap;
use std::sync::Mutex;

use ironclaw_auth::{CredentialAccountService, InMemoryAuthProductServices, NewCredentialAccount};
use ironclaw_host_api::ids::{InvocationId, SecretHandle, UserId};
use ironclaw_host_api::resource::ResourceScope;

use super::*;

/// A material seam that records every key it is asked about and keeps blobs in
/// process memory, keyed by account id. Deliberately not named
/// `InMemory*Store`: the extensions family bans that name for test doubles
/// living under `src/`.
#[derive(Default)]
struct RecordingLinkedSessionMaterial {
    records: Mutex<BTreeMap<CredentialAccountId, MaterialRecord>>,
    loads: Mutex<Vec<LinkedSessionMaterialKey>>,
    replaces: Mutex<Vec<LinkedSessionMaterialKey>>,
}

struct MaterialRecord {
    link_revision: u64,
    blob: Option<Vec<u8>>,
    version: u64,
}

impl RecordingLinkedSessionMaterial {
    fn with_account(
        self,
        account_id: CredentialAccountId,
        link_revision: u64,
        blob: Option<&[u8]>,
    ) -> Self {
        self.records.lock().expect("records").insert(
            account_id,
            MaterialRecord {
                link_revision,
                blob: blob.map(<[u8]>::to_vec),
                version: if blob.is_some() { 1 } else { 0 },
            },
        );
        self
    }

    fn load_keys(&self) -> Vec<LinkedSessionMaterialKey> {
        self.loads.lock().expect("loads").clone()
    }

    fn replace_keys(&self) -> Vec<LinkedSessionMaterialKey> {
        self.replaces.lock().expect("replaces").clone()
    }

    fn stored_blob(&self, account_id: &CredentialAccountId) -> Option<Vec<u8>> {
        self.records
            .lock()
            .expect("records")
            .get(account_id)
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
        key: &LinkedSessionMaterialKey,
    ) -> Result<Option<LinkedSessionSnapshot>, LinkedSessionError> {
        self.loads.lock().expect("loads").push(key.clone());
        let records = self.records.lock().expect("records");
        let Some(record) = records.get(&key.account_id) else {
            // The account no longer exists: mirroring the production mapping,
            // where CredentialMissing collapses to Revoked.
            return Err(LinkedSessionError::Revoked);
        };
        if record.link_revision != key.link_revision {
            return Err(LinkedSessionError::Revoked);
        }
        Ok(match &record.blob {
            Some(bytes) => Some(LinkedSessionSnapshot {
                blob: SessionBytes::new(bytes.clone())?,
                version: version_token(record.version),
            }),
            None => None,
        })
    }

    async fn replace(
        &self,
        key: &LinkedSessionMaterialKey,
        expected: LinkedSessionVersion,
        blob: SessionBytes,
    ) -> Result<LinkedSessionVersion, LinkedSessionError> {
        self.replaces.lock().expect("replaces").push(key.clone());
        let mut records = self.records.lock().expect("records");
        let Some(record) = records.get_mut(&key.account_id) else {
            return Err(LinkedSessionError::Revoked);
        };
        if record.link_revision != key.link_revision {
            return Err(LinkedSessionError::Revoked);
        }
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

fn account_ref_for(account_id: &CredentialAccountId) -> LinkedAccountRef {
    LinkedAccountRef::new(account_id.to_string()).expect("account ref")
}

fn blob(bytes: &[u8]) -> SessionBytes {
    SessionBytes::new(bytes.to_vec()).expect("session bytes")
}

fn scope(user: &str) -> AuthProductScope {
    AuthProductScope::new(
        ResourceScope::local_default(UserId::new(user).expect("user id"), InvocationId::new())
            .expect("resource scope"),
        ironclaw_auth::AuthSurface::Web,
    )
}

// -------------------------------------------------------------------------
// The provisional (pre-mint) space
// -------------------------------------------------------------------------

#[tokio::test]
async fn a_provisional_handle_parks_blobs_in_process_memory() {
    let material = Arc::new(RecordingLinkedSessionMaterial::default());
    let store =
        LinkedSessionStore::new(Arc::clone(&material) as Arc<dyn LinkedSessionMaterialStore>);
    let pending = LinkedAccountRef::new("pending-link.flow-1").expect("ref");
    let handle = store
        .custody_for(extension("acme-link"))
        .open(&LinkedAccountGrant::new(pending.clone(), 0));

    assert!(
        handle.load().await.expect("empty load").is_none(),
        "nothing parked yet"
    );
    let v1 = handle
        .save(LinkedSessionVersion::absent(), blob(b"handshake"))
        .await
        .expect("first provisional save");
    let snapshot = handle
        .load()
        .await
        .expect("load")
        .expect("the parked blob reads back");
    assert_eq!(snapshot.blob.expose(), b"handshake");
    assert_eq!(snapshot.version, v1);

    // The adapter's own compare-and-swap discipline holds pre-mint too.
    let error = handle
        .save(LinkedSessionVersion::absent(), blob(b"clobber"))
        .await
        .expect_err("a stale expectation loses");
    assert_eq!(
        error,
        LinkedSessionError::VersionConflict {
            current: v1.clone()
        }
    );
    let v2 = handle
        .save(v1, blob(b"handshake-2"))
        .await
        .expect("swap with the current version");
    assert_ne!(v2, LinkedSessionVersion::absent());

    // The host reads the parked blob for the completion mint, then discards.
    assert_eq!(
        store
            .provisional_blob(&extension("acme-link"), &pending)
            .expect("parked blob visible to the host")
            .expose(),
        b"handshake-2"
    );
    store.discard_provisional(&extension("acme-link"), &pending);
    assert!(handle.load().await.expect("load").is_none());
    assert!(
        material.load_keys().is_empty() && material.replace_keys().is_empty(),
        "provisional custody never reaches the durable material seam"
    );
}

#[tokio::test]
async fn the_provisional_space_is_bounded() {
    let store =
        LinkedSessionStore::new(Arc::new(RecordingLinkedSessionMaterial::default())
            as Arc<dyn LinkedSessionMaterialStore>);
    let custody = store.custody_for(extension("acme-link"));
    for index in 0..MAX_PROVISIONAL_SESSIONS {
        let pending = LinkedAccountRef::new(format!("pending-link.flow-{index}")).expect("ref");
        custody
            .open(&LinkedAccountGrant::new(pending, 0))
            .save(LinkedSessionVersion::absent(), blob(b"x"))
            .await
            .expect("within the bound");
    }

    let overflow = LinkedAccountRef::new("pending-link.flow-overflow").expect("ref");
    let error = custody
        .open(&LinkedAccountGrant::new(overflow, 0))
        .save(LinkedSessionVersion::absent(), blob(b"x"))
        .await
        .expect_err("the provisional space is bounded");
    assert!(matches!(error, LinkedSessionError::Unavailable { .. }));
}

// -------------------------------------------------------------------------
// The directory and durable custody routing
// -------------------------------------------------------------------------

#[tokio::test]
async fn an_unregistered_ref_asks_the_caller_to_re_resolve() {
    let material = Arc::new(RecordingLinkedSessionMaterial::default());
    let store =
        LinkedSessionStore::new(Arc::clone(&material) as Arc<dyn LinkedSessionMaterialStore>);
    let account_id = CredentialAccountId::new();
    let handle = store
        .custody_for(extension("acme-link"))
        .open(&LinkedAccountGrant::new(account_ref_for(&account_id), 1));

    let error = handle.load().await.expect_err("unregistered");
    assert!(
        matches!(&error, LinkedSessionError::Unavailable { reason } if reason.contains("re-resolve")),
        "{error:?}"
    );
    let error = handle
        .save(LinkedSessionVersion::absent(), blob(b"anything"))
        .await
        .expect_err("unregistered save");
    assert!(matches!(error, LinkedSessionError::Unavailable { .. }));
    assert!(
        material.load_keys().is_empty() && material.replace_keys().is_empty(),
        "no coordinates, no material access"
    );
}

#[tokio::test]
async fn a_registered_ref_routes_to_its_account_coordinates() {
    let account_id = CredentialAccountId::new();
    let material = Arc::new(RecordingLinkedSessionMaterial::default().with_account(
        account_id,
        2,
        Some(b"alpha"),
    ));
    let store =
        LinkedSessionStore::new(Arc::clone(&material) as Arc<dyn LinkedSessionMaterialStore>);
    let owner = scope("alice");
    store.register_account(
        extension("acme-link"),
        account_ref_for(&account_id),
        owner.clone(),
        account_id,
    );
    let handle = store
        .custody_for(extension("acme-link"))
        .open(&LinkedAccountGrant::new(account_ref_for(&account_id), 2));

    let snapshot = handle
        .load()
        .await
        .expect("load")
        .expect("stored blob reads back");
    assert_eq!(snapshot.blob.expose(), b"alpha");
    let keys = material.load_keys();
    assert_eq!(keys.len(), 1);
    assert_eq!(keys[0].account_id, account_id);
    assert_eq!(keys[0].scope, owner);
    assert_eq!(keys[0].requester_extension.as_str(), "acme-link");
    assert_eq!(keys[0].link_revision, 2);

    store.unregister_account(&extension("acme-link"), &account_ref_for(&account_id));
    let error = handle.load().await.expect_err("unregistered again");
    assert!(matches!(error, LinkedSessionError::Unavailable { .. }));
}

#[tokio::test]
async fn a_handle_addresses_only_the_account_its_grant_named() {
    let first_id = CredentialAccountId::new();
    let second_id = CredentialAccountId::new();
    let material = Arc::new(
        RecordingLinkedSessionMaterial::default()
            .with_account(first_id, 1, Some(b"alpha"))
            .with_account(second_id, 1, Some(b"beta")),
    );
    let store =
        LinkedSessionStore::new(Arc::clone(&material) as Arc<dyn LinkedSessionMaterialStore>);
    store.register_account(
        extension("acme-link"),
        account_ref_for(&first_id),
        scope("alice"),
        first_id,
    );
    store.register_account(
        extension("acme-link"),
        account_ref_for(&second_id),
        scope("bob"),
        second_id,
    );
    let custody = store.custody_for(extension("acme-link"));

    let first = custody.open(&LinkedAccountGrant::new(account_ref_for(&first_id), 1));
    let second = custody.open(&LinkedAccountGrant::new(account_ref_for(&second_id), 1));

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
    let keys = material.load_keys();
    assert_eq!(
        (keys[0].account_id, keys[1].account_id),
        (first_id, second_id),
        "each handle reached exactly the account its grant named"
    );
}

#[tokio::test]
async fn opening_a_handle_performs_no_io() {
    let account_id = CredentialAccountId::new();
    let material = Arc::new(RecordingLinkedSessionMaterial::default().with_account(
        account_id,
        1,
        Some(b"alpha"),
    ));
    let store =
        LinkedSessionStore::new(Arc::clone(&material) as Arc<dyn LinkedSessionMaterialStore>);
    store.register_account(
        extension("acme-link"),
        account_ref_for(&account_id),
        scope("alice"),
        account_id,
    );

    let _handle = store
        .custody_for(extension("acme-link"))
        .open(&LinkedAccountGrant::new(account_ref_for(&account_id), 1));

    assert!(
        material.load_keys().is_empty() && material.replace_keys().is_empty(),
        "bind stays side-effect-free: opening a handle only allocates"
    );
}

#[tokio::test]
async fn a_stale_link_revision_is_revoked_rather_than_served() {
    let account_id = CredentialAccountId::new();
    let material = Arc::new(RecordingLinkedSessionMaterial::default().with_account(
        account_id,
        2,
        Some(b"relinked"),
    ));
    let store =
        LinkedSessionStore::new(Arc::clone(&material) as Arc<dyn LinkedSessionMaterialStore>);
    store.register_account(
        extension("acme-link"),
        account_ref_for(&account_id),
        scope("alice"),
        account_id,
    );
    // A handle minted before the relink still names revision 1.
    let stale = store
        .custody_for(extension("acme-link"))
        .open(&LinkedAccountGrant::new(account_ref_for(&account_id), 1));

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
    assert_eq!(
        material.stored_blob(&account_id),
        Some(b"relinked".to_vec()),
        "the current credential is untouched"
    );
}

#[tokio::test]
async fn a_lost_compare_and_swap_returns_the_current_version() {
    let account_id = CredentialAccountId::new();
    let material = Arc::new(RecordingLinkedSessionMaterial::default().with_account(
        account_id,
        1,
        Some(b"current"),
    ));
    let store =
        LinkedSessionStore::new(Arc::clone(&material) as Arc<dyn LinkedSessionMaterialStore>);
    store.register_account(
        extension("acme-link"),
        account_ref_for(&account_id),
        scope("alice"),
        account_id,
    );
    let handle = store
        .custody_for(extension("acme-link"))
        .open(&LinkedAccountGrant::new(account_ref_for(&account_id), 1));

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
        material.stored_blob(&account_id),
        Some(b"current".to_vec()),
        "last-writer-wins would have killed the link"
    );
}

#[tokio::test]
async fn custody_that_is_not_wired_fails_closed() {
    let store = LinkedSessionStore::unavailable();
    let account_id = CredentialAccountId::new();
    store.register_account(
        extension("acme-link"),
        account_ref_for(&account_id),
        scope("alice"),
        account_id,
    );
    let handle = store
        .custody_for(extension("acme-link"))
        .open(&LinkedAccountGrant::new(account_ref_for(&account_id), 1));

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

// -------------------------------------------------------------------------
// The credential-service-backed material seam, end to end over the auth fake
// -------------------------------------------------------------------------

#[tokio::test]
async fn the_credential_material_seam_round_trips_through_the_auth_domain() {
    let auth = Arc::new(InMemoryAuthProductServices::new());
    let owner = scope("alice");
    let ext = extension("acme-link");
    let account = auth
        .create_account(NewCredentialAccount::for_linked_device(
            owner.clone(),
            ironclaw_auth::AuthProviderId::new("acme-link").expect("provider"),
            ironclaw_auth::CredentialAccountLabel::new("Linked").expect("label"),
            ext.clone(),
            SecretHandle::new("linked-session").expect("handle"),
        ))
        .await
        .expect("create");
    let account = auth
        .bump_link_revision(&owner, account.id)
        .await
        .expect("bump");
    assert_eq!(account.link_revision, 1);

    let store = LinkedSessionStore::new(Arc::new(CredentialServiceLinkedSessionMaterial::new(
        Arc::clone(&auth) as Arc<dyn CredentialAccountService>,
    )));
    store.register_account(
        ext.clone(),
        account_ref_for(&account.id),
        owner.clone(),
        account.id,
    );
    let handle = store
        .custody_for(ext.clone())
        .open(&LinkedAccountGrant::new(account_ref_for(&account.id), 1));

    assert!(handle.load().await.expect("load").is_none());
    let v1 = handle
        .save(LinkedSessionVersion::absent(), blob(b"blob-1"))
        .await
        .expect("first durable save");
    let snapshot = handle.load().await.expect("load").expect("stored");
    assert_eq!(snapshot.blob.expose(), b"blob-1");
    assert_eq!(snapshot.version, v1);

    // The auth-domain compare-and-swap surfaces as a version conflict.
    let error = handle
        .save(LinkedSessionVersion::absent(), blob(b"imposter"))
        .await
        .expect_err("stale expectation");
    assert!(matches!(error, LinkedSessionError::VersionConflict { .. }));

    // A re-link bumps the revision: the old handle fails closed as revoked.
    auth.bump_link_revision(&owner, account.id)
        .await
        .expect("re-link bump");
    assert_eq!(
        handle.load().await.expect_err("stale revision"),
        LinkedSessionError::Revoked
    );

    // A handle for an account the auth domain never minted is revoked too.
    let ghost = CredentialAccountId::new();
    store.register_account(ext.clone(), account_ref_for(&ghost), owner, ghost);
    let ghost_handle = store
        .custody_for(ext)
        .open(&LinkedAccountGrant::new(account_ref_for(&ghost), 1));
    assert_eq!(
        ghost_handle.load().await.expect_err("no such account"),
        LinkedSessionError::Revoked
    );
}
