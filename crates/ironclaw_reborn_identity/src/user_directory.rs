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
//! [`FilesystemRebornIdentityStore`](crate::FilesystemRebornIdentityStore), so
//! the composition root gets both surfaces from a single `Arc`.

use std::collections::BTreeMap;
use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_host_api::{TenantId, UserId};
use uuid::Uuid;

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
    pub content_access_policy: UserContentAccessPolicy,
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

/// Immutable authority governing login and administrator-on-behalf access to
/// user-owned content. This is deliberately independent of RBAC role.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum UserContentAccessPolicy {
    /// A human account. Administrators may manage its lifecycle, but may not
    /// read or mutate its user-owned resources on the user's behalf.
    #[default]
    Private,
    /// A non-login subject intentionally created for tenant administrators to
    /// manage. The subject remains a Member and receives no login credential.
    TenantAdminManaged,
}

/// Closed set of administrator-on-behalf operations. Adding a new resource
/// operation requires an explicit identity-domain policy decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdminManagedUserOperation {
    ManageSecrets,
}

/// Mint a fresh canonical user id for an identity record that has not yet been
/// persisted. Callers may use this to prepare credentials before creation; a
/// credential remains unusable until the corresponding active private user
/// record exists.
pub fn new_user_id() -> Result<UserId, RebornIdentityError> {
    UserId::new(Uuid::new_v4().to_string())
        .map_err(|error| RebornIdentityError::InvalidUserId(error.to_string()))
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

/// Complete identity-owned input for persisting a user whose canonical id was
/// allocated before the write.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreallocatedRebornUser {
    pub user_id: UserId,
    pub tenant_id: TenantId,
    pub email: Option<String>,
    pub display_name: Option<String>,
    pub role: RebornUserRole,
    pub content_access_policy: UserContentAccessPolicy,
    pub created_by: UserId,
}

/// Admin CRUD over canonical user records. Implemented by
/// [`FilesystemRebornIdentityStore`](crate::FilesystemRebornIdentityStore).
///
/// This trait is a **port**: it is defined here (bottom of the Reborn stack)
/// and its only production implementor is the filesystem store. The composition
/// root adapts it up to the product-workflow admin service, so admin CRUD never
/// forces `ironclaw_product_workflow` to depend on this crate (the dependency
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

    /// Create a new active user with no external identity. A private user with
    /// an email reserves the verified-email claim index, but an external
    /// identity is linked only after the existing OAuth verification gate.
    async fn create_user(
        &self,
        tenant_id: &TenantId,
        email: Option<String>,
        display_name: Option<String>,
        role: RebornUserRole,
        content_access_policy: UserContentAccessPolicy,
        created_by: &UserId,
    ) -> Result<RebornUser, RebornIdentityError>;

    /// Create a user using an identity-domain id allocated before persistence.
    /// The same invariants as [`Self::create_user`] apply.
    async fn create_user_with_id(
        &self,
        user: PreallocatedRebornUser,
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

    /// Canonical authorization decision for an administrator acting on a
    /// managed target's user-owned resources. Requires an active admin actor,
    /// same-tenant actor and target records, a managed target policy, and an
    /// explicitly modeled operation.
    async fn authorize_admin_managed_target(
        &self,
        tenant_id: &TenantId,
        actor_user_id: &UserId,
        subject_user_id: &UserId,
        operation: AdminManagedUserOperation,
    ) -> Result<bool, RebornIdentityError>;

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

/// Narrow identity-owned policy used to issue and authenticate reusable login
/// credentials without exposing the lifecycle directory to host ingress.
#[async_trait]
pub trait RebornLoginPolicy: Send + Sync {
    /// Whether `actor_user_id` may issue a reusable credential for a new
    /// private user in `tenant_id`.
    async fn authorize_admin_login_token_issuance(
        &self,
        tenant_id: &TenantId,
        actor_user_id: &UserId,
    ) -> Result<bool, RebornIdentityError>;

    /// Whether a reusable credential may currently authenticate its subject.
    /// Requires an active, explicitly same-tenant private user record.
    async fn authorize_reusable_login_token(
        &self,
        tenant_id: &TenantId,
        subject_user_id: &UserId,
    ) -> Result<bool, RebornIdentityError>;
}

/// Build the canonical login policy over the lifecycle directory without
/// exposing directory mutation methods to authentication callers.
pub fn login_policy(directory: Arc<dyn RebornUserDirectory>) -> Arc<dyn RebornLoginPolicy> {
    Arc::new(DirectoryLoginPolicy { directory })
}

struct DirectoryLoginPolicy {
    directory: Arc<dyn RebornUserDirectory>,
}

#[async_trait]
impl RebornLoginPolicy for DirectoryLoginPolicy {
    async fn authorize_admin_login_token_issuance(
        &self,
        tenant_id: &TenantId,
        actor_user_id: &UserId,
    ) -> Result<bool, RebornIdentityError> {
        let Some(actor) = self.directory.get_user(actor_user_id).await? else {
            return Ok(false);
        };
        Ok(actor
            .tenant_id
            .as_ref()
            .is_some_and(|owner| owner == tenant_id)
            && actor.status == RebornUserStatus::Active
            && actor.role.is_admin()
            && actor.content_access_policy == UserContentAccessPolicy::Private)
    }

    async fn authorize_reusable_login_token(
        &self,
        tenant_id: &TenantId,
        subject_user_id: &UserId,
    ) -> Result<bool, RebornIdentityError> {
        let Some(subject) = self.directory.get_user(subject_user_id).await? else {
            return Ok(false);
        };
        Ok(subject
            .tenant_id
            .as_ref()
            .is_some_and(|owner| owner == tenant_id)
            && subject.status == RebornUserStatus::Active
            && subject.content_access_policy == UserContentAccessPolicy::Private)
    }
}
