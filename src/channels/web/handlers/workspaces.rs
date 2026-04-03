//! Shared workspace API handlers and scope resolution helpers.

use std::num::NonZeroUsize;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use serde::Deserialize;
use uuid::Uuid;

use crate::channels::web::auth::{AuthenticatedUser, UserIdentity};
use crate::channels::web::permissions::{Permission, Role, superadmin_workspace_role};
use crate::channels::web::server::GatewayState;
use crate::channels::web::types::*;
use crate::db::{Database, WorkspaceMembership, WorkspaceRecord};

pub const WORKSPACE_SCOPE_PREFIX: &str = "workspace:";
const WORKSPACE_ROLE_OWNER: &str = "owner";
const WORKSPACE_ROLE_ADMIN: &str = "admin";
const WORKSPACE_ROLE_MEMBER: &str = "member";
const WORKSPACE_ROLE_VIEWER: &str = "viewer";

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct WorkspaceMembershipCacheKey {
    workspace_id: Uuid,
    user_id: String,
}

#[derive(Debug, Clone)]
struct WorkspaceMembershipCacheEntry {
    role: Option<String>,
    inserted_at: Instant,
}

struct WorkspaceMembershipCache {
    entries: Mutex<lru::LruCache<WorkspaceMembershipCacheKey, WorkspaceMembershipCacheEntry>>,
    ttl: Duration,
}

impl WorkspaceMembershipCache {
    // SAFETY: 4096 is non-zero, so the unwrap in `new()` is infallible.
    const MAX_ENTRIES: NonZeroUsize = match NonZeroUsize::new(4096) {
        Some(value) => value,
        None => unreachable!(),
    };

    fn new() -> Self {
        let ttl_secs = std::env::var("GATEWAY_WORKSPACE_MEMBERSHIP_CACHE_TTL_SECS")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(60);
        Self {
            entries: Mutex::new(lru::LruCache::new(Self::MAX_ENTRIES)),
            ttl: Duration::from_secs(ttl_secs),
        }
    }

    fn get(&self, workspace_id: Uuid, user_id: &str) -> Option<Option<String>> {
        let key = WorkspaceMembershipCacheKey {
            workspace_id,
            user_id: user_id.to_string(),
        };
        let mut entries = self.lock_entries();
        if let Some(entry) = entries.get(&key)
            && entry.inserted_at.elapsed() < self.ttl
        {
            return Some(entry.role.clone());
        }
        entries.pop(&key);
        None
    }

    fn insert(&self, workspace_id: Uuid, user_id: &str, role: Option<String>) {
        self.lock_entries().put(
            WorkspaceMembershipCacheKey {
                workspace_id,
                user_id: user_id.to_string(),
            },
            WorkspaceMembershipCacheEntry {
                role,
                inserted_at: Instant::now(),
            },
        );
    }

    fn invalidate(&self, workspace_id: Uuid, user_id: &str) {
        self.lock_entries().pop(&WorkspaceMembershipCacheKey {
            workspace_id,
            user_id: user_id.to_string(),
        });
    }

    fn lock_entries(
        &self,
    ) -> std::sync::MutexGuard<
        '_,
        lru::LruCache<WorkspaceMembershipCacheKey, WorkspaceMembershipCacheEntry>,
    > {
        match self.entries.lock() {
            Ok(entries) => entries,
            Err(poisoned) => {
                tracing::warn!("WorkspaceMembershipCache lock poisoned; recovering");
                poisoned.into_inner()
            }
        }
    }
}

fn workspace_membership_cache() -> &'static WorkspaceMembershipCache {
    static CACHE: OnceLock<WorkspaceMembershipCache> = OnceLock::new();
    CACHE.get_or_init(WorkspaceMembershipCache::new)
}

pub(crate) fn invalidate_workspace_membership_cache(workspace_id: Uuid, user_id: &str) {
    workspace_membership_cache().invalidate(workspace_id, user_id);
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct WorkspaceQuery {
    pub workspace: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ResolvedWorkspace {
    pub workspace: WorkspaceRecord,
    pub role: String,
}

fn workspace_role_error() -> String {
    format!(
        "role must be one of '{WORKSPACE_ROLE_OWNER}', '{WORKSPACE_ROLE_ADMIN}', '{WORKSPACE_ROLE_MEMBER}', or '{WORKSPACE_ROLE_VIEWER}'"
    )
}

fn is_valid_workspace_role(role: &str) -> bool {
    Role::parse(role).is_ok()
}

fn workspace_role_is_owner(role: &str) -> bool {
    role == WORKSPACE_ROLE_OWNER
}

fn validate_workspace_name(name: &str) -> Result<(), (StatusCode, String)> {
    if name.is_empty() {
        Err((
            StatusCode::BAD_REQUEST,
            "Workspace name is required".to_string(),
        ))
    } else {
        Ok(())
    }
}

fn validate_workspace_slug(slug: &str) -> Result<(), (StatusCode, String)> {
    let bytes = slug.as_bytes();
    if !(3..=64).contains(&bytes.len())
        || !bytes[0].is_ascii_lowercase() && !bytes[0].is_ascii_digit()
        || !bytes[bytes.len() - 1].is_ascii_lowercase() && !bytes[bytes.len() - 1].is_ascii_digit()
        || !bytes
            .iter()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || *b == b'-')
    {
        return Err((
            StatusCode::BAD_REQUEST,
            "Workspace slug must match ^[a-z0-9][a-z0-9-]{1,62}[a-z0-9]$".to_string(),
        ));
    }
    Ok(())
}

fn validate_workspace_settings(settings: &serde_json::Value) -> Result<(), (StatusCode, String)> {
    if settings.is_object() {
        Ok(())
    } else {
        Err((
            StatusCode::BAD_REQUEST,
            "Workspace settings must be a JSON object".to_string(),
        ))
    }
}

pub fn workspace_scope_user_id(workspace_id: Uuid) -> String {
    format!("{WORKSPACE_SCOPE_PREFIX}{workspace_id}")
}

async fn resolve_workspace_scope_for_workspace(
    store: &Arc<dyn Database>,
    user: &UserIdentity,
    workspace: WorkspaceRecord,
) -> Result<ResolvedWorkspace, (StatusCode, String)> {
    if workspace.status == "archived" {
        return Err((StatusCode::GONE, "Workspace is archived".to_string()));
    }

    if user.is_superadmin {
        return Ok(ResolvedWorkspace {
            workspace,
            role: superadmin_workspace_role().as_str().to_string(),
        });
    }

    let membership_role = match workspace_membership_cache().get(workspace.id, &user.user_id) {
        Some(cached) => cached,
        None => {
            let fetched = store
                .get_member_role(workspace.id, &user.user_id)
                .await
                .map_err(internal_db_error)?;
            workspace_membership_cache().insert(workspace.id, &user.user_id, fetched.clone());
            fetched
        }
    };
    let role = match membership_role {
        Some(role) => role,
        None => return Err((StatusCode::FORBIDDEN, "Workspace access denied".to_string())),
    };
    Role::parse(&role)?;

    Ok(ResolvedWorkspace { workspace, role })
}

pub async fn resolve_workspace_scope(
    store: &Arc<dyn Database>,
    user: &UserIdentity,
    workspace_slug: Option<&str>,
) -> Result<Option<ResolvedWorkspace>, (StatusCode, String)> {
    let Some(slug) = workspace_slug else {
        return Ok(None);
    };

    let workspace = store
        .get_workspace_by_slug(slug)
        .await
        .map_err(internal_db_error)?
        .ok_or((StatusCode::NOT_FOUND, "Workspace not found".to_string()))?;

    resolve_workspace_scope_for_workspace(store, user, workspace)
        .await
        .map(Some)
}

pub async fn resolve_workspace_scope_by_id(
    store: &Arc<dyn Database>,
    user: &UserIdentity,
    workspace_id: Uuid,
) -> Result<ResolvedWorkspace, (StatusCode, String)> {
    let workspace = store
        .get_workspace(workspace_id)
        .await
        .map_err(internal_db_error)?;
    let workspace = workspace.ok_or((StatusCode::NOT_FOUND, "Workspace not found".to_string()))?;
    resolve_workspace_scope_for_workspace(store, user, workspace).await
}

pub fn require_workspace_manager(role: &str) -> Result<(), (StatusCode, String)> {
    if Role::parse(role)?.has_permission(Permission::WorkspaceManageMembers) {
        Ok(())
    } else {
        Err((
            StatusCode::FORBIDDEN,
            "Workspace admin or owner role required".to_string(),
        ))
    }
}

pub fn require_workspace_owner(role: &str) -> Result<(), (StatusCode, String)> {
    if Role::parse(role)? >= Role::Owner {
        Ok(())
    } else {
        Err((
            StatusCode::FORBIDDEN,
            "Workspace owner role required".to_string(),
        ))
    }
}

pub async fn resolve_requested_workspace_id(
    state: &GatewayState,
    user: &UserIdentity,
    workspace_slug: Option<&str>,
) -> Result<Option<Uuid>, (StatusCode, String)> {
    let Some(store) = state.store.as_ref() else {
        if workspace_slug.is_some() {
            return Err((
                StatusCode::SERVICE_UNAVAILABLE,
                "Database not available".to_string(),
            ));
        }
        return Ok(None);
    };
    Ok(resolve_workspace_scope(store, user, workspace_slug)
        .await?
        .map(|scope| scope.workspace.id))
}

fn workspace_info_from_membership(membership: WorkspaceMembership) -> WorkspaceInfo {
    WorkspaceInfo {
        id: membership.workspace.id,
        name: membership.workspace.name,
        slug: membership.workspace.slug,
        description: membership.workspace.description,
        status: membership.workspace.status,
        role: membership.role,
        created_at: membership.workspace.created_at.to_rfc3339(),
        updated_at: membership.workspace.updated_at.to_rfc3339(),
        created_by: membership.workspace.created_by,
        settings: membership.workspace.settings,
    }
}

fn workspace_info(workspace: WorkspaceRecord, role: String) -> WorkspaceInfo {
    WorkspaceInfo {
        id: workspace.id,
        name: workspace.name,
        slug: workspace.slug,
        description: workspace.description,
        status: workspace.status,
        role,
        created_at: workspace.created_at.to_rfc3339(),
        updated_at: workspace.updated_at.to_rfc3339(),
        created_by: workspace.created_by,
        settings: workspace.settings,
    }
}

fn internal_db_error(e: impl std::fmt::Display) -> (StatusCode, String) {
    tracing::error!("Workspace database error: {e}");
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        "Internal database error".to_string(),
    )
}

pub async fn workspaces_list_handler(
    State(state): State<Arc<GatewayState>>,
    AuthenticatedUser(user): AuthenticatedUser,
) -> Result<Json<WorkspaceListResponse>, (StatusCode, String)> {
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;

    let workspaces = if user.is_superadmin {
        store
            .list_all_workspaces()
            .await
            .map_err(internal_db_error)?
            .into_iter()
            .map(|workspace| {
                workspace_info(workspace, superadmin_workspace_role().as_str().to_string())
            })
            .collect()
    } else {
        store
            .list_workspaces_for_user(&user.user_id)
            .await
            .map_err(internal_db_error)?
            .into_iter()
            .map(workspace_info_from_membership)
            .collect()
    };

    Ok(Json(WorkspaceListResponse { workspaces }))
}

pub async fn workspaces_create_handler(
    State(state): State<Arc<GatewayState>>,
    AuthenticatedUser(user): AuthenticatedUser,
    Json(body): Json<WorkspaceCreateRequest>,
) -> Result<Json<WorkspaceInfo>, (StatusCode, String)> {
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;

    let name = body.name.trim();
    let slug = body.slug.trim();
    validate_workspace_name(name)?;
    validate_workspace_slug(slug)?;
    validate_workspace_settings(&body.settings)?;

    let workspace = store
        .create_workspace(name, slug, &body.description, &user.user_id, &body.settings)
        .await
        .map_err(internal_db_error)?;

    Ok(Json(workspace_info(
        workspace,
        WORKSPACE_ROLE_OWNER.to_string(),
    )))
}

pub async fn workspaces_detail_handler(
    State(state): State<Arc<GatewayState>>,
    AuthenticatedUser(user): AuthenticatedUser,
    Path(slug): Path<String>,
) -> Result<Json<WorkspaceInfo>, (StatusCode, String)> {
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;
    let resolved = resolve_workspace_scope(store, &user, Some(&slug)).await?;
    let resolved = resolved.ok_or((StatusCode::NOT_FOUND, "Workspace not found".to_string()))?;
    Ok(Json(workspace_info(resolved.workspace, resolved.role)))
}

pub async fn workspaces_update_handler(
    State(state): State<Arc<GatewayState>>,
    AuthenticatedUser(user): AuthenticatedUser,
    Path(slug): Path<String>,
    Json(body): Json<WorkspaceUpdateRequest>,
) -> Result<Json<WorkspaceInfo>, (StatusCode, String)> {
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;
    let resolved = resolve_workspace_scope(store, &user, Some(&slug)).await?;
    let resolved = resolved.ok_or((StatusCode::NOT_FOUND, "Workspace not found".to_string()))?;
    require_workspace_manager(&resolved.role)?;
    validate_workspace_name(body.name.trim())?;
    validate_workspace_settings(&body.settings)?;

    let updated = store
        .update_workspace(
            resolved.workspace.id,
            body.name.trim(),
            &body.description,
            &body.settings,
        )
        .await
        .map_err(internal_db_error)?
        .ok_or((StatusCode::NOT_FOUND, "Workspace not found".to_string()))?;

    Ok(Json(workspace_info(updated, resolved.role)))
}

pub async fn workspaces_archive_handler(
    State(state): State<Arc<GatewayState>>,
    AuthenticatedUser(user): AuthenticatedUser,
    Path(slug): Path<String>,
) -> Result<StatusCode, (StatusCode, String)> {
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;
    let resolved = resolve_workspace_scope(store, &user, Some(&slug)).await?;
    let resolved = resolved.ok_or((StatusCode::NOT_FOUND, "Workspace not found".to_string()))?;
    require_workspace_owner(&resolved.role)?;

    let archived = store
        .archive_workspace(resolved.workspace.id)
        .await
        .map_err(internal_db_error)?;
    if archived {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err((StatusCode::NOT_FOUND, "Workspace not found".to_string()))
    }
}

pub async fn workspace_members_list_handler(
    State(state): State<Arc<GatewayState>>,
    AuthenticatedUser(user): AuthenticatedUser,
    Path(slug): Path<String>,
) -> Result<Json<WorkspaceMembersResponse>, (StatusCode, String)> {
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;
    let resolved = resolve_workspace_scope(store, &user, Some(&slug)).await?;
    let resolved = resolved.ok_or((StatusCode::NOT_FOUND, "Workspace not found".to_string()))?;

    let members = store
        .list_workspace_members(resolved.workspace.id)
        .await
        .map_err(internal_db_error)?
        .into_iter()
        .map(|(user, membership)| WorkspaceMemberInfo {
            user_id: user.id,
            email: user.email,
            display_name: user.display_name,
            status: user.status,
            role: membership.role,
            joined_at: membership.joined_at.to_rfc3339(),
            invited_by: membership.invited_by,
        })
        .collect();

    Ok(Json(WorkspaceMembersResponse { members }))
}

pub async fn workspace_members_upsert_handler(
    State(state): State<Arc<GatewayState>>,
    AuthenticatedUser(user): AuthenticatedUser,
    Path((slug, member_user_id)): Path<(String, String)>,
    Json(body): Json<WorkspaceMemberWriteRequest>,
) -> Result<StatusCode, (StatusCode, String)> {
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;
    let resolved = resolve_workspace_scope(store, &user, Some(&slug)).await?;
    let resolved = resolved.ok_or((StatusCode::NOT_FOUND, "Workspace not found".to_string()))?;
    require_workspace_manager(&resolved.role)?;
    let role = body.role.trim();
    if !is_valid_workspace_role(role) {
        return Err((StatusCode::BAD_REQUEST, workspace_role_error()));
    }

    let existing_role = store
        .get_member_role(resolved.workspace.id, &member_user_id)
        .await
        .map_err(internal_db_error)?;
    if workspace_role_is_owner(role)
        || existing_role
            .as_deref()
            .is_some_and(workspace_role_is_owner)
    {
        require_workspace_owner(&resolved.role)?;
    }

    if existing_role
        .as_deref()
        .is_some_and(workspace_role_is_owner)
        && !workspace_role_is_owner(role)
        && store
            .is_last_workspace_owner(resolved.workspace.id, &member_user_id)
            .await
            .map_err(internal_db_error)?
    {
        return Err((
            StatusCode::CONFLICT,
            "Cannot remove the last workspace owner".to_string(),
        ));
    }

    let target_user_exists = store
        .get_user(&member_user_id)
        .await
        .map_err(internal_db_error)?
        .is_some();
    if !target_user_exists {
        return Err((StatusCode::NOT_FOUND, "User not found".to_string()));
    }

    if workspace_role_is_owner(role) && member_user_id != user.user_id {
        if existing_role
            .as_deref()
            .is_some_and(workspace_role_is_owner)
        {
            return Ok(StatusCode::NO_CONTENT);
        }

        let owners: Vec<String> = store
            .list_workspace_members(resolved.workspace.id)
            .await
            .map_err(internal_db_error)?
            .into_iter()
            .filter(|(_, membership)| workspace_role_is_owner(&membership.role))
            .map(|(owner_user, _)| owner_user.id)
            .collect();
        let [current_owner_id] = owners.as_slice() else {
            return Err((
                StatusCode::CONFLICT,
                "Workspace ownership transfer requires exactly one current owner".to_string(),
            ));
        };

        let transferred = store
            .transfer_workspace_ownership(
                resolved.workspace.id,
                current_owner_id,
                &member_user_id,
                Some(&user.user_id),
            )
            .await
            .map_err(internal_db_error)?;
        if !transferred {
            return Err((
                StatusCode::CONFLICT,
                "Workspace ownership transfer failed".to_string(),
            ));
        }

        invalidate_workspace_membership_cache(resolved.workspace.id, current_owner_id);
        invalidate_workspace_membership_cache(resolved.workspace.id, &member_user_id);
        return Ok(StatusCode::NO_CONTENT);
    }

    store
        .add_workspace_member(
            resolved.workspace.id,
            &member_user_id,
            role,
            Some(&user.user_id),
        )
        .await
        .map_err(internal_db_error)?;
    invalidate_workspace_membership_cache(resolved.workspace.id, &member_user_id);

    Ok(StatusCode::NO_CONTENT)
}

pub async fn workspace_members_delete_handler(
    State(state): State<Arc<GatewayState>>,
    AuthenticatedUser(user): AuthenticatedUser,
    Path((slug, member_user_id)): Path<(String, String)>,
) -> Result<StatusCode, (StatusCode, String)> {
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;
    let resolved = resolve_workspace_scope(store, &user, Some(&slug)).await?;
    let resolved = resolved.ok_or((StatusCode::NOT_FOUND, "Workspace not found".to_string()))?;
    require_workspace_manager(&resolved.role)?;

    let member_role = store
        .get_member_role(resolved.workspace.id, &member_user_id)
        .await
        .map_err(internal_db_error)?
        .ok_or((
            StatusCode::NOT_FOUND,
            "Workspace member not found".to_string(),
        ))?;
    if workspace_role_is_owner(&member_role) {
        require_workspace_owner(&resolved.role)?;
        if store
            .is_last_workspace_owner(resolved.workspace.id, &member_user_id)
            .await
            .map_err(internal_db_error)?
        {
            return Err((
                StatusCode::CONFLICT,
                "Cannot remove the last workspace owner".to_string(),
            ));
        }
    }

    let deleted = store
        .remove_workspace_member(resolved.workspace.id, &member_user_id)
        .await
        .map_err(internal_db_error)?;
    if deleted {
        invalidate_workspace_membership_cache(resolved.workspace.id, &member_user_id);
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err((
            StatusCode::NOT_FOUND,
            "Workspace member not found".to_string(),
        ))
    }
}
