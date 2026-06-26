//! Durable local-dev user directory + admin REST surface (#5272).
//!
//! Users are created **through REST** (the same admin contract the future UI
//! will drive), not pre-listed in an env var. This owns the **identity** side —
//! users, roles, and the bearer tokens that let one operator act as several
//! users on localhost; #5268 owns the **capability** side (what they can do).
//!
//! - Durable store: `user_id → { role, token_hash }` + a `token_hash →
//!   user_id` index, persisted over the `/tenants` libSQL-backed mount (survives
//!   restart, like the grants). Tokens are stored **hashed**, never plaintext.
//! - Admin REST (gated by `WebUiAuthenticatedCaller::is_admin()`): create user
//!   (mints a token — a local-dev affordance), set role, list, delete.
//! - The dynamic authenticator that resolves those tokens lives in
//!   `ironclaw_reborn_webui_ingress` and reads [`LocalUserDirectoryStore`].
//!
//! The endpoints consume an authenticated `caller{user, role}` agnostic to how
//! auth happened, so the production/UI path (session/SSO) reuses them unchanged.

use std::num::{NonZeroU32, NonZeroU64};
use std::sync::Arc;

use async_trait::async_trait;
use axum::{
    Json, Router,
    extract::{Extension, Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::get,
};
use ironclaw_filesystem::{CasExpectation, Entry, Filter, Page, RecordKind, RootFilesystem};
use ironclaw_host_api::ingress::{
    AllowedEffectPath, AuditTraceClass, BodyLimitPolicy, CorsPolicy, IngressAuthPolicy,
    IngressAuthScheme, IngressPolicy, IngressPolicyParts, IngressRouteDescriptor,
    IngressScopeSource, ListenerClass, RateLimitPolicy, RateLimitScope, StreamingMode,
    WebSocketOriginPolicy,
};
use ironclaw_host_api::{NetworkMethod, TenantId, UserId, UserRole, VirtualPath};
use ironclaw_product_workflow::WebUiAuthenticatedCaller;
use rand::RngCore;
use rand::rngs::OsRng;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::ProtectedRouteMount;

/// Durable root for the local-dev user directory, under the `/tenants` libSQL
/// mount (the `/engine` default has no backend in the local-dev composite).
const LOCAL_USER_DIRECTORY_ROOT: &str = "/tenants/local_users";
const USER_RECORD_KIND: &str = "reborn_local_user";
const TOKEN_RECORD_KIND: &str = "reborn_local_user_token";

const ADMIN_USERS_PATH: &str = "/api/webchat/v2/admin/users";
const ADMIN_USER_ITEM_PATH: &str = "/api/webchat/v2/admin/users/{user_id}";
const ADMIN_USER_ROLE_PATH: &str = "/api/webchat/v2/admin/users/{user_id}/role";
const ADMIN_USERS_CREATE_ROUTE_ID: &str = "webui.v2.admin.users.create";
const ADMIN_USERS_LIST_ROUTE_ID: &str = "webui.v2.admin.users.list";
const ADMIN_USERS_DELETE_ROUTE_ID: &str = "webui.v2.admin.users.delete";
const ADMIN_USERS_SET_ROLE_ROUTE_ID: &str = "webui.v2.admin.users.set_role";
const ADMIN_USERS_BODY_LIMIT_BYTES: NonZeroU64 = NonZeroU64::new(16 * 1024).unwrap(); // safety: 16 KiB is non-zero.
const ADMIN_USERS_MAX_REQUESTS: NonZeroU32 = NonZeroU32::new(60).unwrap(); // safety: 60 is non-zero.
const ADMIN_USERS_RATE_WINDOW_SECONDS: NonZeroU32 = NonZeroU32::new(60).unwrap(); // safety: 60 is non-zero.

/// Hash a bearer token for at-rest storage + lookup. Domain-separated SHA-256;
/// the raw token is never persisted.
pub fn hash_user_token(token: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"reborn-local-user-token-v1::");
    hasher.update(token.as_bytes());
    hex_lower(&hasher.finalize())
}

fn hex_lower(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

/// A resolved user: who they are and their role.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalUserRecord {
    pub user_id: UserId,
    pub role: UserRole,
}

/// Failure modes of the user directory. Sanitized — no backend internals leak
/// to the client (the cause is logged at the boundary).
#[derive(Debug, thiserror::Error)]
pub enum LocalUserDirectoryError {
    #[error("user directory backend failure: {0}")]
    Backend(String),
    #[error("user directory record is malformed: {0}")]
    Malformed(String),
    #[error("user not found")]
    NotFound,
}

/// Durable store of REST-created local-dev users. See the module docs.
#[async_trait]
pub trait LocalUserDirectoryStore: Send + Sync {
    /// Create (or overwrite) `user_id` with `role`, addressable by `token_hash`.
    async fn create_user(
        &self,
        tenant_id: &TenantId,
        user_id: &UserId,
        role: UserRole,
        token_hash: &str,
    ) -> Result<(), LocalUserDirectoryError>;

    /// Set an existing user's role.
    async fn set_role(
        &self,
        tenant_id: &TenantId,
        user_id: &UserId,
        role: UserRole,
    ) -> Result<(), LocalUserDirectoryError>;

    /// List all users in the tenant.
    async fn list_users(
        &self,
        tenant_id: &TenantId,
    ) -> Result<Vec<LocalUserRecord>, LocalUserDirectoryError>;

    /// Remove a user (and its token index entry).
    async fn delete_user(
        &self,
        tenant_id: &TenantId,
        user_id: &UserId,
    ) -> Result<(), LocalUserDirectoryError>;

    /// Resolve a hashed token to its user + role, or `None` if unknown.
    async fn resolve_token(
        &self,
        tenant_id: &TenantId,
        token_hash: &str,
    ) -> Result<Option<LocalUserRecord>, LocalUserDirectoryError>;

    /// Resolve a user by id to its current record (role), or `None` if unknown.
    /// Used by the admin guards to compare the caller's rank against the
    /// target's before a destructive mutation.
    async fn resolve_user(
        &self,
        tenant_id: &TenantId,
        user_id: &UserId,
    ) -> Result<Option<LocalUserRecord>, LocalUserDirectoryError>;
}

#[derive(Debug, Serialize, Deserialize)]
struct StoredUser {
    user_id: String,
    role: UserRole,
    token_hash: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct StoredToken {
    user_id: String,
}

/// Filesystem-backed [`LocalUserDirectoryStore`] over a [`RootFilesystem`]
/// (the local-dev composite, rooted under the durable `/tenants` mount).
///
/// Crate-internal: callers receive an `Arc<dyn LocalUserDirectoryStore>` from
/// [`build_local_user_directory_store`] rather than naming this concrete type.
pub(crate) struct FilesystemLocalUserDirectoryStore {
    filesystem: Arc<dyn RootFilesystem>,
    root: VirtualPath,
}

impl FilesystemLocalUserDirectoryStore {
    pub(crate) fn new(
        filesystem: Arc<dyn RootFilesystem>,
    ) -> Result<Self, LocalUserDirectoryError> {
        let root = VirtualPath::new(LOCAL_USER_DIRECTORY_ROOT)
            .map_err(|error| LocalUserDirectoryError::Backend(error.to_string()))?;
        Ok(Self { filesystem, root })
    }

    fn users_dir(&self, tenant_id: &TenantId) -> Result<VirtualPath, LocalUserDirectoryError> {
        self.path(format!(
            "{}/{}/users",
            self.root.as_str().trim_end_matches('/'),
            hex_lower(tenant_id.as_str().as_bytes())
        ))
    }

    fn user_path(
        &self,
        tenant_id: &TenantId,
        user_id: &UserId,
    ) -> Result<VirtualPath, LocalUserDirectoryError> {
        self.path(format!(
            "{}/{}/users/{}.json",
            self.root.as_str().trim_end_matches('/'),
            hex_lower(tenant_id.as_str().as_bytes()),
            hex_lower(user_id.as_str().as_bytes())
        ))
    }

    fn token_path(
        &self,
        tenant_id: &TenantId,
        token_hash: &str,
    ) -> Result<VirtualPath, LocalUserDirectoryError> {
        self.path(format!(
            "{}/{}/tokens/{}.json",
            self.root.as_str().trim_end_matches('/'),
            hex_lower(tenant_id.as_str().as_bytes()),
            token_hash
        ))
    }

    fn path(&self, raw: String) -> Result<VirtualPath, LocalUserDirectoryError> {
        VirtualPath::new(raw).map_err(|error| LocalUserDirectoryError::Backend(error.to_string()))
    }

    async fn put_json<T: Serialize>(
        &self,
        path: &VirtualPath,
        kind: &str,
        record: &T,
    ) -> Result<(), LocalUserDirectoryError> {
        let value = serde_json::to_value(record)
            .map_err(|error| LocalUserDirectoryError::Malformed(error.to_string()))?;
        let kind = RecordKind::new(kind)
            .map_err(|error| LocalUserDirectoryError::Backend(error.to_string()))?;
        let entry = Entry::record(kind, &value)
            .map_err(|error| LocalUserDirectoryError::Malformed(error.to_string()))?;
        self.filesystem
            .put(path, entry, CasExpectation::Any)
            .await
            .map_err(|error| LocalUserDirectoryError::Backend(error.to_string()))?;
        Ok(())
    }

    async fn get_json<T: serde::de::DeserializeOwned>(
        &self,
        path: &VirtualPath,
    ) -> Result<Option<T>, LocalUserDirectoryError> {
        match self
            .filesystem
            .get(path)
            .await
            .map_err(|error| LocalUserDirectoryError::Backend(error.to_string()))?
        {
            Some(entry) => {
                Ok(Some(entry.entry.parse_json::<T>().map_err(|error| {
                    LocalUserDirectoryError::Malformed(error.to_string())
                })?))
            }
            None => Ok(None),
        }
    }
}

#[async_trait]
impl LocalUserDirectoryStore for FilesystemLocalUserDirectoryStore {
    async fn create_user(
        &self,
        tenant_id: &TenantId,
        user_id: &UserId,
        role: UserRole,
        token_hash: &str,
    ) -> Result<(), LocalUserDirectoryError> {
        let user = StoredUser {
            user_id: user_id.as_str().to_string(),
            role,
            token_hash: token_hash.to_string(),
        };
        self.put_json(
            &self.user_path(tenant_id, user_id)?,
            USER_RECORD_KIND,
            &user,
        )
        .await?;
        let token = StoredToken {
            user_id: user_id.as_str().to_string(),
        };
        self.put_json(
            &self.token_path(tenant_id, token_hash)?,
            TOKEN_RECORD_KIND,
            &token,
        )
        .await
    }

    async fn set_role(
        &self,
        tenant_id: &TenantId,
        user_id: &UserId,
        role: UserRole,
    ) -> Result<(), LocalUserDirectoryError> {
        let path = self.user_path(tenant_id, user_id)?;
        let mut user: StoredUser = self
            .get_json(&path)
            .await?
            .ok_or(LocalUserDirectoryError::NotFound)?;
        user.role = role;
        self.put_json(&path, USER_RECORD_KIND, &user).await
    }

    async fn list_users(
        &self,
        tenant_id: &TenantId,
    ) -> Result<Vec<LocalUserRecord>, LocalUserDirectoryError> {
        let dir = self.users_dir(tenant_id)?;
        let entries = self
            .filesystem
            .query(&dir, &Filter::All, Page::new(0, Page::MAX_LIMIT))
            .await
            .map_err(|error| LocalUserDirectoryError::Backend(error.to_string()))?;
        let mut users = Vec::with_capacity(entries.len());
        for entry in entries {
            let stored: StoredUser = entry
                .entry
                .parse_json()
                .map_err(|error| LocalUserDirectoryError::Malformed(error.to_string()))?;
            users.push(local_user_record(stored)?);
        }
        Ok(users)
    }

    async fn delete_user(
        &self,
        tenant_id: &TenantId,
        user_id: &UserId,
    ) -> Result<(), LocalUserDirectoryError> {
        let path = self.user_path(tenant_id, user_id)?;
        // r5272-3 (404 not 503): fetch the record first so a missing user maps
        // to `NotFound` (→ 404) instead of letting the record-delete's
        // `FilesystemError::NotFound` collapse into `Backend` (→ 503). With the
        // record in hand we delete the token index then the record; both deletes
        // target known-present leaves, so a `NotFound` there would be a genuine
        // backend race and is left classified as `Backend`.
        let user = self
            .get_json::<StoredUser>(&path)
            .await?
            .ok_or(LocalUserDirectoryError::NotFound)?;
        let token_path = self.token_path(tenant_id, &user.token_hash)?;
        self.filesystem
            .delete(&token_path)
            .await
            .map_err(|error| LocalUserDirectoryError::Backend(error.to_string()))?;
        self.filesystem
            .delete(&path)
            .await
            .map_err(|error| LocalUserDirectoryError::Backend(error.to_string()))
    }

    async fn resolve_token(
        &self,
        tenant_id: &TenantId,
        token_hash: &str,
    ) -> Result<Option<LocalUserRecord>, LocalUserDirectoryError> {
        let Some(token): Option<StoredToken> = self
            .get_json(&self.token_path(tenant_id, token_hash)?)
            .await?
        else {
            return Ok(None);
        };
        let user_id = UserId::new(&token.user_id)
            .map_err(|error| LocalUserDirectoryError::Malformed(error.to_string()))?;
        let Some(user): Option<StoredUser> =
            self.get_json(&self.user_path(tenant_id, &user_id)?).await?
        else {
            return Ok(None);
        };
        // r5272-1 (token rotation + orphan index): the user record is the
        // source of truth for the *current* token. A user re-created (or whose
        // token was rotated) keeps a stale `token_hash → user_id` index entry
        // pointing at a user whose record now stores a DIFFERENT hash. Resolve
        // only when the presented hash matches the user's current hash, so an
        // OLD bearer — and a stale index leaf — stops authenticating.
        if user.token_hash != token_hash {
            return Ok(None);
        }
        Ok(Some(local_user_record(user)?))
    }

    async fn resolve_user(
        &self,
        tenant_id: &TenantId,
        user_id: &UserId,
    ) -> Result<Option<LocalUserRecord>, LocalUserDirectoryError> {
        let Some(user): Option<StoredUser> =
            self.get_json(&self.user_path(tenant_id, user_id)?).await?
        else {
            return Ok(None);
        };
        Ok(Some(local_user_record(user)?))
    }
}

fn local_user_record(stored: StoredUser) -> Result<LocalUserRecord, LocalUserDirectoryError> {
    Ok(LocalUserRecord {
        user_id: UserId::new(&stored.user_id)
            .map_err(|error| LocalUserDirectoryError::Malformed(error.to_string()))?,
        role: stored.role,
    })
}

/// Host config for the user-directory admin routes.
#[derive(Clone)]
pub struct LocalUserAdminRouteConfig {
    tenant_id: TenantId,
    store: Arc<dyn LocalUserDirectoryStore>,
}

impl LocalUserAdminRouteConfig {
    pub fn new(tenant_id: TenantId, store: Arc<dyn LocalUserDirectoryStore>) -> Self {
        Self { tenant_id, store }
    }
}

/// Build the user-directory admin routes as a [`ProtectedRouteMount`].
pub fn local_user_admin_route_mount(config: LocalUserAdminRouteConfig) -> ProtectedRouteMount {
    let router = Router::new()
        .route(
            ADMIN_USERS_PATH,
            get(list_users_handler).post(create_user_handler),
        )
        .route(
            ADMIN_USER_ITEM_PATH,
            axum::routing::delete(delete_user_handler),
        )
        .route(ADMIN_USER_ROLE_PATH, axum::routing::put(set_role_handler))
        .with_state(config);
    ProtectedRouteMount::new(router, local_user_admin_descriptors())
}

/// Build the user-directory store over the runtime's durable filesystem.
/// `None` for a runtime with no local substrate.
pub fn build_local_user_directory_store(
    runtime: &crate::runtime::RebornRuntime,
) -> Option<Arc<dyn LocalUserDirectoryStore>> {
    // No local substrate (production / migration-dry-run) → legitimately `None`;
    // the directory simply isn't offered.
    let local_runtime = runtime.services().local_runtime.as_ref()?;
    // XC-3: a `::new(...)` failure is a CONSTRUCTION fault, not "no local
    // runtime". Do not `.ok()?`-collapse it into the same `None` — that would
    // silently disable the admin user directory on a malformed durable root.
    // Log the cause (server context, never the REPL surface) before returning
    // `None` so the failure is diagnosable.
    match FilesystemLocalUserDirectoryStore::new(
        Arc::clone(&local_runtime.extension_filesystem) as Arc<dyn RootFilesystem>
    ) {
        Ok(store) => Some(Arc::new(store)),
        Err(error) => {
            tracing::error!(
                %error,
                "failed to construct local user directory store; admin user directory unavailable"
            );
            None
        }
    }
}

fn local_user_admin_descriptors() -> Vec<IngressRouteDescriptor> {
    vec![
        IngressRouteDescriptor::new(
            ADMIN_USERS_CREATE_ROUTE_ID,
            NetworkMethod::Post,
            ADMIN_USERS_PATH,
            route_policy(BodyLimitPolicy::Limited {
                max_bytes: ADMIN_USERS_BODY_LIMIT_BYTES,
            }),
        )
        .expect("admin users create descriptor must validate at startup"),
        IngressRouteDescriptor::new(
            ADMIN_USERS_LIST_ROUTE_ID,
            NetworkMethod::Get,
            ADMIN_USERS_PATH,
            route_policy(BodyLimitPolicy::NoBody),
        )
        .expect("admin users list descriptor must validate at startup"),
        IngressRouteDescriptor::new(
            ADMIN_USERS_DELETE_ROUTE_ID,
            NetworkMethod::Delete,
            ADMIN_USER_ITEM_PATH,
            route_policy(BodyLimitPolicy::NoBody),
        )
        .expect("admin users delete descriptor must validate at startup"),
        IngressRouteDescriptor::new(
            ADMIN_USERS_SET_ROLE_ROUTE_ID,
            NetworkMethod::Put,
            ADMIN_USER_ROLE_PATH,
            route_policy(BodyLimitPolicy::Limited {
                max_bytes: ADMIN_USERS_BODY_LIMIT_BYTES,
            }),
        )
        .expect("admin users set-role descriptor must validate at startup"),
    ]
}

fn route_policy(body_limit: BodyLimitPolicy) -> IngressPolicy {
    IngressPolicy::new(IngressPolicyParts {
        listener_class: ListenerClass::LocalGateway,
        auth: IngressAuthPolicy::Required {
            schemes: vec![IngressAuthScheme::BearerToken],
        },
        scope_source: IngressScopeSource::AuthenticatedCaller,
        body_limit,
        rate_limit: RateLimitPolicy::Limited {
            scope: RateLimitScope::PerCaller,
            max_requests: ADMIN_USERS_MAX_REQUESTS,
            window_seconds: ADMIN_USERS_RATE_WINDOW_SECONDS,
        },
        cors: CorsPolicy::SameOriginOnly,
        websocket_origin: WebSocketOriginPolicy::NotApplicable,
        streaming: StreamingMode::None,
        audit: AuditTraceClass::UserAction,
        effect_path: AllowedEffectPath::ProductWorkflow,
    })
    .expect("admin users policy must validate")
}

fn ensure_admin(
    config: &LocalUserAdminRouteConfig,
    caller: &WebUiAuthenticatedCaller,
) -> Result<(), LocalUserAdminError> {
    if caller.tenant_id != config.tenant_id {
        return Err(LocalUserAdminError::NotFound);
    }
    if !caller.is_admin() {
        return Err(LocalUserAdminError::Forbidden);
    }
    Ok(())
}

#[derive(Debug, Deserialize)]
struct CreateUserRequest {
    user_id: String,
    #[serde(default)]
    role: UserRole,
}

#[derive(Debug, Serialize)]
struct CreateUserResponse {
    user_id: String,
    role: UserRole,
    /// Local-dev affordance: the raw bearer for this user, returned once.
    token: String,
}

#[derive(Debug, Deserialize)]
struct SetRoleRequest {
    role: UserRole,
}

#[derive(Debug, Serialize)]
struct UserSummary {
    user_id: String,
    role: UserRole,
}

#[derive(Debug, Serialize)]
struct ListUsersResponse {
    users: Vec<UserSummary>,
}

async fn create_user_handler(
    State(config): State<LocalUserAdminRouteConfig>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
    Json(request): Json<CreateUserRequest>,
) -> Result<Json<CreateUserResponse>, LocalUserAdminError> {
    ensure_admin(&config, &caller)?;
    let user_id = UserId::new(&request.user_id).map_err(|_| LocalUserAdminError::BadRequest)?;
    let token = mint_token();
    let token_hash = hash_user_token(&token);
    config
        .store
        .create_user(&config.tenant_id, &user_id, request.role, &token_hash)
        .await?;
    Ok(Json(CreateUserResponse {
        user_id: user_id.as_str().to_string(),
        role: request.role,
        token,
    }))
}

async fn set_role_handler(
    State(config): State<LocalUserAdminRouteConfig>,
    Path(user_id): Path<String>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
    Json(request): Json<SetRoleRequest>,
) -> Result<Json<UserSummary>, LocalUserAdminError> {
    ensure_admin(&config, &caller)?;
    let user_id = UserId::new(&user_id).map_err(|_| LocalUserAdminError::BadRequest)?;
    config
        .store
        .set_role(&config.tenant_id, &user_id, request.role)
        .await?;
    Ok(Json(UserSummary {
        user_id: user_id.as_str().to_string(),
        role: request.role,
    }))
}

async fn list_users_handler(
    State(config): State<LocalUserAdminRouteConfig>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
) -> Result<Json<ListUsersResponse>, LocalUserAdminError> {
    ensure_admin(&config, &caller)?;
    let users = config
        .store
        .list_users(&config.tenant_id)
        .await?
        .into_iter()
        .map(|user| UserSummary {
            user_id: user.user_id.as_str().to_string(),
            role: user.role,
        })
        .collect();
    Ok(Json(ListUsersResponse { users }))
}

async fn delete_user_handler(
    State(config): State<LocalUserAdminRouteConfig>,
    Path(user_id): Path<String>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
) -> Result<StatusCode, LocalUserAdminError> {
    ensure_admin(&config, &caller)?;
    let user_id = UserId::new(&user_id).map_err(|_| LocalUserAdminError::BadRequest)?;

    // (a) SELF-DELETE: nobody deletes themselves through this route. Checked
    // FIRST and named explicitly so an Owner's self-delete 403s as self-delete
    // (not as last-owner) regardless of owner count, and an Admin's self-delete
    // 403s here rather than falling through to the rank check (a role never
    // strictly outranks itself, so the rank arm would also reject — but the
    // self-delete guard is the clearer, intended reason).
    if caller.user_id == user_id {
        return Err(LocalUserAdminError::Forbidden);
    }

    // (b) Resolve the target so we can compare ranks and count owners. A missing
    // target maps to NotFound (→ 404), preserving the existing
    // 404-for-missing-user contract (see `delete_missing_user_is_not_found`).
    let target = config
        .store
        .resolve_user(&config.tenant_id, &user_id)
        .await?
        .ok_or(LocalUserAdminError::NotFound)?;

    // (c) RANK: a caller may delete a target only if it STRICTLY outranks it.
    // This makes Admin→Owner and Admin→peer-Admin both 403, and Owner→Admin a
    // 204.
    if !caller.role.outranks(target.role) {
        return Err(LocalUserAdminError::Forbidden);
    }

    // (d) LAST-OWNER: deleting the sole remaining Owner is refused so a tenant
    // can never be left ownerless. The owner count via `list_users` is a
    // read-then-write that could race two concurrent owner-deletes in theory;
    // acceptable for the local-dev single-operator profile.
    if target.role.is_owner() {
        let owner_count = config
            .store
            .list_users(&config.tenant_id)
            .await?
            .into_iter()
            .filter(|user| user.role.is_owner())
            .count();
        if owner_count <= 1 {
            return Err(LocalUserAdminError::Forbidden);
        }
    }

    config
        .store
        .delete_user(&config.tenant_id, &user_id)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

fn mint_token() -> String {
    let mut bytes = [0u8; 32];
    OsRng.fill_bytes(&mut bytes);
    hex_lower(&bytes)
}

/// Sanitized error surface — never leaks store internals to the client.
#[derive(Debug)]
enum LocalUserAdminError {
    BadRequest,
    Forbidden,
    NotFound,
    Unavailable,
}

impl From<LocalUserDirectoryError> for LocalUserAdminError {
    fn from(error: LocalUserDirectoryError) -> Self {
        match error {
            LocalUserDirectoryError::NotFound => Self::NotFound,
            LocalUserDirectoryError::Malformed(_) | LocalUserDirectoryError::Backend(_) => {
                Self::Unavailable
            }
        }
    }
}

impl IntoResponse for LocalUserAdminError {
    fn into_response(self) -> Response {
        let status = match self {
            Self::BadRequest => StatusCode::BAD_REQUEST,
            Self::Forbidden => StatusCode::FORBIDDEN,
            Self::NotFound => StatusCode::NOT_FOUND,
            Self::Unavailable => StatusCode::SERVICE_UNAVAILABLE,
        };
        status.into_response()
    }
}

#[cfg(all(test, feature = "capability-policy"))]
mod tests {
    use super::*;

    use axum::body::Body;
    use axum::http::Request;
    use ironclaw_filesystem::InMemoryBackend;
    use tower::ServiceExt;

    const TENANT: &str = "tenant:acme";

    /// Filesystem store on a root-mounted in-memory backend (so the
    /// `/tenants/local_users` root is reachable), plus the admin route mount
    /// over the SAME store `Arc` — driving the routes mutates what
    /// `resolve_token`/`list_users` read, exactly like production.
    fn mount() -> (ProtectedRouteMount, Arc<dyn LocalUserDirectoryStore>) {
        let backend = Arc::new(InMemoryBackend::new()) as Arc<dyn RootFilesystem>;
        let store: Arc<dyn LocalUserDirectoryStore> =
            Arc::new(FilesystemLocalUserDirectoryStore::new(backend).expect("store constructs"));
        let config =
            LocalUserAdminRouteConfig::new(TenantId::new(TENANT).expect("tenant"), store.clone());
        (local_user_admin_route_mount(config), store)
    }

    fn request(method: &str, uri: &str, tenant: &str, user: &str, role: UserRole) -> Request<Body> {
        let mut builder = Request::builder()
            .method(method)
            .uri(uri)
            .header("content-type", "application/json")
            .extension(WebUiAuthenticatedCaller {
                tenant_id: TenantId::new(tenant).expect("tenant"),
                user_id: UserId::new(user).expect("user"),
                agent_id: None,
                project_id: None,
                // Admin gating comes from the role, not operator_webui_config.
                operator_webui_config: false,
                role,
            });
        if method == "GET" || method == "DELETE" {
            builder = builder.header("content-length", "0");
        }
        let body = match method {
            "POST" => "{\"user_id\":\"user:bob\",\"role\":\"member\"}",
            "PUT" => "{\"role\":\"admin\"}",
            _ => "",
        };
        builder.body(Body::from(body)).expect("request builds")
    }

    async fn body_json(response: axum::response::Response) -> serde_json::Value {
        let bytes = axum::body::to_bytes(response.into_body(), 64 * 1024)
            .await
            .expect("body");
        serde_json::from_slice(&bytes).expect("json")
    }

    #[tokio::test]
    async fn admin_create_list_set_role_delete_round_trip() {
        let (mount, _store) = mount();

        // create → mints a token, echoes the user + role.
        let created = mount
            .router
            .clone()
            .oneshot(request(
                "POST",
                "/api/webchat/v2/admin/users",
                TENANT,
                "user:director",
                UserRole::Admin,
            ))
            .await
            .expect("create responds");
        assert_eq!(created.status(), StatusCode::OK);
        let created = body_json(created).await;
        assert_eq!(created["user_id"], "user:bob");
        assert_eq!(created["role"], "member");
        assert!(
            created["token"].as_str().is_some_and(|t| !t.is_empty()),
            "create must return a non-empty bearer token"
        );

        // list → the created user appears.
        let listed = mount
            .router
            .clone()
            .oneshot(request(
                "GET",
                "/api/webchat/v2/admin/users",
                TENANT,
                "user:director",
                UserRole::Admin,
            ))
            .await
            .expect("list responds");
        assert_eq!(listed.status(), StatusCode::OK);
        let listed = body_json(listed).await;
        assert_eq!(listed["users"][0]["user_id"], "user:bob");

        // set-role → bumps user:bob to admin.
        let set_role = mount
            .router
            .clone()
            .oneshot(request(
                "PUT",
                "/api/webchat/v2/admin/users/user:bob/role",
                TENANT,
                "user:director",
                UserRole::Admin,
            ))
            .await
            .expect("set-role responds");
        assert_eq!(set_role.status(), StatusCode::OK);
        let set_role = body_json(set_role).await;
        assert_eq!(set_role["role"], "admin");

        // delete → 204. The caller must strictly outrank the target: user:bob
        // was just promoted to Admin, so an Owner (not a peer Admin) deletes it.
        let deleted = mount
            .router
            .oneshot(request(
                "DELETE",
                "/api/webchat/v2/admin/users/user:bob",
                TENANT,
                "user:director",
                UserRole::Owner,
            ))
            .await
            .expect("delete responds");
        assert_eq!(deleted.status(), StatusCode::NO_CONTENT);
    }

    #[tokio::test]
    async fn non_admin_is_forbidden_and_wrong_tenant_is_not_found() {
        let (mount, store) = mount();

        let member = mount
            .router
            .clone()
            .oneshot(request(
                "POST",
                "/api/webchat/v2/admin/users",
                TENANT,
                "user:bob",
                UserRole::Member,
            ))
            .await
            .expect("member responds");
        assert_eq!(member.status(), StatusCode::FORBIDDEN);

        let other_tenant = mount
            .router
            .oneshot(request(
                "POST",
                "/api/webchat/v2/admin/users",
                "tenant:other",
                "user:director",
                UserRole::Admin,
            ))
            .await
            .expect("cross-tenant responds");
        assert_eq!(other_tenant.status(), StatusCode::NOT_FOUND);

        // Neither a forbidden nor a wrong-tenant call may write.
        let users = store
            .list_users(&TenantId::new(TENANT).expect("tenant"))
            .await
            .expect("list");
        assert!(users.is_empty(), "no user may have been created");
    }

    #[tokio::test]
    async fn delete_missing_user_is_not_found() {
        // r5272-3: deleting a user that was never created is a 404, not a 503.
        let (mount, _store) = mount();
        let response = mount
            .router
            .oneshot(request(
                "DELETE",
                "/api/webchat/v2/admin/users/user:ghost",
                TENANT,
                "user:director",
                UserRole::Admin,
            ))
            .await
            .expect("delete responds");
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    /// Seed a user directly through the store (the admin handlers mint their
    /// own tokens; the delete-guard matrix only needs the user records to
    /// exist with the right roles).
    async fn seed_user(store: &Arc<dyn LocalUserDirectoryStore>, user: &str, role: UserRole) {
        let tenant = TenantId::new(TENANT).expect("tenant");
        let user_id = UserId::new(user).expect("user");
        store
            .create_user(&tenant, &user_id, role, &hash_user_token(user))
            .await
            .expect("seed user");
    }

    async fn delete(
        mount: &ProtectedRouteMount,
        target: &str,
        caller: &str,
        caller_role: UserRole,
    ) -> StatusCode {
        mount
            .router
            .clone()
            .oneshot(request(
                "DELETE",
                &format!("/api/webchat/v2/admin/users/{target}"),
                TENANT,
                caller,
                caller_role,
            ))
            .await
            .expect("delete responds")
            .status()
    }

    async fn assert_present(store: &Arc<dyn LocalUserDirectoryStore>, user: &str) {
        let tenant = TenantId::new(TENANT).expect("tenant");
        let user_id = UserId::new(user).expect("user");
        assert!(
            store
                .resolve_user(&tenant, &user_id)
                .await
                .expect("resolve")
                .is_some(),
            "{user} must still exist after a refused delete"
        );
    }

    #[tokio::test]
    async fn delete_user_handler_enforces_step12_guard_matrix() {
        // The full step-12 deletion matrix, driven through the handler (not
        // `UserRole::outranks` alone) per .claude/rules/testing.md — the guards
        // gate the destructive `store.delete_user` side effect.
        let (mount, store) = mount();
        seed_user(&store, "user:director", UserRole::Owner).await; // sole owner
        seed_user(&store, "user:officer", UserRole::Admin).await;
        seed_user(&store, "user:peeradmin", UserRole::Admin).await;
        seed_user(&store, "user:member", UserRole::Member).await;

        // officer(Admin) -> director(Owner): rank guard → 403.
        assert_eq!(
            delete(&mount, "user:director", "user:officer", UserRole::Admin).await,
            StatusCode::FORBIDDEN,
            "admin must not delete the owner"
        );
        assert_present(&store, "user:director").await;

        // officer(Admin) -> peeradmin(Admin): rank guard (no peer-admin) → 403.
        assert_eq!(
            delete(&mount, "user:peeradmin", "user:officer", UserRole::Admin).await,
            StatusCode::FORBIDDEN,
            "admin must not delete a peer admin"
        );
        assert_present(&store, "user:peeradmin").await;

        // officer(Admin) -> self: self-delete guard → 403.
        assert_eq!(
            delete(&mount, "user:officer", "user:officer", UserRole::Admin).await,
            StatusCode::FORBIDDEN,
            "admin must not delete themselves"
        );
        assert_present(&store, "user:officer").await;

        // director(Owner) -> self: self-delete guard (sole owner) → 403.
        assert_eq!(
            delete(&mount, "user:director", "user:director", UserRole::Owner).await,
            StatusCode::FORBIDDEN,
            "the owner must not delete themselves"
        );
        assert_present(&store, "user:director").await;

        // director(Owner) -> officer(Admin): strictly outranks → 204, gone.
        assert_eq!(
            delete(&mount, "user:officer", "user:director", UserRole::Owner).await,
            StatusCode::NO_CONTENT,
            "the owner may delete an admin"
        );
        let tenant = TenantId::new(TENANT).expect("tenant");
        assert!(
            store
                .resolve_user(&tenant, &UserId::new("user:officer").expect("user"))
                .await
                .expect("resolve")
                .is_none(),
            "officer must be gone after the owner deletes them"
        );
    }

    #[tokio::test]
    async fn delete_missing_target_is_not_found_with_admin_caller() {
        // The 404-for-missing-target contract holds after the new guards: a
        // privileged caller deleting a non-existent (non-self) user still gets
        // NotFound, sourced from `resolve_user` returning `None` rather than
        // from the inner `delete_user`.
        let (mount, store) = mount();
        seed_user(&store, "user:director", UserRole::Owner).await;
        assert_eq!(
            delete(&mount, "user:ghost", "user:director", UserRole::Owner).await,
            StatusCode::NOT_FOUND,
            "deleting a missing user is a 404"
        );
    }

    #[tokio::test]
    async fn resolve_token_rejects_rotated_bearer() {
        // r5272-1: re-creating a user mints a new token; the OLD bearer's hash
        // no longer matches the user record, so it stops resolving even though a
        // stale token-index leaf may still point at the user.
        let backend = Arc::new(InMemoryBackend::new()) as Arc<dyn RootFilesystem>;
        let store = FilesystemLocalUserDirectoryStore::new(backend).expect("store constructs");
        let tenant = TenantId::new(TENANT).expect("tenant");
        let user_id = UserId::new("user:bob").expect("user");

        let old_token = "old-bearer-token";
        let old_hash = hash_user_token(old_token);
        store
            .create_user(&tenant, &user_id, UserRole::Member, &old_hash)
            .await
            .expect("create with old token");
        assert!(
            store
                .resolve_token(&tenant, &old_hash)
                .await
                .expect("resolve")
                .is_some(),
            "the freshly-minted token resolves"
        );

        // Rotate: re-create the same user with a new token. This overwrites the
        // user record's `token_hash` and writes a new token-index leaf; the OLD
        // leaf is left dangling.
        let new_token = "new-bearer-token";
        let new_hash = hash_user_token(new_token);
        store
            .create_user(&tenant, &user_id, UserRole::Member, &new_hash)
            .await
            .expect("re-create with new token");

        assert!(
            store
                .resolve_token(&tenant, &new_hash)
                .await
                .expect("resolve")
                .is_some(),
            "the current (new) token resolves"
        );
        assert!(
            store
                .resolve_token(&tenant, &old_hash)
                .await
                .expect("resolve")
                .is_none(),
            "the rotated (old) bearer must NOT resolve via the stale index leaf"
        );
    }
}
