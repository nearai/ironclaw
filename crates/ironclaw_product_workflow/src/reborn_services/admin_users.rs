//! Admin user-management port + wire contract.
//!
//! The [`AdminUserService`] port is defined here (the contract owner) and
//! implemented by lifecycle and managed-resource adapters in
//! `ironclaw_reborn_composition`. Defining the ports
//! here keeps `ironclaw_product_workflow` and `ironclaw_webui` free of a
//! dependency on `ironclaw_reborn_identity` (the crate boundary the
//! architecture tests enforce) — this is dependency inversion, a single-impl
//! trait by design.
//!
//! The `Reborn*` request/response types are the stable HTTP wire contract the
//! WebChat v2 admin routes serialize; both the facade and the route handlers
//! import them from here.

use std::collections::BTreeMap;

use async_trait::async_trait;
use ironclaw_host_api::{SecretHandle, TenantId, UserId};
use secrecy::SecretString;
use serde::{Deserialize, Serialize};

/// Account status. Wire-stable snake_case.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AdminUserStatus {
    Active,
    Suspended,
}

/// Account role. Wire-stable snake_case. `Owner` and `Admin` clear the admin
/// authorization boundary; `Member` does not.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AdminUserRole {
    Owner,
    Admin,
    Member,
}

/// Immutable user-owned content/login policy. Kept separate from RBAC role.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AdminUserContentAccessPolicy {
    #[default]
    Private,
    TenantAdminManaged,
}

impl AdminUserRole {
    /// Whether this role clears the admin authorization boundary.
    pub fn is_admin(self) -> bool {
        matches!(self, AdminUserRole::Owner | AdminUserRole::Admin)
    }
}

/// One user as seen by the admin surface — doubles as the domain record the
/// port returns and the JSON body the WebUI renders. Never carries an API
/// token: a freshly minted token is exposed exactly once via
/// [`RebornAdminUserCreatedResponse`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdminUserRecord {
    pub user_id: UserId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    pub status: AdminUserStatus,
    pub role: AdminUserRole,
    pub content_access_policy: AdminUserContentAccessPolicy,
    pub created_at: String,
    pub updated_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_by: Option<UserId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_login_at: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub metadata: BTreeMap<String, String>,
}

/// Metadata for one provisioned per-user secret. Never carries the material.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdminUserSecretMeta {
    pub handle: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
}

/// Fields for creating a private human user record. No credential is issued.
#[derive(Debug, Clone)]
pub struct AdminCreatePrivateUserFields {
    pub email: Option<String>,
    pub display_name: Option<String>,
    pub role: AdminUserRole,
}

/// Fields for creating a non-login tenant-admin-managed subject.
#[derive(Debug, Clone)]
pub struct AdminCreateManagedUserFields {
    pub display_name: Option<String>,
}

/// A newly created user. Creation never returns a login credential.
pub struct AdminCreatedUser {
    pub record: AdminUserRecord,
}

/// Failure modes of the admin user port. Deliberately coarse and free of
/// backend detail — the composition adapter maps identity/secret errors into
/// these, and the facade maps these into the sanitized `RebornServicesError`
/// wire taxonomy. Authorization and last-admin protection are enforced in the
/// facade, not here, so they are not modeled as port errors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdminUserError {
    /// The targeted user id has no record.
    NotFound,
    /// A caller-supplied value is malformed (e.g. an invalid secret handle).
    /// Maps to a 400, not a 500 — it is the client's input at fault, not the
    /// backend.
    InvalidInput,
    /// The authenticated actor is not permitted to act on the target.
    Forbidden,
    /// A transient backend failure; the caller may retry.
    Unavailable,
    /// A backend inconsistency or unexpected failure; not retryable.
    Internal,
}

/// Admin user-management operations. Implemented by the composition adapter
/// over the identity user-directory + per-user secret store.
///
/// Every method is tenant-scoped from the trusted caller (never a request
/// body). `get_user` must return `Ok(None)` — not `Err(NotFound)` — for a user
/// that does not exist in the tenant, so the facade can distinguish "no such
/// user" (404) from "exists but you may not" (403) at the authorization seam.
/// Default page size for `list_users` when the caller omits `limit`.
pub const ADMIN_USER_LIST_DEFAULT_LIMIT: usize = 100;
/// Hard ceiling on the `list_users` page size, so a caller cannot widen the
/// response (and the backing directory scan) by passing a huge `limit`.
pub const ADMIN_USER_LIST_MAX_LIMIT: usize = 200;

#[async_trait]
pub trait AdminUserService: Send + Sync {
    /// One bounded page of users in `tenant`, optionally filtered by `status`,
    /// ordered by `user_id` ascending and starting strictly after the `after`
    /// cursor. At most `limit` records are returned; the facade derives the
    /// next cursor from the last record when a full page comes back.
    async fn list_users(
        &self,
        tenant: &TenantId,
        status: Option<AdminUserStatus>,
        after: Option<&UserId>,
        limit: usize,
    ) -> Result<Vec<AdminUserRecord>, AdminUserError>;

    async fn get_user(
        &self,
        tenant: &TenantId,
        user_id: &UserId,
    ) -> Result<Option<AdminUserRecord>, AdminUserError>;

    async fn create_private_user(
        &self,
        tenant: &TenantId,
        actor: &UserId,
        fields: AdminCreatePrivateUserFields,
    ) -> Result<AdminCreatedUser, AdminUserError>;

    async fn create_managed_user(
        &self,
        tenant: &TenantId,
        actor: &UserId,
        fields: AdminCreateManagedUserFields,
    ) -> Result<AdminCreatedUser, AdminUserError>;

    async fn update_profile(
        &self,
        tenant: &TenantId,
        user_id: &UserId,
        display_name: Option<String>,
        metadata: Option<BTreeMap<String, String>>,
    ) -> Result<AdminUserRecord, AdminUserError>;

    async fn set_status(
        &self,
        tenant: &TenantId,
        user_id: &UserId,
        status: AdminUserStatus,
    ) -> Result<AdminUserRecord, AdminUserError>;

    async fn set_role(
        &self,
        tenant: &TenantId,
        user_id: &UserId,
        role: AdminUserRole,
    ) -> Result<AdminUserRecord, AdminUserError>;

    async fn delete_user(&self, tenant: &TenantId, user_id: &UserId) -> Result<(), AdminUserError>;

    async fn count_active_admins(&self, tenant: &TenantId) -> Result<usize, AdminUserError>;
}

/// Narrow data-plane port for administrator-on-behalf access to managed-user
/// resources. Implementations must invoke the canonical identity-domain gate
/// with both actor and subject identities before every operation.
#[async_trait]
pub trait AdminManagedResourceService: Send + Sync {
    async fn list_secrets(
        &self,
        tenant: &TenantId,
        actor_user_id: &UserId,
        subject_user_id: &UserId,
    ) -> Result<Vec<AdminUserSecretMeta>, AdminUserError>;

    async fn put_secret(
        &self,
        tenant: &TenantId,
        actor_user_id: &UserId,
        subject_user_id: &UserId,
        handle: SecretHandle,
        material: SecretString,
    ) -> Result<AdminUserSecretMeta, AdminUserError>;

    async fn delete_secret(
        &self,
        tenant: &TenantId,
        actor_user_id: &UserId,
        subject_user_id: &UserId,
        handle: SecretHandle,
    ) -> Result<bool, AdminUserError>;
}

/// Fail-closed default wired into `RebornServices` before composition installs
/// the real adapter. Every operation reports the service unavailable, so a
/// deployment that never wires the admin surface serves 503s rather than
/// panicking or silently succeeding. Mirrors the `Rejecting*` default pattern
/// used for the other optional-but-live services on `RebornServices`.
pub(crate) struct RejectingAdminUserService;

#[async_trait]
impl AdminUserService for RejectingAdminUserService {
    async fn list_users(
        &self,
        _tenant: &TenantId,
        _status: Option<AdminUserStatus>,
        _after: Option<&UserId>,
        _limit: usize,
    ) -> Result<Vec<AdminUserRecord>, AdminUserError> {
        Err(AdminUserError::Unavailable)
    }

    async fn get_user(
        &self,
        _tenant: &TenantId,
        _user_id: &UserId,
    ) -> Result<Option<AdminUserRecord>, AdminUserError> {
        Err(AdminUserError::Unavailable)
    }

    async fn create_private_user(
        &self,
        _tenant: &TenantId,
        _actor: &UserId,
        _fields: AdminCreatePrivateUserFields,
    ) -> Result<AdminCreatedUser, AdminUserError> {
        Err(AdminUserError::Unavailable)
    }

    async fn create_managed_user(
        &self,
        _tenant: &TenantId,
        _actor: &UserId,
        _fields: AdminCreateManagedUserFields,
    ) -> Result<AdminCreatedUser, AdminUserError> {
        Err(AdminUserError::Unavailable)
    }

    async fn update_profile(
        &self,
        _tenant: &TenantId,
        _user_id: &UserId,
        _display_name: Option<String>,
        _metadata: Option<BTreeMap<String, String>>,
    ) -> Result<AdminUserRecord, AdminUserError> {
        Err(AdminUserError::Unavailable)
    }

    async fn set_status(
        &self,
        _tenant: &TenantId,
        _user_id: &UserId,
        _status: AdminUserStatus,
    ) -> Result<AdminUserRecord, AdminUserError> {
        Err(AdminUserError::Unavailable)
    }

    async fn set_role(
        &self,
        _tenant: &TenantId,
        _user_id: &UserId,
        _role: AdminUserRole,
    ) -> Result<AdminUserRecord, AdminUserError> {
        Err(AdminUserError::Unavailable)
    }

    async fn delete_user(
        &self,
        _tenant: &TenantId,
        _user_id: &UserId,
    ) -> Result<(), AdminUserError> {
        Err(AdminUserError::Unavailable)
    }

    async fn count_active_admins(&self, _tenant: &TenantId) -> Result<usize, AdminUserError> {
        Err(AdminUserError::Unavailable)
    }
}

pub(crate) struct RejectingAdminManagedResourceService;

#[async_trait]
impl AdminManagedResourceService for RejectingAdminManagedResourceService {
    async fn list_secrets(
        &self,
        _tenant: &TenantId,
        _actor_user_id: &UserId,
        _subject_user_id: &UserId,
    ) -> Result<Vec<AdminUserSecretMeta>, AdminUserError> {
        Err(AdminUserError::Unavailable)
    }

    async fn put_secret(
        &self,
        _tenant: &TenantId,
        _actor_user_id: &UserId,
        _subject_user_id: &UserId,
        _handle: SecretHandle,
        _material: SecretString,
    ) -> Result<AdminUserSecretMeta, AdminUserError> {
        Err(AdminUserError::Unavailable)
    }

    async fn delete_secret(
        &self,
        _tenant: &TenantId,
        _actor_user_id: &UserId,
        _subject_user_id: &UserId,
        _handle: SecretHandle,
    ) -> Result<bool, AdminUserError> {
        Err(AdminUserError::Unavailable)
    }
}

// --- Wire contract (WebChat v2 admin routes) ---------------------------------

/// Query params for `GET /admin/users`.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct RebornAdminUserListQuery {
    #[serde(default)]
    pub status: Option<AdminUserStatus>,
    /// Page size. Clamped to `[1, ADMIN_USER_LIST_MAX_LIMIT]`; omitted means
    /// `ADMIN_USER_LIST_DEFAULT_LIMIT`.
    #[serde(default)]
    pub limit: Option<u32>,
    /// Opaque forward cursor: the `next_cursor` echoed from a prior response
    /// (a `user_id`). The browser never interprets it.
    #[serde(default)]
    pub cursor: Option<String>,
}

/// Response for `GET /admin/users`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RebornAdminUserListResponse {
    pub users: Vec<AdminUserRecord>,
    /// Cursor to pass as `?cursor=` for the next page, or `None` when the
    /// caller has reached the end of the tenant's users.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}

/// Body for `POST /admin/users`.
#[derive(Debug, Clone, Deserialize)]
pub struct RebornAdminCreateUserRequest {
    #[serde(default)]
    pub email: Option<String>,
    #[serde(default)]
    pub display_name: Option<String>,
    pub role: AdminUserRole,
}

/// Body for `POST /admin/agents`.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct RebornAdminCreateManagedUserRequest {
    #[serde(default)]
    pub display_name: Option<String>,
}

/// Route-normalized creation intent carried through the existing admin-user
/// facade operation. The HTTP route selects the variant, so request JSON
/// cannot switch private-user creation into managed-subject creation.
#[derive(Debug, Clone)]
pub enum AdminUserCreationRequest {
    Private(RebornAdminCreateUserRequest),
    Managed(RebornAdminCreateManagedUserRequest),
}

impl From<RebornAdminCreateUserRequest> for AdminUserCreationRequest {
    fn from(request: RebornAdminCreateUserRequest) -> Self {
        Self::Private(request)
    }
}

impl From<RebornAdminCreateManagedUserRequest> for AdminUserCreationRequest {
    fn from(request: RebornAdminCreateManagedUserRequest) -> Self {
        Self::Managed(request)
    }
}

/// Response for private-user or managed-agent creation. It intentionally has
/// no bearer/token field.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RebornAdminUserCreatedResponse {
    pub user: AdminUserRecord,
}

/// Body for `PATCH /admin/users/{id}` — partial profile update.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct RebornAdminUpdateUserRequest {
    #[serde(default)]
    pub display_name: Option<String>,
    #[serde(default)]
    pub metadata: Option<BTreeMap<String, String>>,
}

/// Body for `POST /admin/users/{id}/status`.
#[derive(Debug, Clone, Deserialize)]
pub struct RebornAdminSetStatusRequest {
    pub status: AdminUserStatus,
}

/// Body for `POST /admin/users/{id}/role`.
#[derive(Debug, Clone, Deserialize)]
pub struct RebornAdminSetRoleRequest {
    pub role: AdminUserRole,
}

/// Response for the single-user reads/mutations (get, update, status, role).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RebornAdminUserResponse {
    pub user: AdminUserRecord,
}

/// Response for `DELETE /admin/users/{id}`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RebornAdminUserDeletedResponse {
    pub user_id: UserId,
    pub deleted: bool,
}

/// Response for `GET /admin/users/{id}/secrets`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RebornAdminUserSecretsListResponse {
    pub secrets: Vec<AdminUserSecretMeta>,
}

/// Body for `PUT /admin/users/{id}/secrets/{handle}` (handle is in the path).
#[derive(Debug, Clone, Deserialize)]
pub struct RebornAdminPutSecretRequest {
    pub value: String,
}

/// Response for `PUT /admin/users/{id}/secrets/{handle}`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RebornAdminSecretResponse {
    pub secret: AdminUserSecretMeta,
}

/// Response for `DELETE /admin/users/{id}/secrets/{handle}`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RebornAdminSecretDeletedResponse {
    pub handle: String,
    pub deleted: bool,
}
