//! Host-side custody for linked-account sessions: the record-owning store and
//! the pre-scoped handles an extension package gets to see.
//!
//! **Three layers, deliberately separate.**
//!
//! 1. [`LinkedSessionMaterialStore`] — the encrypted-material seam. Opaque
//!    bytes in, opaque bytes out, compare-and-swap on the way in. The one
//!    production implementation, [`CredentialServiceLinkedSessionMaterial`],
//!    forwards to the auth domain's `CredentialAccountService` — never a
//!    second custody path.
//! 2. [`LinkedSessionStore`] — the **record owner**. It owns the key grammar,
//!    the provisional (pre-mint) space, and the directory that maps a
//!    host-issued [`LinkedAccountRef`] back to the credential-account
//!    coordinates the material seam needs. The contracts family bans a
//!    record-owning store trait from the vocabulary tier, and this is the
//!    record it was banning on behalf of: it lives here, beside
//!    `InstallationRecordStore`.
//! 3. [`ExtensionLinkedSessionCustody`] — the factory an extension receives at
//!    bind time, and the pre-scoped [`LinkedSessionPort`] handles it mints.
//!
//! **Scoping is the security boundary.** A handle is opened against one
//! [`LinkedAccountGrant`] and one [`ExtensionId`], and both are captured by the
//! host at mint time. Nothing an adapter can say afterwards re-addresses the
//! handle: the port's methods take no account, no user, and no key. An adapter
//! holding a handle for user A cannot express a read of user B's credential.
//!
//! Be exact about how much that proves. The containment is a *wiring* property
//! — the host is the only producer of grants in production — not a sealed
//! witness. `LinkedAccountGrant::new` is callable by any crate that can name
//! it. The contracts module says the same thing about itself, and the ADR
//! (`ADR-device-link-auth-hook.md`, in this feature's design record under
//! `docs/internal/design/`) records that the tool-side factory's containment
//! depends on the host-issued grant, not on adapter discipline.
//!
//! **Two custody spaces, split by revision.**
//!
//! - Revision `0` is the **provisional** space: a link mid-handshake has no
//!   credential account yet (store precedes mint — PROPOSAL §4.3), so the
//!   blob the adapter writes during login parks in bounded process memory. A
//!   process restart legitimately loses it: the parked vendor connection died
//!   with the process, and the flow re-mints as `Failed { restartable }`.
//! - Revision `>= 1` is **durable custody**: the directory resolves the grant
//!   to its credential-account coordinates and the material seam carries the
//!   bytes to the auth domain, where the revision gate
//!   (`LinkRevisionStale`) makes a stale handle fail closed as
//!   [`LinkedSessionError::Revoked`].

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use ironclaw_auth::{
    AuthProductError, AuthProductScope, CredentialAccountId, CredentialAccountService,
    OpaqueMaterialRequest, OpaqueMaterialWrite, OpaqueMaterialWriteOutcome,
};
use ironclaw_extension_contracts::linked_session::{
    LinkedAccountGrant, LinkedAccountRef, LinkedAccountResolutionError, LinkedAccountResolver,
    LinkedSessionError, LinkedSessionPort, LinkedSessionPortFactory, LinkedSessionSnapshot,
    LinkedSessionVersion, SessionBytes,
};
use ironclaw_host_api::ids::ExtensionId;
use ironclaw_host_api::resource::ResourceScope;

/// The link revision a provisional (pre-mint) grant carries. A real account's
/// first link is revision 1, so 0 can never collide with one.
pub const PENDING_LINK_REVISION: u64 = 0;

/// Provisional blobs one process will park at once. Mirrors the device-link
/// driver's active-flow bound: a provisional blob exists only while a flow is
/// mid-handshake.
pub const MAX_PROVISIONAL_SESSIONS: usize =
    crate::device_link_driver::DeviceLinkLimits::DEFAULT_MAX_ACTIVE_FLOWS;

/// The durable identity of one linked-account session record.
///
/// Keyed by `(extension, account)` and nothing else. The extension is part of
/// the key because a credential account is owned by the extension that linked
/// it (`ownership = ExtensionOwned`), so two extensions can never observe one
/// another's custody even if they somehow named the same account ref.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct LinkedSessionKey {
    extension: ExtensionId,
    account: LinkedAccountRef,
}

impl LinkedSessionKey {
    pub fn new(extension: ExtensionId, account: LinkedAccountRef) -> Self {
        Self { extension, account }
    }

    pub fn extension(&self) -> &ExtensionId {
        &self.extension
    }

    pub fn account(&self) -> &LinkedAccountRef {
        &self.account
    }
}

/// What the material seam needs to address one account's encrypted blob:
/// the credential-account coordinates, plus the requesting extension and the
/// revision the handle was scoped to. The auth domain re-checks all four.
#[derive(Debug, Clone)]
pub struct LinkedSessionMaterialKey {
    pub scope: AuthProductScope,
    pub account_id: CredentialAccountId,
    pub requester_extension: ExtensionId,
    pub link_revision: u64,
}

/// The encrypted-material seam: opaque bytes under compare-and-swap.
///
/// Nothing here parses the blob: the semantic merge on a CAS conflict is
/// package-side by design, because only the package can read the vendor
/// format.
#[async_trait]
pub trait LinkedSessionMaterialStore: Send + Sync {
    /// The stored blob, or `None` when the account holds none yet.
    async fn load(
        &self,
        key: &LinkedSessionMaterialKey,
    ) -> Result<Option<LinkedSessionSnapshot>, LinkedSessionError>;

    /// Replace the stored blob if the stored version still matches `expected`.
    ///
    /// Implementations must reject a mismatch with
    /// [`LinkedSessionError::VersionConflict`] carrying the current version.
    /// Last-writer-wins is not an acceptable implementation: a clobbered auth
    /// key is a silently dead link.
    async fn replace(
        &self,
        key: &LinkedSessionMaterialKey,
        expected: LinkedSessionVersion,
        blob: SessionBytes,
    ) -> Result<LinkedSessionVersion, LinkedSessionError>;
}

/// A material seam that stores nothing and says so.
///
/// Wired when a deployment has no linked-account custody. Fail-closed by
/// construction and greppable in a log, which is the alternative to an
/// `Option<Arc<…>>` every caller would have to branch on. Mirrors
/// [`crate::UnavailableExtensionActivationCredentialGate`].
pub struct UnavailableLinkedSessionMaterial;

#[async_trait]
impl LinkedSessionMaterialStore for UnavailableLinkedSessionMaterial {
    async fn load(
        &self,
        _key: &LinkedSessionMaterialKey,
    ) -> Result<Option<LinkedSessionSnapshot>, LinkedSessionError> {
        Err(LinkedSessionError::Unavailable {
            reason: "linked-account custody is not wired in this deployment",
        })
    }

    async fn replace(
        &self,
        _key: &LinkedSessionMaterialKey,
        _expected: LinkedSessionVersion,
        _blob: SessionBytes,
    ) -> Result<LinkedSessionVersion, LinkedSessionError> {
        Err(LinkedSessionError::Unavailable {
            reason: "linked-account custody is not wired in this deployment",
        })
    }
}

/// The production material seam: forwards to the auth domain's credential
/// service, which owns the encrypted bytes, the compare-and-swap, and the
/// revision gate. This is a projection between two vocabularies and nothing
/// else — no custody logic may accrete here.
pub struct CredentialServiceLinkedSessionMaterial {
    accounts: Arc<dyn CredentialAccountService>,
}

impl CredentialServiceLinkedSessionMaterial {
    pub fn new(accounts: Arc<dyn CredentialAccountService>) -> Self {
        Self { accounts }
    }
}

/// Map an auth-domain failure onto the contracts custody vocabulary.
///
/// A stale revision, a missing account, and a scope the requester may not see
/// all collapse to [`LinkedSessionError::Revoked`]: the handle is dead and a
/// new grant is required. Backend detail never crosses — the reason strings
/// are fixed host text.
fn custody_error(error: AuthProductError) -> LinkedSessionError {
    match error {
        AuthProductError::LinkRevisionStale { .. }
        | AuthProductError::CredentialMissing
        | AuthProductError::CrossScopeDenied => LinkedSessionError::Revoked,
        _ => LinkedSessionError::Unavailable {
            reason: "linked-session custody backend failed",
        },
    }
}

#[async_trait]
impl LinkedSessionMaterialStore for CredentialServiceLinkedSessionMaterial {
    async fn load(
        &self,
        key: &LinkedSessionMaterialKey,
    ) -> Result<Option<LinkedSessionSnapshot>, LinkedSessionError> {
        let snapshot = self
            .accounts
            .load_opaque_material(OpaqueMaterialRequest {
                scope: key.scope.clone(),
                account_id: key.account_id,
                requester_extension: Some(key.requester_extension.clone()),
                link_revision: key.link_revision,
            })
            .await
            .map_err(custody_error)?;
        Ok(snapshot.map(|snapshot| LinkedSessionSnapshot {
            blob: snapshot.material,
            version: snapshot.version,
        }))
    }

    async fn replace(
        &self,
        key: &LinkedSessionMaterialKey,
        expected: LinkedSessionVersion,
        blob: SessionBytes,
    ) -> Result<LinkedSessionVersion, LinkedSessionError> {
        let outcome = self
            .accounts
            .store_opaque_material(OpaqueMaterialWrite {
                target: OpaqueMaterialRequest {
                    scope: key.scope.clone(),
                    account_id: key.account_id,
                    requester_extension: Some(key.requester_extension.clone()),
                    link_revision: key.link_revision,
                },
                expected,
                material: blob,
            })
            .await
            .map_err(custody_error)?;
        match outcome {
            OpaqueMaterialWriteOutcome::Stored { version } => Ok(version),
            OpaqueMaterialWriteOutcome::Conflict { current } => {
                Err(LinkedSessionError::VersionConflict { current })
            }
        }
    }
}

/// The credential-account coordinates behind one directory entry.
#[derive(Debug, Clone)]
struct MaterialCoordinates {
    scope: AuthProductScope,
    account_id: CredentialAccountId,
}

/// One parked provisional blob, versioned by a local counter so the adapter's
/// own compare-and-swap discipline holds before custody is durable.
struct ProvisionalBlob {
    blob: SessionBytes,
    version: u64,
}

/// The record-owning linked-session store.
///
/// Owns the key grammar, the provisional space, and the ref → account
/// directory; delegates every durable encrypted byte to a
/// [`LinkedSessionMaterialStore`].
pub struct LinkedSessionStore {
    material: Arc<dyn LinkedSessionMaterialStore>,
    directory: Mutex<HashMap<LinkedSessionKey, MaterialCoordinates>>,
    provisional: Mutex<HashMap<LinkedSessionKey, ProvisionalBlob>>,
}

impl LinkedSessionStore {
    pub fn new(material: Arc<dyn LinkedSessionMaterialStore>) -> Arc<Self> {
        Arc::new(Self {
            material,
            directory: Mutex::new(HashMap::new()),
            provisional: Mutex::new(HashMap::new()),
        })
    }

    /// A store over the fail-closed material seam, for a deployment that wires
    /// no custody.
    pub fn unavailable() -> Arc<Self> {
        Self::new(Arc::new(UnavailableLinkedSessionMaterial))
    }

    /// The bind-time factory for one extension.
    ///
    /// Scoping happens twice on purpose: the extension is fixed here, at bind,
    /// where the host knows which package it is loading; the account is fixed
    /// in [`LinkedSessionPortFactory::open`], from a host-issued grant. An
    /// adapter supplies neither.
    pub fn custody_for(
        self: &Arc<Self>,
        extension: ExtensionId,
    ) -> Arc<dyn LinkedSessionPortFactory> {
        Arc::new(ExtensionLinkedSessionCustody {
            store: Arc::clone(self),
            extension,
        })
    }

    /// Mint one pre-scoped handle directly. Allocation only — no I/O, so a
    /// caller on the bind path stays side-effect-free.
    pub fn open(
        self: &Arc<Self>,
        extension: &ExtensionId,
        grant: &LinkedAccountGrant,
    ) -> Arc<dyn LinkedSessionPort> {
        Arc::new(ScopedLinkedSession {
            store: Arc::clone(self),
            key: LinkedSessionKey::new(extension.clone(), grant.account().clone()),
            link_revision: grant.link_revision(),
        })
    }

    /// Teach the directory which credential account a host-issued ref names.
    ///
    /// Called at every host mint site — completion, tool-path resolution, and
    /// revoke — before any handle under the ref is used. The directory is
    /// wiring state, not authority: the auth domain re-checks scope,
    /// requester, and revision on every material operation.
    pub fn register_account(
        &self,
        extension: ExtensionId,
        account: LinkedAccountRef,
        scope: AuthProductScope,
        account_id: CredentialAccountId,
    ) {
        self.lock_directory().insert(
            LinkedSessionKey::new(extension, account),
            MaterialCoordinates { scope, account_id },
        );
    }

    /// Forget one ref's coordinates (the account was revoked or removed).
    pub fn unregister_account(&self, extension: &ExtensionId, account: &LinkedAccountRef) {
        self.lock_directory()
            .remove(&LinkedSessionKey::new(extension.clone(), account.clone()));
    }

    /// Load a parked provisional blob without consuming it.
    pub fn provisional_blob(
        &self,
        extension: &ExtensionId,
        account: &LinkedAccountRef,
    ) -> Option<SessionBytes> {
        self.lock_provisional()
            .get(&LinkedSessionKey::new(extension.clone(), account.clone()))
            .map(|entry| entry.blob.clone())
    }

    /// Drop a parked provisional blob: the link completed (its material is
    /// durable now), was cancelled, or was reaped.
    pub fn discard_provisional(&self, extension: &ExtensionId, account: &LinkedAccountRef) {
        self.lock_provisional()
            .remove(&LinkedSessionKey::new(extension.clone(), account.clone()));
    }

    async fn load_scoped(
        &self,
        key: &LinkedSessionKey,
        link_revision: u64,
    ) -> Result<Option<LinkedSessionSnapshot>, LinkedSessionError> {
        if link_revision == PENDING_LINK_REVISION {
            let provisional = self.lock_provisional();
            return Ok(provisional.get(key).map(|entry| LinkedSessionSnapshot {
                blob: entry.blob.clone(),
                version: provisional_version(entry.version),
            }));
        }
        let material_key = self.material_key(key, link_revision)?;
        self.material.load(&material_key).await
    }

    async fn save_scoped(
        &self,
        key: &LinkedSessionKey,
        link_revision: u64,
        expected: LinkedSessionVersion,
        blob: SessionBytes,
    ) -> Result<LinkedSessionVersion, LinkedSessionError> {
        // No size check here on purpose: `MAX_LINKED_SESSION_BYTES` is checked
        // by `SessionBytes::new`, the type's only constructor, so a blob that
        // reached this signature is already within the ceiling.
        if link_revision == PENDING_LINK_REVISION {
            return self.save_provisional(key, expected, blob);
        }
        let material_key = self.material_key(key, link_revision)?;
        self.material.replace(&material_key, expected, blob).await
    }

    fn save_provisional(
        &self,
        key: &LinkedSessionKey,
        expected: LinkedSessionVersion,
        blob: SessionBytes,
    ) -> Result<LinkedSessionVersion, LinkedSessionError> {
        let mut provisional = self.lock_provisional();
        match provisional.get_mut(key) {
            None => {
                if !expected.is_absent() {
                    return Err(LinkedSessionError::VersionConflict {
                        current: LinkedSessionVersion::absent(),
                    });
                }
                if provisional.len() >= MAX_PROVISIONAL_SESSIONS {
                    return Err(LinkedSessionError::Unavailable {
                        reason: "too many in-progress links hold provisional session blobs",
                    });
                }
                provisional.insert(key.clone(), ProvisionalBlob { blob, version: 1 });
                Ok(provisional_version(1))
            }
            Some(entry) => {
                let current = provisional_version(entry.version);
                if expected != current {
                    return Err(LinkedSessionError::VersionConflict { current });
                }
                entry.version = entry.version.saturating_add(1);
                entry.blob = blob;
                Ok(provisional_version(entry.version))
            }
        }
    }

    fn material_key(
        &self,
        key: &LinkedSessionKey,
        link_revision: u64,
    ) -> Result<LinkedSessionMaterialKey, LinkedSessionError> {
        let directory = self.lock_directory();
        let Some(coordinates) = directory.get(key) else {
            // An unregistered ref means the handle predates this process (or
            // its account was revoked). The resolver re-registers on every
            // successful resolution, so the honest answer is "resolve again",
            // not a guess at coordinates.
            return Err(LinkedSessionError::Unavailable {
                reason: "linked-account handle is not registered in this process; re-resolve",
            });
        };
        Ok(LinkedSessionMaterialKey {
            scope: coordinates.scope.clone(),
            account_id: coordinates.account_id,
            requester_extension: key.extension().clone(),
            link_revision,
        })
    }

    /// A poisoned lock still guards plain data with no invariant to restore;
    /// keeping custody serving beats propagating the panic.
    fn lock_directory(
        &self,
    ) -> std::sync::MutexGuard<'_, HashMap<LinkedSessionKey, MaterialCoordinates>> {
        match self.directory.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
    }

    fn lock_provisional(
        &self,
    ) -> std::sync::MutexGuard<'_, HashMap<LinkedSessionKey, ProvisionalBlob>> {
        match self.provisional.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
    }
}

fn provisional_version(counter: u64) -> LinkedSessionVersion {
    LinkedSessionVersion::new(format!("provisional-{counter}"))
        .unwrap_or_else(|_| LinkedSessionVersion::absent())
}

/// One extension's bind-time custody factory.
struct ExtensionLinkedSessionCustody {
    store: Arc<LinkedSessionStore>,
    extension: ExtensionId,
}

impl LinkedSessionPortFactory for ExtensionLinkedSessionCustody {
    fn open(&self, grant: &LinkedAccountGrant) -> Arc<dyn LinkedSessionPort> {
        self.store.open(&self.extension, grant)
    }
}

/// A handle bound to exactly one `(extension, account, link_revision)`.
struct ScopedLinkedSession {
    store: Arc<LinkedSessionStore>,
    key: LinkedSessionKey,
    link_revision: u64,
}

#[async_trait]
impl LinkedSessionPort for ScopedLinkedSession {
    async fn load(&self) -> Result<Option<LinkedSessionSnapshot>, LinkedSessionError> {
        self.store.load_scoped(&self.key, self.link_revision).await
    }

    async fn save(
        &self,
        expected: LinkedSessionVersion,
        blob: SessionBytes,
    ) -> Result<LinkedSessionVersion, LinkedSessionError> {
        self.store
            .save_scoped(&self.key, self.link_revision, expected, blob)
            .await
    }
}

/// A resolver for deployments that wire no linked-account custody: every
/// resolution answers `Unavailable`, never `NotLinked` — "go link your
/// account" would be a false instruction when no custody exists to link into.
pub struct UnavailableLinkedAccountResolver;

#[async_trait]
impl LinkedAccountResolver for UnavailableLinkedAccountResolver {
    async fn resolve(
        &self,
        _scope: &ResourceScope,
    ) -> Result<LinkedAccountGrant, LinkedAccountResolutionError> {
        Err(LinkedAccountResolutionError::Unavailable)
    }
}

#[cfg(test)]
mod tests;
