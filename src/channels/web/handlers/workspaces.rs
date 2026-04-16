//! Shared workspace API handlers and scope resolution helpers.

use std::sync::Arc;

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use serde::Deserialize;
use uuid::Uuid;

use crate::channels::web::auth::{AuthenticatedUser, UserIdentity};
use crate::channels::web::server::GatewayState;
use crate::channels::web::types::*;
use crate::db::{Database, WorkspaceMembership, WorkspaceRecord};

pub const WORKSPACE_SCOPE_PREFIX: &str = "workspace:";
const WORKSPACE_ROLE_OWNER: &str = "owner";
const WORKSPACE_ROLE_ADMIN: &str = "admin";
const WORKSPACE_ROLE_MEMBER: &str = "member";
const WORKSPACE_ROLE_VIEWER: &str = "viewer";

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
    matches!(
        role,
        WORKSPACE_ROLE_OWNER | WORKSPACE_ROLE_ADMIN | WORKSPACE_ROLE_MEMBER | WORKSPACE_ROLE_VIEWER
    )
}

fn workspace_role_is_owner(role: &str) -> bool {
    role == WORKSPACE_ROLE_OWNER
}

fn workspace_role_is_manager(role: &str) -> bool {
    matches!(role, WORKSPACE_ROLE_OWNER | WORKSPACE_ROLE_ADMIN)
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
    let len_ok = (3..=64).contains(&bytes.len());
    let first_ok = bytes
        .first()
        .is_some_and(|b| b.is_ascii_lowercase() || b.is_ascii_digit());
    let last_ok = bytes
        .last()
        .is_some_and(|b| b.is_ascii_lowercase() || b.is_ascii_digit());
    let body_ok = bytes
        .iter()
        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || *b == b'-');

    if !len_ok || !first_ok || !last_ok || !body_ok {
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

    if workspace.status == "archived" {
        return Err((StatusCode::GONE, "Workspace is archived".to_string()));
    }

    let role = store
        .get_member_role(workspace.id, &user.user_id)
        .await
        .map_err(internal_db_error)?
        .ok_or((StatusCode::FORBIDDEN, "Workspace access denied".to_string()))?;

    Ok(Some(ResolvedWorkspace { workspace, role }))
}

pub fn require_workspace_manager(role: &str) -> Result<(), (StatusCode, String)> {
    if workspace_role_is_manager(role) {
        Ok(())
    } else {
        Err((
            StatusCode::FORBIDDEN,
            "Workspace admin or owner role required".to_string(),
        ))
    }
}

pub fn require_workspace_owner(role: &str) -> Result<(), (StatusCode, String)> {
    if workspace_role_is_owner(role) {
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

    let workspaces = store
        .list_workspaces_for_user(&user.user_id)
        .await
        .map_err(internal_db_error)?
        .into_iter()
        .map(workspace_info_from_membership)
        .collect();

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

    // Owner-touching operations require the caller to be an owner
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

    let target_user_exists = store
        .get_user(&member_user_id)
        .await
        .map_err(internal_db_error)?
        .is_some();
    if !target_user_exists {
        return Err((StatusCode::NOT_FOUND, "User not found".to_string()));
    }

    // Atomic: check last-owner guard + upsert in one transaction
    store
        .update_member_role_checked(
            resolved.workspace.id,
            &member_user_id,
            role,
            Some(&user.user_id),
        )
        .await
        .map_err(|e| match &e {
            crate::error::DatabaseError::Constraint(_) => {
                (StatusCode::CONFLICT, e.to_string())
            }
            _ => internal_db_error(e),
        })?;

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

    // Removing an owner requires the caller to be an owner
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
    }

    // Atomic: check last-owner guard + delete in one transaction
    let deleted = store
        .remove_workspace_member_checked(resolved.workspace.id, &member_user_id)
        .await
        .map_err(|e| match &e {
            crate::error::DatabaseError::Constraint(_) => {
                (StatusCode::CONFLICT, e.to_string())
            }
            _ => internal_db_error(e),
        })?;
    if deleted {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err((
            StatusCode::NOT_FOUND,
            "Workspace member not found".to_string(),
        ))
    }
}
