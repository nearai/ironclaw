//! Admin-facing user directory: enumeration and lifecycle over the canonical
//! [`StoredUser`](crate::filesystem_store) records this crate already persists.
//!
//! This is a **separate trait** from [`RebornIdentityResolver`](crate::RebornIdentityResolver)
//! on purpose. The resolver owns the security-load-bearing mint/link/create
//! contract (verified-email linking, channel-actor fail-closed); the directory
//! owns the CRUD an operator performs against those records. Keeping them apart
//! means the admin surface cannot accidentally reach into the resolution
//! invariants, and the resolver's contract tests are not perturbed by admin
//! methods.
//!
//! Both traits are implemented by the one
//! [`RebornIdentityStore`](crate::RebornIdentityStore), so
//! the composition root gets both surfaces from a single `Arc`.

use std::collections::BTreeMap;

use async_trait::async_trait;
use ironclaw_host_api::{TenantId, UserId};

use crate::RebornIdentityError;

/// A canonical Reborn user as seen by the admin surface. The public domain
/// mirror of the persisted `StoredUser` row — the on-disk shape stays a private
/// implementation detail, and this type is what the composition adapter maps
/// into the product-workflow wire contract.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RebornUser {
    pub user_id: UserId,
    pub email: Option<String>,
    pub display_name: Option<String>,
    pub status: RebornUserStatus,
    pub role: RebornUserRole,
    pub created_at: String,
    pub updated_at: String,
    /// The admin `UserId` that provisioned this account, if it was created
    /// through the admin surface rather than an SSO first-login.
    pub created_by: Option<UserId>,
    pub last_login_at: Option<String>,
    /// Owning tenant. `None` on records written before the admin surface
    /// existed (treated as the deployment's single configured tenant).
    pub tenant_id: Option<TenantId>,
    pub metadata: BTreeMap<String, String>,
}

/// Account status.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RebornUserStatus {
    Active,
    Suspended,
}

/// Account role. `Owner` and `Admin` both clear the admin boundary; `Member` is
/// an ordinary user.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RebornUserRole {
    Owner,
    Admin,
    Member,
}

impl RebornUserRole {
    /// Whether this role clears the admin authorization boundary.
    pub fn is_admin(self) -> bool {
        matches!(self, RebornUserRole::Owner | RebornUserRole::Admin)
    }
}

/// A partial profile update. Each `None` field is left unchanged (PATCH
/// semantics), so the caller can update the display name without touching
/// metadata and vice versa.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RebornUserProfileUpdate {
    pub display_name: Option<String>,
    pub metadata: Option<BTreeMap<String, String>>,
}

/// Admin CRUD over canonical user records. Implemented by
/// [`RebornIdentityStore`](crate::RebornIdentityStore).
///
/// This trait is a **port**: it is defined here (bottom of the Reborn stack)
/// and its only production implementor is the filesystem store. The composition
/// root adapts it up to the product-workflow admin service, so admin CRUD never
/// forces `ironclaw_product` to depend on this crate (the dependency
/// boundary the architecture tests enforce).
#[async_trait]
pub trait RebornUserDirectory: Send + Sync {
    /// One bounded page of users in `tenant_id`, optionally filtered by status,
    /// ordered by `user_id` ascending and starting strictly after the `after`
    /// cursor. At most `limit` records are returned, so the admin surface never
    /// scans-and-allocates the entire tenant in one call. Records with no
    /// persisted tenant (written before the admin surface) are treated as
    /// belonging to the requested tenant — correct for single-tenant
    /// deployments, which is the only shape that has such records; a returning
    /// user's next login backfills the resolving tenant onto them.
    async fn list_users(
        &self,
        tenant_id: &TenantId,
        status: Option<RebornUserStatus>,
        after: Option<&UserId>,
        limit: usize,
    ) -> Result<Vec<RebornUser>, RebornIdentityError>;

    /// One user by id, or `None` if no record exists.
    async fn get_user(&self, user_id: &UserId) -> Result<Option<RebornUser>, RebornIdentityError>;

    /// Admin-mint a new active user with no external identity. Returns the
    /// created record (carrying the freshly minted `UserId`). Unlike an SSO
    /// first-login this writes only the `users/` record — no verified-email
    /// index — so it does not weaken the resolver's OAuth-surface index gate.
    async fn create_user(
        &self,
        tenant_id: &TenantId,
        email: Option<String>,
        display_name: Option<String>,
        role: RebornUserRole,
        created_by: &UserId,
    ) -> Result<RebornUser, RebornIdentityError>;

    /// Apply a partial profile update. Errors with
    /// [`RebornIdentityError::UserNotFound`] if the user does not exist.
    async fn update_profile(
        &self,
        user_id: &UserId,
        update: RebornUserProfileUpdate,
    ) -> Result<RebornUser, RebornIdentityError>;

    /// Set the account status (suspend / activate).
    async fn update_status(
        &self,
        user_id: &UserId,
        status: RebornUserStatus,
    ) -> Result<RebornUser, RebornIdentityError>;

    /// Set the account role (promote / demote).
    async fn update_role(
        &self,
        user_id: &UserId,
        role: RebornUserRole,
    ) -> Result<RebornUser, RebornIdentityError>;

    /// Record a successful login. Updates `last_login_at` only — it does not
    /// bump `updated_at`, which tracks profile mutations, not login activity.
    async fn record_last_login(
        &self,
        user_id: &UserId,
        at: String,
    ) -> Result<(), RebornIdentityError>;

    /// Delete a user and cascade: every external-identity record bound to the
    /// user (so a later re-login through that identity cannot resolve the
    /// deleted id back to life) and the user's verified-email index. This is
    /// the one sanctioned exception to the resolver's index-integrity
    /// invariants (see CONTRACT.md → "User directory").
    async fn delete_user(
        &self,
        tenant_id: &TenantId,
        user_id: &UserId,
    ) -> Result<(), RebornIdentityError>;

    /// Count active admins/owners in `tenant_id`. Backs last-admin protection
    /// in the product-workflow facade (never demote/suspend/delete the sole
    /// active admin).
    async fn count_active_admins(&self, tenant_id: &TenantId)
    -> Result<usize, RebornIdentityError>;
}
