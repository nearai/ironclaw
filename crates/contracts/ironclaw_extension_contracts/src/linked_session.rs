//! **Linked-account session custody**: the port an extension package uses to
//! load and compare-and-swap the one opaque credential blob a linked device
//! session persists.
//!
//! Declared here, implemented **only** by the extension host, which resolves
//! the credential account and delegates the encrypted material to the auth
//! domain. The package never names the auth domain — its boundary rule forbids
//! that edge — so every type the port's signature mentions has to live on this
//! side of the membrane, trait *and* value types alike.
//!
//! **How much precedent this has, stated precisely.** This is the same
//! *category* as [`crate::tool_adapter::RestrictedEgress`]: a pre-scoped,
//! host-implemented capability handed to an adapter. It is not the same
//! *shape*, and two differences matter:
//!
//! 1. `RestrictedEgress` exists so an adapter never sees secret bytes. This
//!    port does the opposite — it hands the adapter the raw credential,
//!    because a vendor protocol library must have the key to speak the
//!    protocol. That concession is the design's, not this module's, and it is
//!    the reason everything here zeroizes and redacts.
//! 2. `RestrictedEgress` carries no identity scoping. This port does: a handle
//!    is opened against one [`LinkedAccountGrant`] and can address nothing
//!    else, so cross-account access is not expressible through it.
//!
//! **Not a store trait.** The contracts family bans record-owning store traits
//! — "a bare store trait belongs in the domain that owns the records". This
//! carries no record grammar (the blob is opaque bytes), no keys, scopes or
//! queries (the handle is pre-scoped before an adapter ever sees it), and no
//! mechanism. The record-owning store stays host-side.

use std::fmt;
use std::sync::Arc;

use async_trait::async_trait;
use zeroize::{Zeroize, ZeroizeOnDrop};

use ironclaw_host_api::error::HostApiError;

/// Hard ceiling on a persisted linked-session blob (256 KiB).
///
/// Checked at every construction of [`SessionBytes`], so an implementation
/// cannot forget it and a vendor session that grows without bound (an
/// unbounded peer cache is the motivating case) fails loudly instead of
/// silently bloating a secret record.
pub const MAX_LINKED_SESSION_BYTES: usize = 256 * 1024;

/// Maximum length of a [`LinkedAccountRef`], in bytes.
const MAX_LINKED_ACCOUNT_REF_BYTES: usize = 256;

/// Maximum length of a [`LinkedSessionVersion`] token, in bytes.
const MAX_LINKED_SESSION_VERSION_BYTES: usize = 256;

/// The opaque bytes of one linked-account session credential.
///
/// The bytes are a full account credential in vendor-private format. They are
/// zeroized on drop, never rendered by [`fmt::Debug`], and never interpolated
/// into an error. Reading them is a single, greppable call: [`SessionBytes::expose`].
#[derive(Clone)]
pub struct SessionBytes(Vec<u8>);

impl SessionBytes {
    /// Wrap blob bytes, rejecting an empty or oversized payload.
    pub fn new(bytes: Vec<u8>) -> Result<Self, LinkedSessionError> {
        if bytes.is_empty() {
            return Err(LinkedSessionError::EmptyBlob);
        }
        if bytes.len() > MAX_LINKED_SESSION_BYTES {
            return Err(LinkedSessionError::BlobTooLarge {
                bytes: bytes.len(),
                max: MAX_LINKED_SESSION_BYTES,
            });
        }
        Ok(Self(bytes))
    }

    /// The credential bytes. Named `expose` rather than `as_bytes` so every
    /// site that reaches secret material is visible to a reviewer's grep.
    pub fn expose(&self) -> &[u8] {
        &self.0
    }

    /// Blob length in bytes. Safe to log; the bytes themselves are not.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Always `false` — [`SessionBytes::new`] rejects an empty blob. Present
    /// because a bare `len` without it is a lint, and because a caller
    /// asserting the invariant should not have to know it holds.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl fmt::Debug for SessionBytes {
    /// Length only. A blob byte must never reach a log line, an event, or a
    /// panic message.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "SessionBytes({} bytes, redacted)", self.0.len())
    }
}

impl Drop for SessionBytes {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

impl ZeroizeOnDrop for SessionBytes {}

/// Opaque compare-and-swap token for a stored session blob.
///
/// Minted by the host store; an adapter only round-trips the value it was
/// handed. [`LinkedSessionVersion::absent`] is the token meaning "no blob is
/// stored yet" — the only value a first write may expect.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct LinkedSessionVersion(Option<String>);

impl LinkedSessionVersion {
    /// The token meaning "nothing is stored under this account".
    pub fn absent() -> Self {
        Self(None)
    }

    /// Wrap a host-issued version token.
    pub fn new(token: impl Into<String>) -> Result<Self, HostApiError> {
        let token = token.into();
        let invalid = |reason: &str| HostApiError::InvalidId {
            kind: "linked_session_version",
            value: token.clone(),
            reason: reason.to_string(),
        };
        if token.is_empty() {
            return Err(invalid("must not be empty"));
        }
        if token.len() > MAX_LINKED_SESSION_VERSION_BYTES {
            return Err(invalid("exceeds the maximum version-token length"));
        }
        if token.chars().any(char::is_control) {
            return Err(invalid("must not contain control characters"));
        }
        Ok(Self(Some(token)))
    }

    /// The token, or `None` when this version means "absent".
    pub fn as_str(&self) -> Option<&str> {
        self.0.as_deref()
    }

    /// Whether this version expects no stored blob.
    pub fn is_absent(&self) -> bool {
        self.0.is_none()
    }
}

/// One loaded session blob and the version to present on the next write.
#[derive(Debug, Clone)]
pub struct LinkedSessionSnapshot {
    pub blob: SessionBytes,
    pub version: LinkedSessionVersion,
}

/// Host-issued reference to one linked credential account.
///
/// Deliberately opaque: the host mints it from the resolved credential-account
/// identity, and a package cannot name that identity's type (the auth domain
/// sits above the contracts floor, and the package's boundary rule forbids the
/// edge outright). Treat the inner value as a routing key with no parseable
/// structure.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct LinkedAccountRef(String);

impl LinkedAccountRef {
    pub fn new(value: impl Into<String>) -> Result<Self, HostApiError> {
        let value = value.into();
        let invalid = |reason: &str| HostApiError::InvalidId {
            kind: "linked_account_ref",
            value: value.clone(),
            reason: reason.to_string(),
        };
        if value.is_empty() {
            return Err(invalid("must not be empty"));
        }
        if value.len() > MAX_LINKED_ACCOUNT_REF_BYTES {
            return Err(invalid("exceeds the maximum linked-account-ref length"));
        }
        if value.chars().any(char::is_control) {
            return Err(invalid("must not contain control characters"));
        }
        if value.chars().any(char::is_whitespace) {
            return Err(invalid("must not contain whitespace"));
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for LinkedAccountRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// What a session handle is opened against: one account, at one link
/// revision.
///
/// **The revision is part of the identity, not decoration.** A user id never
/// changes across unlink/relink, so a handle (or a cache keyed on one) that
/// ignored the revision would keep serving a session whose credential has been
/// replaced. Bumping the revision is what invalidates live state.
///
/// **On "unforgeable".** The host resolves the account and is the only
/// producer in production wiring — that is what binds a handle to an account
/// rather than trusting an adapter-supplied user id. This type is *not* a
/// sealed witness in the sense [`crate::verified_inbound`] uses the word:
/// nothing in the type system stops another crate calling
/// [`LinkedAccountGrant::new`]. The containment is a wiring property, and
/// stating it as anything stronger would be a claim the API does not back.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct LinkedAccountGrant {
    account: LinkedAccountRef,
    link_revision: u64,
}

impl LinkedAccountGrant {
    pub fn new(account: LinkedAccountRef, link_revision: u64) -> Self {
        Self {
            account,
            link_revision,
        }
    }

    pub fn account(&self) -> &LinkedAccountRef {
        &self.account
    }

    pub fn link_revision(&self) -> u64 {
        self.link_revision
    }
}

/// Typed custody failures. No variant carries blob bytes, and no message
/// interpolates them.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LinkedSessionError {
    /// A blob must carry at least one byte.
    #[error("linked-session blob must not be empty")]
    EmptyBlob,
    /// The blob exceeds [`MAX_LINKED_SESSION_BYTES`].
    #[error("linked-session blob is {bytes} bytes, over the {max}-byte ceiling")]
    BlobTooLarge { bytes: usize, max: usize },
    /// Compare-and-swap rejected the write: someone else stored a newer blob.
    /// The current version is returned so the caller can reload and merge —
    /// a clobbered credential is a silently dead link, so this must never be
    /// resolved by retrying the write unconditionally.
    #[error("linked-session write lost the compare-and-swap")]
    VersionConflict { current: LinkedSessionVersion },
    /// The account no longer exists, or its link revision has moved on. The
    /// handle is dead; a new grant is required.
    #[error("linked-session account is no longer bound to this handle")]
    Revoked,
    /// The custody backend failed. The reason is fixed, host-authored text —
    /// never a backend message, which could carry credential material.
    #[error("linked-session custody is unavailable: {reason}")]
    Unavailable { reason: &'static str },
}

/// Load and compare-and-swap the session blob for **one** account.
///
/// A handle is pre-scoped: it addresses the account its grant named and no
/// other, so an adapter holding it cannot reach a second user's credential.
/// Both methods touch storage — a handle is cheap to open and expensive to
/// use.
#[async_trait]
pub trait LinkedSessionPort: Send + Sync {
    /// The stored blob, or `None` when nothing is stored yet.
    async fn load(&self) -> Result<Option<LinkedSessionSnapshot>, LinkedSessionError>;

    /// Store `blob`, but only if the stored version still matches `expected`.
    ///
    /// Returns the new version on success and
    /// [`LinkedSessionError::VersionConflict`] — carrying the current version
    /// — when it does not. Last-writer-wins is not an acceptable
    /// implementation: a clobbered credential kills the link.
    async fn save(
        &self,
        expected: LinkedSessionVersion,
        blob: SessionBytes,
    ) -> Result<LinkedSessionVersion, LinkedSessionError>;
}

/// Exchange a host-issued grant for a session handle.
///
/// **A factory, not a per-call borrow.** Custody outlives an invocation:
/// debounced write-through, a background runner, a flush before a pooled
/// client is dropped, and idle eviction all fire outside any `invoke`. A
/// handle borrowed for one call cannot serve them, which is why this is
/// supplied once at bind time and yields an owned handle — and why
/// `ToolPorts` is deliberately unchanged.
///
/// **The grant is the containment.** Taking a bare user id here would root the
/// whole scoping chain in a value an adapter supplies, which is exactly the
/// value that cannot be trusted for isolation.
///
/// `open` performs no I/O: it allocates a handle. Binding stays contractually
/// I/O-free.
pub trait LinkedSessionPortFactory: Send + Sync {
    fn open(&self, grant: &LinkedAccountGrant) -> Arc<dyn LinkedSessionPort>;
}

/// Why a caller's linked account could not be resolved.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum LinkedAccountResolutionError {
    /// This caller has no linked account for the extension, or its credential
    /// was revoked. The run parks on the connect gate.
    #[error("no linked account for this caller")]
    NotLinked,
    /// Resolution itself failed (custody unavailable, backend error). Not the
    /// same as "not linked" — telling a user to re-link because a store was
    /// briefly unavailable is a bad instruction.
    #[error("the linked account could not be resolved")]
    Unavailable,
}

/// Resolve which linked account a tool call acts as.
///
/// Supplied by the host at bind, beside [`LinkedSessionPortFactory`], and for
/// the same reason: the tool ABI (`ToolCall`, `ToolPorts`) is deliberately
/// unchanged, so no per-dispatch carrier for a host-issued grant exists.
/// The host resolves the call's scope to a credential account and mints the
/// grant; **an adapter must never construct a grant from a caller-supplied
/// id** — `scope.user_id` is exactly the axis the design refuses to trust for
/// isolation, and an adapter-side resolver would root containment in it.
#[async_trait]
pub trait LinkedAccountResolver: Send + Sync {
    async fn resolve(
        &self,
        scope: &ironclaw_host_api::resource::ResourceScope,
    ) -> Result<LinkedAccountGrant, LinkedAccountResolutionError>;
}

#[cfg(test)]
mod tests;
