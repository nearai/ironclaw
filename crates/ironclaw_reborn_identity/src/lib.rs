//! Canonical Reborn **principal** identity layer.
//!
//! The boundary that maps an external identity to a stable Reborn [`UserId`]
//! *before* any runtime state (conversation binding, thread ownership) is
//! touched, and the only path in the stack that **mints** a user.
//!
//! - Identity provisioning lives HERE, not in WebUI ingress and not in
//!   `ironclaw_conversations` (which stays lookup/binding-oriented and
//!   consumes an already-resolved `UserId`).
//! - WebUI OAuth feeds normalized [`ResolveExternalIdentity`] values into
//!   [`RebornIdentityResolver`].
//! - **Channel actors are not bound here.** `SurfaceKind::ChannelActor` is
//!   rejected on `resolve_or_create` ([`RebornIdentityError::ChannelActorNotMintable`])
//!   and this crate offers no binding path of its own: post-OAuth channel
//!   binding belongs to `ironclaw_extension_host`'s channel identity store,
//!   behind the `ironclaw_host_api::user_identity` ports. The two stores and
//!   the line between them are specified in `CONTRACT.md`, "Two
//!   external-identity stores". `ExternalIdentityKey` and
//!   `RebornIdentityResolver::{lookup, bind}` were retired in #5618 — they
//!   had no production caller and the key was not even constructible
//!   downstream.
//!
//! The external identity is keyed by `(tenant_id, surface_kind,
//! provider_kind, provider_instance_id, external_subject_id)` so two
//! tenants, two adapter installations, or two surfaces cannot collide on
//! the same subject id. Verified email may link OAuth providers within a
//! tenant; an unverified email never links.
//!
//! Persistence ([`RebornIdentityStore`]) goes through the host
//! [`RootFilesystem`](ironclaw_filesystem::RootFilesystem) /
//! `ScopedFilesystem` abstraction — the same substrate boundary every other
//! durable Reborn store sits behind — so substrate choice, tenant scoping,
//! and host ownership stay centralized in the filesystem layer rather than
//! this crate holding a raw database handle.

mod identity_store;
mod key;
mod user_directory;

pub use identity_store::RebornIdentityStore;
pub use key::{ExternalSubjectId, IdentityKeyError, ProviderInstanceId, ProviderKind};
pub use user_directory::{
    RebornUser, RebornUserDirectory, RebornUserProfileUpdate, RebornUserRole, RebornUserStatus,
};

use async_trait::async_trait;
use ironclaw_host_api::ids::{TenantId, UserId};
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
    /// An admin directory operation targeted a user id with no record. Distinct
    /// from `Backend` so the product-workflow service can map it to a 404.
    #[error("no user record for id: {0}")]
    UserNotFound(String),
    /// `resolve_or_create` resolved an external identity to an existing user
    /// whose account is suspended. Distinct from `Backend` so the SSO host
    /// adapter can map it to a fail-closed 403 (login refused) instead of a
    /// 503 backend fault: a suspended user must not mint a fresh session.
    #[error("user account is suspended: {0}")]
    UserSuspended(String),
    /// `resolve_or_create` was called for a `ChannelActor` identity. Channel
    /// actors are never mint-capable, and this crate does not bind them at
    /// all: post-OAuth channel binding is owned by
    /// `ironclaw_extension_host::channel_identity_store` behind the
    /// `ironclaw_host_api::user_identity` ports (see `CONTRACT.md`, "Two
    /// external-identity stores"). The guard keeps a channel actor from
    /// auto-provisioning a Reborn account through this path.
    #[error(
        "channel-actor identities are bound by the channel identity store, not resolve_or_create"
    )]
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
    /// channel actors are never mint-capable, and their binding is owned by
    /// the channel identity store, not by this trait (`CONTRACT.md`, "Two
    /// external-identity stores").
    async fn resolve_or_create(
        &self,
        identity: ResolveExternalIdentity,
    ) -> Result<UserId, RebornIdentityError>;

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
