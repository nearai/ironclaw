//! Host-side custody for linked-account sessions: the record-owning store and
//! the pre-scoped handles an extension package gets to see.
//!
//! **Three layers, deliberately separate.**
//!
//! 1. [`LinkedSessionMaterialStore`] — the encrypted-material seam. Opaque
//!    bytes in, opaque bytes out, compare-and-swap on the way in. This is the
//!    only layer that touches the credential domain.
//! 2. [`LinkedSessionStore`] — the **record owner**. It owns the key grammar
//!    (`(extension, account)`), the revision gate, and the size ceiling. The
//!    contracts family bans a record-owning store trait from the vocabulary
//!    tier, and this is the record it was banning on behalf of: it lives here,
//!    beside `InstallationRecordStore`.
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
//! **The revision gate is not decoration.** A user id never changes across
//! unlink/relink, so a handle that ignored `link_revision` would keep serving a
//! session whose credential had already been replaced. Every load and every
//! save re-checks the stored revision against the grant's and answers
//! [`LinkedSessionError::Revoked`] on a mismatch, so a stale handle fails
//! closed instead of silently reading a superseded account.

use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_extension_contracts::linked_session::{
    LinkedAccountGrant, LinkedAccountRef, LinkedSessionError, LinkedSessionPort,
    LinkedSessionPortFactory, LinkedSessionSnapshot, LinkedSessionVersion, SessionBytes,
};
use ironclaw_host_api::ids::ExtensionId;

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

/// What the material seam knows about one stored session.
///
/// `snapshot` is `None` for an account that exists but has stored no blob yet —
/// the state a link is in between "the account was minted" and "the vendor
/// handshake produced a credential". A record that does not exist at all is the
/// outer `None` from [`LinkedSessionMaterialStore::load`].
#[derive(Debug, Clone)]
pub struct StoredLinkedSession {
    pub link_revision: u64,
    pub snapshot: Option<LinkedSessionSnapshot>,
}

/// The encrypted-material seam: opaque bytes under compare-and-swap.
///
/// # TODO(design): one implementation is still missing, and why
///
/// PROPOSAL §3.3/§8.2 puts the encrypted material on the auth domain, and that
/// API has landed: `CredentialAccountService::{load_opaque_material,
/// store_opaque_material, bump_link_revision}` over `OpaqueMaterialRequest`,
/// with `link_revision` on the account record. This trait is the host-side
/// projection of exactly those two calls, and the intended completion is **one**
/// implementation of it in this crate that forwards to that service — never a
/// second custody path.
///
/// What blocks writing it here: `OpaqueMaterialRequest` needs an
/// `AuthProductScope` and a `CredentialAccountId`, and a
/// [`LinkedAccountGrant`] carries neither — it cannot, since a package must be
/// able to name every type in the port's signature and cannot name the auth
/// domain at all. Synthesizing a scope from a bare user id would be
/// re-deriving security-relevant scope, which this repo bans. So the adapter
/// lands with whichever seam supplies the flow's scope (the device-link driver
/// or composition), and the shape of *this* trait is what that adapter fills.
///
/// Nothing here parses the blob: the semantic merge on a CAS conflict is
/// package-side by design, because only the package can read the vendor format.
#[async_trait]
pub trait LinkedSessionMaterialStore: Send + Sync {
    /// The stored record, or `None` when no account is bound to this key.
    async fn load(
        &self,
        key: &LinkedSessionKey,
    ) -> Result<Option<StoredLinkedSession>, LinkedSessionError>;

    /// Replace the stored blob if the stored version still matches `expected`.
    ///
    /// Implementations must reject a mismatch with
    /// [`LinkedSessionError::VersionConflict`] carrying the current version.
    /// Last-writer-wins is not an acceptable implementation: a clobbered auth
    /// key is a silently dead link.
    async fn replace(
        &self,
        key: &LinkedSessionKey,
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
        _key: &LinkedSessionKey,
    ) -> Result<Option<StoredLinkedSession>, LinkedSessionError> {
        Err(LinkedSessionError::Unavailable {
            reason: "linked-account custody is not wired in this deployment",
        })
    }

    async fn replace(
        &self,
        _key: &LinkedSessionKey,
        _expected: LinkedSessionVersion,
        _blob: SessionBytes,
    ) -> Result<LinkedSessionVersion, LinkedSessionError> {
        Err(LinkedSessionError::Unavailable {
            reason: "linked-account custody is not wired in this deployment",
        })
    }
}

/// The record-owning linked-session store.
///
/// Owns the key grammar, the revision gate, and the size ceiling; delegates
/// every encrypted byte to a [`LinkedSessionMaterialStore`].
pub struct LinkedSessionStore {
    material: Arc<dyn LinkedSessionMaterialStore>,
}

impl LinkedSessionStore {
    pub fn new(material: Arc<dyn LinkedSessionMaterialStore>) -> Arc<Self> {
        Arc::new(Self { material })
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

    /// Load the record behind one scoped handle, enforcing the revision gate.
    async fn load_scoped(
        &self,
        key: &LinkedSessionKey,
        link_revision: u64,
    ) -> Result<Option<LinkedSessionSnapshot>, LinkedSessionError> {
        let Some(record) = self.material.load(key).await? else {
            // No account behind this grant any more: the handle is dead, and
            // saying "nothing stored" would invite the caller to write a fresh
            // blob into a revoked account.
            return Err(LinkedSessionError::Revoked);
        };
        if record.link_revision != link_revision {
            return Err(LinkedSessionError::Revoked);
        }
        Ok(record.snapshot)
    }

    /// Compare-and-swap the blob behind one scoped handle.
    async fn save_scoped(
        &self,
        key: &LinkedSessionKey,
        link_revision: u64,
        expected: LinkedSessionVersion,
        blob: SessionBytes,
    ) -> Result<LinkedSessionVersion, LinkedSessionError> {
        // No size check here on purpose: `MAX_LINKED_SESSION_BYTES` is checked
        // by `SessionBytes::new`, the type's only constructor, so a blob that
        // reached this signature is already within the ceiling. A second check
        // would be an unreachable branch nothing could test.
        let Some(record) = self.material.load(key).await? else {
            return Err(LinkedSessionError::Revoked);
        };
        if record.link_revision != link_revision {
            return Err(LinkedSessionError::Revoked);
        }
        self.material.replace(key, expected, blob).await
    }
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

#[cfg(test)]
mod tests;
