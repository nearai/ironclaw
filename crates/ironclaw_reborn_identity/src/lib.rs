//! Canonical Reborn identity layer.
//!
//! One boundary that maps every external identity — WebUI OAuth logins
//! (`google`, `github`, …) and external channel/product actors
//! (`telegram`, `slack`, triggers, …) — to a stable Reborn [`UserId`]
//! *before* any runtime state (conversation binding, thread ownership) is
//! touched.
//!
//! - Identity provisioning lives HERE, not in WebUI ingress and not in
//!   `ironclaw_conversations` (which stays lookup/binding-oriented and
//!   consumes an already-resolved `UserId`).
//! - WebUI OAuth and product/channel adapters feed normalized
//!   [`ResolveExternalIdentity`] values into [`RebornIdentityResolver`].
//!
//! The external identity is keyed by `(tenant_id, surface_kind,
//! provider_kind, provider_instance_id, external_subject_id)` so two
//! tenants, two adapter installations, or two surfaces cannot collide on
//! the same subject id. Verified email may link OAuth providers within a
//! tenant; an unverified email never links.
//!
//! Persistence ([`FilesystemRebornIdentityStore`]) goes through the host
//! [`RootFilesystem`](ironclaw_filesystem::RootFilesystem) /
//! `ScopedFilesystem` abstraction — the same substrate boundary every other
//! durable Reborn store sits behind — so substrate choice, tenant scoping,
//! and host ownership stay centralized in the filesystem layer rather than
//! this crate holding a raw database handle.

mod filesystem_store;
mod key;

pub use filesystem_store::FilesystemRebornIdentityStore;
pub use key::{ExternalSubjectId, IdentityKeyError, ProviderInstanceId, ProviderKind};

use async_trait::async_trait;
use ironclaw_host_api::{TenantId, UserId};
use serde::{Deserialize, Serialize};

/// Which surface an external identity arrived through. The typed axis that
/// keeps a browser OAuth `google` identity distinct from a (hypothetical)
/// channel-actor `google` identity even when every other key part matches.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SurfaceKind {
    /// Browser SSO login (Google / GitHub / …).
    Oauth,
    /// External channel / product actor (Telegram / Slack / trigger / …).
    ChannelActor,
}

impl SurfaceKind {
    /// Stable wire/DB string. Matches the `#[serde(rename_all =
    /// "snake_case")]` representation; persisted in the identity row, so it
    /// must not drift.
    pub fn as_str(self) -> &'static str {
        match self {
            SurfaceKind::Oauth => "oauth",
            SurfaceKind::ChannelActor => "channel_actor",
        }
    }
}

/// A normalized external identity to resolve. Callers (WebUI ingress,
/// product adapters) construct the typed key parts up front, so this layer
/// never depends on their profile types and the provider/instance/subject
/// ids cross the boundary as validated newtypes rather than raw strings
/// (`.claude/rules/types.md`). The key parts are stored as opaque,
/// separately-columned values (never flattened, so delimiter-like ids
/// cannot collide).
pub struct ResolveExternalIdentity {
    /// Trusted host tenant. Identity resolution and email linking are
    /// scoped to it, so tenants never share users.
    pub tenant_id: TenantId,
    /// Which surface this identity arrived through.
    pub surface_kind: SurfaceKind,
    /// Provider name (`google`, `github`, `telegram`, `slack`, …).
    pub provider_kind: ProviderKind,
    /// Adapter installation id where relevant (channel actors); `None` for
    /// surfaces without an installation (browser OAuth login).
    pub provider_instance_id: Option<ProviderInstanceId>,
    /// Stable per-provider subject id (OAuth `sub`, channel actor id).
    pub external_subject_id: ExternalSubjectId,
    /// Email claimed by the provider, if any.
    pub email: Option<String>,
    /// Whether the provider asserts the email is verified. Only a verified
    /// email may link to an existing account.
    pub email_verified: bool,
    /// Optional display name.
    pub display_name: Option<String>,
}

/// The identity-only key part of an external identity (no email /
/// profile). Used by the link-only [`lookup`](RebornIdentityResolver::lookup)
/// and [`bind`](RebornIdentityResolver::bind) paths that channel actors
/// (e.g. Slack) use, where there is no email and no minting.
pub struct ExternalIdentityKey {
    pub tenant_id: TenantId,
    pub surface_kind: SurfaceKind,
    pub provider_kind: ProviderKind,
    pub provider_instance_id: Option<ProviderInstanceId>,
    pub external_subject_id: ExternalSubjectId,
}

/// A persisted canonical user.
///
/// `status` and `role` are intentionally absent: the store has no typed
/// semantics for them yet, so the `users` table carries them as columns
/// with DB-level defaults (`active` / `member`) rather than threading
/// stringly-typed values through this record (see `.claude/rules/types.md`
/// — fixed small sets must be enums, not strings). Reintroduce them as
/// `UserStatus` / `UserRole` enums when a caller actually reads them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserRecord {
    pub id: UserId,
    pub email: Option<String>,
    pub display_name: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

/// Failure modes of the canonical identity layer.
#[derive(Debug, thiserror::Error)]
pub enum RebornIdentityError {
    /// The persistence backend (connect / migrate / query / commit) failed.
    #[error("reborn identity store backend failure: {0}")]
    Backend(String),
    /// A persisted user id failed `UserId` validation on read-back — a
    /// backend inconsistency, surfaced rather than silently dropped.
    #[error("persisted user id is invalid: {0}")]
    InvalidUserId(String),
    /// `resolve_or_create` was called for a `ChannelActor` identity. Channel
    /// actors are never mint-capable — the resolver contract routes them
    /// through [`lookup`](RebornIdentityResolver::lookup) /
    /// [`bind`](RebornIdentityResolver::bind) so an unbound actor fails closed
    /// instead of auto-provisioning a Reborn account.
    #[error("channel-actor identities must resolve through lookup/bind, not resolve_or_create")]
    ChannelActorNotMintable,
}

/// Resolve an external identity to a stable canonical [`UserId`], creating
/// or linking as needed.
///
/// Implementations must be atomic for the lookup → link → create sequence
/// so concurrent first-contacts for the same identity (or the same
/// verified email) converge on one user instead of splitting.
#[async_trait]
pub trait RebornIdentityResolver: Send + Sync {
    /// Mint-capable resolution: resolve the identity to its user, link by
    /// verified email, or create a new user. Used by surfaces whose
    /// admission is established up front (WebUI OAuth, gated by the
    /// email-domain allowlist). A [`ChannelActor`](SurfaceKind::ChannelActor)
    /// identity is rejected with
    /// [`ChannelActorNotMintable`](RebornIdentityError::ChannelActorNotMintable):
    /// channel actors are never mint-capable and must resolve through
    /// [`lookup`](Self::lookup) / [`bind`](Self::bind).
    async fn resolve_or_create(
        &self,
        identity: ResolveExternalIdentity,
    ) -> Result<UserId, RebornIdentityError>;

    /// Link-only lookup: return the user already bound to this external
    /// identity, or `None`. NEVER creates a user. Channel actors (e.g.
    /// Slack) resolve through this so an unbound actor fails closed
    /// instead of auto-provisioning a Reborn account.
    async fn lookup(&self, key: ExternalIdentityKey)
    -> Result<Option<UserId>, RebornIdentityError>;

    /// Link an external identity to an ALREADY-EXISTING user (no user
    /// creation). Re-binding the same key re-points it at `user_id`. The
    /// caller must have authenticated `user_id` first (e.g. Slack personal
    /// binding proves the actor is a known Reborn user before binding).
    async fn bind(
        &self,
        key: ExternalIdentityKey,
        user_id: &UserId,
    ) -> Result<(), RebornIdentityError>;

    /// Adopt a pre-existing external identity carried over from a legacy
    /// store, preserving BOTH its canonical `user_id` and its
    /// verified-email linkage.
    ///
    /// Unlike [`bind`](Self::bind) (channel actors, no email), this records
    /// the identity's `email` / `email_verified` and — for a verified email
    /// — seeds the canonical verified-email index so a *later* login through
    /// a different provider with the same verified email converges on the
    /// migrated user instead of minting a second one. Unlike
    /// [`resolve_or_create`](Self::resolve_or_create) it never mints: the
    /// supplied `user_id` is authoritative. Idempotent — re-running the
    /// migration must not clobber records a returning user already created,
    /// so existing identity / index records win.
    async fn adopt_migrated_identity(
        &self,
        identity: ResolveExternalIdentity,
        user_id: &UserId,
    ) -> Result<(), RebornIdentityError>;
}

#[async_trait]
impl<T> RebornIdentityResolver for std::sync::Arc<T>
where
    T: RebornIdentityResolver + ?Sized,
{
    async fn resolve_or_create(
        &self,
        identity: ResolveExternalIdentity,
    ) -> Result<UserId, RebornIdentityError> {
        self.as_ref().resolve_or_create(identity).await
    }

    async fn lookup(
        &self,
        key: ExternalIdentityKey,
    ) -> Result<Option<UserId>, RebornIdentityError> {
        self.as_ref().lookup(key).await
    }

    async fn bind(
        &self,
        key: ExternalIdentityKey,
        user_id: &UserId,
    ) -> Result<(), RebornIdentityError> {
        self.as_ref().bind(key, user_id).await
    }

    async fn adopt_migrated_identity(
        &self,
        identity: ResolveExternalIdentity,
        user_id: &UserId,
    ) -> Result<(), RebornIdentityError> {
        self.as_ref()
            .adopt_migrated_identity(identity, user_id)
            .await
    }
}
