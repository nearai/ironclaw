//! WebChat v2 HTTP handlers.
//!
//! Every handler:
//!
//! 1. Receives an authenticated caller as an `Extension<WebUiAuthenticatedCaller>`.
//!    Host composition is responsible for running the bearer-token middleware
//!    that builds that extension; the handler never sees a raw bearer token.
//! 2. Dispatches through [`ProductSurface`]. No direct access to the
//!    dispatcher, `HostRuntime`, run-state, DB stores, or any runtime lane.
//! 3. Maps every error through [`WebUiV2HttpError`] so the wire shape stays
//!    redacted and stable.
//!
//! [`ProductSurface`]: ironclaw_product_workflow::ProductSurface

// arch-exempt: large_file, ProductSurface facade-collapse routes stay in the existing WebUI handler table until the WebUI route split lands, plan #5985

mod run_artifact;
pub use run_artifact::get_run_artifact;

use std::convert::Infallible;
use std::time::Duration;

use axum::Json;
use axum::body::Body;
use axum::extract::{Extension, Path, Query, State};
use axum::http::{HeaderMap, HeaderName, HeaderValue, StatusCode, header};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use futures::SinkExt;
use futures::stream::Stream;
use ironclaw_product_workflow::{
    ADMIN_CONFIGURATION_REPLACE_CAPABILITY, ADMIN_CONFIGURATION_VIEW, ADMIN_USER_CREATE_OPERATION,
    ADMIN_USER_DELETE_CAPABILITY, ADMIN_USER_DELETE_SECRET_OPERATION,
    ADMIN_USER_PUT_SECRET_CAPABILITY, ADMIN_USER_SECRETS_VIEW, ADMIN_USER_SET_ROLE_CAPABILITY,
    ADMIN_USER_SET_STATUS_CAPABILITY, ADMIN_USER_UPDATE_CAPABILITY, ADMIN_USER_VIEW,
    ADMIN_USERS_VIEW, ATTACHMENT_READ_OPERATION, AUTOMATION_DELETE_OPERATION,
    AUTOMATION_PAUSE_OPERATION, AUTOMATION_RENAME_OPERATION, AUTOMATION_RESUME_OPERATION,
    AUTOMATIONS_VIEW, CANCEL_RUN_OPERATION, CREATE_THREAD_OPERATION, CodexLoginStart,
    EXTENSION_ACTIVATE_CAPABILITY, EXTENSION_IMPORT_CAPABILITY, EXTENSION_INSTALL_CAPABILITY,
    EXTENSION_REGISTRY_VIEW, EXTENSION_REMOVE_CAPABILITY, EXTENSION_SETUP_SUBMIT_CAPABILITY,
    EXTENSION_SETUP_VIEW, EXTENSIONS_VIEW, FS_LIST_VIEW, FS_MOUNTS_VIEW, FS_READ_OPERATION,
    FS_STAT_VIEW, FsMount, GLOBAL_AUTO_APPROVE_VIEW, IdempotencyKey, LLM_ACTIVE_SET_CAPABILITY,
    LLM_CODEX_LOGIN_OPERATION, LLM_CONFIG_VIEW, LLM_LIST_MODELS_OPERATION,
    LLM_NEARAI_LOGIN_OPERATION, LLM_NEARAI_WALLET_LOGIN_OPERATION, LLM_PROVIDER_DELETE_CAPABILITY,
    LLM_PROVIDER_UPSERT_CAPABILITY, LLM_TEST_CONNECTION_OPERATION, LOGS_VIEW, LifecyclePackageKind,
    LifecyclePackageRef, LlmConfigSnapshot, LlmModelsResult, LlmProbeResult, NearAiLoginStart,
    NearAiWalletLoginResult, OPERATOR_CONFIG_KEY_VIEW, OPERATOR_CONFIG_LIST_VIEW,
    OPERATOR_CONFIG_SET_AUTO_APPROVE_CAPABILITY, OPERATOR_CONFIG_SET_KEY_OPERATION,
    OPERATOR_CONFIG_VALIDATE_VIEW, OPERATOR_DIAGNOSTICS_VIEW, OPERATOR_LOGS_VIEW,
    OPERATOR_SERVICE_LIFECYCLE_OPERATION, OPERATOR_SETUP_RUN_CAPABILITY, OPERATOR_SETUP_VIEW,
    OPERATOR_STATUS_VIEW, OUTBOUND_DELIVERY_TARGETS_VIEW, OUTBOUND_PREFERENCES_SET_CAPABILITY,
    OUTBOUND_PREFERENCES_VIEW, PROJECT_CREATE_OPERATION, PROJECT_DELETE_CAPABILITY,
    PROJECT_FS_LIST_VIEW, PROJECT_FS_READ_OPERATION, PROJECT_FS_STAT_VIEW,
    PROJECT_MEMBER_ADD_CAPABILITY, PROJECT_MEMBER_REMOVE_CAPABILITY,
    PROJECT_MEMBER_UPDATE_CAPABILITY, PROJECT_MEMBERS_VIEW, PROJECT_UPDATE_CAPABILITY,
    PROJECT_VIEW, PROJECTS_VIEW, ProductCapabilityDescriptor, ProductCapabilityInput,
    ProductOutboundEnvelope, ProductSurface, ProductWorkflowError, ProjectFsFile, ProjectionCursor,
    RESOLVE_GATE_OPERATION, RETRY_RUN_OPERATION, RebornAccountLoginLinkResponse,
    RebornAccountTracesResponse, RebornAddMemberRequest, RebornAdminCreateUserRequest,
    RebornAdminDeleteSecretProductRequest, RebornAdminPutSecretProductRequest,
    RebornAdminPutSecretRequest, RebornAdminSecretDeletedResponse, RebornAdminSecretResponse,
    RebornAdminSetRoleProductRequest, RebornAdminSetRoleRequest,
    RebornAdminSetStatusProductRequest, RebornAdminSetStatusRequest,
    RebornAdminUpdateUserProductRequest, RebornAdminUpdateUserRequest,
    RebornAdminUserCreatedResponse, RebornAdminUserDeletedResponse, RebornAdminUserListQuery,
    RebornAdminUserListResponse, RebornAdminUserRequest, RebornAdminUserResponse,
    RebornAdminUserSecretsListResponse, RebornAttachmentRequest, RebornAutomationMutationResponse,
    RebornAutomationRequest, RebornCancelRunResponse, RebornCreateProjectRequest,
    RebornCreateThreadResponse, RebornDeleteProjectRequest, RebornDeleteThreadRequest,
    RebornDeleteThreadResponse, RebornExtensionActionResponse, RebornExtensionListResponse,
    RebornExtensionOnboardingState, RebornExtensionRegistryResponse, RebornFsListRequest,
    RebornFsListResponse, RebornFsMountsRequest, RebornFsMountsResponse, RebornFsReadRequest,
    RebornFsStatRequest, RebornFsStatResponse, RebornGetProjectRequest,
    RebornGlobalAutoApproveRequest, RebornListAutomationsResponse, RebornListMembersRequest,
    RebornListMembersResponse, RebornListProjectsRequest, RebornListProjectsResponse,
    RebornListThreadsResponse, RebornLogQueryRequest, RebornLogQueryResponse,
    RebornOperatorCommandPlaneResponse, RebornOperatorConfigGetResponse,
    RebornOperatorConfigListResponse, RebornOperatorConfigSetProductRequest,
    RebornOperatorConfigSetRequest, RebornOperatorConfigValidateRequest,
    RebornOperatorConfigValidateResponse, RebornOperatorLogsQuery,
    RebornOperatorServiceLifecycleRequest, RebornOperatorSetupResponse,
    RebornOutboundDeliveryTargetListResponse, RebornOutboundPreferencesResponse,
    RebornProjectFsListRequest, RebornProjectFsListResponse, RebornProjectFsReadRequest,
    RebornProjectFsStatRequest, RebornProjectFsStatResponse, RebornProjectMemberInfo,
    RebornProjectResponse, RebornRemoveMemberRequest, RebornRenameAutomationProductRequest,
    RebornResolveGateResponse, RebornRetryRunResponse, RebornServicesError,
    RebornServicesErrorCode, RebornServicesErrorKind, RebornSetOutboundPreferencesRequest,
    RebornSetupExtensionResponse, RebornSkillActionResponse, RebornSkillContentResponse,
    RebornSkillListResponse, RebornSkillSearchResponse, RebornStreamEventsRequest,
    RebornSubmitTurnResponse, RebornTimelineRequest, RebornTimelineResponse,
    RebornTraceCreditsResponse, RebornTraceHoldAuthorizeProductRequest,
    RebornTraceHoldAuthorizeResponse, RebornUpdateMemberRoleRequest, RebornUpdateProjectRequest,
    RebornViewDescriptor, RebornViewQuery, SKILL_AUTO_ACTIVATE_LEARNED_SET_CAPABILITY,
    SKILL_AUTO_ACTIVATE_SET_CAPABILITY, SKILL_CONTENT_VIEW, SKILL_INSTALL_CAPABILITY,
    SKILL_REMOVE_CAPABILITY, SKILL_SEARCH_VIEW, SKILL_UPDATE_CAPABILITY, SKILLS_VIEW,
    SUBMIT_TURN_OPERATION, SetActiveLlmRequest, SettingsToolPermissionState,
    THREAD_DELETE_CAPABILITY, THREADS_VIEW, TIMELINE_VIEW, TRACE_ACCOUNT_LOGIN_LINK_OPERATION,
    TRACE_ACCOUNT_TRACES_VIEW, TRACE_CREDITS_VIEW, TRACE_HOLD_AUTHORIZE_OPERATION,
    UpsertLlmProviderRequest, WebUiAttachmentCapabilities, WebUiAuthenticatedCaller,
    WebUiCancelRunRequest, WebUiCreateThreadRequest, WebUiInboundValidationCode,
    WebUiInboundValidationError, WebUiListAutomationsRequest, WebUiListThreadsRequest,
    WebUiRenameAutomationRequest, WebUiResolveGateRequest, WebUiRetryRunRequest,
    WebUiSendMessageRequest, WebUiSetupExtensionRequest, parse_webui_client_action_id,
    webui_attachment_capabilities,
};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use ironclaw_host_api::{
    ActivityId, Blocked, FailureKind, Resolution, SecretHandle, ThreadId, UserId,
};
use uuid::Uuid;

use crate::webui_v2::error::WebUiV2HttpError;
use crate::webui_v2::router::{WebUiV2Capabilities, WebUiV2State};
use crate::webui_v2::schema::WebChatV2EventFrame;
use crate::webui_v2::sse_capacity::{SSE_MAX_LIFETIME, SseSlot};

// Session bootstrap must stay cheap and non-blocking: this flag only tunes
// initial approval UI state. It is mutable through `/settings/tools`, so do
// not cache it across requests; the settings route remains authoritative.
const GLOBAL_AUTO_APPROVE_FEATURE_TIMEOUT: Duration = Duration::from_millis(250);
const SETTINGS_TOOLS_AUTO_APPROVE_KEY: &str = "agent.auto_approve_tools";
const SETTINGS_TOOL_CONFIG_PREFIX: &str = "tool.";
const SETTINGS_TOOL_CAPABILITY_ID_MAX_BYTES: usize =
    OPERATOR_CONFIG_KEY_MAX_BYTES - SETTINGS_TOOL_CONFIG_PREFIX.len();
const ADMIN_CONFIGURATION_IDEMPOTENCY_KEY_MAX_BYTES: usize = 256;

#[derive(Debug, Clone, Serialize)]
pub struct WebUiV2SessionResponse {
    pub tenant_id: String,
    pub user_id: String,
    pub capabilities: WebUiV2Capabilities,
    /// Deployment-wide feature gates the browser uses to show/hide
    /// not-yet-finished surfaces. Distinct from `capabilities`, which are
    /// per-token authorization flags.
    pub features: WebUiV2Features,
    /// Inline-attachment contract (allowed `accept` tokens + size budgets)
    /// the browser advertises on its file picker. Generated from the shared
    /// format registry so the picker can never drift from the server's
    /// allowed set; the send-message decode remains authoritative.
    pub attachments: WebUiAttachmentCapabilities,
}

/// Deployment-wide WebUI feature gates surfaced to the browser on
/// `GET /session`. These are global "is this surface ready to show"
/// toggles, not per-caller authorization — keep authorization in
/// [`WebUiV2Capabilities`].
#[derive(Debug, Clone, Copy, Default, Serialize)]
pub struct WebUiV2Features {
    /// Reborn Projects surface (the conversations-panel entry + the
    /// `/projects` route). Hidden unless the deployment sets
    /// `IRONCLAW_REBORN_PROJECTS`, while the surface is still being
    /// finished.
    pub reborn_projects: bool,
    /// Effective global auto-approve setting for the authenticated caller.
    /// The browser treats it as a bootstrap UI flag and does not inspect the
    /// operator settings payload shape. Settings mutations should update local
    /// UI state directly or re-fetch `/session`; this field is only a snapshot.
    pub global_auto_approve: bool,
}

/// `GET /api/webchat/v2/session`
pub async fn get_session(
    State(state): State<WebUiV2State>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
    Extension(capabilities): Extension<WebUiV2Capabilities>,
) -> Json<WebUiV2SessionResponse> {
    let tenant_id = caller.tenant_id.to_string();
    let user_id = caller.user_id.to_string();
    let global_auto_approve = global_auto_approve_enabled(&state, caller).await;
    Json(WebUiV2SessionResponse {
        tenant_id,
        user_id,
        capabilities,
        features: WebUiV2Features {
            reborn_projects: state.reborn_projects_enabled(),
            global_auto_approve,
        },
        attachments: webui_attachment_capabilities(),
    })
}

async fn global_auto_approve_enabled(
    state: &WebUiV2State,
    caller: WebUiAuthenticatedCaller,
) -> bool {
    match tokio::time::timeout(
        GLOBAL_AUTO_APPROVE_FEATURE_TIMEOUT,
        GLOBAL_AUTO_APPROVE_VIEW.query_on(
            state.services().as_ref(),
            caller,
            RebornGlobalAutoApproveRequest {},
            None,
        ),
    )
    .await
    {
        Ok(Ok(response)) => response.enabled,
        Ok(Err(error)) => {
            tracing::debug!(?error, "failed to read global auto-approve session feature");
            false
        }
        Err(_) => {
            tracing::debug!(
                timeout_ms = GLOBAL_AUTO_APPROVE_FEATURE_TIMEOUT.as_millis(),
                "timed out reading global auto-approve session feature"
            );
            false
        }
    }
}

/// `POST /api/webchat/v2/threads`
///
/// Body shape: [`WebUiCreateThreadRequest`].
pub async fn create_thread(
    State(state): State<WebUiV2State>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
    Json(body): Json<WebUiCreateThreadRequest>,
) -> Result<Json<RebornCreateThreadResponse>, WebUiV2HttpError> {
    let response = CREATE_THREAD_OPERATION
        .execute_on(state.services().as_ref(), caller, body)
        .await?;
    Ok(Json(response))
}

/// `DELETE /api/webchat/v2/threads/{thread_id}`
pub async fn delete_thread(
    State(state): State<WebUiV2State>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
    Path(thread_id): Path<String>,
) -> Result<Json<RebornDeleteThreadResponse>, WebUiV2HttpError> {
    let resolution = invoke_product_capability(
        state.services(),
        caller,
        THREAD_DELETE_CAPABILITY,
        RebornDeleteThreadRequest {
            thread_id: thread_id.clone(),
        },
    )
    .await?;
    capability_resolution_succeeded(
        resolution,
        "thread delete",
        true,
        outbound_preferences_forbidden,
        outbound_preferences_unavailable,
    )?;
    let thread_id = parse_thread_id_for_response("thread_id", thread_id)?;
    let response = RebornDeleteThreadResponse {
        thread_id,
        deleted: true,
    };
    Ok(Json(response))
}

// --- Admin user management ---------------------------------------------------
//
// Every handler delegates straight to the facade, which enforces admin
// authorization (operator token or admin/owner role) and last-admin protection.
// The `{user_id}` and `{handle}` path segments are parsed into their domain
// types (`UserId` / `SecretHandle`) here so a malformed value is a sanitized
// 400 before the facade runs — raw strings are a boundary format and never
// travel deeper than this edge (see `.claude/rules/types.md`).

/// Parse a `{user_id}` path segment into a `UserId`, mapping a malformed value
/// to a sanitized `400 invalid_request` before the facade is touched.
fn parse_admin_user_id(raw: String) -> Result<UserId, WebUiV2HttpError> {
    UserId::new(raw).map_err(|_| {
        WebUiV2HttpError::from(RebornServicesError::from(WebUiInboundValidationError::new(
            "user_id",
            WebUiInboundValidationCode::InvalidId,
        )))
    })
}

/// Parse a `{handle}` path segment into a `SecretHandle`, mapping a malformed
/// value to a sanitized `400 invalid_request` before the facade is touched.
/// Keeps a bad handle a client fault (400), never an internal 500 downstream.
fn parse_admin_secret_handle(raw: String) -> Result<SecretHandle, WebUiV2HttpError> {
    SecretHandle::new(raw).map_err(|_| {
        WebUiV2HttpError::from(RebornServicesError::from(WebUiInboundValidationError::new(
            "handle",
            WebUiInboundValidationCode::InvalidId,
        )))
    })
}

async fn read_admin_user_secret(
    services: &std::sync::Arc<dyn ProductSurface>,
    caller: WebUiAuthenticatedCaller,
    user_id: UserId,
    handle: String,
) -> Result<ironclaw_product_workflow::AdminUserSecretMeta, WebUiV2HttpError> {
    let response = ADMIN_USER_SECRETS_VIEW
        .query_on(
            services.as_ref(),
            caller,
            RebornAdminUserRequest { user_id },
            None,
        )
        .await?;
    response
        .secrets
        .into_iter()
        .find(|secret| secret.handle == handle)
        .ok_or_else(|| RebornServicesError::internal_from("updated admin user secret missing"))
        .map_err(WebUiV2HttpError::from)
}

/// `GET /api/webchat/v2/admin/users`
pub async fn admin_list_users(
    State(state): State<WebUiV2State>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
    Query(query): Query<RebornAdminUserListQuery>,
) -> Result<Json<RebornAdminUserListResponse>, WebUiV2HttpError> {
    let mut request = query;
    let cursor = request.cursor.take();
    let response = ADMIN_USERS_VIEW
        .query_on(state.services().as_ref(), caller, request, cursor)
        .await?;
    Ok(Json(response))
}

/// `POST /api/webchat/v2/admin/users`
pub async fn admin_create_user(
    State(state): State<WebUiV2State>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
    Json(body): Json<RebornAdminCreateUserRequest>,
) -> Result<Json<RebornAdminUserCreatedResponse>, WebUiV2HttpError> {
    let response = ADMIN_USER_CREATE_OPERATION
        .execute_on(state.services().as_ref(), caller, body)
        .await?;
    Ok(Json(response))
}

/// `GET /api/webchat/v2/admin/users/{user_id}`
pub async fn admin_get_user(
    State(state): State<WebUiV2State>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
    Path(user_id): Path<String>,
) -> Result<Json<RebornAdminUserResponse>, WebUiV2HttpError> {
    let user_id = parse_admin_user_id(user_id)?;
    let response = ADMIN_USER_VIEW
        .query_on(
            state.services().as_ref(),
            caller,
            RebornAdminUserRequest { user_id },
            None,
        )
        .await?;
    Ok(Json(response))
}

/// `PATCH /api/webchat/v2/admin/users/{user_id}`
pub async fn admin_update_user(
    State(state): State<WebUiV2State>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
    Path(user_id): Path<String>,
    Json(body): Json<RebornAdminUpdateUserRequest>,
) -> Result<Json<RebornAdminUserResponse>, WebUiV2HttpError> {
    let user_id = parse_admin_user_id(user_id)?;
    let resolution = invoke_product_capability(
        state.services(),
        caller.clone(),
        ADMIN_USER_UPDATE_CAPABILITY,
        RebornAdminUpdateUserProductRequest {
            user_id: user_id.clone(),
            display_name: body.display_name,
            metadata: body.metadata,
        },
    )
    .await?;
    capability_resolution_succeeded(
        resolution,
        "admin user update",
        true,
        outbound_preferences_forbidden,
        outbound_preferences_unavailable,
    )?;
    let response = ADMIN_USER_VIEW
        .query_on(
            state.services().as_ref(),
            caller,
            RebornAdminUserRequest { user_id },
            None,
        )
        .await?;
    Ok(Json(response))
}

/// `DELETE /api/webchat/v2/admin/users/{user_id}`
pub async fn admin_delete_user(
    State(state): State<WebUiV2State>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
    Path(user_id): Path<String>,
) -> Result<Json<RebornAdminUserDeletedResponse>, WebUiV2HttpError> {
    let user_id = parse_admin_user_id(user_id)?;
    let resolution = invoke_product_capability(
        state.services(),
        caller,
        ADMIN_USER_DELETE_CAPABILITY,
        RebornAdminUserRequest {
            user_id: user_id.clone(),
        },
    )
    .await?;
    capability_resolution_succeeded(
        resolution,
        "admin user delete",
        true,
        outbound_preferences_forbidden,
        outbound_preferences_unavailable,
    )?;
    Ok(Json(RebornAdminUserDeletedResponse {
        user_id,
        deleted: true,
    }))
}

/// `POST /api/webchat/v2/admin/users/{user_id}/status`
pub async fn admin_set_user_status(
    State(state): State<WebUiV2State>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
    Path(user_id): Path<String>,
    Json(body): Json<RebornAdminSetStatusRequest>,
) -> Result<Json<RebornAdminUserResponse>, WebUiV2HttpError> {
    let user_id = parse_admin_user_id(user_id)?;
    let resolution = invoke_product_capability(
        state.services(),
        caller.clone(),
        ADMIN_USER_SET_STATUS_CAPABILITY,
        RebornAdminSetStatusProductRequest {
            user_id: user_id.clone(),
            status: body.status,
        },
    )
    .await?;
    capability_resolution_succeeded(
        resolution,
        "admin user status",
        true,
        outbound_preferences_forbidden,
        outbound_preferences_unavailable,
    )?;
    let response = ADMIN_USER_VIEW
        .query_on(
            state.services().as_ref(),
            caller,
            RebornAdminUserRequest { user_id },
            None,
        )
        .await?;
    Ok(Json(response))
}

/// `POST /api/webchat/v2/admin/users/{user_id}/role`
pub async fn admin_set_user_role(
    State(state): State<WebUiV2State>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
    Path(user_id): Path<String>,
    Json(body): Json<RebornAdminSetRoleRequest>,
) -> Result<Json<RebornAdminUserResponse>, WebUiV2HttpError> {
    let user_id = parse_admin_user_id(user_id)?;
    let resolution = invoke_product_capability(
        state.services(),
        caller.clone(),
        ADMIN_USER_SET_ROLE_CAPABILITY,
        RebornAdminSetRoleProductRequest {
            user_id: user_id.clone(),
            role: body.role,
        },
    )
    .await?;
    capability_resolution_succeeded(
        resolution,
        "admin user role",
        true,
        outbound_preferences_forbidden,
        outbound_preferences_unavailable,
    )?;
    let response = ADMIN_USER_VIEW
        .query_on(
            state.services().as_ref(),
            caller,
            RebornAdminUserRequest { user_id },
            None,
        )
        .await?;
    Ok(Json(response))
}

/// `GET /api/webchat/v2/admin/users/{user_id}/secrets`
pub async fn admin_list_user_secrets(
    State(state): State<WebUiV2State>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
    Path(user_id): Path<String>,
) -> Result<Json<RebornAdminUserSecretsListResponse>, WebUiV2HttpError> {
    let user_id = parse_admin_user_id(user_id)?;
    let response = ADMIN_USER_SECRETS_VIEW
        .query_on(
            state.services().as_ref(),
            caller,
            RebornAdminUserRequest { user_id },
            None,
        )
        .await?;
    Ok(Json(response))
}

/// `PUT /api/webchat/v2/admin/users/{user_id}/secrets/{handle}`
pub async fn admin_put_user_secret(
    State(state): State<WebUiV2State>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
    Path((user_id, handle)): Path<(String, String)>,
    Json(body): Json<RebornAdminPutSecretRequest>,
) -> Result<Json<RebornAdminSecretResponse>, WebUiV2HttpError> {
    let user_id = parse_admin_user_id(user_id)?;
    let handle = parse_admin_secret_handle(handle)?;
    let handle_name = handle.as_str().to_string();
    let resolution = invoke_product_capability(
        state.services(),
        caller.clone(),
        ADMIN_USER_PUT_SECRET_CAPABILITY,
        RebornAdminPutSecretProductRequest {
            user_id: user_id.clone(),
            handle: handle_name.clone(),
            value: body.value,
        },
    )
    .await?;
    capability_resolution_succeeded(
        resolution,
        "admin user secret put",
        true,
        outbound_preferences_forbidden,
        outbound_preferences_unavailable,
    )?;
    let secret = read_admin_user_secret(state.services(), caller, user_id, handle_name).await?;
    Ok(Json(RebornAdminSecretResponse { secret }))
}

/// `DELETE /api/webchat/v2/admin/users/{user_id}/secrets/{handle}`
pub async fn admin_delete_user_secret(
    State(state): State<WebUiV2State>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
    Path((user_id, handle)): Path<(String, String)>,
) -> Result<Json<RebornAdminSecretDeletedResponse>, WebUiV2HttpError> {
    let user_id = parse_admin_user_id(user_id)?;
    let handle = parse_admin_secret_handle(handle)?;
    let handle = handle.as_str().to_string();
    let response = ADMIN_USER_DELETE_SECRET_OPERATION
        .execute_on(
            state.services().as_ref(),
            caller,
            RebornAdminDeleteSecretProductRequest { user_id, handle },
        )
        .await?;
    Ok(Json(response))
}

/// `POST /api/webchat/v2/threads/{thread_id}/messages`
///
/// Body shape: [`WebUiSendMessageRequest`] (the path `thread_id` overrides
/// any value in the body).
pub async fn send_message(
    State(state): State<WebUiV2State>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
    Path(thread_id): Path<String>,
    Json(mut body): Json<WebUiSendMessageRequest>,
) -> Result<Json<RebornSubmitTurnResponse>, WebUiV2HttpError> {
    body.thread_id = Some(thread_id);
    let response = SUBMIT_TURN_OPERATION
        .execute_on(state.services().as_ref(), caller, body)
        .await?;
    Ok(Json(response))
}

/// `GET /api/webchat/v2/threads/{thread_id}/timeline`
///
/// Optional query parameters:
/// - `limit`: maximum number of messages per response. The facade
///   clamps to a hard ceiling so an unbounded value cannot widen the
///   response.
/// - `cursor`: opaque cursor echoed from the previous response's
///   `next_cursor` to load the page preceding it.
pub async fn get_timeline(
    State(state): State<WebUiV2State>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
    Path(thread_id): Path<String>,
    Query(query): Query<TimelineQuery>,
) -> Result<Json<RebornTimelineResponse>, WebUiV2HttpError> {
    let request = RebornTimelineRequest {
        thread_id,
        limit: query.limit,
        cursor: query.cursor,
    };
    let mut request = request;
    let cursor = request.cursor.take();
    let response = TIMELINE_VIEW
        .query_on(state.services().as_ref(), caller, request, cursor)
        .await?;
    Ok(Json(response))
}

/// Query parameters for `get_timeline`. Both fields are optional — a
/// caller with neither gets the most recent page (default size).
#[derive(Debug, Default, Deserialize)]
pub struct TimelineQuery {
    #[serde(default)]
    pub limit: Option<u32>,
    #[serde(default)]
    pub cursor: Option<String>,
}

/// Default workspace root listed when a `list_project_files` request omits
/// `?path=`. The facade confines all paths to this alias regardless.
const PROJECT_FS_ROOT: &str = "/workspace";

/// Query parameters for the project-filesystem read routes. `path` is a scoped
/// path under `/workspace`; optional only for directory listing (defaults to
/// the workspace root).
#[derive(Debug, Default, Deserialize)]
pub struct ProjectFsQuery {
    #[serde(default)]
    pub path: Option<String>,
}

/// `GET /api/webchat/v2/threads/{thread_id}/files`
///
/// List a directory under the thread's project workspace. Generic filesystem
/// navigation — also the listing surface a future file browser consumes.
pub async fn list_project_files(
    State(state): State<WebUiV2State>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
    Path(thread_id): Path<String>,
    Query(query): Query<ProjectFsQuery>,
) -> Result<Json<RebornProjectFsListResponse>, WebUiV2HttpError> {
    let request = RebornProjectFsListRequest {
        thread_id,
        path: project_fs_list_path(query.path),
    };
    let response = PROJECT_FS_LIST_VIEW
        .query_on(state.services().as_ref(), caller, request, None)
        .await?;
    Ok(Json(response))
}

/// `GET /api/webchat/v2/threads/{thread_id}/files/stat`
///
/// Return metadata for a path under the thread's project workspace.
pub async fn stat_project_file(
    State(state): State<WebUiV2State>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
    Path(thread_id): Path<String>,
    Query(query): Query<ProjectFsQuery>,
) -> Result<Json<RebornProjectFsStatResponse>, WebUiV2HttpError> {
    let request = RebornProjectFsStatRequest {
        thread_id,
        path: require_project_fs_path(query.path)?,
    };
    let response = PROJECT_FS_STAT_VIEW
        .query_on(state.services().as_ref(), caller, request, None)
        .await?;
    Ok(Json(response))
}

/// `GET /api/webchat/v2/threads/{thread_id}/files/content`
///
/// Download a file's bytes from the thread's project workspace. This is the
/// retrieval path for agent-produced attachments (an `AttachmentRef`'s
/// `storage_key` is passed as `?path=`).
///
/// The response is always served as an attachment with `nosniff` so a generated
/// `.html`/`.svg` cannot execute in the app origin.
pub async fn read_project_file(
    State(state): State<WebUiV2State>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
    Path(thread_id): Path<String>,
    Query(query): Query<ProjectFsQuery>,
) -> Result<Response, WebUiV2HttpError> {
    let request = RebornProjectFsReadRequest {
        thread_id,
        path: require_project_fs_path(query.path)?,
    };
    let file = PROJECT_FS_READ_OPERATION
        .execute_file_on(state.services().as_ref(), caller, request)
        .await?;
    project_fs_download_response(file)
}

/// Build the always-attachment, `nosniff` download response shared by the
/// thread-scoped project-file route and the standalone filesystem-browser route.
/// Serving every file as an attachment with `nosniff` means a generated
/// `.html`/`.svg` cannot execute in the app origin.
fn project_fs_download_response(file: ProjectFsFile) -> Result<Response, WebUiV2HttpError> {
    let filename = sanitized_download_filename(file.filename.as_deref());
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, file.mime_type)
        .header(header::CONTENT_LENGTH, file.size_bytes)
        .header(
            header::CONTENT_DISPOSITION,
            format!("attachment; filename=\"{filename}\""),
        )
        .header(header::X_CONTENT_TYPE_OPTIONS, "nosniff")
        .body(Body::from(file.bytes))
        .map_err(|error| {
            // Keep the client response sanitized (bare 500), but log the
            // builder cause so a malformed download header is diagnosable
            // server-side rather than vanishing into an opaque internal error.
            tracing::debug!(
                target = "ironclaw_webui_v2::project_fs",
                error = %error,
                "failed to build project-file download response",
            );
            WebUiV2HttpError::from(RebornServicesError::internal())
        })
}

/// Query parameters for the standalone filesystem-browser read routes. `mount`
/// selects which logical mount to read (memory/workspace/…); `path` is a
/// mount-relative path (absent/blank means the mount root for listing), and
/// `project_id` optionally selects an authorized project scope.
#[derive(Debug, Deserialize)]
pub struct FsBrowseQuery {
    pub mount: FsMount,
    #[serde(default)]
    pub path: Option<String>,
    /// Optional project to browse, authorized by the product-workflow facade.
    #[serde(default)]
    pub project_id: Option<ironclaw_host_api::ProjectId>,
}

/// `GET /api/webchat/v2/fs/mounts`
///
/// List the mounts the read-only filesystem viewer can browse for this caller.
pub async fn list_fs_mounts(
    State(state): State<WebUiV2State>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
) -> Result<Json<RebornFsMountsResponse>, WebUiV2HttpError> {
    let response = FS_MOUNTS_VIEW
        .query_on(
            state.services().as_ref(),
            caller,
            RebornFsMountsRequest {},
            None,
        )
        .await?;
    Ok(Json(response))
}

/// `GET /api/webchat/v2/fs/list?mount=…&path=…&project_id=…`
///
/// List a directory on a browsable mount. Caller-scoped read-only navigation
/// over the agent's internal filesystem.
pub async fn browse_fs_dir(
    State(state): State<WebUiV2State>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
    Query(query): Query<FsBrowseQuery>,
) -> Result<Json<RebornFsListResponse>, WebUiV2HttpError> {
    let request = RebornFsListRequest {
        mount: query.mount,
        // Absent, empty, or whitespace-only path lists the mount root.
        path: query
            .path
            .filter(|path| !path.trim().is_empty())
            .unwrap_or_default(),
        project_id: query.project_id,
    };
    let response = FS_LIST_VIEW
        .query_on(state.services().as_ref(), caller, request, None)
        .await?;
    Ok(Json(response))
}

/// `GET /api/webchat/v2/fs/stat?mount=…&path=…&project_id=…`
///
/// Return metadata for a path on a browsable mount.
pub async fn stat_fs_path(
    State(state): State<WebUiV2State>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
    Query(query): Query<FsBrowseQuery>,
) -> Result<Json<RebornFsStatResponse>, WebUiV2HttpError> {
    let request = RebornFsStatRequest {
        mount: query.mount,
        path: require_fs_browse_path(query.path)?,
        project_id: query.project_id,
    };
    let response = FS_STAT_VIEW
        .query_on(state.services().as_ref(), caller, request, None)
        .await?;
    Ok(Json(response))
}

/// `GET /api/webchat/v2/fs/content?mount=…&path=…&project_id=…`
///
/// Download/preview a file's bytes from a browsable mount. Served as an
/// attachment with `nosniff`, exactly like the project-file route.
pub async fn read_fs_file(
    State(state): State<WebUiV2State>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
    Query(query): Query<FsBrowseQuery>,
) -> Result<Response, WebUiV2HttpError> {
    let request = RebornFsReadRequest {
        mount: query.mount,
        path: require_fs_browse_path(query.path)?,
        project_id: query.project_id,
    };
    let file = FS_READ_OPERATION
        .execute_file_on(state.services().as_ref(), caller, request)
        .await?;
    project_fs_download_response(file)
}

/// Reject a missing/blank `?path=` on the stat/download fs-browse routes with a
/// field-scoped 400, mirroring [`require_project_fs_path`].
fn require_fs_browse_path(path: Option<String>) -> Result<String, WebUiV2HttpError> {
    match path {
        Some(path) if !path.trim().is_empty() => Ok(path),
        _ => Err(RebornServicesError::from(WebUiInboundValidationError::new(
            "path",
            WebUiInboundValidationCode::Blank,
        ))
        .into()),
    }
}

/// Reject a missing or blank `?path=` on the stat/download routes with a
/// field-scoped 400, rather than forwarding an empty string to the facade where
/// it surfaces as a murkier downstream invalid-path error.
/// Resolve the directory-listing path. An absent, empty, or whitespace-only
/// `?path=` means "list the workspace root" — mirrors `require_project_fs_path`'s
/// `trim`-based blank handling (so `?path=%20%20` isn't forwarded as a bogus
/// path), but defaults to the root instead of erroring, since listing the root
/// is a valid request.
fn project_fs_list_path(path: Option<String>) -> String {
    path.filter(|path| !path.trim().is_empty())
        .unwrap_or_else(|| PROJECT_FS_ROOT.to_string())
}

fn require_project_fs_path(path: Option<String>) -> Result<String, WebUiV2HttpError> {
    match path {
        Some(path) if !path.trim().is_empty() => Ok(path),
        _ => Err(RebornServicesError::from(WebUiInboundValidationError::new(
            "path",
            WebUiInboundValidationCode::Blank,
        ))
        .into()),
    }
}

/// Query parameters for `list_projects`.
#[derive(Debug, Default, Deserialize)]
pub struct ListProjectsQuery {
    #[serde(default)]
    pub limit: Option<u32>,
}

/// `GET /api/webchat/v2/projects`
pub async fn list_projects(
    State(state): State<WebUiV2State>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
    Query(query): Query<ListProjectsQuery>,
) -> Result<Json<RebornListProjectsResponse>, WebUiV2HttpError> {
    let request = RebornListProjectsRequest { limit: query.limit };
    let response = PROJECTS_VIEW
        .query_on(state.services().as_ref(), caller, request, None)
        .await?;
    Ok(Json(response))
}

/// `POST /api/webchat/v2/projects`
pub async fn create_project(
    State(state): State<WebUiV2State>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
    Json(body): Json<RebornCreateProjectRequest>,
) -> Result<Json<RebornProjectResponse>, WebUiV2HttpError> {
    let response = PROJECT_CREATE_OPERATION
        .execute_on(state.services().as_ref(), caller, body)
        .await?;
    Ok(Json(response))
}

/// `GET /api/webchat/v2/projects/{project_id}`
pub async fn get_project(
    State(state): State<WebUiV2State>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
    Path(project_id): Path<String>,
) -> Result<Json<RebornProjectResponse>, WebUiV2HttpError> {
    let response = PROJECT_VIEW
        .query_on(
            state.services().as_ref(),
            caller,
            RebornGetProjectRequest { project_id },
            None,
        )
        .await?;
    Ok(Json(response))
}

/// `POST /api/webchat/v2/projects/{project_id}` — update (path `project_id`
/// overrides any body value).
pub async fn update_project(
    State(state): State<WebUiV2State>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
    Path(project_id): Path<String>,
    Json(mut body): Json<RebornUpdateProjectRequest>,
) -> Result<Json<RebornProjectResponse>, WebUiV2HttpError> {
    body.project_id = project_id;
    let resolution = invoke_product_capability(
        state.services(),
        caller.clone(),
        PROJECT_UPDATE_CAPABILITY,
        body.clone(),
    )
    .await?;
    capability_resolution_succeeded(
        resolution,
        "project",
        true,
        outbound_preferences_forbidden,
        outbound_preferences_unavailable,
    )?;
    let response = PROJECT_VIEW
        .query_on(
            state.services().as_ref(),
            caller,
            RebornGetProjectRequest {
                project_id: body.project_id,
            },
            None,
        )
        .await?;
    Ok(Json(response))
}

/// `DELETE /api/webchat/v2/projects/{project_id}`
pub async fn delete_project(
    State(state): State<WebUiV2State>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
    Path(project_id): Path<String>,
) -> Result<StatusCode, WebUiV2HttpError> {
    let resolution = invoke_product_capability(
        state.services(),
        caller,
        PROJECT_DELETE_CAPABILITY,
        RebornDeleteProjectRequest { project_id },
    )
    .await?;
    capability_resolution_succeeded(
        resolution,
        "project",
        true,
        outbound_preferences_forbidden,
        outbound_preferences_unavailable,
    )?;
    Ok(StatusCode::NO_CONTENT)
}

/// `GET /api/webchat/v2/projects/{project_id}/members`
pub async fn list_project_members(
    State(state): State<WebUiV2State>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
    Path(project_id): Path<String>,
) -> Result<Json<RebornListMembersResponse>, WebUiV2HttpError> {
    let response = PROJECT_MEMBERS_VIEW
        .query_on(
            state.services().as_ref(),
            caller,
            RebornListMembersRequest { project_id },
            None,
        )
        .await?;
    Ok(Json(response))
}

/// `POST /api/webchat/v2/projects/{project_id}/members` — grant a member
/// (path `project_id` overrides any body value).
pub async fn add_project_member(
    State(state): State<WebUiV2State>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
    Path(project_id): Path<String>,
    Json(mut body): Json<RebornAddMemberRequest>,
) -> Result<Json<RebornProjectMemberInfo>, WebUiV2HttpError> {
    body.project_id = project_id;
    let resolution = invoke_product_capability(
        state.services(),
        caller.clone(),
        PROJECT_MEMBER_ADD_CAPABILITY,
        body.clone(),
    )
    .await?;
    capability_resolution_succeeded(
        resolution,
        "project",
        true,
        outbound_preferences_forbidden,
        outbound_preferences_unavailable,
    )?;
    let response =
        read_project_member(state.services(), caller, body.project_id, body.user_id).await?;
    Ok(Json(response))
}

/// `POST /api/webchat/v2/projects/{project_id}/members/{user_id}` — change a
/// member's role (path ids override any body value).
pub async fn update_project_member(
    State(state): State<WebUiV2State>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
    Path((project_id, user_id)): Path<(String, String)>,
    Json(mut body): Json<RebornUpdateMemberRoleRequest>,
) -> Result<Json<RebornProjectMemberInfo>, WebUiV2HttpError> {
    body.project_id = project_id;
    body.user_id = user_id;
    let resolution = invoke_product_capability(
        state.services(),
        caller.clone(),
        PROJECT_MEMBER_UPDATE_CAPABILITY,
        body.clone(),
    )
    .await?;
    capability_resolution_succeeded(
        resolution,
        "project",
        true,
        outbound_preferences_forbidden,
        outbound_preferences_unavailable,
    )?;
    let response =
        read_project_member(state.services(), caller, body.project_id, body.user_id).await?;
    Ok(Json(response))
}

/// `DELETE /api/webchat/v2/projects/{project_id}/members/{user_id}`
pub async fn remove_project_member(
    State(state): State<WebUiV2State>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
    Path((project_id, user_id)): Path<(String, String)>,
) -> Result<StatusCode, WebUiV2HttpError> {
    let resolution = invoke_product_capability(
        state.services(),
        caller,
        PROJECT_MEMBER_REMOVE_CAPABILITY,
        RebornRemoveMemberRequest {
            project_id,
            user_id,
        },
    )
    .await?;
    capability_resolution_succeeded(
        resolution,
        "project",
        true,
        outbound_preferences_forbidden,
        outbound_preferences_unavailable,
    )?;
    Ok(StatusCode::NO_CONTENT)
}

async fn read_project_member(
    services: &std::sync::Arc<dyn ProductSurface>,
    caller: WebUiAuthenticatedCaller,
    project_id: String,
    user_id: String,
) -> Result<RebornProjectMemberInfo, WebUiV2HttpError> {
    let response = PROJECT_MEMBERS_VIEW
        .query_on(
            services.as_ref(),
            caller,
            RebornListMembersRequest { project_id },
            None,
        )
        .await?;
    response
        .members
        .into_iter()
        .find(|member| member.user_id == user_id)
        .ok_or_else(|| RebornServicesError::internal_from("updated project member missing"))
        .map_err(WebUiV2HttpError::from)
}

/// Upper bound on the sanitized `Content-Disposition` filename. A filesystem can
/// hold names far longer than is safe to splice into a header; cap well under
/// typical header-size limits so an oversized name degrades to a truncated label
/// rather than failing the whole download with a builder error (500).
const MAX_DOWNLOAD_FILENAME_BYTES: usize = 200;

/// Produce a `Content-Disposition` filename that cannot inject header bytes or
/// path separators. Keeps a conservative set of characters and falls back to a
/// neutral name when nothing safe survives.
fn sanitized_download_filename(filename: Option<&str>) -> String {
    let candidate: String = filename
        .unwrap_or("download")
        .chars()
        .map(|c| match c {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '.' | '-' | '_' | ' ' => c,
            _ => '_',
        })
        .collect();
    // Bound the length on a char boundary (every retained char is ASCII here, so
    // each is one byte) before trimming, so the cap can't leave a stray leading
    // dot/space at the new end.
    let bounded = if candidate.len() > MAX_DOWNLOAD_FILENAME_BYTES {
        &candidate[..MAX_DOWNLOAD_FILENAME_BYTES]
    } else {
        candidate.as_str()
    };
    let trimmed = bounded.trim_matches([' ', '.']).to_string();
    if trimmed.is_empty() {
        "download".to_string()
    } else {
        trimmed
    }
}

/// `GET /api/webchat/v2/threads/{thread_id}/messages/{message_id}/attachments/{attachment_id}`
///
/// Serves one landed attachment's raw bytes so the browser can render an image
/// thumbnail (or download a file) for a persisted message. The `(thread_id,
/// message_id, attachment_id)` triple addresses the attachment; the caller's
/// authority comes from the authenticated session, and the facade derives the
/// scope and resolves the storage path server-side. The response sets the
/// authoritative `Content-Type` from the stored ref plus `nosniff` and a short
/// private cache so the browser can reuse the bytes without re-fetching.
pub async fn get_attachment(
    State(state): State<WebUiV2State>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
    Path((thread_id, message_id, attachment_id)): Path<(String, String, String)>,
) -> Result<Response, WebUiV2HttpError> {
    let attachment = ATTACHMENT_READ_OPERATION
        .execute_attachment_on(
            state.services().as_ref(),
            caller,
            RebornAttachmentRequest {
                thread_id,
                message_id,
                attachment_id,
            },
        )
        .await?;

    let mut headers = HeaderMap::new();
    // The mime came from the stored ref; fall back to octet-stream if it is not
    // a valid header value rather than failing the read.
    let content_type = HeaderValue::from_str(&attachment.mime_type)
        .unwrap_or_else(|_| HeaderValue::from_static("application/octet-stream"));
    headers.insert(header::CONTENT_TYPE, content_type);
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("private, max-age=300"),
    );
    Ok((StatusCode::OK, headers, attachment.bytes).into_response())
}

/// SSE polling cadence for `stream_events`. The facade only exposes a
/// drain-style read; once the backlog is flushed the handler waits this
/// long before checking for newly arrived events.
const SSE_POLL_INTERVAL: Duration = Duration::from_secs(1);

/// Upper bound for idle `stream_events` polling. A browser tab with no
/// pending projection events should not keep revalidating/draining through
/// remote durable storage every second forever, especially on high-RTT
/// hosted Postgres.
const SSE_IDLE_POLL_MAX_INTERVAL: Duration = Duration::from_secs(3);

/// SSE keep-alive cadence. axum emits an SSE comment line every interval
/// to keep proxies from closing the idle connection.
const SSE_KEEPALIVE_INTERVAL: Duration = Duration::from_secs(15);

/// HTTP header the browser's `EventSource` sends on auto-reconnect to
/// resume an SSE stream. The value is the `id:` of the last successfully
/// delivered event; for this surface the handler sets that to the JSON-
/// serialized projection cursor.
const LAST_EVENT_ID_HEADER: &str = "last-event-id";

fn sse_poll_interval_for_idle_polls(idle_polls: u32) -> Duration {
    match idle_polls {
        0 | 1 => SSE_POLL_INTERVAL,
        2 => Duration::from_secs(2),
        _ => SSE_IDLE_POLL_MAX_INTERVAL,
    }
}

/// `GET /api/webchat/v2/threads/{thread_id}/events`
///
/// Server-Sent Events stream. Each event carries one
/// [`WebChatV2EventFrame`] as JSON with the projection cursor as the
/// SSE `id` so the browser can resume from the last delivered event.
///
/// Resume cursor precedence: `Last-Event-ID` header (sent automatically
/// by the browser's `EventSource` on reconnect) wins over the
/// `?after_cursor=...` query parameter. Both are optional — first
/// connects pass neither and start from the projection origin.
///
/// The handler acquires a per-`(tenant, user)` concurrency slot before
/// returning the stream; callers at or above the configured cap receive
/// `429 Too Many Requests` with `retryable: true`. Each stream is also
/// closed after [`SSE_MAX_LIFETIME`] so the browser must reconnect with
/// `Last-Event-ID`, which bounds drift and recycles slots even under
/// long-running tab leaks.
///
/// When the facade supports subscriptions, the handler forwards that live
/// stream directly. Older compositions fall back to drain/poll semantics,
/// documented on [`ProductSurface::stream_events`].
///
/// [`WebChatV2EventFrame`]: crate::webui_v2::schema::WebChatV2EventFrame
/// [`ProductSurface::stream_events`]: ironclaw_product_workflow::ProductSurface::stream_events
/// [`SSE_MAX_LIFETIME`]: crate::webui_v2::sse_capacity::SSE_MAX_LIFETIME
pub async fn stream_events(
    State(state): State<WebUiV2State>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
    Path(thread_id): Path<String>,
    headers: HeaderMap,
    Query(query): Query<StreamEventsQuery>,
) -> Result<Response, WebUiV2HttpError> {
    let connection_id = stream_connection_id(query.connection_id.as_deref());
    let slot = match state.sse_capacity().try_acquire_ordered(
        &caller.tenant_id,
        &caller.user_id,
        connection_id,
        connection_id.and(query.connection_generation),
    ) {
        crate::webui_v2::sse_capacity::SseAcquireResult::Acquired(slot) => slot,
        crate::webui_v2::sse_capacity::SseAcquireResult::AtCapacity => {
            return Err(sse_concurrency_exhausted());
        }
        crate::webui_v2::sse_capacity::SseAcquireResult::StaleGeneration => {
            return Ok(StatusCode::NO_CONTENT.into_response());
        }
    };
    let services = state.services().clone();
    let initial_cursor = headers
        .get(LAST_EVENT_ID_HEADER)
        // silent-ok: non-visible-ASCII Last-Event-ID is treated as absent so the
        // handler falls back to the query param / origin, matching the standard
        // EventSource contract (server SHOULD ignore a malformed Last-Event-ID).
        .and_then(|value| value.to_str().ok())
        .map(str::to_string)
        .or(query.after_cursor);
    let stream = build_sse_stream(services, caller, thread_id, initial_cursor, slot);
    let mut response = Sse::new(stream)
        .keep_alive(KeepAlive::new().interval(SSE_KEEPALIVE_INTERVAL))
        .into_response();
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("no-cache, no-transform"),
    );
    response.headers_mut().insert(
        HeaderName::from_static("x-accel-buffering"),
        HeaderValue::from_static("no"),
    );
    Ok(response)
}

/// Build the 429 response for SSE openings that exceed the per-caller
/// concurrency cap. `retryable: true` because the slot will free as soon
/// as one of the caller's existing streams closes.
fn sse_concurrency_exhausted() -> WebUiV2HttpError {
    WebUiV2HttpError::from(RebornServicesError {
        code: RebornServicesErrorCode::RateLimited,
        kind: RebornServicesErrorKind::Busy,
        status_code: 429,
        retryable: true,
        field: None,
        validation_code: None,
    })
}

/// Query parameters for `stream_events`. `after_cursor` is the opaque
/// projection cursor the browser saw last; on first connect it is omitted
/// so the handler drains from the origin.
#[derive(Debug, Default, Deserialize)]
pub struct StreamEventsQuery {
    #[serde(default)]
    pub after_cursor: Option<String>,
    #[serde(default)]
    pub connection_id: Option<String>,
    #[serde(default)]
    pub connection_generation: Option<u64>,
}

fn stream_connection_id(connection_id: Option<&str>) -> Option<&str> {
    connection_id.filter(|connection_id| {
        !connection_id.is_empty()
            && connection_id.len() <= 64
            && connection_id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    })
}

/// Redacted SSE error payload. Defined as a typed struct (not built with
/// `serde_json::json!`) so the `Serialize` derive is total — serialization
/// cannot fail on a tagged enum + bool, so there is no fallback branch.
#[derive(Debug, Clone, Serialize)]
struct SseErrorPayload {
    error: RebornServicesErrorCode,
    kind: RebornServicesErrorKind,
    retryable: bool,
}

fn webchat_sse_event_from_envelope(envelope: ProductOutboundEnvelope) -> Option<Event> {
    let frame = WebChatV2EventFrame::from_outbound(envelope);
    let id = cursor_token(frame.cursor());
    match serde_json::to_string(&frame) {
        Ok(payload) => {
            let mut event = Event::default().event(frame.event_name()).data(payload);
            if let Some(id) = id {
                event = event.id(id);
            }
            Some(event)
        }
        Err(error) => {
            // debug, not warn: this is an internal diagnostic, not
            // user-facing status, and info!/warn! corrupts the REPL/TUI
            // per CLAUDE.md.
            tracing::debug!(
                target = "ironclaw_webui_v2::sse",
                error = %error,
                "failed to serialize WebChatV2EventFrame for SSE",
            );
            None
        }
    }
}

fn sse_error_event(error: RebornServicesError) -> Event {
    let payload = SseErrorPayload {
        error: error.code,
        kind: error.kind,
        retryable: error.retryable,
    };
    // `error` is a reserved EventSource transport event in browsers. Using it
    // for an application frame invokes both the message listener and the
    // connection-error handler, which can leave the SPA in a phantom
    // reconnect loop even though the server delivered a classified error.
    match Event::default().event("stream_error").json_data(payload) {
        Ok(event) => event,
        Err(error) => {
            tracing::debug!(
                target = "ironclaw_webui_v2::sse",
                error = %error,
                "failed to serialize redacted SSE error payload",
            );
            Event::default()
                .event("stream_error")
                .data(r#"{"error":"unavailable","kind":"service_unavailable","retryable":true}"#)
        }
    }
}

const STREAM_READY_PAYLOAD: &str = r#"{"type":"keep_alive"}"#;

fn sse_ready_event() -> Event {
    Event::default()
        .event("keep_alive")
        .data(STREAM_READY_PAYLOAD)
}

fn build_sse_stream(
    services: std::sync::Arc<dyn ProductSurface>,
    caller: WebUiAuthenticatedCaller,
    thread_id: String,
    initial_cursor: Option<String>,
    slot: SseSlot,
) -> impl Stream<Item = Result<Event, Infallible>> {
    async_stream::stream! {
        // The slot guard moves into the generator and stays alive for
        // the lifetime of this stream. It drops automatically when the
        // generator is dropped (client disconnect, max-lifetime expiry,
        // or facade error), releasing the per-caller concurrency slot.
        let mut slot_guard = slot;
        let started_at = tokio::time::Instant::now();
        let mut after_cursor = initial_cursor.and_then(parse_cursor_token);
        if services.supports_stream_events_subscription() {
            let remaining = SSE_MAX_LIFETIME.saturating_sub(started_at.elapsed());
            if remaining.is_zero() {
                return;
            }
            let request = RebornStreamEventsRequest {
                thread_id: thread_id.clone(),
                after_cursor: after_cursor.clone(),
            };
            let subscription_result = tokio::select! {
                biased;
                _ = slot_guard.cancelled() => return,
                result = tokio::time::timeout(
                    remaining,
                    services.subscribe_events(caller.clone(), request),
                ) => result,
            };
            let mut subscription = match subscription_result {
                Err(_elapsed) => {
                    tracing::debug!(
                        target = "ironclaw_webui_v2::sse",
                        "stream_events subscription pending past SSE_MAX_LIFETIME; closing stream"
                    );
                    return;
                }
                Ok(Ok(subscription)) => subscription,
                Ok(Err(error)) => {
                    tracing::debug!(
                        target = "ironclaw_webui_v2::sse",
                        error = ?error,
                        "facade rejected SSE subscription; closing stream",
                    );
                    yield Ok(sse_error_event(error));
                    return;
                }
            };
            // Axum can complete the HTTP handshake before the facade has
            // admitted its projection subscription. This application frame
            // proves the stream is actually ready, so a route switch can clear
            // a stale reconnecting state without waiting for the next model
            // delta or the transport keep-alive interval.
            yield Ok(sse_ready_event());
            loop {
                let remaining = SSE_MAX_LIFETIME.saturating_sub(started_at.elapsed());
                if remaining.is_zero() {
                    return;
                }
                let next = tokio::select! {
                    biased;
                    _ = slot_guard.cancelled() => return,
                    result = tokio::time::timeout(remaining, subscription.next()) => result,
                };
                match next {
                    Err(_elapsed) => {
                        tracing::debug!(
                            target = "ironclaw_webui_v2::sse",
                            "stream_events subscription pending past SSE_MAX_LIFETIME; closing stream"
                        );
                        return;
                    }
                    Ok(Some(Ok(envelope))) => {
                        if let Some(event) = webchat_sse_event_from_envelope(envelope) {
                            yield Ok(event);
                        }
                    }
                    Ok(Some(Err(error))) => {
                        tracing::debug!(
                            target = "ironclaw_webui_v2::sse",
                            error = ?error,
                            "facade rejected SSE subscription event; closing stream",
                        );
                        yield Ok(sse_error_event(error));
                        return;
                    }
                    Ok(None) => return,
                }
            }
        }

        let mut idle_polls = 0_u32;
        loop {
            // Force a clean close once the budget is exhausted so the
            // browser can reconnect with Last-Event-ID; this caps single-
            // stream lifetime regardless of client behavior and recycles
            // the slot. `remaining` also bounds the await below so a
            // stuck projection drain cannot pin the slot past the budget.
            let remaining = SSE_MAX_LIFETIME.saturating_sub(started_at.elapsed());
            if remaining.is_zero() {
                return;
            }
            let request = RebornStreamEventsRequest {
                thread_id: thread_id.clone(),
                after_cursor: after_cursor.clone(),
            };
            let drain = tokio::select! {
                biased;
                _ = slot_guard.cancelled() => return,
                result = tokio::time::timeout(
                    remaining,
                    services.stream_events(caller.clone(), request),
                ) => result,
            };
            match drain {
                Err(_elapsed) => {
                    // The facade drain was still pending when SSE_MAX_LIFETIME
                    // ran out. Returning here drops the generator (and the
                    // SseSlot it owns), so the per-caller concurrency budget
                    // recovers even under a stuck projection stream — without
                    // this bound, an unbounded `.await` on a non-resolving
                    // facade would pin the slot indefinitely.
                    tracing::debug!(
                        target = "ironclaw_webui_v2::sse",
                        "stream_events drain pending past SSE_MAX_LIFETIME; closing stream"
                    );
                    return;
                }
                Ok(Ok(response)) => {
                    let had_events = !response.events.is_empty();
                    if let Some(latest) = response.events.last() {
                        after_cursor = Some(latest.projection_cursor.clone());
                    }
                    for envelope in response.events {
                        if let Some(event) = webchat_sse_event_from_envelope(envelope) {
                            yield Ok(event);
                        }
                    }
                    if had_events {
                        // The production projection facade waits on its live
                        // subscription when no new item is replayable. Re-enter
                        // it immediately after delivering a batch so assistant
                        // text deltas are not delayed by the idle poll cadence.
                        idle_polls = 0;
                        continue;
                    }
                    idle_polls = idle_polls.saturating_add(1);
                    // Bound the poll sleep too so we never oversleep past the
                    // lifetime budget; the top-of-loop check then fires.
                    let sleep_for = sse_poll_interval_for_idle_polls(idle_polls)
                        .min(SSE_MAX_LIFETIME.saturating_sub(started_at.elapsed()));
                    if sleep_for.is_zero() {
                        return;
                    }
                    tokio::select! {
                        biased;
                        _ = slot_guard.cancelled() => return,
                        _ = tokio::time::sleep(sleep_for) => {}
                    }
                }
                Ok(Err(error)) => {
                    // Surface a redacted error event and close the stream.
                    // Reconnect logic is the browser's responsibility.
                    tracing::debug!(
                        target = "ironclaw_webui_v2::sse",
                        error = ?error,
                        "facade rejected SSE drain; closing stream",
                    );
                    yield Ok(sse_error_event(error));
                    return;
                }
            }
        }
    }
}

fn parse_cursor_token(token: String) -> Option<ProjectionCursor> {
    // The wire form is the JSON-serialized cursor; we accept it verbatim
    // so the browser can echo back the `id` of the last SSE event it saw
    // (which is exactly that JSON).
    serde_json::from_str(&token).ok()
}

fn cursor_token(cursor: &ProjectionCursor) -> Option<String> {
    serde_json::to_string(cursor).ok()
}

/// `POST /api/webchat/v2/threads/{thread_id}/runs/{run_id}/cancel`
///
/// Body shape: [`WebUiCancelRunRequest`] (path `thread_id` and `run_id`
/// override body values).
pub async fn cancel_run(
    State(state): State<WebUiV2State>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
    Path(CancelRunPath { thread_id, run_id }): Path<CancelRunPath>,
    Json(mut body): Json<WebUiCancelRunRequest>,
) -> Result<Json<RebornCancelRunResponse>, WebUiV2HttpError> {
    body.thread_id = Some(thread_id);
    body.run_id = Some(run_id);
    let response = CANCEL_RUN_OPERATION
        .execute_on(state.services().as_ref(), caller, body)
        .await?;
    Ok(Json(response))
}

#[derive(Debug, Deserialize)]
pub struct CancelRunPath {
    pub thread_id: String,
    pub run_id: String,
}

/// `POST /api/webchat/v2/threads/{thread_id}/runs/{run_id}/gates/{gate_ref}/resolve`
///
/// Body shape: [`WebUiResolveGateRequest`] (path overrides body for
/// `thread_id`, `run_id`, `gate_ref`).
pub async fn resolve_gate(
    State(state): State<WebUiV2State>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
    Path(ResolveGatePath {
        thread_id,
        run_id,
        gate_ref,
    }): Path<ResolveGatePath>,
    Json(mut body): Json<WebUiResolveGateRequest>,
) -> Result<Json<RebornResolveGateResponse>, WebUiV2HttpError> {
    body.thread_id = Some(thread_id);
    body.run_id = Some(run_id);
    body.gate_ref = Some(gate_ref);
    let response = RESOLVE_GATE_OPERATION
        .execute_on(state.services().as_ref(), caller, body)
        .await?;
    Ok(Json(response))
}

#[derive(Debug, Deserialize)]
pub struct ResolveGatePath {
    pub thread_id: String,
    pub run_id: String,
    pub gate_ref: String,
}

/// `POST /api/webchat/v2/threads/{thread_id}/runs/{run_id}/retry`
///
/// Body shape: [`WebUiRetryRunRequest`] (path overrides body for
/// `thread_id` and `run_id`).
pub async fn retry_run(
    State(state): State<WebUiV2State>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
    Path(RetryRunPath { thread_id, run_id }): Path<RetryRunPath>,
    Json(mut body): Json<WebUiRetryRunRequest>,
) -> Result<Json<RebornRetryRunResponse>, WebUiV2HttpError> {
    body.thread_id = Some(thread_id);
    body.run_id = Some(run_id);
    let response = RETRY_RUN_OPERATION
        .execute_on(state.services().as_ref(), caller, body)
        .await?;
    Ok(Json(response))
}

#[derive(Debug, Deserialize)]
pub struct RetryRunPath {
    pub thread_id: String,
    pub run_id: String,
}

/// `GET /api/webchat/v2/threads`
///
/// Lists threads scoped to the authenticated caller. Pagination is
/// opaque: the response carries an optional `next_cursor` the browser
/// echoes back as `?cursor=...` on the next page request.
pub async fn list_threads(
    State(state): State<WebUiV2State>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
    Query(query): Query<ListThreadsQuery>,
) -> Result<Json<RebornListThreadsResponse>, WebUiV2HttpError> {
    let mut request = WebUiListThreadsRequest {
        limit: query.limit,
        cursor: query.cursor,
        candidate_thread_id: query.candidate_thread_id,
        needs_approval: query.needs_approval,
    };
    let cursor = request.cursor.take();
    let response = THREADS_VIEW
        .query_on(state.services().as_ref(), caller, request, cursor)
        .await?;
    Ok(Json(response))
}

#[derive(Debug, Default, Deserialize)]
pub struct ListThreadsQuery {
    #[serde(default)]
    pub limit: Option<u32>,
    #[serde(default)]
    pub cursor: Option<String>,
    #[serde(default)]
    pub candidate_thread_id: Option<String>,
    #[serde(default)]
    pub needs_approval: bool,
}

/// `GET /api/webchat/v2/automations`
///
/// Lists the caller-scoped schedule automations visible to the browser. The
/// optional `?limit=N` and `?run_limit=N` queries are capped by the product
/// workflow facade; the response is a single bounded page and does not include
/// a cursor. By default only active automations are returned; pass
/// `?include_completed=true` to also include soft-completed (fire-once)
/// automations. See [`ListAutomationsQuery`] for the full per-parameter parse
/// behavior.
pub async fn list_automations(
    State(state): State<WebUiV2State>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
    Query(query): Query<ListAutomationsQuery>,
) -> Result<Json<RebornListAutomationsResponse>, WebUiV2HttpError> {
    let request = WebUiListAutomationsRequest {
        limit: query.limit,
        run_limit: query.run_limit,
        include_completed: query.include_completed,
    };
    let response = AUTOMATIONS_VIEW
        .query_on(state.services().as_ref(), caller, request, None)
        .await?;
    Ok(Json(response))
}

/// `POST /api/webchat/v2/automations/:automation_id/pause`
pub async fn pause_automation(
    State(state): State<WebUiV2State>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
    Path(automation_id): Path<String>,
) -> Result<Json<RebornAutomationMutationResponse>, WebUiV2HttpError> {
    let response = AUTOMATION_PAUSE_OPERATION
        .execute_on(
            state.services().as_ref(),
            caller,
            RebornAutomationRequest { automation_id },
        )
        .await?;
    Ok(Json(response))
}

/// `POST /api/webchat/v2/automations/:automation_id/resume`
pub async fn resume_automation(
    State(state): State<WebUiV2State>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
    Path(automation_id): Path<String>,
) -> Result<Json<RebornAutomationMutationResponse>, WebUiV2HttpError> {
    let response = AUTOMATION_RESUME_OPERATION
        .execute_on(
            state.services().as_ref(),
            caller,
            RebornAutomationRequest { automation_id },
        )
        .await?;
    Ok(Json(response))
}

/// `POST /api/webchat/v2/automations/:automation_id`
pub async fn rename_automation(
    State(state): State<WebUiV2State>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
    Path(automation_id): Path<String>,
    Json(request): Json<WebUiRenameAutomationRequest>,
) -> Result<Json<RebornAutomationMutationResponse>, WebUiV2HttpError> {
    let response = AUTOMATION_RENAME_OPERATION
        .execute_on(
            state.services().as_ref(),
            caller,
            RebornRenameAutomationProductRequest {
                automation_id,
                name: request.name,
            },
        )
        .await?;
    Ok(Json(response))
}

/// `DELETE /api/webchat/v2/automations/:automation_id`
pub async fn delete_automation(
    State(state): State<WebUiV2State>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
    Path(automation_id): Path<String>,
) -> Result<Json<RebornAutomationMutationResponse>, WebUiV2HttpError> {
    let response = AUTOMATION_DELETE_OPERATION
        .execute_on(
            state.services().as_ref(),
            caller,
            RebornAutomationRequest { automation_id },
        )
        .await?;
    Ok(Json(response))
}

#[derive(Debug, Default, Deserialize)]
pub struct ListAutomationsQuery {
    /// Optional maximum number of schedule automations to return.
    #[serde(default)]
    pub limit: Option<u32>,
    /// Optional maximum number of recent runs to return per automation row.
    #[serde(default)]
    pub run_limit: Option<u32>,
    /// When `true`, soft-completed (fire-once) automations are included
    /// alongside active ones.
    ///
    /// Parse behavior (via `serde_urlencoded` / axum `Query<T>`):
    /// - **Absent** (`?` or no param): defaults to `false` (active-only).
    /// - **`true`** / **`false`**: parsed as the corresponding boolean.
    /// - **Malformed** (e.g. `?include_completed=garbage`): deserialization
    ///   fails at the `Query` extractor and the request is rejected with
    ///   `400 Bad Request` before the handler runs. There is no silent
    ///   fallback to `false` for unparseable values.
    #[serde(default)]
    pub include_completed: bool,
}

/// `GET /api/webchat/v2/traces/credit`
///
/// Read-only Trace Commons credit summary scoped strictly to the
/// authenticated caller — the facade derives the trace scope from the
/// caller's user id; no scope input is accepted from the request. The
/// response is the contributor-local view as of the last credit sync;
/// the authoritative ledger is server-side. A caller with no local
/// Trace Commons state receives the unenrolled zero-state, not an
/// error.
pub async fn trace_credits(
    State(state): State<WebUiV2State>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
) -> Result<Json<RebornTraceCreditsResponse>, WebUiV2HttpError> {
    let response = query_product_view(
        state.services(),
        caller,
        TRACE_CREDITS_VIEW.descriptor(),
        serde_json::json!({}),
        None,
    )
    .await?;
    Ok(Json(response))
}

/// `GET /api/webchat/v2/traces/account`
///
/// Read-only list of the authenticated caller's submitted Trace Commons traces,
/// fetched per-user from the server. Scope is derived from the caller; no input
/// is accepted. Unenrolled callers receive the zero-state, not an error.
pub async fn trace_account_traces(
    State(state): State<WebUiV2State>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
) -> Result<Json<RebornAccountTracesResponse>, WebUiV2HttpError> {
    let response = query_product_view(
        state.services(),
        caller,
        TRACE_ACCOUNT_TRACES_VIEW.descriptor(),
        serde_json::json!({}),
        None,
    )
    .await?;
    Ok(Json(response))
}

/// `POST /api/webchat/v2/traces/account-login-link`
///
/// Mint a one-time Trace Commons browser login link for the authenticated
/// caller (hosted users have no host-file access; this response is the only
/// delivery channel). Scope is derived from the caller; no input is accepted.
/// Unenrolled callers receive the zero-state, not an error. SECURITY: the
/// returned URL is a one-time account credential — it must never be logged.
pub async fn trace_account_login_link(
    State(state): State<WebUiV2State>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
) -> Result<Json<RebornAccountLoginLinkResponse>, WebUiV2HttpError> {
    let response = TRACE_ACCOUNT_LOGIN_LINK_OPERATION
        .execute_on(state.services().as_ref(), caller, serde_json::json!({}))
        .await?;
    Ok(Json(response))
}

/// `POST /api/webchat/v2/traces/holds/{submission_id}/authorize`
///
/// Authorize a held manual-review trace for submission (promote-as-is). The
/// trace scope is derived from the authenticated caller; the `submission_id`
/// path segment is never authority to cross scopes. A missing/already-resolved
/// hold returns `{ authorized: false }`, not an error.
pub async fn authorize_trace_hold(
    State(state): State<WebUiV2State>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
    Path(submission_id): Path<String>,
) -> Result<Json<RebornTraceHoldAuthorizeResponse>, WebUiV2HttpError> {
    let response = TRACE_HOLD_AUTHORIZE_OPERATION
        .execute_on(
            state.services().as_ref(),
            caller,
            RebornTraceHoldAuthorizeProductRequest { submission_id },
        )
        .await?;
    Ok(Json(response))
}

/// `GET /api/webchat/v2/outbound/preferences`
pub async fn get_outbound_preferences(
    State(state): State<WebUiV2State>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
) -> Result<Json<RebornOutboundPreferencesResponse>, WebUiV2HttpError> {
    let response = query_product_view(
        state.services(),
        caller,
        OUTBOUND_PREFERENCES_VIEW.descriptor(),
        serde_json::json!({}),
        None,
    )
    .await?;
    Ok(Json(response))
}

#[derive(Deserialize)]
pub struct SetOutboundPreferencesBody {
    #[serde(default)]
    pub client_action_id: Option<String>,
    #[serde(flatten)]
    pub request: RebornSetOutboundPreferencesRequest,
}

/// `POST /api/webchat/v2/outbound/preferences`
///
/// Body shape: [`RebornSetOutboundPreferencesRequest`]. Sending
/// `{"final_reply_target_id": null}` clears the configured final-reply target.
/// `client_action_id` scopes HTTP retry replay without becoming capability input.
pub async fn set_outbound_preferences(
    State(state): State<WebUiV2State>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
    Json(body): Json<SetOutboundPreferencesBody>,
) -> Result<Json<RebornOutboundPreferencesResponse>, WebUiV2HttpError> {
    let client_action_id =
        parse_webui_client_action_id(body.client_action_id).map_err(RebornServicesError::from)?;
    let activity_id = outbound_preferences_activity_id(&caller, &client_action_id)?;
    let resolution = invoke_product_capability_with_activity_id(
        state.services(),
        caller.clone(),
        OUTBOUND_PREFERENCES_SET_CAPABILITY,
        body.request,
        activity_id,
    )
    .await?;
    capability_resolution_succeeded(
        resolution,
        "outbound preferences",
        false,
        outbound_preferences_forbidden,
        outbound_preferences_unavailable,
    )?;

    let response = query_product_view(
        state.services(),
        caller,
        OUTBOUND_PREFERENCES_VIEW.descriptor(),
        serde_json::json!({}),
        None,
    )
    .await?;
    Ok(Json(response))
}

fn capability_resolution_succeeded(
    resolution: Resolution,
    label: &'static str,
    operation_failed_is_invalid_request: bool,
    forbidden: fn() -> RebornServicesError,
    unavailable: fn(bool) -> RebornServicesError,
) -> Result<(), RebornServicesError> {
    match resolution {
        Resolution::Done(outcome) if outcome.verdict.is_success() => Ok(()),
        Resolution::Done(outcome) => match outcome.verdict.error_kind() {
            Some(FailureKind::InvalidInput) => Err(RebornServicesError {
                code: RebornServicesErrorCode::InvalidRequest,
                kind: RebornServicesErrorKind::Validation,
                status_code: 400,
                retryable: false,
                field: None,
                validation_code: Some(WebUiInboundValidationCode::InvalidValue),
            }),
            Some(FailureKind::OperationFailed) if operation_failed_is_invalid_request => {
                Err(RebornServicesError {
                    code: RebornServicesErrorCode::InvalidRequest,
                    kind: RebornServicesErrorKind::Validation,
                    status_code: 400,
                    retryable: false,
                    field: None,
                    validation_code: Some(WebUiInboundValidationCode::InvalidValue),
                })
            }
            Some(FailureKind::Authorization | FailureKind::PolicyDenied) => Err(forbidden()),
            Some(FailureKind::Backend | FailureKind::Transient | FailureKind::Unavailable) => {
                Err(unavailable(true))
            }
            _ => Err(RebornServicesError::internal_from(format!(
                "{label} capability did not complete successfully"
            ))),
        },
        Resolution::Denied(_) => Err(forbidden()),
        Resolution::Blocked(_) | Resolution::Suspended(_) => Err(unavailable(true)),
    }
}

fn parse_thread_id_for_response(
    field: &'static str,
    value: String,
) -> Result<ThreadId, WebUiV2HttpError> {
    ThreadId::new(value).map_err(|_| {
        RebornServicesError::from(WebUiInboundValidationError::new(
            field,
            WebUiInboundValidationCode::InvalidId,
        ))
        .into()
    })
}

fn outbound_preferences_forbidden() -> RebornServicesError {
    RebornServicesError {
        code: RebornServicesErrorCode::Forbidden,
        kind: RebornServicesErrorKind::ParticipantDenied,
        status_code: 403,
        retryable: false,
        field: None,
        validation_code: None,
    }
}

fn outbound_preferences_unavailable(retryable: bool) -> RebornServicesError {
    RebornServicesError {
        code: RebornServicesErrorCode::Unavailable,
        kind: RebornServicesErrorKind::ServiceUnavailable,
        status_code: 503,
        retryable,
        field: None,
        validation_code: None,
    }
}

/// `GET /api/webchat/v2/outbound/targets`
pub async fn list_outbound_delivery_targets(
    State(state): State<WebUiV2State>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
) -> Result<Json<RebornOutboundDeliveryTargetListResponse>, WebUiV2HttpError> {
    let response = query_product_view(
        state.services(),
        caller,
        OUTBOUND_DELIVERY_TARGETS_VIEW.descriptor(),
        serde_json::json!({}),
        None,
    )
    .await?;
    Ok(Json(response))
}

/// `GET /api/webchat/v2/extensions`
pub async fn list_extensions(
    State(state): State<WebUiV2State>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
) -> Result<Json<RebornExtensionListResponse>, WebUiV2HttpError> {
    let response = query_product_view(
        state.services(),
        caller,
        EXTENSIONS_VIEW.descriptor(),
        serde_json::json!({}),
        None,
    )
    .await?;
    Ok(Json(response))
}

/// `GET /api/webchat/v2/skills`
pub async fn list_skills(
    State(state): State<WebUiV2State>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
) -> Result<Json<RebornSkillListResponse>, WebUiV2HttpError> {
    let response = query_product_view(
        state.services(),
        caller,
        SKILLS_VIEW.descriptor(),
        serde_json::json!({}),
        None,
    )
    .await?;
    Ok(Json(response))
}

/// `POST /api/webchat/v2/skills/search`
pub async fn search_skills(
    State(state): State<WebUiV2State>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
    Json(body): Json<SearchSkillsBody>,
) -> Result<Json<RebornSkillSearchResponse>, WebUiV2HttpError> {
    let response = query_product_view(
        state.services(),
        caller,
        SKILL_SEARCH_VIEW.descriptor(),
        serde_json::json!({ "query": body.query }),
        None,
    )
    .await?;
    Ok(Json(response))
}

/// `POST /api/webchat/v2/skills/install`
pub async fn install_skill(
    State(state): State<WebUiV2State>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
    Json(body): Json<InstallSkillBody>,
) -> Result<Json<RebornSkillActionResponse>, WebUiV2HttpError> {
    let name = body.name;
    let resolution = invoke_product_capability(
        state.services(),
        caller,
        SKILL_INSTALL_CAPABILITY,
        serde_json::json!({
            "name": name.clone(),
            "content": body.content,
        }),
    )
    .await?;
    skill_mutation_succeeded(resolution)?;
    Ok(Json(RebornSkillActionResponse {
        success: true,
        message: format!("Skill '{name}' installed"),
    }))
}

/// `GET /api/webchat/v2/skills/{name}`
pub async fn get_skill_content(
    State(state): State<WebUiV2State>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
    Path(SkillPath { name }): Path<SkillPath>,
) -> Result<Json<RebornSkillContentResponse>, WebUiV2HttpError> {
    let response = SKILL_CONTENT_VIEW
        .query_on(
            state.services().as_ref(),
            caller,
            serde_json::json!({ "name": name }),
            None,
        )
        .await?;
    Ok(Json(response))
}

/// `PUT /api/webchat/v2/skills/{name}`
pub async fn update_skill(
    State(state): State<WebUiV2State>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
    Path(SkillPath { name }): Path<SkillPath>,
    Json(body): Json<UpdateSkillBody>,
) -> Result<Json<RebornSkillActionResponse>, WebUiV2HttpError> {
    let resolution = invoke_product_capability(
        state.services(),
        caller,
        SKILL_UPDATE_CAPABILITY,
        serde_json::json!({
            "name": name.clone(),
            "content": body.content,
        }),
    )
    .await?;
    skill_mutation_succeeded(resolution)?;
    Ok(Json(RebornSkillActionResponse {
        success: true,
        message: format!("Skill '{name}' updated"),
    }))
}

/// `DELETE /api/webchat/v2/skills/{name}`
pub async fn remove_skill(
    State(state): State<WebUiV2State>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
    Path(SkillPath { name }): Path<SkillPath>,
) -> Result<Json<RebornSkillActionResponse>, WebUiV2HttpError> {
    let resolution = invoke_product_capability(
        state.services(),
        caller,
        SKILL_REMOVE_CAPABILITY,
        serde_json::json!({ "name": name.clone() }),
    )
    .await?;
    skill_mutation_succeeded(resolution)?;
    Ok(Json(RebornSkillActionResponse {
        success: true,
        message: format!("Skill '{name}' removed"),
    }))
}

/// `POST /api/webchat/v2/skills/{name}/auto-activate`
pub async fn set_skill_auto_activate(
    State(state): State<WebUiV2State>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
    Path(SkillPath { name }): Path<SkillPath>,
    Json(body): Json<SetSkillAutoActivateBody>,
) -> Result<Json<RebornSkillActionResponse>, WebUiV2HttpError> {
    let enabled = body.enabled;
    let resolution = invoke_product_capability(
        state.services(),
        caller,
        SKILL_AUTO_ACTIVATE_SET_CAPABILITY,
        serde_json::json!({
            "name": name.clone(),
            "enabled": enabled,
        }),
    )
    .await?;
    skill_mutation_succeeded(resolution)?;
    Ok(Json(RebornSkillActionResponse {
        success: true,
        message: format!(
            "Skill '{}' auto-activation {}",
            name,
            if enabled { "enabled" } else { "disabled" }
        ),
    }))
}

/// `POST /api/webchat/v2/skills/auto-activate-learned`
pub async fn set_auto_activate_learned(
    State(state): State<WebUiV2State>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
    Json(body): Json<SetSkillAutoActivateBody>,
) -> Result<Json<RebornSkillActionResponse>, WebUiV2HttpError> {
    let enabled = body.enabled;
    let resolution = invoke_product_capability(
        state.services(),
        caller,
        SKILL_AUTO_ACTIVATE_LEARNED_SET_CAPABILITY,
        serde_json::json!({ "enabled": enabled }),
    )
    .await?;
    skill_mutation_succeeded(resolution)?;
    Ok(Json(RebornSkillActionResponse {
        success: true,
        message: format!(
            "Default skill auto-activation {}",
            if enabled { "enabled" } else { "disabled" }
        ),
    }))
}

fn skill_mutation_succeeded(resolution: Resolution) -> Result<(), RebornServicesError> {
    match resolution {
        Resolution::Done(outcome) if outcome.verdict.is_success() => Ok(()),
        Resolution::Done(outcome) => match outcome.verdict.error_kind() {
            Some(FailureKind::InvalidInput | FailureKind::OperationFailed) => {
                Err(RebornServicesError {
                    code: RebornServicesErrorCode::InvalidRequest,
                    kind: RebornServicesErrorKind::Validation,
                    status_code: 400,
                    retryable: false,
                    field: None,
                    validation_code: Some(WebUiInboundValidationCode::InvalidValue),
                })
            }
            Some(FailureKind::Authorization | FailureKind::PolicyDenied) => {
                Err(skill_mutation_forbidden())
            }
            Some(FailureKind::Backend | FailureKind::Transient | FailureKind::Unavailable) => {
                Err(skill_mutation_unavailable(true))
            }
            _ => Err(RebornServicesError::internal_from(
                "skill capability did not complete successfully",
            )),
        },
        Resolution::Denied(_) => Err(skill_mutation_forbidden()),
        Resolution::Blocked(_) | Resolution::Suspended(_) => Err(skill_mutation_unavailable(true)),
    }
}

fn skill_mutation_forbidden() -> RebornServicesError {
    RebornServicesError {
        code: RebornServicesErrorCode::Forbidden,
        kind: RebornServicesErrorKind::ParticipantDenied,
        status_code: 403,
        retryable: false,
        field: None,
        validation_code: None,
    }
}

fn skill_mutation_unavailable(retryable: bool) -> RebornServicesError {
    RebornServicesError {
        code: RebornServicesErrorCode::Unavailable,
        kind: RebornServicesErrorKind::ServiceUnavailable,
        status_code: 503,
        retryable,
        field: None,
        validation_code: None,
    }
}

/// `GET /api/webchat/v2/extensions/registry`
pub async fn list_extension_registry(
    State(state): State<WebUiV2State>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
) -> Result<Json<RebornExtensionRegistryResponse>, WebUiV2HttpError> {
    let response = EXTENSION_REGISTRY_VIEW
        .query_on(
            state.services().as_ref(),
            caller,
            serde_json::json!({}),
            None,
        )
        .await?;
    Ok(Json(response))
}

/// `POST /api/webchat/v2/extensions/install`
pub async fn install_extension(
    State(state): State<WebUiV2State>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
    Json(body): Json<InstallExtensionBody>,
) -> Result<Json<RebornExtensionActionResponse>, WebUiV2HttpError> {
    let package_ref = extension_package_ref_for_request(Ok(body.package_ref), "package_ref")?;
    let client_action_id =
        parse_webui_client_action_id(body.client_action_id).map_err(RebornServicesError::from)?;
    let activity_id = extension_lifecycle_activity_id(
        &caller,
        EXTENSION_INSTALL_CAPABILITY,
        &package_ref,
        &client_action_id,
    )?;
    let resolution = invoke_product_capability_with_activity_id(
        state.services(),
        caller,
        EXTENSION_INSTALL_CAPABILITY,
        serde_json::json!({ "extension_id": package_ref.id.as_str() }),
        activity_id,
    )
    .await?;
    extension_lifecycle_mutation_succeeded(resolution)?;
    let response = extension_action_completed("Extension installed.", None);
    Ok(Json(response))
}

/// `POST /api/webchat/v2/extensions/import` — admin-only: upload a standalone
/// tool bundle (a zip with manifest.toml + wasm/ + schemas/ + prompts/). The
/// bundle is unpacked, validated, written under `/system/extensions/<id>/`, and
/// added to the Registry. Gated on `operator_webui_config` (admin).
pub async fn import_extension(
    State(state): State<WebUiV2State>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
    Extension(capabilities): Extension<WebUiV2Capabilities>,
    body: axum::body::Bytes,
) -> Result<Json<RebornExtensionActionResponse>, WebUiV2HttpError> {
    require_operator_webui_config(capabilities)?;
    let resolution = invoke_product_capability(
        state.services(),
        caller,
        EXTENSION_IMPORT_CAPABILITY,
        serde_json::json!({ "bundle_base64": STANDARD.encode(body.as_ref()) }),
    )
    .await?;
    extension_lifecycle_mutation_succeeded(resolution)?;
    let response = extension_action_completed("Extension imported.", None);
    Ok(Json(response))
}

/// `POST /api/webchat/v2/extensions/{package_id}/activate`
pub async fn activate_extension(
    State(state): State<WebUiV2State>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
    Path(ExtensionPackagePath { package_id }): Path<ExtensionPackagePath>,
    Json(body): Json<ExtensionLifecycleActionBody>,
) -> Result<Json<RebornExtensionActionResponse>, WebUiV2HttpError> {
    let package_ref = extension_package_ref_for_request(
        LifecyclePackageRef::new(LifecyclePackageKind::Extension, package_id),
        "package_id",
    )?;
    let client_action_id =
        parse_webui_client_action_id(body.client_action_id).map_err(RebornServicesError::from)?;
    let activity_id = extension_lifecycle_activity_id(
        &caller,
        EXTENSION_ACTIVATE_CAPABILITY,
        &package_ref,
        &client_action_id,
    )?;
    let resolution = invoke_product_capability_with_activity_id(
        state.services(),
        caller.clone(),
        EXTENSION_ACTIVATE_CAPABILITY,
        serde_json::json!({ "extension_id": package_ref.id.as_str() }),
        activity_id,
    )
    .await?;
    let response =
        extension_activation_response(state.services(), caller, package_ref, resolution).await?;
    Ok(Json(response))
}

/// `POST /api/webchat/v2/extensions/{package_id}/remove`
pub async fn remove_extension(
    State(state): State<WebUiV2State>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
    Path(ExtensionPackagePath { package_id }): Path<ExtensionPackagePath>,
    Json(body): Json<ExtensionLifecycleActionBody>,
) -> Result<Json<RebornExtensionActionResponse>, WebUiV2HttpError> {
    let package_ref = extension_package_ref_for_request(
        LifecyclePackageRef::new(LifecyclePackageKind::Extension, package_id),
        "package_id",
    )?;
    let client_action_id =
        parse_webui_client_action_id(body.client_action_id).map_err(RebornServicesError::from)?;
    let activity_id = extension_lifecycle_activity_id(
        &caller,
        EXTENSION_REMOVE_CAPABILITY,
        &package_ref,
        &client_action_id,
    )?;
    let resolution = invoke_product_capability_with_activity_id(
        state.services(),
        caller,
        EXTENSION_REMOVE_CAPABILITY,
        serde_json::json!({ "extension_id": package_ref.id.as_str() }),
        activity_id,
    )
    .await?;
    extension_lifecycle_mutation_succeeded(resolution)?;
    let response = extension_action_completed("Extension removed.", None);
    Ok(Json(response))
}

fn extension_lifecycle_mutation_succeeded(
    resolution: Resolution,
) -> Result<(), RebornServicesError> {
    match resolution {
        Resolution::Done(outcome) if outcome.verdict.is_success() => Ok(()),
        Resolution::Done(outcome) => match outcome.verdict.error_kind() {
            Some(FailureKind::InvalidInput) => Err(RebornServicesError {
                code: RebornServicesErrorCode::InvalidRequest,
                kind: RebornServicesErrorKind::Validation,
                status_code: 400,
                retryable: false,
                field: None,
                validation_code: Some(WebUiInboundValidationCode::InvalidValue),
            }),
            Some(FailureKind::OperationFailed) => Err(RebornServicesError {
                code: RebornServicesErrorCode::InvalidRequest,
                kind: RebornServicesErrorKind::Validation,
                status_code: 400,
                retryable: false,
                field: None,
                validation_code: None,
            }),
            Some(FailureKind::Authorization | FailureKind::PolicyDenied) => {
                Err(extension_lifecycle_forbidden())
            }
            Some(FailureKind::Backend | FailureKind::Transient | FailureKind::Unavailable) => {
                Err(extension_lifecycle_unavailable(true))
            }
            _ => Err(RebornServicesError::internal_from(
                "extension lifecycle capability did not complete successfully",
            )),
        },
        Resolution::Denied(_) => Err(extension_lifecycle_forbidden()),
        Resolution::Blocked(_) | Resolution::Suspended(_) => {
            Err(extension_lifecycle_unavailable(true))
        }
    }
}

async fn extension_activation_response(
    services: &std::sync::Arc<dyn ProductSurface>,
    caller: WebUiAuthenticatedCaller,
    package_ref: LifecyclePackageRef,
    resolution: Resolution,
) -> Result<RebornExtensionActionResponse, RebornServicesError> {
    match resolution {
        Resolution::Done(outcome) if outcome.verdict.is_success() => {
            let activated = query_extension_active_state(services, caller, &package_ref).await?;
            Ok(extension_action_completed(
                "Extension activated.",
                Some(activated),
            ))
        }
        Resolution::Blocked(Blocked::Auth(_)) => Ok(RebornExtensionActionResponse {
            success: true,
            message: "Extension authentication required.".to_string(),
            activated: Some(false),
            auth_url: None,
            awaiting_token: None,
            instructions: Some(
                "Configure the extension credentials, then activate it again.".to_string(),
            ),
            onboarding_state: Some(RebornExtensionOnboardingState::AuthRequired),
            onboarding: None,
        }),
        other => {
            extension_lifecycle_mutation_succeeded(other)?;
            Ok(extension_action_completed(
                "Extension activated.",
                Some(false),
            ))
        }
    }
}

async fn query_extension_active_state(
    services: &std::sync::Arc<dyn ProductSurface>,
    caller: WebUiAuthenticatedCaller,
    package_ref: &LifecyclePackageRef,
) -> Result<bool, RebornServicesError> {
    let page = services
        .query(
            caller,
            RebornViewQuery {
                view_id: EXTENSIONS_VIEW.id.to_string(),
                params: serde_json::json!({}),
                cursor: None,
            },
        )
        .await?;
    let response: RebornExtensionListResponse =
        serde_json::from_value(page.payload).map_err(RebornServicesError::internal_from)?;
    response
        .extensions
        .iter()
        .find(|extension| extension.package_ref == *package_ref)
        .map(|extension| extension.active)
        .ok_or_else(|| extension_lifecycle_unavailable(true))
}

fn extension_lifecycle_forbidden() -> RebornServicesError {
    RebornServicesError {
        code: RebornServicesErrorCode::Forbidden,
        kind: RebornServicesErrorKind::ParticipantDenied,
        status_code: 403,
        retryable: false,
        field: None,
        validation_code: None,
    }
}

fn extension_lifecycle_unavailable(retryable: bool) -> RebornServicesError {
    RebornServicesError {
        code: RebornServicesErrorCode::Unavailable,
        kind: RebornServicesErrorKind::ServiceUnavailable,
        status_code: 503,
        retryable,
        field: None,
        validation_code: None,
    }
}

fn extension_action_completed(
    message: impl Into<String>,
    activated: Option<bool>,
) -> RebornExtensionActionResponse {
    RebornExtensionActionResponse {
        success: true,
        message: message.into(),
        activated,
        auth_url: None,
        awaiting_token: None,
        instructions: None,
        onboarding_state: None,
        onboarding: None,
    }
}

/// `GET /api/webchat/v2/extensions/{package_id}/setup`
pub async fn get_extension_setup(
    State(state): State<WebUiV2State>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
    Path(ExtensionPackagePath { package_id }): Path<ExtensionPackagePath>,
) -> Result<Json<RebornSetupExtensionResponse>, WebUiV2HttpError> {
    let package_ref = extension_package_ref_for_request(
        LifecyclePackageRef::new(LifecyclePackageKind::Extension, package_id),
        "package_id",
    )?;
    let page = state
        .services()
        .query(
            caller,
            RebornViewQuery {
                view_id: EXTENSION_SETUP_VIEW.id.to_string(),
                params: serde_json::json!({ "package_id": package_ref.id.as_str() }),
                cursor: None,
            },
        )
        .await?;
    let response =
        serde_json::from_value(page.payload).map_err(RebornServicesError::internal_from)?;
    Ok(Json(response))
}

/// `POST /api/webchat/v2/extensions/{package_id}/setup`
///
/// V2-native route that returns a product-safe lifecycle projection. The route
/// exists so the v2 entrypoint inventory is complete and so future onboarding
/// port work has a stable surface to fill in without coupling this crate to v1
/// onboarding controllers.
///
/// The path segment is lifted into a lifecycle package ref at the
/// handler/facade boundary; a malformed identifier returns `400
/// invalid_argument` before the facade is called.
pub async fn setup_extension(
    State(state): State<WebUiV2State>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
    Path(ExtensionPackagePath { package_id }): Path<ExtensionPackagePath>,
    Json(body): Json<WebUiSetupExtensionRequest>,
) -> Result<Json<RebornSetupExtensionResponse>, WebUiV2HttpError> {
    let package_ref = extension_package_ref_for_request(
        LifecyclePackageRef::new(LifecyclePackageKind::Extension, package_id),
        "package_id",
    )?;
    let client_action_id = parse_webui_client_action_id(body.client_action_id.clone())
        .map_err(RebornServicesError::from)?;
    let activity_id = extension_lifecycle_activity_id(
        &caller,
        EXTENSION_SETUP_SUBMIT_CAPABILITY,
        &package_ref,
        &client_action_id,
    )?;
    let mut input = serde_json::to_value(body).map_err(RebornServicesError::internal_from)?;
    let input_object = input
        .as_object_mut()
        .ok_or_else(|| RebornServicesError::internal_from("setup request did not encode object"))?;
    input_object.remove("client_action_id");
    input_object.insert(
        "extension_id".to_string(),
        serde_json::Value::String(package_ref.id.as_str().to_string()),
    );
    let resolution = invoke_product_capability_with_activity_id(
        state.services(),
        caller.clone(),
        EXTENSION_SETUP_SUBMIT_CAPABILITY,
        input,
        activity_id,
    )
    .await?;
    extension_lifecycle_mutation_succeeded(resolution)?;
    let page = state
        .services()
        .query(
            caller,
            RebornViewQuery {
                view_id: EXTENSION_SETUP_VIEW.id.to_string(),
                params: serde_json::json!({ "package_id": package_ref.id.as_str() }),
                cursor: None,
            },
        )
        .await?;
    let response =
        serde_json::from_value(page.payload).map_err(RebornServicesError::internal_from)?;
    Ok(Json(response))
}

fn require_operator_webui_config(
    capabilities: WebUiV2Capabilities,
) -> Result<(), WebUiV2HttpError> {
    if capabilities.operator_webui_config {
        return Ok(());
    }
    Err(RebornServicesError {
        code: RebornServicesErrorCode::Forbidden,
        kind: RebornServicesErrorKind::ParticipantDenied,
        status_code: 403,
        retryable: false,
        field: None,
        validation_code: None,
    }
    .into())
}

#[derive(Deserialize)]
pub struct ExtensionAdminConfigurationPath {
    pub group_id: String,
}

#[derive(Deserialize, Serialize)]
pub struct ExtensionAdminConfigurationValue {
    pub handle: String,
    pub value: String,
}

#[derive(Deserialize)]
pub struct ReplaceExtensionAdminConfigurationBody {
    pub values: Vec<ExtensionAdminConfigurationValue>,
    pub expected_revision: u64,
    pub idempotency_key: String,
}

#[derive(Serialize)]
struct ReplaceExtensionAdminConfigurationInput {
    group_id: String,
    values: Vec<ExtensionAdminConfigurationValue>,
    expected_revision: u64,
}

/// `GET /api/webchat/v2/operator/extension-configuration`
pub async fn list_extension_admin_configuration(
    State(state): State<WebUiV2State>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
    Extension(capabilities): Extension<WebUiV2Capabilities>,
) -> Result<Json<serde_json::Value>, WebUiV2HttpError> {
    require_operator_webui_config(capabilities)?;
    let payload = query_extension_admin_configuration(&state, caller).await?;
    Ok(Json(payload))
}

/// `PUT /api/webchat/v2/operator/extension-configuration/{group_id}`
///
/// This ingress adapter carries only the manifest group designator and values
/// through the generic product capability conduit. The client retry key is
/// consumed here into a scoped [`ActivityId`]; it never enters capability
/// input as authority.
pub async fn replace_extension_admin_configuration(
    State(state): State<WebUiV2State>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
    Extension(capabilities): Extension<WebUiV2Capabilities>,
    Path(path): Path<ExtensionAdminConfigurationPath>,
    Json(body): Json<ReplaceExtensionAdminConfigurationBody>,
) -> Result<Json<serde_json::Value>, WebUiV2HttpError> {
    require_operator_webui_config(capabilities)?;
    let activity_id =
        admin_configuration_activity_id(&caller, &path.group_id, &body.idempotency_key)?;
    let expected_revision = body.expected_revision;
    let resolution = invoke_product_capability_with_activity_id(
        state.services(),
        caller.clone(),
        ADMIN_CONFIGURATION_REPLACE_CAPABILITY,
        ReplaceExtensionAdminConfigurationInput {
            group_id: path.group_id.clone(),
            values: body.values,
            expected_revision,
        },
        activity_id,
    )
    .await?;

    match resolution {
        Resolution::Done(outcome) => {
            let payload = query_extension_admin_configuration(&state, caller).await?;
            let group = select_extension_admin_configuration_group(&payload, &path.group_id)?;
            if outcome.verdict.is_success() {
                return Ok(Json(group));
            }

            // The generic runtime failure taxonomy deliberately has no HTTP
            // conflict variant. The authoritative query is the typed CAS
            // witness: a failed replacement whose active revision moved past
            // the submitted revision is a 409, without parsing prose.
            let current_revision = group
                .get("revision")
                .and_then(serde_json::Value::as_u64)
                .ok_or_else(|| {
                    RebornServicesError::internal_from(
                        "admin configuration view omitted a numeric revision",
                    )
                })?;
            if current_revision != expected_revision {
                return Err(admin_configuration_conflict().into());
            }
            Err(admin_configuration_done_failure(outcome.verdict.error_kind()).into())
        }
        Resolution::Denied(_) => Err(RebornServicesError {
            code: RebornServicesErrorCode::Forbidden,
            kind: RebornServicesErrorKind::ParticipantDenied,
            status_code: 403,
            retryable: false,
            field: None,
            validation_code: None,
        }
        .into()),
        Resolution::Blocked(blocked) => Err(admin_configuration_blocked(blocked).into()),
        Resolution::Suspended(_) => Err(admin_configuration_unavailable(true).into()),
    }
}

async fn invoke_product_capability<T>(
    services: &std::sync::Arc<dyn ProductSurface>,
    caller: WebUiAuthenticatedCaller,
    capability: ProductCapabilityDescriptor,
    input: T,
) -> Result<Resolution, RebornServicesError>
where
    T: Serialize,
{
    let input = serde_json::to_value(input).map_err(RebornServicesError::internal_from)?;
    let activity_id = generic_product_capability_activity_id();
    invoke_product_capability_with_activity_id(services, caller, capability, input, activity_id)
        .await
}

async fn invoke_product_capability_with_activity_id<T>(
    services: &std::sync::Arc<dyn ProductSurface>,
    caller: WebUiAuthenticatedCaller,
    capability: ProductCapabilityDescriptor,
    input: T,
    activity_id: ActivityId,
) -> Result<Resolution, RebornServicesError>
where
    T: Serialize,
{
    capability
        .invoke_on(services.as_ref(), caller, input, activity_id)
        .await
}

fn generic_product_capability_activity_id() -> ActivityId {
    ActivityId::new()
}

fn llm_provider_upsert_activity_id(
    caller: &WebUiAuthenticatedCaller,
    client_action_id: &IdempotencyKey,
) -> Result<ActivityId, RebornServicesError> {
    let capability_id = LLM_PROVIDER_UPSERT_CAPABILITY.capability_id()?;
    let mut seed = Vec::new();
    for segment in [
        "webui-product-capability",
        caller.tenant_id.as_str(),
        caller.user_id.as_str(),
        caller.agent_id.as_ref().map(|id| id.as_str()).unwrap_or(""),
        caller
            .project_id
            .as_ref()
            .map(|id| id.as_str())
            .unwrap_or(""),
        capability_id.as_str(),
        client_action_id.as_str(),
    ] {
        seed.extend_from_slice(&(segment.len() as u64).to_be_bytes());
        seed.extend_from_slice(segment.as_bytes());
    }
    Ok(ActivityId::from_uuid(Uuid::new_v5(
        &Uuid::NAMESPACE_OID,
        &seed,
    )))
}

fn extension_lifecycle_activity_id(
    caller: &WebUiAuthenticatedCaller,
    capability: ProductCapabilityDescriptor,
    package_ref: &LifecyclePackageRef,
    client_action_id: &IdempotencyKey,
) -> Result<ActivityId, RebornServicesError> {
    let capability_id = capability.capability_id()?;
    let mut seed = Vec::new();
    for segment in [
        "webui-extension-lifecycle",
        caller.tenant_id.as_str(),
        caller.user_id.as_str(),
        caller.agent_id.as_ref().map(|id| id.as_str()).unwrap_or(""),
        caller
            .project_id
            .as_ref()
            .map(|id| id.as_str())
            .unwrap_or(""),
        capability_id.as_str(),
        package_ref.id.as_str(),
        client_action_id.as_str(),
    ] {
        seed.extend_from_slice(&(segment.len() as u64).to_be_bytes());
        seed.extend_from_slice(segment.as_bytes());
    }
    Ok(ActivityId::from_uuid(Uuid::new_v5(
        &Uuid::NAMESPACE_OID,
        &seed,
    )))
}

fn outbound_preferences_activity_id(
    caller: &WebUiAuthenticatedCaller,
    client_action_id: &IdempotencyKey,
) -> Result<ActivityId, RebornServicesError> {
    let capability_id = OUTBOUND_PREFERENCES_SET_CAPABILITY.capability_id()?;
    let mut seed = Vec::new();
    for segment in [
        "webui-outbound-preferences",
        caller.tenant_id.as_str(),
        caller.user_id.as_str(),
        caller.agent_id.as_ref().map(|id| id.as_str()).unwrap_or(""),
        caller
            .project_id
            .as_ref()
            .map(|id| id.as_str())
            .unwrap_or(""),
        capability_id.as_str(),
        client_action_id.as_str(),
    ] {
        seed.extend_from_slice(&(segment.len() as u64).to_be_bytes());
        seed.extend_from_slice(segment.as_bytes());
    }
    Ok(ActivityId::from_uuid(Uuid::new_v5(
        &Uuid::NAMESPACE_OID,
        &seed,
    )))
}

async fn query_product_view<T>(
    services: &std::sync::Arc<dyn ProductSurface>,
    caller: WebUiAuthenticatedCaller,
    view: RebornViewDescriptor,
    params: serde_json::Value,
    cursor: Option<String>,
) -> Result<T, RebornServicesError>
where
    T: DeserializeOwned,
{
    let page = services
        .query(
            caller,
            RebornViewQuery {
                view_id: view.id.to_string(),
                params,
                cursor,
            },
        )
        .await?;
    serde_json::from_value(page.payload).map_err(RebornServicesError::internal_from)
}

async fn query_extension_admin_configuration(
    state: &WebUiV2State,
    caller: WebUiAuthenticatedCaller,
) -> Result<serde_json::Value, RebornServicesError> {
    let page = state
        .services()
        .query(
            caller,
            RebornViewQuery {
                view_id: ADMIN_CONFIGURATION_VIEW.id.to_string(),
                params: serde_json::json!({}),
                cursor: None,
            },
        )
        .await?;
    if !page
        .payload
        .get("groups")
        .is_some_and(serde_json::Value::is_array)
    {
        return Err(RebornServicesError::internal_from(
            "admin configuration view returned an invalid list payload",
        ));
    }
    Ok(page.payload)
}

fn select_extension_admin_configuration_group(
    payload: &serde_json::Value,
    group_id: &str,
) -> Result<serde_json::Value, RebornServicesError> {
    payload
        .get("groups")
        .and_then(serde_json::Value::as_array)
        .and_then(|groups| {
            groups.iter().find(|group| {
                group.get("group_id").and_then(serde_json::Value::as_str) == Some(group_id)
            })
        })
        .cloned()
        .ok_or_else(RebornServicesError::not_found)
}

fn admin_configuration_activity_id(
    caller: &WebUiAuthenticatedCaller,
    group_id: &str,
    idempotency_key: &str,
) -> Result<ActivityId, RebornServicesError> {
    let validation_code = if idempotency_key.is_empty() {
        Some(WebUiInboundValidationCode::Blank)
    } else if idempotency_key.len() > ADMIN_CONFIGURATION_IDEMPOTENCY_KEY_MAX_BYTES {
        Some(WebUiInboundValidationCode::TooLong)
    } else if idempotency_key.trim() != idempotency_key {
        Some(WebUiInboundValidationCode::InvalidId)
    } else if idempotency_key.chars().any(char::is_control) {
        Some(WebUiInboundValidationCode::InvalidControlCharacter)
    } else {
        None
    };
    if let Some(validation_code) = validation_code {
        return Err(RebornServicesError {
            code: RebornServicesErrorCode::InvalidRequest,
            kind: RebornServicesErrorKind::Validation,
            status_code: 400,
            retryable: false,
            field: Some("idempotency_key".to_string()),
            validation_code: Some(validation_code),
        });
    }

    let mut seed = Vec::new();
    for segment in [
        "webui-extension-admin-configuration",
        caller.tenant_id.as_str(),
        caller.user_id.as_str(),
        caller.agent_id.as_ref().map(|id| id.as_str()).unwrap_or(""),
        caller
            .project_id
            .as_ref()
            .map(|id| id.as_str())
            .unwrap_or(""),
        group_id,
        idempotency_key,
    ] {
        seed.extend_from_slice(&(segment.len() as u64).to_be_bytes());
        seed.extend_from_slice(segment.as_bytes());
    }
    Ok(ActivityId::from_uuid(Uuid::new_v5(
        &Uuid::NAMESPACE_OID,
        &seed,
    )))
}

fn admin_configuration_conflict() -> RebornServicesError {
    RebornServicesError {
        code: RebornServicesErrorCode::Conflict,
        kind: RebornServicesErrorKind::Conflict,
        status_code: 409,
        retryable: false,
        field: Some("expected_revision".to_string()),
        validation_code: None,
    }
}

fn admin_configuration_unavailable(retryable: bool) -> RebornServicesError {
    RebornServicesError {
        code: RebornServicesErrorCode::Unavailable,
        kind: RebornServicesErrorKind::ServiceUnavailable,
        status_code: 503,
        retryable,
        field: None,
        validation_code: None,
    }
}

fn admin_configuration_done_failure(error_kind: Option<&FailureKind>) -> RebornServicesError {
    match error_kind {
        Some(FailureKind::InvalidInput) => RebornServicesError {
            code: RebornServicesErrorCode::InvalidRequest,
            kind: RebornServicesErrorKind::Validation,
            status_code: 400,
            retryable: false,
            field: None,
            validation_code: Some(WebUiInboundValidationCode::InvalidValue),
        },
        Some(
            FailureKind::Backend
            | FailureKind::Network
            | FailureKind::Resource
            | FailureKind::Transient
            | FailureKind::Unavailable,
        ) => admin_configuration_unavailable(true),
        Some(
            FailureKind::Authorization | FailureKind::PolicyDenied | FailureKind::GateDeclined,
        ) => RebornServicesError {
            code: RebornServicesErrorCode::Forbidden,
            kind: RebornServicesErrorKind::ParticipantDenied,
            status_code: 403,
            retryable: false,
            field: None,
            validation_code: None,
        },
        Some(
            FailureKind::Cancelled
            | FailureKind::Dispatcher
            | FailureKind::InvalidOutput
            | FailureKind::MissingRuntime
            | FailureKind::OperationFailed
            | FailureKind::OutputTooLarge
            | FailureKind::Process
            | FailureKind::Internal
            | FailureKind::Permanent
            | FailureKind::Unknown(_),
        )
        | None => RebornServicesError::internal(),
    }
}

fn admin_configuration_blocked(blocked: Blocked) -> RebornServicesError {
    let kind = match blocked {
        Blocked::Approval(_) => RebornServicesErrorKind::BlockedApproval,
        Blocked::Auth(_) => RebornServicesErrorKind::BlockedAuthentication,
        Blocked::Resource(_) => RebornServicesErrorKind::BlockedResource,
    };
    RebornServicesError {
        code: RebornServicesErrorCode::Conflict,
        kind,
        status_code: 409,
        retryable: true,
        field: None,
        validation_code: None,
    }
}

/// `GET /api/webchat/v2/operator/setup`
pub async fn get_operator_setup(
    State(state): State<WebUiV2State>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
    Extension(capabilities): Extension<WebUiV2Capabilities>,
) -> Result<Json<RebornOperatorSetupResponse>, WebUiV2HttpError> {
    require_operator_webui_config(capabilities)?;
    let response = query_operator_setup_response(state.services(), caller).await?;
    Ok(Json(response))
}

async fn query_operator_setup_response(
    services: &std::sync::Arc<dyn ProductSurface>,
    caller: WebUiAuthenticatedCaller,
) -> Result<RebornOperatorSetupResponse, RebornServicesError> {
    let page = services
        .query(
            caller,
            RebornViewQuery {
                view_id: OPERATOR_SETUP_VIEW.id.to_string(),
                params: serde_json::json!({}),
                cursor: None,
            },
        )
        .await?;
    serde_json::from_value(page.payload).map_err(RebornServicesError::internal_from)
}

/// `POST /api/webchat/v2/operator/setup`
pub async fn run_operator_setup(
    State(state): State<WebUiV2State>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
    Extension(capabilities): Extension<WebUiV2Capabilities>,
    Json(body): Json<serde_json::Value>,
) -> Result<Json<RebornOperatorSetupResponse>, WebUiV2HttpError> {
    require_operator_webui_config(capabilities)?;
    let resolution = invoke_product_capability(
        state.services(),
        caller.clone(),
        OPERATOR_SETUP_RUN_CAPABILITY,
        body,
    )
    .await?;
    capability_resolution_succeeded(
        resolution,
        "llm config",
        true,
        extension_lifecycle_forbidden,
        extension_lifecycle_unavailable,
    )?;
    let response = query_operator_setup_response(state.services(), caller).await?;
    Ok(Json(response))
}

/// `GET /api/webchat/v2/settings/tools`
pub async fn list_settings_tools(
    State(state): State<WebUiV2State>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
    Extension(_capabilities): Extension<WebUiV2Capabilities>,
) -> Result<Json<RebornOperatorConfigListResponse>, WebUiV2HttpError> {
    let page = state
        .services()
        .query(
            caller,
            RebornViewQuery {
                view_id: OPERATOR_CONFIG_LIST_VIEW.id.to_string(),
                params: serde_json::json!({}),
                cursor: None,
            },
        )
        .await?;
    let mut response: RebornOperatorConfigListResponse =
        serde_json::from_value(page.payload).map_err(RebornServicesError::internal_from)?;
    response.entries.retain(|entry| {
        entry.key == SETTINGS_TOOLS_AUTO_APPROVE_KEY
            || entry.key.starts_with(SETTINGS_TOOL_CONFIG_PREFIX)
    });
    Ok(Json(response))
}

#[derive(Debug, Deserialize)]
pub struct SettingsToolsAutoApproveRequest {
    pub enabled: bool,
}

/// `POST /api/webchat/v2/settings/tools`
pub async fn set_settings_tools_auto_approve(
    State(state): State<WebUiV2State>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
    Extension(_capabilities): Extension<WebUiV2Capabilities>,
    Json(body): Json<SettingsToolsAutoApproveRequest>,
) -> Result<Json<RebornOperatorConfigGetResponse>, WebUiV2HttpError> {
    let resolution = invoke_product_capability(
        state.services(),
        caller.clone(),
        OPERATOR_CONFIG_SET_AUTO_APPROVE_CAPABILITY,
        serde_json::json!({ "enabled": body.enabled }),
    )
    .await?;
    capability_resolution_succeeded(
        resolution,
        "settings tools auto approve",
        true,
        outbound_preferences_forbidden,
        outbound_preferences_unavailable,
    )?;
    let response = query_operator_config_key_response(
        state.services(),
        caller,
        SETTINGS_TOOLS_AUTO_APPROVE_KEY.to_string(),
    )
    .await?;
    validate_settings_tool_config_response(&response)?;
    Ok(Json(response))
}

#[derive(Debug, Deserialize)]
pub struct SettingsToolPermissionPath {
    pub capability_id: String,
}

#[derive(Debug, Deserialize)]
pub struct SettingsToolPermissionRequest {
    pub state: SettingsToolPermissionState,
}

/// `POST /api/webchat/v2/settings/tools/{capability_id}`
pub async fn set_settings_tool_permission(
    State(state): State<WebUiV2State>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
    Extension(_capabilities): Extension<WebUiV2Capabilities>,
    Path(SettingsToolPermissionPath { capability_id }): Path<SettingsToolPermissionPath>,
    Json(body): Json<SettingsToolPermissionRequest>,
) -> Result<Json<RebornOperatorConfigGetResponse>, WebUiV2HttpError> {
    validate_settings_tool_capability_id(&capability_id)?;
    let key =
        validate_operator_config_key(format!("{SETTINGS_TOOL_CONFIG_PREFIX}{capability_id}"))?;
    let response = OPERATOR_CONFIG_SET_KEY_OPERATION
        .execute_on(
            state.services().as_ref(),
            caller,
            RebornOperatorConfigSetProductRequest {
                key,
                value: serde_json::json!({ "state": body.state }),
            },
        )
        .await?;
    validate_settings_tool_config_response(&response)?;
    Ok(Json(response))
}

fn validate_settings_tool_capability_id(capability_id: &str) -> Result<(), WebUiV2HttpError> {
    if capability_id.len() > SETTINGS_TOOL_CAPABILITY_ID_MAX_BYTES {
        return Err(RebornServicesError::from(WebUiInboundValidationError::new(
            "capability_id",
            WebUiInboundValidationCode::TooLong,
        ))
        .into());
    }
    Ok(())
}

fn validate_settings_tool_config_response(
    response: &RebornOperatorConfigGetResponse,
) -> Result<(), WebUiV2HttpError> {
    if response.entry.key == SETTINGS_TOOLS_AUTO_APPROVE_KEY
        || response.entry.key.starts_with(SETTINGS_TOOL_CONFIG_PREFIX)
    {
        return Ok(());
    }

    Err(RebornServicesError::from(WebUiInboundValidationError::new(
        "key",
        WebUiInboundValidationCode::InvalidValue,
    ))
    .into())
}

/// `GET /api/webchat/v2/operator/config`
pub async fn list_operator_config(
    State(state): State<WebUiV2State>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
    Extension(capabilities): Extension<WebUiV2Capabilities>,
) -> Result<Json<RebornOperatorConfigListResponse>, WebUiV2HttpError> {
    require_operator_webui_config(capabilities)?;
    let page = state
        .services()
        .query(
            caller,
            RebornViewQuery {
                view_id: OPERATOR_CONFIG_LIST_VIEW.id.to_string(),
                params: serde_json::json!({}),
                cursor: None,
            },
        )
        .await?;
    let response =
        serde_json::from_value(page.payload).map_err(RebornServicesError::internal_from)?;
    Ok(Json(response))
}

#[derive(Debug, Deserialize)]
pub struct OperatorConfigKeyPath {
    pub key: String,
}

const OPERATOR_CONFIG_KEY_MAX_BYTES: usize = 128;
const OPERATOR_CONFIG_RESERVED_VALIDATE_KEY: &str = "validate";

fn validate_operator_config_key(key: String) -> Result<String, WebUiV2HttpError> {
    let validation_code = if key.is_empty() {
        Some(WebUiInboundValidationCode::Blank)
    } else if key.len() > OPERATOR_CONFIG_KEY_MAX_BYTES {
        Some(WebUiInboundValidationCode::TooLong)
    } else if key == OPERATOR_CONFIG_RESERVED_VALIDATE_KEY {
        Some(WebUiInboundValidationCode::InvalidValue)
    } else if key.bytes().all(|byte| {
        byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'_' | b'.' | b'-')
    }) {
        None
    } else {
        Some(WebUiInboundValidationCode::InvalidValue)
    };

    match validation_code {
        None => Ok(key),
        Some(code) => Err(operator_config_key_error(code)),
    }
}

fn operator_config_key_error(code: WebUiInboundValidationCode) -> WebUiV2HttpError {
    RebornServicesError::from(WebUiInboundValidationError::new("key", code)).into()
}

async fn query_operator_config_key_response(
    services: &std::sync::Arc<dyn ProductSurface>,
    caller: WebUiAuthenticatedCaller,
    key: String,
) -> Result<RebornOperatorConfigGetResponse, RebornServicesError> {
    let page = services
        .query(
            caller,
            RebornViewQuery {
                view_id: OPERATOR_CONFIG_KEY_VIEW.id.to_string(),
                params: serde_json::json!({ "key": key }),
                cursor: None,
            },
        )
        .await?;
    serde_json::from_value(page.payload).map_err(RebornServicesError::internal_from)
}

/// `GET /api/webchat/v2/operator/config/{key}`
pub async fn get_operator_config_key(
    State(state): State<WebUiV2State>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
    Extension(capabilities): Extension<WebUiV2Capabilities>,
    Path(OperatorConfigKeyPath { key }): Path<OperatorConfigKeyPath>,
) -> Result<Json<RebornOperatorConfigGetResponse>, WebUiV2HttpError> {
    require_operator_webui_config(capabilities)?;
    let key = validate_operator_config_key(key)?;
    let response = query_operator_config_key_response(state.services(), caller, key).await?;
    Ok(Json(response))
}

/// `POST /api/webchat/v2/operator/config/{key}`
pub async fn set_operator_config_key(
    State(state): State<WebUiV2State>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
    Extension(capabilities): Extension<WebUiV2Capabilities>,
    Path(OperatorConfigKeyPath { key }): Path<OperatorConfigKeyPath>,
    Json(body): Json<RebornOperatorConfigSetRequest>,
) -> Result<Json<RebornOperatorConfigGetResponse>, WebUiV2HttpError> {
    require_operator_webui_config(capabilities)?;
    let key = validate_operator_config_key(key)?;
    let response = OPERATOR_CONFIG_SET_KEY_OPERATION
        .execute_on(
            state.services().as_ref(),
            caller,
            RebornOperatorConfigSetProductRequest {
                key,
                value: body.value,
            },
        )
        .await?;
    Ok(Json(response))
}

/// `GET /api/webchat/v2/operator/config/validate`
///
/// `validate` is reserved for the validation operation and is not a readable
/// config key. This explicit static-path handler keeps axum static route
/// priority from surfacing an ambiguous 405.
pub async fn reject_reserved_operator_config_key(
    Extension(capabilities): Extension<WebUiV2Capabilities>,
) -> Result<Json<RebornOperatorConfigGetResponse>, WebUiV2HttpError> {
    require_operator_webui_config(capabilities)?;
    Err(operator_config_key_error(
        WebUiInboundValidationCode::InvalidValue,
    ))
}

/// `POST /api/webchat/v2/operator/config/validate`
pub async fn validate_operator_config(
    State(state): State<WebUiV2State>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
    Extension(capabilities): Extension<WebUiV2Capabilities>,
    Json(body): Json<RebornOperatorConfigValidateRequest>,
) -> Result<Json<RebornOperatorConfigValidateResponse>, WebUiV2HttpError> {
    require_operator_webui_config(capabilities)?;
    let page = state
        .services()
        .query(
            caller,
            RebornViewQuery {
                view_id: OPERATOR_CONFIG_VALIDATE_VIEW.id.to_string(),
                params: serde_json::to_value(body).map_err(RebornServicesError::internal_from)?,
                cursor: None,
            },
        )
        .await?;
    let response: RebornOperatorConfigValidateResponse =
        serde_json::from_value(page.payload).map_err(RebornServicesError::internal_from)?;
    Ok(Json(response))
}

/// `GET /api/webchat/v2/operator/diagnostics`
pub async fn get_operator_diagnostics(
    State(state): State<WebUiV2State>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
    Extension(capabilities): Extension<WebUiV2Capabilities>,
) -> Result<Json<RebornOperatorCommandPlaneResponse>, WebUiV2HttpError> {
    require_operator_webui_config(capabilities)?;
    let page = state
        .services()
        .query(
            caller,
            RebornViewQuery {
                view_id: OPERATOR_DIAGNOSTICS_VIEW.id.to_string(),
                params: serde_json::json!({}),
                cursor: None,
            },
        )
        .await?;
    let response =
        serde_json::from_value(page.payload).map_err(RebornServicesError::internal_from)?;
    Ok(Json(response))
}

/// `GET /api/webchat/v2/operator/status`
pub async fn get_operator_status(
    State(state): State<WebUiV2State>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
    Extension(capabilities): Extension<WebUiV2Capabilities>,
) -> Result<Json<RebornOperatorCommandPlaneResponse>, WebUiV2HttpError> {
    require_operator_webui_config(capabilities)?;
    let page = state
        .services()
        .query(
            caller,
            RebornViewQuery {
                view_id: OPERATOR_STATUS_VIEW.id.to_string(),
                params: serde_json::json!({}),
                cursor: None,
            },
        )
        .await?;
    let response =
        serde_json::from_value(page.payload).map_err(RebornServicesError::internal_from)?;
    Ok(Json(response))
}

/// `GET /api/webchat/v2/operator/logs`
///
/// Operator-gated version of the logs projection. The non-operator
/// projection lives at `GET /api/webchat/v2/logs`.
pub async fn query_operator_logs(
    State(state): State<WebUiV2State>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
    Extension(capabilities): Extension<WebUiV2Capabilities>,
    Query(mut query): Query<RebornOperatorLogsQuery>,
) -> Result<Json<RebornOperatorCommandPlaneResponse>, WebUiV2HttpError> {
    require_operator_webui_config(capabilities)?;
    let cursor = query.cursor.take();
    let params = serde_json::to_value(query).map_err(RebornServicesError::internal_from)?;
    let page = state
        .services()
        .query(
            caller,
            RebornViewQuery {
                view_id: OPERATOR_LOGS_VIEW.id.to_string(),
                params,
                cursor,
            },
        )
        .await?;
    let response =
        serde_json::from_value(page.payload).map_err(RebornServicesError::internal_from)?;
    Ok(Json(response))
}

/// `GET /api/webchat/v2/logs`
///
/// Read-only caller-scoped logs projection for non-operator WebUI sessions.
/// The operator-wide log surface remains `GET /api/webchat/v2/operator/logs`.
pub async fn query_logs(
    State(state): State<WebUiV2State>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
    Query(query): Query<RebornOperatorLogsQuery>,
) -> Result<Json<RebornLogQueryResponse>, WebUiV2HttpError> {
    // The public and operator HTTP query strings intentionally share fields;
    // convert at the handler boundary so the facade can enforce public scope.
    let mut request = RebornLogQueryRequest {
        limit: query.limit,
        cursor: query.cursor,
        level: query.level,
        target: query.target,
        thread_id: query.thread_id,
        run_id: query.run_id,
        turn_id: query.turn_id,
        tool_call_id: query.tool_call_id,
        tool_name: query.tool_name,
        source: query.source,
        tail: query.tail,
        follow: query.follow,
    };
    let cursor = request.cursor.take();
    let params = serde_json::to_value(request).map_err(RebornServicesError::internal_from)?;
    let page = state
        .services()
        .query(
            caller,
            RebornViewQuery {
                view_id: LOGS_VIEW.id.to_string(),
                params,
                cursor,
            },
        )
        .await?;
    let response =
        serde_json::from_value(page.payload).map_err(RebornServicesError::internal_from)?;
    Ok(Json(response))
}

/// `POST /api/webchat/v2/operator/service`
pub async fn run_operator_service_lifecycle(
    State(state): State<WebUiV2State>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
    Extension(capabilities): Extension<WebUiV2Capabilities>,
    Json(body): Json<RebornOperatorServiceLifecycleRequest>,
) -> Result<Json<RebornOperatorCommandPlaneResponse>, WebUiV2HttpError> {
    require_operator_webui_config(capabilities)?;
    let response = OPERATOR_SERVICE_LIFECYCLE_OPERATION
        .execute_on(state.services().as_ref(), caller, body)
        .await?;
    Ok(Json(response))
}

/// Path param carrying the LLM provider id.
#[derive(Debug, Deserialize)]
pub struct LlmProviderPath {
    pub provider_id: String,
}

/// `GET /api/webchat/v2/llm/providers`
pub async fn get_llm_config(
    State(state): State<WebUiV2State>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
    Extension(capabilities): Extension<WebUiV2Capabilities>,
) -> Result<Json<LlmConfigSnapshot>, WebUiV2HttpError> {
    require_operator_webui_config(capabilities)?;
    let response = query_llm_config_snapshot(state.services(), caller).await?;
    Ok(Json(response))
}

async fn query_llm_config_snapshot(
    services: &std::sync::Arc<dyn ProductSurface>,
    caller: WebUiAuthenticatedCaller,
) -> Result<LlmConfigSnapshot, RebornServicesError> {
    let page = services
        .query(
            caller,
            RebornViewQuery {
                view_id: LLM_CONFIG_VIEW.id.to_string(),
                params: serde_json::json!({}),
                cursor: None,
            },
        )
        .await?;
    serde_json::from_value(page.payload).map_err(RebornServicesError::internal_from)
}

/// `POST /api/webchat/v2/llm/providers`
pub async fn upsert_llm_provider(
    State(state): State<WebUiV2State>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
    Extension(capabilities): Extension<WebUiV2Capabilities>,
    Json(mut body): Json<UpsertLlmProviderRequest>,
) -> Result<Json<LlmConfigSnapshot>, WebUiV2HttpError> {
    require_operator_webui_config(capabilities)?;
    let client_action_id = parse_webui_client_action_id(body.client_action_id.take())
        .map_err(RebornServicesError::from)?;
    let activity_id = llm_provider_upsert_activity_id(&caller, &client_action_id)?;
    let resolution = state
        .services()
        .invoke(
            caller.clone(),
            LLM_PROVIDER_UPSERT_CAPABILITY.capability_id()?,
            ProductCapabilityInput::llm_provider_upsert(body),
            activity_id,
        )
        .await?;
    capability_resolution_succeeded(
        resolution,
        "llm config",
        true,
        extension_lifecycle_forbidden,
        extension_lifecycle_unavailable,
    )?;
    let response = query_llm_config_snapshot(state.services(), caller).await?;
    Ok(Json(response))
}

/// `POST /api/webchat/v2/llm/providers/{provider_id}/delete`
pub async fn delete_llm_provider(
    State(state): State<WebUiV2State>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
    Extension(capabilities): Extension<WebUiV2Capabilities>,
    Path(LlmProviderPath { provider_id }): Path<LlmProviderPath>,
) -> Result<Json<LlmConfigSnapshot>, WebUiV2HttpError> {
    require_operator_webui_config(capabilities)?;
    let resolution = invoke_product_capability(
        state.services(),
        caller.clone(),
        LLM_PROVIDER_DELETE_CAPABILITY,
        serde_json::json!({ "provider_id": provider_id }),
    )
    .await?;
    capability_resolution_succeeded(
        resolution,
        "llm config",
        true,
        extension_lifecycle_forbidden,
        extension_lifecycle_unavailable,
    )?;
    let response = query_llm_config_snapshot(state.services(), caller).await?;
    Ok(Json(response))
}

/// `POST /api/webchat/v2/llm/active`
pub async fn set_active_llm(
    State(state): State<WebUiV2State>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
    Extension(capabilities): Extension<WebUiV2Capabilities>,
    Json(body): Json<SetActiveLlmRequest>,
) -> Result<Json<LlmConfigSnapshot>, WebUiV2HttpError> {
    require_operator_webui_config(capabilities)?;
    let resolution = invoke_product_capability(
        state.services(),
        caller.clone(),
        LLM_ACTIVE_SET_CAPABILITY,
        body,
    )
    .await?;
    capability_resolution_succeeded(
        resolution,
        "llm config",
        true,
        extension_lifecycle_forbidden,
        extension_lifecycle_unavailable,
    )?;
    let response = query_llm_config_snapshot(state.services(), caller).await?;
    Ok(Json(response))
}

/// `POST /api/webchat/v2/llm/test-connection`
pub async fn test_llm_connection(
    State(state): State<WebUiV2State>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
    Extension(capabilities): Extension<WebUiV2Capabilities>,
    Json(body): Json<serde_json::Value>,
) -> Result<Json<LlmProbeResult>, WebUiV2HttpError> {
    require_operator_webui_config(capabilities)?;
    let response = LLM_TEST_CONNECTION_OPERATION
        .execute_on(state.services().as_ref(), caller, body)
        .await?;
    Ok(Json(response))
}

/// `POST /api/webchat/v2/llm/list-models`
pub async fn list_llm_models(
    State(state): State<WebUiV2State>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
    Extension(capabilities): Extension<WebUiV2Capabilities>,
    Json(body): Json<serde_json::Value>,
) -> Result<Json<LlmModelsResult>, WebUiV2HttpError> {
    require_operator_webui_config(capabilities)?;
    let response = LLM_LIST_MODELS_OPERATION
        .execute_on(state.services().as_ref(), caller, body)
        .await?;
    Ok(Json(response))
}

/// `POST /api/webchat/v2/llm/nearai/login`
pub async fn start_nearai_login(
    State(state): State<WebUiV2State>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
    Extension(capabilities): Extension<WebUiV2Capabilities>,
    headers: HeaderMap,
    Json(mut body): Json<serde_json::Value>,
) -> Result<Json<NearAiLoginStart>, WebUiV2HttpError> {
    require_operator_webui_config(capabilities)?;
    // The NEAR AI callback carries the login token in its redirect, so the
    // callback origin must come from trusted request context, not arbitrary
    // body input. This route's descriptor is `CorsPolicy::SameOriginOnly`, so a
    // present `Origin` header has been gateway-validated as same-origin; prefer
    // it over the body field (which stays as a fallback for non-browser callers).
    if let Some(origin) = headers
        .get(axum::http::header::ORIGIN)
        .and_then(|value| value.to_str().ok())
        .filter(|value| !value.is_empty())
    {
        body.as_object_mut()
            .ok_or_else(|| {
                RebornServicesError::from(WebUiInboundValidationError::new(
                    "body",
                    WebUiInboundValidationCode::InvalidValue,
                ))
            })?
            .insert(
                "origin".to_string(),
                serde_json::Value::String(origin.to_string()),
            );
    }
    let response = LLM_NEARAI_LOGIN_OPERATION
        .execute_on(state.services().as_ref(), caller, body)
        .await?;
    Ok(Json(response))
}

/// `POST /api/webchat/v2/llm/nearai/wallet`
///
/// Completes a NEAR AI wallet (NEP-413) login from a browser-signed message:
/// relays the signature to NEAR AI, stores the session token, and makes NEAR AI
/// active.
pub async fn complete_nearai_wallet_login(
    State(state): State<WebUiV2State>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
    Extension(capabilities): Extension<WebUiV2Capabilities>,
    Json(body): Json<serde_json::Value>,
) -> Result<Json<NearAiWalletLoginResult>, WebUiV2HttpError> {
    require_operator_webui_config(capabilities)?;
    let response = LLM_NEARAI_WALLET_LOGIN_OPERATION
        .execute_on(state.services().as_ref(), caller, body)
        .await?;
    Ok(Json(response))
}

/// `POST /api/webchat/v2/llm/codex/login`
///
/// Begins an OpenAI Codex device-code login. Takes no body — returns the user
/// code + verification URL to display; a background task completes the flow.
pub async fn start_codex_login(
    State(state): State<WebUiV2State>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
    Extension(capabilities): Extension<WebUiV2Capabilities>,
) -> Result<Json<CodexLoginStart>, WebUiV2HttpError> {
    require_operator_webui_config(capabilities)?;
    let response = LLM_CODEX_LOGIN_OPERATION
        .execute_on(state.services().as_ref(), caller, serde_json::json!({}))
        .await?;
    Ok(Json(response))
}

#[derive(Debug, Deserialize)]
pub struct ExtensionPackagePath {
    pub package_id: String,
}

#[derive(Debug, Deserialize)]
pub struct InstallExtensionBody {
    pub package_ref: LifecyclePackageRef,
    #[serde(default)]
    pub client_action_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ExtensionLifecycleActionBody {
    #[serde(default)]
    pub client_action_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct SkillPath {
    pub name: String,
}

#[derive(Debug, Deserialize)]
pub struct SearchSkillsBody {
    pub query: String,
}

#[derive(Debug, Deserialize)]
pub struct InstallSkillBody {
    pub name: String,
    pub content: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateSkillBody {
    pub content: String,
}

#[derive(Debug, Deserialize)]
pub struct SetSkillAutoActivateBody {
    pub enabled: bool,
}

fn extension_package_ref_for_request(
    package_ref: Result<LifecyclePackageRef, ProductWorkflowError>,
    field: &'static str,
) -> Result<LifecyclePackageRef, RebornServicesError> {
    package_ref
        .and_then(LifecyclePackageRef::require_extension)
        .map_err(|_| {
            RebornServicesError::from(WebUiInboundValidationError::new(
                field,
                WebUiInboundValidationCode::InvalidId,
            ))
        })
}

/// `GET /api/webchat/v2/threads/{thread_id}/ws`
///
/// WebSocket transport variant of [`stream_events`]. The handler
/// accepts the WS upgrade, drains the same `ProductSurface::
/// stream_events` facade as the SSE handler, and writes each event as
/// a JSON text frame. Same lifetime + per-caller concurrency caps as
/// SSE.
///
/// Same-origin enforcement is the responsibility of host composition's
/// origin-check middleware — the descriptor declares
/// `WebSocketOriginPolicy::SameOriginRequired` so a future override
/// to a host-allowlist is one descriptor change away. This handler
/// trusts the composition layer to have already rejected
/// disallowed-origin upgrades.
pub async fn stream_events_ws(
    State(state): State<WebUiV2State>,
    Extension(caller): Extension<WebUiAuthenticatedCaller>,
    Path(thread_id): Path<String>,
    headers: HeaderMap,
    Query(query): Query<StreamEventsQuery>,
    upgrade: axum::extract::ws::WebSocketUpgrade,
) -> Result<axum::response::Response, WebUiV2HttpError> {
    let slot = state
        .sse_capacity()
        .try_acquire(
            &caller.tenant_id,
            &caller.user_id,
            stream_connection_id(query.connection_id.as_deref()),
        )
        .ok_or_else(sse_concurrency_exhausted)?;
    let services = state.services().clone();
    let initial_cursor = headers
        .get(LAST_EVENT_ID_HEADER)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string)
        .or(query.after_cursor);
    Ok(upgrade.on_upgrade(move |socket| {
        ws_drain_loop(services, caller, thread_id, initial_cursor, slot, socket)
    }))
}

async fn ws_drain_loop(
    services: std::sync::Arc<dyn ProductSurface>,
    caller: WebUiAuthenticatedCaller,
    thread_id: String,
    initial_cursor: Option<String>,
    slot: SseSlot,
    mut socket: axum::extract::ws::WebSocket,
) {
    // Mirror the SSE generator: own the slot guard, bound stream
    // lifetime, drain stream_events with the same idle cadence, emit
    // each envelope as a JSON text frame.
    //
    // Two failure modes the loop must observe:
    //
    // 1. **Peer close / socket error.** The browser may close an
    //    idle tab without trading a close frame; the OS surfaces
    //    that as either a `Close` message or a socket-error on the
    //    next read. The loop watches `socket.recv()` on every
    //    `.await` so a dropped tab exits immediately instead of
    //    pinning the per-caller `SseSlot` for up to `SSE_MAX_LIFETIME`.
    // 2. **TCP backpressure on send.** A slow / hostile reader can
    //    leave bytes queued indefinitely. Each `socket.send().await`
    //    runs under `ws_send_with_timeout` so the per-caller slot
    //    is released within the lifetime budget regardless.
    let mut slot_guard = slot;
    let started_at = tokio::time::Instant::now();
    let mut after_cursor = initial_cursor.and_then(parse_cursor_token);
    if services.supports_stream_events_subscription() {
        let remaining = SSE_MAX_LIFETIME.saturating_sub(started_at.elapsed());
        if remaining.is_zero() {
            let _ =
                ws_send_with_timeout(&mut socket, None, std::time::Duration::from_millis(0)).await;
            return;
        }
        let request = RebornStreamEventsRequest {
            thread_id: thread_id.clone(),
            after_cursor: after_cursor.clone(),
        };
        let subscription_result = tokio::select! {
            biased;
            _ = slot_guard.cancelled() => {
                let _ = socket.close().await;
                return;
            }
            result = tokio::time::timeout(
                remaining,
                services.subscribe_events(caller, request),
            ) => result,
        };
        let mut subscription = match subscription_result {
            Err(_elapsed) => {
                let _ = socket.close().await;
                return;
            }
            Ok(Ok(subscription)) => subscription,
            Ok(Err(error)) => {
                tracing::debug!(
                    target = "ironclaw_webui_v2::ws",
                    error = ?error,
                    "facade rejected WS subscription; closing stream",
                );
                let payload = SseErrorPayload {
                    error: error.code,
                    kind: error.kind,
                    retryable: error.retryable,
                };
                if let Ok(text) = serde_json::to_string(&payload) {
                    let send_budget = SSE_MAX_LIFETIME.saturating_sub(started_at.elapsed());
                    let _ = ws_send_with_timeout(
                        &mut socket,
                        Some(axum::extract::ws::Message::Text(text.into())),
                        send_budget,
                    )
                    .await;
                }
                let _ = socket.close().await;
                return;
            }
        };
        let send_budget = SSE_MAX_LIFETIME.saturating_sub(started_at.elapsed());
        if ws_send_with_timeout(
            &mut socket,
            Some(axum::extract::ws::Message::Text(
                STREAM_READY_PAYLOAD.into(),
            )),
            send_budget,
        )
        .await
        .is_err()
        {
            return;
        }
        loop {
            let remaining = SSE_MAX_LIFETIME.saturating_sub(started_at.elapsed());
            if remaining.is_zero() {
                let _ = socket.close().await;
                return;
            }
            let outcome = tokio::select! {
                biased;
                _ = slot_guard.cancelled() => {
                    let _ = socket.close().await;
                    return;
                }
                incoming = socket.recv() => {
                    match incoming {
                        None | Some(Err(_)) => return,
                        Some(Ok(axum::extract::ws::Message::Close(_))) => return,
                        Some(Ok(_)) => continue,
                    }
                }
                next = tokio::time::timeout(remaining, subscription.next()) => next,
            };
            match outcome {
                Err(_elapsed) => {
                    let _ = socket.close().await;
                    return;
                }
                Ok(Some(Ok(envelope))) => match serde_json::to_string(&envelope) {
                    Ok(text) => {
                        let send_budget = SSE_MAX_LIFETIME.saturating_sub(started_at.elapsed());
                        if send_budget.is_zero() {
                            let _ = socket.close().await;
                            return;
                        }
                        if ws_send_with_timeout(
                            &mut socket,
                            Some(axum::extract::ws::Message::Text(text.into())),
                            send_budget,
                        )
                        .await
                        .is_err()
                        {
                            return;
                        }
                    }
                    Err(error) => {
                        tracing::debug!(
                            target = "ironclaw_webui_v2::ws",
                            error = %error,
                            "failed to serialize ProductOutboundEnvelope for WS",
                        );
                    }
                },
                Ok(Some(Err(error))) => {
                    tracing::debug!(
                        target = "ironclaw_webui_v2::ws",
                        error = ?error,
                        "facade rejected WS subscription event; closing stream",
                    );
                    let payload = SseErrorPayload {
                        error: error.code,
                        kind: error.kind,
                        retryable: error.retryable,
                    };
                    if let Ok(text) = serde_json::to_string(&payload) {
                        let send_budget = SSE_MAX_LIFETIME.saturating_sub(started_at.elapsed());
                        let _ = ws_send_with_timeout(
                            &mut socket,
                            Some(axum::extract::ws::Message::Text(text.into())),
                            send_budget,
                        )
                        .await;
                    }
                    let _ = socket.close().await;
                    return;
                }
                Ok(None) => {
                    let _ = socket.close().await;
                    return;
                }
            }
        }
    }

    let mut idle_polls = 0_u32;
    loop {
        let remaining = SSE_MAX_LIFETIME.saturating_sub(started_at.elapsed());
        if remaining.is_zero() {
            let _ =
                ws_send_with_timeout(&mut socket, None, std::time::Duration::from_millis(0)).await;
            return;
        }
        let request = RebornStreamEventsRequest {
            thread_id: thread_id.clone(),
            after_cursor: after_cursor.clone(),
        };
        let facade_call = services.stream_events(caller.clone(), request);
        let outcome = tokio::select! {
            biased;
            _ = slot_guard.cancelled() => {
                let _ = socket.close().await;
                return;
            }
            // Peer close / socket error wins over the facade poll —
            // if the browser already dropped the connection we want
            // to free the slot immediately, not wait for stream_events
            // to return.
            incoming = socket.recv() => {
                match incoming {
                    None | Some(Err(_)) => return,
                    Some(Ok(axum::extract::ws::Message::Close(_))) => return,
                    // Ignore other inbound frames (Ping/Pong are
                    // handled internally by axum; Text/Binary from
                    // the browser are not part of this contract).
                    Some(Ok(_)) => continue,
                }
            }
            facade = tokio::time::timeout(remaining, facade_call) => facade,
        };
        match outcome {
            Err(_elapsed) => {
                let _ = socket.close().await;
                return;
            }
            Ok(Ok(response)) => {
                let had_events = !response.events.is_empty();
                if let Some(latest) = response.events.last() {
                    after_cursor = Some(latest.projection_cursor.clone());
                }
                for envelope in response.events {
                    match serde_json::to_string(&envelope) {
                        Ok(text) => {
                            let send_budget = SSE_MAX_LIFETIME.saturating_sub(started_at.elapsed());
                            if send_budget.is_zero() {
                                let _ = socket.close().await;
                                return;
                            }
                            if ws_send_with_timeout(
                                &mut socket,
                                Some(axum::extract::ws::Message::Text(text.into())),
                                send_budget,
                            )
                            .await
                            .is_err()
                            {
                                // Peer hung up, TCP backpressure
                                // exceeded budget, or socket otherwise
                                // unwritable. Drop the task and
                                // release the slot.
                                return;
                            }
                        }
                        Err(error) => {
                            tracing::debug!(
                                target = "ironclaw_webui_v2::ws",
                                error = %error,
                                "failed to serialize ProductOutboundEnvelope for WS",
                            );
                        }
                    }
                }
                if had_events {
                    // Match SSE semantics: do not sleep after a delivered
                    // batch, because the production facade waits on the live
                    // projection subscription for the next item.
                    idle_polls = 0;
                    continue;
                }
                idle_polls = idle_polls.saturating_add(1);
                let sleep_for = sse_poll_interval_for_idle_polls(idle_polls)
                    .min(SSE_MAX_LIFETIME.saturating_sub(started_at.elapsed()));
                if sleep_for.is_zero() {
                    let _ = socket.close().await;
                    return;
                }
                // Race the poll-interval sleep against socket close
                // for the same reason as the facade call above: if
                // the peer drops during the idle window, free the
                // slot immediately.
                tokio::select! {
                    biased;
                    _ = slot_guard.cancelled() => {
                        let _ = socket.close().await;
                        return;
                    }
                    incoming = socket.recv() => match incoming {
                        None | Some(Err(_)) => return,
                        Some(Ok(axum::extract::ws::Message::Close(_))) => return,
                        Some(Ok(_)) => {}
                    },
                    _ = tokio::time::sleep(sleep_for) => {}
                }
            }
            Ok(Err(error)) => {
                tracing::debug!(
                    target = "ironclaw_webui_v2::ws",
                    error = ?error,
                    "facade rejected WS drain; closing stream",
                );
                let payload = SseErrorPayload {
                    error: error.code,
                    kind: error.kind,
                    retryable: error.retryable,
                };
                if let Ok(text) = serde_json::to_string(&payload) {
                    let send_budget = SSE_MAX_LIFETIME.saturating_sub(started_at.elapsed());
                    let _ = ws_send_with_timeout(
                        &mut socket,
                        Some(axum::extract::ws::Message::Text(text.into())),
                        send_budget,
                    )
                    .await;
                }
                let _ = socket.close().await;
                return;
            }
        }
    }
}

/// Send a WS frame (or close, when `frame` is `None`) bounded by
/// `budget`. Returns `Err(())` on timeout, peer hangup, or close
/// error so callers can release the per-caller `SseSlot` instead of
/// hanging indefinitely on a stalled socket.
async fn ws_send_with_timeout(
    socket: &mut axum::extract::ws::WebSocket,
    frame: Option<axum::extract::ws::Message>,
    budget: std::time::Duration,
) -> Result<(), ()> {
    if budget.is_zero() {
        let _ = socket.close().await;
        return Err(());
    }
    let send_future = async {
        match frame {
            Some(message) => socket.send(message).await.map_err(|_| ()),
            None => socket.close().await.map_err(|_| ()),
        }
    };
    match tokio::time::timeout(budget, send_future).await {
        Ok(result) => result,
        Err(_elapsed) => {
            tracing::debug!(
                target = "ironclaw_webui_v2::ws",
                budget_ms = budget.as_millis() as u64,
                "WS send exceeded lifetime budget; releasing slot",
            );
            Err(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extension_lifecycle_activity_id_uses_validated_client_action_id() {
        let caller = test_caller();
        let package_ref =
            LifecyclePackageRef::new(LifecyclePackageKind::Extension, "web-access".to_string())
                .expect("valid package ref");
        let first = parse_webui_client_action_id(Some("setup-action-a".to_string()))
            .expect("valid action id");
        let same = parse_webui_client_action_id(Some("setup-action-a".to_string()))
            .expect("valid action id");
        let second = parse_webui_client_action_id(Some("setup-action-b".to_string()))
            .expect("valid action id");

        let first_activity = extension_lifecycle_activity_id(
            &caller,
            EXTENSION_SETUP_SUBMIT_CAPABILITY,
            &package_ref,
            &first,
        )
        .expect("activity id");
        let same_activity = extension_lifecycle_activity_id(
            &caller,
            EXTENSION_SETUP_SUBMIT_CAPABILITY,
            &package_ref,
            &same,
        )
        .expect("activity id");
        let second_activity = extension_lifecycle_activity_id(
            &caller,
            EXTENSION_SETUP_SUBMIT_CAPABILITY,
            &package_ref,
            &second,
        )
        .expect("activity id");

        assert_eq!(first_activity, same_activity);
        assert_ne!(first_activity, second_activity);
    }

    #[test]
    fn llm_provider_upsert_activity_id_uses_opaque_client_action_id() {
        let caller = test_caller();
        let first = parse_webui_client_action_id(Some("provider-save-a".to_string()))
            .expect("valid action id");
        let same = parse_webui_client_action_id(Some("provider-save-a".to_string()))
            .expect("valid action id");
        let second = parse_webui_client_action_id(Some("provider-save-b".to_string()))
            .expect("valid action id");

        let first_activity = llm_provider_upsert_activity_id(&caller, &first).expect("activity id");
        let same_activity = llm_provider_upsert_activity_id(&caller, &same).expect("activity id");
        let second_activity =
            llm_provider_upsert_activity_id(&caller, &second).expect("activity id");

        assert_eq!(first_activity, same_activity);
        assert_ne!(first_activity, second_activity);
    }

    #[test]
    fn outbound_preferences_activity_id_uses_opaque_client_action_id() {
        let caller = test_caller();
        let first = parse_webui_client_action_id(Some("outbound-save-a".to_string()))
            .expect("valid action id");
        let same = parse_webui_client_action_id(Some("outbound-save-a".to_string()))
            .expect("valid action id");
        let second = parse_webui_client_action_id(Some("outbound-save-b".to_string()))
            .expect("valid action id");

        let first_activity =
            outbound_preferences_activity_id(&caller, &first).expect("activity id");
        let same_activity = outbound_preferences_activity_id(&caller, &same).expect("activity id");
        let second_activity =
            outbound_preferences_activity_id(&caller, &second).expect("activity id");

        assert_eq!(first_activity, same_activity);
        assert_ne!(first_activity, second_activity);
    }

    #[test]
    fn sse_poll_interval_backs_off_only_after_repeated_idle_drains() {
        assert_eq!(sse_poll_interval_for_idle_polls(0), SSE_POLL_INTERVAL);
        assert_eq!(sse_poll_interval_for_idle_polls(1), SSE_POLL_INTERVAL);
        assert_eq!(sse_poll_interval_for_idle_polls(2), Duration::from_secs(2));
        assert_eq!(
            sse_poll_interval_for_idle_polls(3),
            SSE_IDLE_POLL_MAX_INTERVAL
        );
        assert_eq!(
            sse_poll_interval_for_idle_polls(u32::MAX),
            SSE_IDLE_POLL_MAX_INTERVAL
        );
    }

    #[test]
    fn sanitized_filename_neutralizes_header_injection() {
        // Quote + CRLF injection attempts collapse to underscores so nothing can
        // break out of the quoted `Content-Disposition` value or inject a header.
        assert_eq!(
            sanitized_download_filename(Some("a\"; rm -rf /.txt")),
            "a__ rm -rf _.txt"
        );
        assert_eq!(
            sanitized_download_filename(Some("evil\r\nSet-Cookie: x.csv")),
            "evil__Set-Cookie_ x.csv"
        );
        // Path separators never survive — a download can't address another dir.
        // (Leading dots are also trimmed, so a `../` prefix can't linger.)
        assert_eq!(
            sanitized_download_filename(Some("../../etc/passwd")),
            "_.._etc_passwd"
        );
    }

    #[test]
    fn sanitized_filename_falls_back_to_neutral_name() {
        assert_eq!(sanitized_download_filename(None), "download");
        // A dots/spaces-only name trims to empty and falls back to the neutral
        // name (illegal non-space chars instead map to `_` and survive).
        assert_eq!(sanitized_download_filename(Some("   ...  ")), "download");
        // A normal name is preserved verbatim.
        assert_eq!(
            sanitized_download_filename(Some("report.csv")),
            "report.csv"
        );
    }

    #[test]
    fn sanitized_filename_is_length_capped() {
        let long = format!("{}.csv", "a".repeat(500));
        let out = sanitized_download_filename(Some(&long));
        assert!(
            out.len() <= MAX_DOWNLOAD_FILENAME_BYTES,
            "filename must be capped, got {} bytes",
            out.len()
        );
    }

    #[test]
    fn require_project_fs_path_rejects_missing_or_blank() {
        assert!(require_project_fs_path(None).is_err());
        assert!(require_project_fs_path(Some(String::new())).is_err());
        assert!(require_project_fs_path(Some("   ".to_string())).is_err());
    }

    #[test]
    fn require_project_fs_path_accepts_non_blank() {
        assert_eq!(
            require_project_fs_path(Some("/workspace/report.csv".to_string()))
                .expect("non-blank path is accepted"),
            "/workspace/report.csv"
        );
    }

    #[test]
    fn project_fs_list_path_defaults_root_for_missing_or_blank() {
        // Absent, empty, and whitespace-only all mean "list the workspace root"
        // rather than forwarding a bogus path the facade would reject.
        assert_eq!(project_fs_list_path(None), PROJECT_FS_ROOT);
        assert_eq!(project_fs_list_path(Some(String::new())), PROJECT_FS_ROOT);
        assert_eq!(
            project_fs_list_path(Some("   ".to_string())),
            PROJECT_FS_ROOT
        );
    }

    #[test]
    fn project_fs_list_path_preserves_explicit_path() {
        assert_eq!(
            project_fs_list_path(Some("/workspace/sub".to_string())),
            "/workspace/sub"
        );
    }

    fn test_caller() -> WebUiAuthenticatedCaller {
        WebUiAuthenticatedCaller::new(
            ironclaw_host_api::TenantId::new("tenant-webui-test").expect("valid tenant"),
            ironclaw_host_api::UserId::new("user-webui-test").expect("valid user"),
            Some(ironclaw_host_api::AgentId::new("agent-webui-test").expect("valid agent")),
            Some(ironclaw_host_api::ProjectId::new("project-webui-test").expect("valid project")),
        )
    }
}
