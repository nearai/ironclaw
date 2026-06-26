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
        if let Some(user) = self.get_json::<StoredUser>(&path).await? {
            let token_path = self.token_path(tenant_id, &user.token_hash)?;
            self.filesystem
                .delete(&token_path)
                .await
                .map_err(|error| LocalUserDirectoryError::Backend(error.to_string()))?;
        }
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
    let local_runtime = runtime.services().local_runtime.as_ref()?;
    let store = FilesystemLocalUserDirectoryStore::new(Arc::clone(
        &local_runtime.extension_filesystem,
    ) as Arc<dyn RootFilesystem>)
    .ok()?;
    Some(Arc::new(store))
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
