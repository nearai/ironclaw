//! HTTP boundary adapters for admin user lifecycle and managed-user secrets.

use axum::Json;
use axum::extract::{Extension, Path, Query, State};
use ironclaw_host_api::{SecretHandle, UserId};
use ironclaw_product_workflow::{
    RebornAdminCreateManagedUserRequest, RebornAdminCreateUserRequest, RebornAdminPutSecretRequest,
    RebornAdminSecretDeletedResponse, RebornAdminSecretResponse, RebornAdminSetRoleRequest,
    RebornAdminSetStatusRequest, RebornAdminUpdateUserRequest, RebornAdminUserCreatedResponse,
    RebornAdminUserDeletedResponse, RebornAdminUserListQuery, RebornAdminUserListResponse,
    RebornAdminUserResponse, RebornAdminUserSecretsListResponse, RebornServicesError,
    WebUiAuthenticatedCaller, WebUiInboundValidationCode, WebUiInboundValidationError,
};

use crate::webui_v2::{WebUiV2HttpError, WebUiV2State};

fn parse_user_id(raw: String) -> Result<UserId, WebUiV2HttpError> {
    UserId::new(raw).map_err(|_| {
        WebUiV2HttpError::from(RebornServicesError::from(WebUiInboundValidationError::new(
            "user_id",
            WebUiInboundValidationCode::InvalidId,
        )))
    })
}

fn parse_secret_handle(raw: String) -> Result<SecretHandle, WebUiV2HttpError> {
    SecretHandle::new(raw).map_err(|_| {
        WebUiV2HttpError::from(RebornServicesError::from(WebUiInboundValidationError::new(
            "handle",
            WebUiInboundValidationCode::InvalidId,
        )))
    })
}

pub async fn admin_list_users(
    State(state): State<WebUiV2State>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
    Query(query): Query<RebornAdminUserListQuery>,
) -> Result<Json<RebornAdminUserListResponse>, WebUiV2HttpError> {
    Ok(Json(
        state.services().list_admin_users(caller, query).await?,
    ))
}

pub async fn admin_create_user(
    State(state): State<WebUiV2State>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
    Json(body): Json<RebornAdminCreateUserRequest>,
) -> Result<Json<RebornAdminUserCreatedResponse>, WebUiV2HttpError> {
    Ok(Json(
        state
            .services()
            .create_admin_user(caller, body.into())
            .await?,
    ))
}

pub async fn admin_create_managed_user(
    State(state): State<WebUiV2State>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
    Json(body): Json<RebornAdminCreateManagedUserRequest>,
) -> Result<Json<RebornAdminUserCreatedResponse>, WebUiV2HttpError> {
    Ok(Json(
        state
            .services()
            .create_admin_user(caller, body.into())
            .await?,
    ))
}

pub async fn admin_get_user(
    State(state): State<WebUiV2State>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
    Path(user_id): Path<String>,
) -> Result<Json<RebornAdminUserResponse>, WebUiV2HttpError> {
    Ok(Json(
        state
            .services()
            .get_admin_user(caller, parse_user_id(user_id)?)
            .await?,
    ))
}

pub async fn admin_update_user(
    State(state): State<WebUiV2State>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
    Path(user_id): Path<String>,
    Json(body): Json<RebornAdminUpdateUserRequest>,
) -> Result<Json<RebornAdminUserResponse>, WebUiV2HttpError> {
    Ok(Json(
        state
            .services()
            .update_admin_user(caller, parse_user_id(user_id)?, body)
            .await?,
    ))
}

pub async fn admin_delete_user(
    State(state): State<WebUiV2State>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
    Path(user_id): Path<String>,
) -> Result<Json<RebornAdminUserDeletedResponse>, WebUiV2HttpError> {
    Ok(Json(
        state
            .services()
            .delete_admin_user(caller, parse_user_id(user_id)?)
            .await?,
    ))
}

pub async fn admin_set_user_status(
    State(state): State<WebUiV2State>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
    Path(user_id): Path<String>,
    Json(body): Json<RebornAdminSetStatusRequest>,
) -> Result<Json<RebornAdminUserResponse>, WebUiV2HttpError> {
    Ok(Json(
        state
            .services()
            .set_admin_user_status(caller, parse_user_id(user_id)?, body)
            .await?,
    ))
}

pub async fn admin_set_user_role(
    State(state): State<WebUiV2State>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
    Path(user_id): Path<String>,
    Json(body): Json<RebornAdminSetRoleRequest>,
) -> Result<Json<RebornAdminUserResponse>, WebUiV2HttpError> {
    Ok(Json(
        state
            .services()
            .set_admin_user_role(caller, parse_user_id(user_id)?, body)
            .await?,
    ))
}

pub async fn admin_list_user_secrets(
    State(state): State<WebUiV2State>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
    Path(user_id): Path<String>,
) -> Result<Json<RebornAdminUserSecretsListResponse>, WebUiV2HttpError> {
    Ok(Json(
        state
            .services()
            .list_admin_user_secrets(caller, parse_user_id(user_id)?)
            .await?,
    ))
}

pub async fn admin_put_user_secret(
    State(state): State<WebUiV2State>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
    Path((user_id, handle)): Path<(String, String)>,
    Json(body): Json<RebornAdminPutSecretRequest>,
) -> Result<Json<RebornAdminSecretResponse>, WebUiV2HttpError> {
    Ok(Json(
        state
            .services()
            .put_admin_user_secret(
                caller,
                parse_user_id(user_id)?,
                parse_secret_handle(handle)?,
                body,
            )
            .await?,
    ))
}

pub async fn admin_delete_user_secret(
    State(state): State<WebUiV2State>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
    Path((user_id, handle)): Path<(String, String)>,
) -> Result<Json<RebornAdminSecretDeletedResponse>, WebUiV2HttpError> {
    Ok(Json(
        state
            .services()
            .delete_admin_user_secret(
                caller,
                parse_user_id(user_id)?,
                parse_secret_handle(handle)?,
            )
            .await?,
    ))
}
